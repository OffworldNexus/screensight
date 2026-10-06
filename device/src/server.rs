//! Home Assistant WebSocket server.
//!
//! One endpoint, `/ws`, carries both the pairing handshake and the steady-state
//! link. The Noise pattern is negotiated up front through the
//! `Sec-WebSocket-Protocol` subprotocol (`screensight.noise.xx` for first
//! contact, `screensight.noise.ik` for a remembered key); an upgrade with
//! neither token is rejected before the socket is handed over.
//!
//! After the upgrade every frame is a WebSocket **binary** frame holding exactly
//! one Noise message. The handshake runs first, under a hard timeout so a
//! stalled peer cannot hold device resources; only then does the encrypted
//! application loop begin. A connection is authenticated when the initiator's
//! static key matches a paired instance.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::SinkExt;
use futures_util::StreamExt;

use crate::noise::{self, Handshake, Mode, NoiseSession, HANDSHAKE_TIMEOUT};
use crate::pairing::WINDOW;
use crate::protocol::{ClientMessage, PairErrorReason, PairRejection, ServerMessage};
use crate::runtime::Runtime;

/// How often a pairing request polls for the on-panel confirmation.
const CONFIRM_POLL: Duration = Duration::from_millis(200);

/// Small allowance beyond the window for the user to finish confirming.
const CONFIRM_GRACE: Duration = Duration::from_secs(5);

/// Build the router (also used by integration tests).
pub fn router(runtime: Arc<Runtime>) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(|| async { "ok" }))
        .with_state(runtime)
}

/// Bind `0.0.0.0:<port>` for the WebSocket server.
pub async fn bind(port: u16) -> Result<tokio::net::TcpListener> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding websocket server on {addr}"))
}

/// Bind `0.0.0.0:<port>` and serve until the process exits.
pub async fn serve(runtime: Arc<Runtime>, port: u16) -> Result<()> {
    let listener = bind(port).await?;
    serve_listener(runtime, listener).await
}

