//! End-to-end WebSocket tests: discovery-free pairing, the on-panel
//! confirmation, token auth and the `set_value` round trip.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use screensight::identity::DeviceIdentity;
use screensight::protocol::{ClientMessage, PairErrorReason, PairRejection, ServerMessage};
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

async fn connect(addr: SocketAddr, token: Option<&str>) -> Ws {
    let url = format!("ws://{addr}/ws");
    let mut request = url.as_str().into_client_request().unwrap();
    if let Some(token) = token {
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
    }
    tokio_tungstenite::connect_async(request).await.unwrap().0
}

async fn send(ws: &mut Ws, message: &ClientMessage) {
    ws.send(Message::Text(serde_json::to_string(message).unwrap()))
        .await
        .unwrap();
}

async fn recv(ws: &mut Ws) -> ServerMessage {
    match ws.next().await.unwrap().unwrap() {
        Message::Text(text) => serde_json::from_str(&text).unwrap(),
        other => panic!("unexpected frame: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_confirm_and_set_value_round_trip() {
    let rt = runtime();
    let code = rt.arm_pairing().unwrap();
    let device_id = rt.identity().id;
    let addr = spawn_server(Arc::clone(&rt)).await;

    let mut ws = connect(addr, None).await;
    send(
        &mut ws,
        &ClientMessage::Pair {
            device_id: device_id.clone(),
            code,
            ha_id: "ha1".to_owned(),
            ha_name: "My home".to_owned(),
        },
    )
    .await;
    assert!(matches!(
        recv(&mut ws).await,
        ServerMessage::PairPending { .. }
    ));

    // Simulate the user tapping "Pair" on the panel.
    let confirmer = Arc::clone(&rt);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        confirmer.confirm_pairing().unwrap();
    });

    let token = match recv(&mut ws).await {
        ServerMessage::PairSuccess { token, .. } => token,
        other => panic!("expected pair_success, got {other:?}"),
    };
    assert_eq!(token.len(), 64);

    // Steady state: authenticate with the token, read and set a value.
    let mut ws = connect(addr, Some(&token)).await;
    assert!(matches!(
        recv(&mut ws).await,
        ServerMessage::State { values, selected }
            if values.is_empty() && selected
    ));

    send(
        &mut ws,
        &ClientMessage::SetValue {
            key: "text".to_owned(),
            value: "Hello 🌧".to_owned(),
        },
    )
    .await;
    match recv(&mut ws).await {
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

    // A full-state resend replaces the whole map.
    send(
        &mut ws,
        &ClientMessage::SetState {
            values: std::collections::BTreeMap::from([
                ("text".to_owned(), "Resent".to_owned()),
                ("accent".to_owned(), "gorget".to_owned()),
            ]),
        },
    )
    .await;
    match recv(&mut ws).await {
        ServerMessage::State { values, .. } => {
            assert_eq!(values.get("text").map(String::as_str), Some("Resent"));
            assert_eq!(values.get("accent").map(String::as_str), Some("gorget"));
        }
        other => panic!("expected state, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_code_is_rejected_without_pairing() {
    let rt = runtime();
    let _ = rt.arm_pairing().unwrap();
    let device_id = rt.identity().id;
    let addr = spawn_server(Arc::clone(&rt)).await;

    let mut ws = connect(addr, None).await;
    send(
        &mut ws,
        &ClientMessage::Pair {
            device_id,
            code: "000000".to_owned(),
            ha_id: "ha1".to_owned(),
            ha_name: "My home".to_owned(),
        },
    )
    .await;
    match recv(&mut ws).await {
        ServerMessage::PairError {
            reason,
            attempts_left,
            ..
        } => {
            assert_eq!(reason, PairErrorReason::InvalidCode);
            assert!(attempts_left.is_some());
        }
        other => panic!("expected pair_error, got {other:?}"),
    }
    assert!(!rt.status().instances.iter().any(|i| i.ha_id == "ha1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declining_on_the_panel_reports_a_decline() {
    let rt = runtime();
    let code = rt.arm_pairing().unwrap();
    let device_id = rt.identity().id;
    let addr = spawn_server(Arc::clone(&rt)).await;

    let mut ws = connect(addr, None).await;
    send(
        &mut ws,
        &ClientMessage::Pair {
            device_id,
            code,
            ha_id: "ha1".to_owned(),
            ha_name: "My home".to_owned(),
        },
    )
    .await;
    assert!(matches!(
        recv(&mut ws).await,
        ServerMessage::PairPending { .. }
    ));

    // Simulate the user tapping "Cancel".
    let decliner = Arc::clone(&rt);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        decliner.reject_pairing();
    });

    match recv(&mut ws).await {
        ServerMessage::PairRejected { reason } => {
            assert_eq!(reason, PairRejection::Declined);
        }
        other => panic!("expected pair_rejected, got {other:?}"),
    }
    assert!(rt.status().instances.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unauthenticated_connections_cannot_set_values() {
    let rt = runtime();
    let addr = spawn_server(Arc::clone(&rt)).await;

    let mut ws = connect(addr, None).await;
    send(
        &mut ws,
        &ClientMessage::SetValue {
            key: "text".to_owned(),
            value: "nope".to_owned(),
        },
    )
    .await;
    match recv(&mut ws).await {
        ServerMessage::Error { message } => assert!(message.contains("unauthenticated")),
        other => panic!("expected error, got {other:?}"),
    }
    assert!(rt.values_for("ha1").is_empty());
}
