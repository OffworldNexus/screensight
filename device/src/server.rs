//! Home Assistant WebSocket server.
//!
//! One endpoint, `/ws`, carries both the pairing handshake and the steady-state
//! link. A connection is *authenticated* when its `Authorization: Bearer
//! <token>` header matches a paired instance; unauthenticated connections may
//! only send `pair` (and are still gated by the pairing window and per-address
//! rate limiting). Pairing completes only once the user confirms on the panel,
//! after which the token is returned over the same socket.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::stream::SplitSink;
use futures_util::SinkExt;
use futures_util::StreamExt;

use crate::pairing::{SubmitOutcome, WINDOW};
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
    headers: HeaderMap,
) -> impl IntoResponse {
    let token = bearer_token(&headers);
    ws.on_upgrade(move |socket| handle_socket(socket, runtime, peer, token))
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|value| value.trim().to_owned())
}

async fn handle_socket(
    socket: WebSocket,
    runtime: Arc<Runtime>,
    peer: SocketAddr,
    token: Option<String>,
) {
    let (mut sender, mut receiver) = socket.split();
    let instance = token.as_deref().and_then(|t| runtime.authenticate(t));

    if let Some(instance) = &instance {
        runtime.set_connected(&instance.ha_id, true);
        log::info!("{} connected ({})", instance.ha_name, instance.ha_id);
        let _ = send(&mut sender, &current_state(&runtime, &instance.ha_id)).await;
    }

    while let Some(frame) = receiver.next().await {
        let Ok(frame) = frame else { break };
        let text = match frame {
            Message::Text(text) => text,
            Message::Close(_) => break,
            _ => continue,
        };
        let message = match serde_json::from_str::<ClientMessage>(&text) {
            Ok(message) => message,
            Err(err) => {
                let _ = send(
                    &mut sender,
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
                let _ = send(&mut sender, &ServerMessage::Pong).await;
            }
            ClientMessage::Pair {
                device_id,
                code,
                ha_id,
                ha_name,
            } => {
                if instance.is_some() {
                    let _ = send(
                        &mut sender,
                        &ServerMessage::Error {
                            message: "already paired on this connection".to_owned(),
                        },
                    )
                    .await;
                    continue;
                }
                handle_pair(
                    &mut sender,
                    &runtime,
                    peer,
                    &device_id,
                    &code,
                    &ha_id,
                    &ha_name,
                )
                .await;
            }
            ClientMessage::SetValue { key, value } => {
                if let Some(instance) = &instance {
                    runtime.set_value(&instance.ha_id, &key, &value);
                    let _ = send(&mut sender, &current_state(&runtime, &instance.ha_id)).await;
                } else {
                    let _ = send_unauthenticated(&mut sender).await;
                }
            }
            ClientMessage::SetState { values } => {
                if let Some(instance) = &instance {
                    runtime.set_values(&instance.ha_id, values);
                    let _ = send(&mut sender, &current_state(&runtime, &instance.ha_id)).await;
                } else {
                    let _ = send_unauthenticated(&mut sender).await;
                }
            }
        }
    }

    if let Some(instance) = &instance {
        runtime.set_connected(&instance.ha_id, false);
        log::info!("{} disconnected", instance.ha_id);
    }
}

/// Run one pairing attempt, waiting for the on-panel confirmation.
async fn handle_pair(
    sender: &mut SplitSink<WebSocket, Message>,
    runtime: &Arc<Runtime>,
    peer: SocketAddr,
    device_id: &str,
    code: &str,
    ha_id: &str,
    ha_name: &str,
) {
    let identity = runtime.identity();
    if device_id != identity.id {
        let _ = send(
            sender,
            &ServerMessage::PairError {
                reason: PairErrorReason::WrongDevice,
                retry_after_secs: None,
                attempts_left: None,
            },
        )
        .await;
        return;
    }

    let outcome = runtime.submit_code(peer.ip(), code, ha_id, ha_name, Instant::now());
    match outcome {
        SubmitOutcome::Pending => {
            let _ = send(
                sender,
                &ServerMessage::PairPending {
                    device_id: identity.id.clone(),
                    name: identity.name.clone(),
                    ha_name: ha_name.to_owned(),
                },
            )
            .await;
            await_confirmation(sender, runtime, ha_id).await;
        }
        SubmitOutcome::InvalidCode { attempts_left } => {
            let _ = send(
                sender,
                &ServerMessage::PairError {
                    reason: PairErrorReason::InvalidCode,
                    retry_after_secs: None,
                    attempts_left: Some(attempts_left),
                },
            )
            .await;
        }
        SubmitOutcome::RateLimited { retry_after_secs } => {
            let _ = send(
                sender,
                &ServerMessage::PairError {
                    reason: PairErrorReason::RateLimited,
                    retry_after_secs: Some(retry_after_secs),
                    attempts_left: None,
                },
            )
            .await;
        }
        SubmitOutcome::WindowClosed => {
            let _ = send(
                sender,
                &ServerMessage::PairError {
                    reason: PairErrorReason::WindowClosed,
                    retry_after_secs: None,
                    attempts_left: None,
                },
            )
            .await;
        }
    }
}

/// Poll until the user confirms (success), declines, or the window lapses.
async fn await_confirmation(
    sender: &mut SplitSink<WebSocket, Message>,
    runtime: &Arc<Runtime>,
    ha_id: &str,
) {
    let deadline = Instant::now() + WINDOW + CONFIRM_GRACE;
    loop {
        tokio::time::sleep(CONFIRM_POLL).await;

        // Confirmed: the instance now exists with a token.
        if let Some(instance) = runtime.paired_instance(ha_id) {
            let identity = runtime.identity();
            let _ = send(
                sender,
                &ServerMessage::PairSuccess {
                    token: instance.token,
                    device_id: identity.id,
                    name: identity.name,
                    ha_id: ha_id.to_owned(),
                },
            )
            .await;
            return;
        }

        // Pending cleared without pairing: declined or timed out.
        if runtime.pending_pair().is_none() {
            let reason = runtime.take_rejection().unwrap_or(PairRejection::TimedOut);
            let _ = send(sender, &ServerMessage::PairRejected { reason }).await;
            return;
        }

        if Instant::now() > deadline {
            let _ = send(
                sender,
                &ServerMessage::PairRejected {
                    reason: PairRejection::TimedOut,
                },
            )
            .await;
            return;
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

async fn send(sender: &mut SplitSink<WebSocket, Message>, message: &ServerMessage) -> Result<()> {
    let json = serde_json::to_string(message).context("encoding server message")?;
    sender
        .send(Message::Text(json.into()))
        .await
        .context("sending websocket frame")
}

async fn send_unauthenticated(sender: &mut SplitSink<WebSocket, Message>) -> Result<()> {
    send(
        sender,
        &ServerMessage::Error {
            message: "unauthenticated: pair first or supply a token".to_owned(),
        },
    )
    .await
}