/// Serve on an already-bound listener (used by tests, which bind port 0).
pub async fn serve_listener(
    runtime: Arc<Runtime>,
    listener: tokio::net::TcpListener,
) -> Result<()> {
    log::info!(
        "websocket server listening on {}",
        listener.local_addr().context("reading listener address")?
    );
    axum::serve(
        listener,
        router(runtime).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("websocket server terminated")
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(runtime): State<Arc<Runtime>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    // The Noise pattern is negotiated entirely by subprotocol: Home Assistant
    // asks for exactly one, and we reject anything we do not speak before the
    // upgrade. axum's `.protocols()` only advertises support, so the request is
    // inspected and matched here.
    let mode = ws
        .requested_protocols()
        .filter_map(|value| value.to_str().ok())
        .find_map(Mode::from_subprotocol);
    let Some(mode) = mode else {
        log::debug!("rejecting websocket without a known Noise subprotocol");
        return (
            StatusCode::BAD_REQUEST,
            "unsupported Sec-WebSocket-Protocol",
        )
            .into_response();
    };

    let mut ws = ws;
    ws.set_selected_protocol(HeaderValue::from_static(mode.subprotocol()));
    ws.on_upgrade(move |socket| handle_socket(socket, runtime, peer, mode))
}

/// A handshake that completed, with the transport and the peer's static key.
struct Established {
    noise: NoiseSession,
    /// The peer's static public key, hex-encoded.
    peer_key_hex: String,
}

async fn handle_socket(socket: WebSocket, runtime: Arc<Runtime>, peer: SocketAddr, mode: Mode) {
    let (mut sender, mut receiver) = socket.split();

    let Some(Established {
        mut noise,
        peer_key_hex,
    }) = handshake(&mut sender, &mut receiver, &runtime, peer, mode).await
    else {
        return;
    };

    // Home Assistant authenticates the device with the handshake. The device
    // authenticates Home Assistant by its static key. An IK reconnect from an
    // unknown key is told why (inside the encrypted channel) and then dropped
    // before any application frame is processed, so Home Assistant can raise a
    // repair flow instead of retrying forever.
    let mut instance = runtime.authenticate_by_static_key(&peer_key_hex);
    if mode == Mode::Ik && instance.is_none() {
        log::warn!("rejecting IK reconnect from unknown static key");
        let _ = send_message(
            &mut sender,
            &mut noise,
            &ServerMessage::Error {
                message: "unknown static key: pair this device again".to_owned(),
            },
        )
        .await;
        return;
    }
    if let Some(instance) = &instance {
        runtime.set_connected(&instance.ha_id, true);
        log::info!("{} connected ({})", instance.ha_name, instance.ha_id);
        let state = current_state(&runtime, &instance.ha_id);
        let _ = send_message(&mut sender, &mut noise, &state).await;
    }

    while let Some(frame) = receiver.next().await {
        let Ok(frame) = frame else { break };
        let ciphertext = match frame {
            Message::Binary(bytes) => bytes,
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => continue,
            // Application frames are always binary; a text frame is a protocol
            // violation and ends the connection.
            Message::Text(_) => break,
        };
        let plaintext = match noise.decrypt(&ciphertext) {
            Ok(plaintext) => plaintext,
            Err(err) => {
                log::debug!("dropping connection after a bad transport frame: {err:#}");
                break;
            }
        };
        let message = match serde_json::from_slice::<ClientMessage>(&plaintext) {
            Ok(message) => message,
            Err(err) => {
                let _ = send_message(
                    &mut sender,
                    &mut noise,
                    &ServerMessage::Error {
                        message: format!("malformed frame: {err}"),
                    },
                )
                .await;
                continue;
            }
        };

        match message {
            ClientMessage::Ping => {
                let _ = send_message(&mut sender, &mut noise, &ServerMessage::Pong).await;
            }
            ClientMessage::Pair { ha_id, ha_name } => {
                if mode != Mode::Xx || instance.is_some() {
                    let _ = send_message(
                        &mut sender,
                        &mut noise,
                        &ServerMessage::Error {
                            message: "pairing is not available on this connection".to_owned(),
                        },
                    )
                    .await;
                    continue;
                }
                if !runtime.pairing_open(Instant::now()) {
                    let _ = send_message(
                        &mut sender,
                        &mut noise,
                        &ServerMessage::PairError {
                            reason: PairErrorReason::WindowClosed,
                            retry_after_secs: None,
                        },
                    )
                    .await;
                    continue;
                }
                let identity = runtime.identity();
                runtime.accept_pair_request(&ha_id, &ha_name);
                let _ = send_message(
                    &mut sender,
                    &mut noise,
                    &ServerMessage::PairPending {
                        device_id: identity.id.clone(),
                        name: identity.name.clone(),
                        ha_name: ha_name.clone(),
                    },
                )
                .await;

                if await_confirmation(&mut sender, &mut noise, &runtime, &ha_id).await {
                    instance = runtime.authenticate_by_static_key(&peer_key_hex);
                    if let Some(instance) = &instance {
                        runtime.set_connected(&instance.ha_id, true);
                        let state = current_state(&runtime, &instance.ha_id);
                        let _ = send_message(&mut sender, &mut noise, &state).await;
                    }
                }
            }
            ClientMessage::SetValue { key, value } => {
                if let Some(instance) = &instance {
                    runtime.set_value(&instance.ha_id, &key, &value);
                    let state = current_state(&runtime, &instance.ha_id);
                    let _ = send_message(&mut sender, &mut noise, &state).await;
                } else {
                    let _ = send_unauthenticated(&mut sender, &mut noise).await;
                }
            }
            ClientMessage::SetState { values } => {
                if let Some(instance) = &instance {
                    runtime.set_values(&instance.ha_id, values);
                    let state = current_state(&runtime, &instance.ha_id);
                    let _ = send_message(&mut sender, &mut noise, &state).await;
                } else {
                    let _ = send_unauthenticated(&mut sender, &mut noise).await;
                }
            }
        }
    }

    if let Some(instance) = &instance {
        runtime.set_connected(&instance.ha_id, false);
        log::info!("{} disconnected", instance.ha_id);
    }
}

/// Run the Noise handshake as the responder. Returns `None` (and drops the
/// connection) on any failure, timeout, or refused attempt.
async fn handshake(
    sender: &mut SplitSink<WebSocket, Message>,
    receiver: &mut SplitStream<WebSocket>,
    runtime: &Arc<Runtime>,
    peer: SocketAddr,
    mode: Mode,
) -> Option<Established> {
    let now = Instant::now();
    if mode == Mode::Xx {
        if !runtime.pairing_open(now) {
            log::debug!("refusing XX handshake: pairing window closed");
            return None;
        }
        if let Some(retry) = runtime.handshake_retry_after(peer.ip(), now) {
            log::warn!(
                "refusing XX handshake from {}: locked out for {retry}s",
                peer.ip()
            );
            return None;
        }
        runtime.begin_handshake();
    }

    let keys = runtime.keys();
    let mut state = match Handshake::responder(mode, &keys.private) {
        Ok(state) => state,
        Err(err) => {
            log::error!("could not start the Noise responder: {err:#}");
            return fail(runtime, peer, mode);
        }
    };

    // One binary WS frame per Noise message, each step bounded so a stalled
    // peer cannot hold the connection open indefinitely.
    let step = async {
        let first = recv_binary(receiver).await?;
        state.read(&first)?;
        let reply = state.write(&[])?;
        send_binary(sender, &reply).await?;
        if mode == Mode::Xx {
            let third = recv_binary(receiver).await?;
            state.read(&third)?;
        }
        if !state.is_finished() {
            bail!("handshake ended before it completed");
        }
        anyhow::Ok(())
    }
    .await;
    if let Err(err) = step {
        log::debug!(
            "Noise {mode:?} handshake with {} failed: {err:#}",
            peer.ip()
        );
        return fail(runtime, peer, mode);
    }

    let peer_static = match state.peer_static() {
        Some(key) => key,
        None => {
            log::debug!("handshake revealed no peer static key");
            return fail(runtime, peer, mode);
        }
    };
    let session = match state.into_session() {
        Ok(session) => session,
        Err(err) => {
            log::debug!("cannot enter Noise transport mode: {err:#}");
            return fail(runtime, peer, mode);
        }
    };

    let peer_key_hex = noise::to_hex(&peer_static);
    if mode == Mode::Xx {
        runtime.set_pairing_sas(&peer_key_hex, &session.sas);
    }
    Some(Established {
        noise: session,
        peer_key_hex,
    })
}

/// Record a failed pairing attempt (XX only) and return `None`.
fn fail(runtime: &Runtime, peer: SocketAddr, mode: Mode) -> Option<Established> {
    if mode == Mode::Xx {
        runtime.record_handshake_failure(peer.ip(), Instant::now());
        runtime.fail_pairing();
    }
    None
}

/// Read one handshake frame, enforcing [`HANDSHAKE_TIMEOUT`].
async fn recv_binary(receiver: &mut SplitStream<WebSocket>) -> Result<Vec<u8>> {
    loop {
        let next = tokio::time::timeout(HANDSHAKE_TIMEOUT, receiver.next())
            .await
            .map_err(|_| anyhow::anyhow!("handshake timed out"))?;
        match next {
            None => bail!("connection closed during the handshake"),
            Some(Err(err)) => return Err(err).context("reading a handshake frame"),
            Some(Ok(Message::Binary(bytes))) => return Ok(bytes.to_vec()),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(Message::Close(_))) => bail!("connection closed during the handshake"),
            Some(Ok(Message::Text(_))) => bail!("expected a binary handshake frame"),
        }
    }
}

