//! Wire types shared by the Home Assistant WebSocket protocol and the local
//! control socket. Keeping them in one module (and serialising with serde)
//! makes the contract the single source of truth for both sides.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A message sent by Home Assistant to the device over the WebSocket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Pairing request. `code` is the human-entered panel code; it is only
    /// accepted while the pairing window is open and never appears on mDNS.
    Pair {
        device_id: String,
        code: String,
        ha_id: String,
        ha_name: String,
    },
    /// Set one dashboard value. Other values are left untouched.
    SetValue { key: String, value: String },
    /// Replace this instance's whole dashboard state. Sent on every
    /// (re)connection so the device and Home Assistant agree on the full set,
    /// then whenever more than one value changes at once.
    SetState { values: BTreeMap<String, String> },
    /// Heartbeat.
    Ping,
}

/// A message sent by the device back to Home Assistant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// The code was correct; the device is now waiting for the on-panel
    /// confirmation.
    PairPending {
        device_id: String,
        name: String,
        ha_name: String,
    },
    /// Pairing completed and a long-lived token was minted.
    PairSuccess {
        token: String,
        device_id: String,
        name: String,
        ha_id: String,
    },
    /// The user declined on the panel, or the window closed first.
    PairRejected { reason: PairRejection },
    /// The pairing request could not be accepted.
    PairError {
        reason: PairErrorReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        retry_after_secs: Option<u64>,
        /// Attempts remaining before this source is locked out.
        #[serde(skip_serializing_if = "Option::is_none")]
        attempts_left: Option<u32>,
    },
    /// The authenticated instance's full dashboard state.
    State {
        values: BTreeMap<String, String>,
        /// Whether this instance is the one currently shown on the panel.
        selected: bool,
    },
    /// Reply to [`ClientMessage::Ping`].
    Pong,
    /// A generic error (bad auth, malformed frame, ...).
    Error { message: String },
}

/// Why a pairing request was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairErrorReason {
    InvalidCode,
    RateLimited,
    WindowClosed,
    WrongDevice,
}

/// Why a pending pairing was abandoned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairRejection {
    /// The user explicitly declined on the panel.
    Declined,
    /// The window expired before the user confirmed.
    TimedOut,
}

/// A request sent by the `screensight` CLI to the daemon control socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum ControlRequest {
    /// Report identity, pairing window and paired instances.
    Status,
    /// Open (or re-arm) the pairing window.
    Pair,
    /// Close the pairing window without pairing.
    CancelPair,
    /// Confirm the pending pairing as if the user tapped "Pair" on the panel.
    ///
    /// Intended for headless/automated testing; the panel remains the normal
    /// path.
    Confirm,
    /// Decline the pending pairing as if the user tapped "Cancel".
    Reject,
    /// Forget one instance, or all of them.
    Unpair {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default)]
        all: bool,
    },
    /// Choose which paired instance drives the display.
    Select { id: String },
}

/// The daemon's reply to a [`ControlRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusReport>,
}

impl ControlResponse {
    #[must_use]
    pub fn ok() -> Self {
        Self {
            ok: true,
            error: None,
            status: None,
        }
    }

    #[must_use]
    pub fn status(status: StatusReport) -> Self {
        Self {
            ok: true,
            error: None,
            status: Some(status),
        }
    }

    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            status: None,
        }
    }
}

/// Full device status, as reported by `screensight status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusReport {
    pub id: String,
    pub name: String,
    pub model: String,
    pub version: String,
    /// Whether the pairing window is currently open.
    pub pairing: bool,
    /// The current pairing code, if the window is open (local socket only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pairing_code: Option<String>,
    /// Paired instances.
    pub instances: Vec<PairedSummary>,
    pub selected: Option<String>,
}

/// A paired instance as shown in `screensight status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedSummary {
    pub ha_id: String,
    pub ha_name: String,
    pub selected: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_messages_round_trip() {
        let cases = [
            ClientMessage::Pair {
                device_id: "abc".into(),
                code: "123456".into(),
                ha_id: "ha1".into(),
                ha_name: "My home".into(),
            },
            ClientMessage::SetValue {
                key: "text".into(),
                value: "Hello 🌧".into(),
            },
            ClientMessage::SetState {
                values: BTreeMap::from([
                    ("text".to_owned(), "Hello".to_owned()),
                    ("accent".to_owned(), "gorget".to_owned()),
                ]),
            },
            ClientMessage::Ping,
        ];
        for msg in cases {
            let json = serde_json::to_string(&msg).unwrap();
            assert_eq!(serde_json::from_str::<ClientMessage>(&json).unwrap(), msg);
        }
    }

    #[test]
    fn server_messages_use_snake_case_types() {
        let json = serde_json::to_string(&ServerMessage::PairError {
            reason: PairErrorReason::RateLimited,
            retry_after_secs: Some(30),
            attempts_left: None,
        })
        .unwrap();
        assert!(json.contains("\"type\":\"pair_error\""));
        assert!(json.contains("\"reason\":\"rate_limited\""));
    }

    #[test]
    fn state_frame_carries_the_values_map() {
        let json = serde_json::to_string(&ServerMessage::State {
            values: BTreeMap::from([("text".to_owned(), "Hi".to_owned())]),
            selected: true,
        })
        .unwrap();
        assert!(json.contains("\"values\":{\"text\":\"Hi\"}"));
        assert!(json.contains("\"selected\":true"));
    }

    #[test]
    fn control_requests_round_trip() {
        let cases = [
            ControlRequest::Status,
            ControlRequest::Pair,
            ControlRequest::CancelPair,
            ControlRequest::Confirm,
            ControlRequest::Reject,
            ControlRequest::Unpair {
                id: None,
                all: true,
            },
            ControlRequest::Unpair {
                id: Some("ha1".into()),
                all: false,
            },
            ControlRequest::Select { id: "ha1".into() },
        ];
        for msg in cases {
            let json = serde_json::to_string(&msg).unwrap();
            assert_eq!(serde_json::from_str::<ControlRequest>(&json).unwrap(), msg);
        }
    }
}
