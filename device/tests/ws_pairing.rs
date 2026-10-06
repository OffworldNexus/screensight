//! End-to-end WebSocket tests: Noise XX pairing, the on-panel confirmation, the
//! 8-digit SAS, key authentication (IK) and the `set_value` round trip.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use screensight::identity::DeviceIdentity;
use screensight::noise::{self, Handshake, Mode, NoiseSession, SUBPROTOCOL_IK, SUBPROTOCOL_XX};
use screensight::protocol::{ClientMessage, ServerMessage};
use screensight::runtime::Runtime;
use screensight::store::{snapshot_with, NoopPersistence};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

fn runtime() -> Arc<Runtime> {
    let identity = DeviceIdentity::generate("Test", "0.0.0").unwrap();
    Arc::new(Runtime::new(
        snapshot_with(identity),
        Arc::new(NoopPersistence),
    ))
}

async fn spawn_server(rt: Arc<Runtime>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = screensight::server::serve_listener(rt, listener).await;
    });
    addr
}

async fn connect(addr: SocketAddr, protocol: &str) -> Ws {
    let url = format!("ws://{addr}/ws");
    let mut request = url.as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_str(protocol).unwrap(),
    );
    tokio_tungstenite::connect_async(request).await.unwrap().0
}

/// Read the next binary frame, ignoring WebSocket control frames.
async fn next_binary(ws: &mut Ws) -> Vec<u8> {
    loop {
        match ws.next().await.unwrap().unwrap() {
            Message::Binary(bytes) => return bytes.to_vec(),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("expected a binary frame, got {other:?}"),
        }
    }
}

/// Run the Noise XX handshake as Home Assistant would, returning the session.
async fn xx_handshake(ws: &mut Ws, ha_private: &[u8]) -> NoiseSession {
    let mut hs = Handshake::initiator(Mode::Xx, ha_private, None).unwrap();
    let m1 = hs.write(&[]).unwrap();
    ws.send(Message::Binary(m1)).await.unwrap();
    let m2 = next_binary(ws).await;
    hs.read(&m2).unwrap();
    let m3 = hs.write(&[]).unwrap();
    ws.send(Message::Binary(m3)).await.unwrap();
    assert!(hs.is_finished());
    hs.into_session().unwrap()
}

/// Run the Noise IK handshake with the device's known static key.
async fn ik_handshake(ws: &mut Ws, ha_private: &[u8], device_public: &[u8]) -> NoiseSession {
    let mut hs = Handshake::initiator(Mode::Ik, ha_private, Some(device_public)).unwrap();
    let m1 = hs.write(&[]).unwrap();
    ws.send(Message::Binary(m1)).await.unwrap();
    let m2 = next_binary(ws).await;
    hs.read(&m2).unwrap();
    assert!(hs.is_finished());
    hs.into_session().unwrap()
}

async fn send_client(ws: &mut Ws, noise: &mut NoiseSession, message: &ClientMessage) {
    let json = serde_json::to_vec(message).unwrap();
    let ciphertext = noise.encrypt(&json).unwrap();
    ws.send(Message::Binary(ciphertext)).await.unwrap();
}

async fn recv_server(ws: &mut Ws, noise: &mut NoiseSession) -> ServerMessage {
    let ciphertext = next_binary(ws).await;
    let plaintext = noise.decrypt(&ciphertext).unwrap();
    serde_json::from_slice(&plaintext).unwrap()
}