async fn send_binary(sender: &mut SplitSink<WebSocket, Message>, bytes: &[u8]) -> Result<()> {
    sender
        .send(Message::Binary(bytes.to_vec().into()))
        .await
        .context("sending a websocket frame")
}

/// Poll until the user confirms (success), declines, or the window lapses.
async fn await_confirmation(
    sender: &mut SplitSink<WebSocket, Message>,
    noise: &mut NoiseSession,
    runtime: &Arc<Runtime>,
    ha_id: &str,
) -> bool {
    let deadline = Instant::now() + WINDOW + CONFIRM_GRACE;
    loop {
        tokio::time::sleep(CONFIRM_POLL).await;

        // Confirmed: the instance now exists, keyed by the handshake static key.
        if let Some(instance) = runtime.paired_instance(ha_id) {
            let identity = runtime.identity();
            let _ = send_message(
                sender,
                noise,
                &ServerMessage::PairSuccess {
                    device_id: identity.id,
                    name: identity.name,
                    ha_id: instance.ha_id,
                },
            )
            .await;
            return true;
        }

        // Pending cleared without pairing: declined or the window lapsed.
        if runtime.pending_pair().is_none() {
            let reason = runtime.take_rejection().unwrap_or(PairRejection::TimedOut);
            let _ = send_message(sender, noise, &ServerMessage::PairRejected { reason }).await;
            return false;
        }

        if Instant::now() > deadline {
            let _ = send_message(
                sender,
                noise,
                &ServerMessage::PairRejected {
                    reason: PairRejection::TimedOut,
                },
            )
            .await;
            return false;
        }
    }
}

/// The full `state` frame describing one instance right now.
fn current_state(runtime: &Runtime, ha_id: &str) -> ServerMessage {
    ServerMessage::State {
        values: runtime.values_for(ha_id),
        selected: runtime.status().selected.as_deref() == Some(ha_id),
    }
}

async fn send_message(
    sender: &mut SplitSink<WebSocket, Message>,
    noise: &mut NoiseSession,
    message: &ServerMessage,
) -> Result<()> {
    let json = serde_json::to_vec(message).context("encoding server message")?;
    let ciphertext = noise.encrypt(&json)?;
    sender
        .send(Message::Binary(ciphertext.into()))
        .await
        .context("sending websocket frame")
}

async fn send_unauthenticated(
    sender: &mut SplitSink<WebSocket, Message>,
    noise: &mut NoiseSession,
) -> Result<()> {
    send_message(
        sender,
        noise,
        &ServerMessage::Error {
            message: "unauthenticated: this static key is not paired".to_owned(),
        },
    )
    .await
}