/// Poll the runtime until its pairing SAS is available (the device derives it
/// asynchronously, just after it reads the third XX message).
async fn wait_for_sas(rt: &Runtime) -> String {
    for _ in 0..100 {
        if let Some(sas) = rt.status().sas {
            return sas;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("device never derived a SAS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_over_noise_then_set_value_round_trip() {
    let rt = runtime();
    rt.arm_pairing().unwrap();
    let device_id = rt.identity().id;
    let ha = noise::generate_keypair().unwrap();
    let addr = spawn_server(Arc::clone(&rt)).await;

    let mut ws = connect(addr, SUBPROTOCOL_XX).await;
    let mut noise = xx_handshake(&mut ws, &ha.private).await;

    // The panel derives the same SAS Home Assistant did; it is not transmitted.
    let device_sas = wait_for_sas(&rt).await;
    assert_eq!(device_sas, noise.sas);
    assert_eq!(device_sas.len(), 8);

    // SAS matched in Home Assistant, which then names itself inside the channel.
    send_client(
        &mut ws,
        &mut noise,
        &ClientMessage::Pair {
            ha_id: "ha1".to_owned(),
            ha_name: "My home".to_owned(),
        },
    )
    .await;
    assert!(matches!(
        recv_server(&mut ws, &mut noise).await,
        ServerMessage::PairPending { device_id: got, .. } if got == device_id
    ));

    // Simulate the user tapping "Pair" on the panel.
    let confirmer = Arc::clone(&rt);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        confirmer.confirm_pairing().unwrap();
    });

    match recv_server(&mut ws, &mut noise).await {
        ServerMessage::PairSuccess { ha_id, .. } => assert_eq!(ha_id, "ha1"),
        other => panic!("expected pair_success, got {other:?}"),
    }
    // The freshly paired channel is immediately authenticated.
    assert!(matches!(
        recv_server(&mut ws, &mut noise).await,
        ServerMessage::State { values, selected } if values.is_empty() && selected
    ));

    send_client(
        &mut ws,
        &mut noise,
        &ClientMessage::SetValue {
            key: "text".to_owned(),
            value: "Hello 🌧".to_owned(),
        },
    )
    .await;
    match recv_server(&mut ws, &mut noise).await {
        ServerMessage::State { values, selected } => {
            assert_eq!(values.get("text").map(String::as_str), Some("Hello 🌧"));
            assert!(selected);
        }
        other => panic!("expected state, got {other:?}"),
    }
    assert_eq!(
        rt.values_for("ha1").get("text").map(String::as_str),
        Some("Hello 🌧")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ik_reconnect_with_a_known_key_is_authenticated() {
    let rt = runtime();
    let ha = noise::generate_keypair().unwrap();
    let ha_key_hex = noise::to_hex(&ha.public);
    rt.arm_pairing().unwrap();
    rt.begin_handshake();
    rt.set_pairing_sas(&ha_key_hex, "12345678");
    rt.accept_pair_request("ha1", "My home");
    rt.confirm_pairing().unwrap();

    let device_public = rt.keys().public;
    let addr = spawn_server(Arc::clone(&rt)).await;

    let mut ws = connect(addr, SUBPROTOCOL_IK).await;
    let mut noise = ik_handshake(&mut ws, &ha.private, &device_public).await;

    // The device authenticates by static key and pushes the current state.
    assert!(matches!(
        recv_server(&mut ws, &mut noise).await,
        ServerMessage::State { values, selected } if values.is_empty() && selected
    ));

    send_client(
        &mut ws,
        &mut noise,
        &ClientMessage::SetState {
            values: std::collections::BTreeMap::from([
                ("text".to_owned(), "Resent".to_owned()),
                ("accent".to_owned(), "gorget".to_owned()),
            ]),
        },
    )
    .await;
    match recv_server(&mut ws, &mut noise).await {
        ServerMessage::State { values, .. } => {
            assert_eq!(values.get("text").map(String::as_str), Some("Resent"));
            assert_eq!(values.get("accent").map(String::as_str), Some("gorget"));
        }
        other => panic!("expected state, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_static_key_is_rejected_before_any_application_frame() {
    let rt = runtime();
    let addr = spawn_server(Arc::clone(&rt)).await;

    let stranger = noise::generate_keypair().unwrap();
    let device_public = rt.keys().public;
    let mut ws = connect(addr, SUBPROTOCOL_IK).await;
    let mut noise = ik_handshake(&mut ws, &stranger.private, &device_public).await;

    // The device explains the rejection inside the encrypted channel, then
    // drops the socket instead of processing any application frame.
    match recv_server(&mut ws, &mut noise).await {
        ServerMessage::Error { message } => assert!(message.contains("unknown static key")),
        other => panic!("expected an error, got {other:?}"),
    }
    let next = ws.next().await;
    assert!(
        !matches!(next, Some(Ok(Message::Binary(_)))),
        "expected the socket to close, got {next:?}"
    );
    assert!(rt.status().instances.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn xx_is_refused_when_the_pairing_window_is_closed() {
    let rt = runtime();
    let ha = noise::generate_keypair().unwrap();
    let addr = spawn_server(Arc::clone(&rt)).await;

    let mut ws = connect(addr, SUBPROTOCOL_XX).await;
    let mut hs = Handshake::initiator(Mode::Xx, &ha.private, None).unwrap();
    let m1 = hs.write(&[]).unwrap();
    ws.send(Message::Binary(m1)).await.unwrap();

    let next = ws.next().await;
    assert!(
        !matches!(next, Some(Ok(Message::Binary(_)))),
        "expected the socket to close, got {next:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upgrade_without_a_noise_subprotocol_is_rejected() {
    let rt = runtime();
    let addr = spawn_server(Arc::clone(&rt)).await;
    let url = format!("ws://{addr}/ws");
    let result = tokio_tungstenite::connect_async(url).await;
    assert!(result.is_err(), "an upgrade with no subprotocol must fail");
}
