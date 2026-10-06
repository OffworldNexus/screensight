//! Shared daemon runtime: identity, the pairing/Noise screen machine,
//! per-instance dashboard state and persistence, behind interior mutability so
//! the control socket, the WebSocket server and the renderer can all share one
//! `Arc<Runtime>`.
//!
//! The runtime is deliberately free of async and GPU types: it is the testable
//! core that every transport drives. A [`Notify`] wakes the panel loop whenever
//! anything visible changes.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use tokio::sync::Notify;

use crate::identity::{DeviceIdentity, DeviceKeys};
use crate::pairing::PairingManager;
use crate::protocol::{PairRejection, PairedSummary, StatusReport};
use crate::state::StateManager;
use crate::store::{new_instance, PairedInstance, Persistence, Snapshot, Store};

/// A pairing whose SAS matched and which is waiting for the on-panel
/// "Pair" / "Cancel" decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingPair {
    /// Home Assistant instance id.
    pub ha_id: String,
    /// Friendly name shown on the confirm screen.
    pub ha_name: String,
}

/// State of one in-flight Noise XX pairing, once the handshake has completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingSession {
    /// Home Assistant's static Noise public key, hex-encoded, learned from the
    /// handshake. Becomes the credential stored on confirmation.
    pub ha_static_key: String,
    /// The 8-digit SAS derived from the handshake, shown on the panel.
    pub sas: String,
}

/// What the panel should show right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Paired, but Home Assistant has not been heard from since boot: the
    /// device is still coming up and waiting for its first state.
    Splash { name: String },
    /// Not paired and no pairing window open.
    Idle,
    /// Pairing window open, waiting for Home Assistant to connect (loader).
    PairingWaiting { name: String },
    /// XX handshake in progress ("Exchanging keys…", loader).
    PairingHandshake { name: String },
    /// XX complete; the SAS the user must type into Home Assistant.
    PairingCode { sas: String },
    /// The SAS matched and Home Assistant named itself; panel approval.
    Confirm { ha_name: String },
    /// Any pairing failure: one generic error screen with a retry action.
    PairingError,
    /// Paired: the dashboard, driven by the selected instance's values.
    Dashboard { values: BTreeMap<String, String> },
}

impl Screen {
    /// The coarse "view" identity, ignoring any animation phase. Two frames with
    /// the same view key must not trigger a CRT transition, even if the loader's
    /// pixels differ.
    #[must_use]
    pub fn view_key(&self) -> u8 {
        match self {
            Self::Splash { .. } => 0,
            Self::Idle => 1,
            Self::PairingWaiting { .. } => 2,
            Self::PairingHandshake { .. } => 3,
            Self::PairingCode { .. } => 4,
            Self::Confirm { .. } => 5,
            Self::PairingError => 6,
            Self::Dashboard { .. } => 7,
        }
    }

    /// Whether the panel must keep repainting this screen for its animation.
    #[must_use]
    pub fn is_animated(&self) -> bool {
        matches!(
            self,
            Self::PairingWaiting { .. } | Self::PairingHandshake { .. }
        )
    }
}

/// Shared runtime state.
pub struct Runtime {
    identity: DeviceIdentity,
    keys: DeviceKeys,
    store: Mutex<Store>,
    persist: Arc<dyn Persistence>,
    pairing: Mutex<PairingManager>,
    pending: Mutex<Option<PendingPair>>,
    /// The completed-but-unconfirmed XX pairing, if any.
    session: Mutex<Option<PairingSession>>,
    /// Whether an XX handshake is currently in progress (loader: handshake).
    handshaking: Mutex<bool>,
    /// Whether the current pairing window has recorded a failure (generic error).
    pairing_error: Mutex<bool>,
    /// Why the last pending pairing ended, for the waiting Home Assistant.
    rejection: Mutex<Option<PairRejection>>,
    state: StateManager,
    connected: Mutex<HashSet<String>>,
    /// Instances that have been heard from (connected or sent state) since
    /// boot. Used to hold the splash screen until the first state arrives, and
    /// to avoid flicking back to it if the socket later drops.
    ready: Mutex<HashSet<String>>,
    dirty: Arc<Notify>,
}

impl Runtime {
    /// Build the runtime from a loaded [`Snapshot`] and its persistence sink.
    #[must_use]
    pub fn new(snapshot: Snapshot, persist: Arc<dyn Persistence>) -> Self {
        let state = StateManager::new();
        for (ha_id, values) in snapshot.values {
            state.set_values(&ha_id, values);
        }
        let store = Store::new(snapshot.instances, snapshot.selected, Arc::clone(&persist));
        Self {
            identity: snapshot.identity,
            keys: snapshot.keys,
            store: Mutex::new(store),
            persist,
            pairing: Mutex::new(PairingManager::new()),
            pending: Mutex::new(None),
            session: Mutex::new(None),
            handshaking: Mutex::new(false),
            pairing_error: Mutex::new(false),
            rejection: Mutex::new(None),
            state,
            connected: Mutex::new(HashSet::new()),
            ready: Mutex::new(HashSet::new()),
            dirty: Arc::new(Notify::new()),
        }
    }

    /// A [`Notify`] signalled whenever visible state changes.
    #[must_use]
    pub fn dirty(&self) -> Arc<Notify> {
        Arc::clone(&self.dirty)
    }

    fn mark_dirty(&self) {
        self.dirty.notify_one();
    }

    /// Device status, including the pairing SAS while a window is open.
    #[must_use]
    pub fn status(&self) -> StatusReport {
        let now = Instant::now();
        let pairing_open = self.pairing_open(now);
        let sas = self.pairing_sas(now);
        let store = self.store.lock().expect("store mutex poisoned");
        let instances = store
            .instances()
            .iter()
            .map(|i| PairedSummary {
                ha_id: i.ha_id.clone(),
                ha_name: i.ha_name.clone(),
                selected: store.selected() == Some(i.ha_id.as_str()),
            })
            .collect();
        StatusReport {
            id: self.identity.id.clone(),
            name: self.identity.name.clone(),
            model: self.identity.model.clone(),
            version: self.identity.version.clone(),
            pairing: pairing_open,
            sas,
            instances,
            selected: store.selected().map(str::to_owned),
        }
    }

    /// Identity getter used by mDNS advertisement.
    #[must_use]
    pub fn identity(&self) -> DeviceIdentity {
        self.identity.clone()
    }

    /// The device's Noise static keypair.
    #[must_use]
    pub fn keys(&self) -> DeviceKeys {
        self.keys.clone()
    }

    /// The device's static public key, hex-encoded (mDNS `key=`).
    #[must_use]
    pub fn public_key_hex(&self) -> String {
        self.keys.public_hex()
    }

    /// Open (or re-arm) the pairing window and clear any previous attempt.
    pub fn arm_pairing(&self) -> Result<()> {
        let now = Instant::now();
        self.pairing
            .lock()
            .expect("pairing mutex poisoned")
            .arm(now);
        self.reset_attempt();
        *self.rejection.lock().expect("rejection mutex poisoned") = None;
        log::info!("pairing window opened");
        self.mark_dirty();
        Ok(())
    }

    /// Close the pairing window without pairing, returning to idle.
    pub fn cancel_pairing(&self) {
        self.pairing
            .lock()
            .expect("pairing mutex poisoned")
            .cancel();
        self.reset_attempt();
        log::info!("pairing window closed");
        self.mark_dirty();
    }

    /// Clear all per-attempt pairing state (pending, session, flags).
    fn reset_attempt(&self) {
        *self.pending.lock().expect("pending mutex poisoned") = None;
        *self.session.lock().expect("session mutex poisoned") = None;
        *self.handshaking.lock().expect("handshaking mutex poisoned") = false;
        *self
            .pairing_error
            .lock()
            .expect("pairing_error mutex poisoned") = false;
    }

    /// Whether the pairing window is open at `now`.
    #[must_use]
    pub fn pairing_open(&self, now: Instant) -> bool {
        self.pairing
            .lock()
            .expect("pairing mutex poisoned")
            .is_open(now)
    }

    /// The derived SAS while a handshake has completed in an open window.
    #[must_use]
    pub fn pairing_sas(&self, now: Instant) -> Option<String> {
        if !self.pairing_open(now) {
            return None;
        }
        self.session
            .lock()
            .expect("session mutex poisoned")
            .as_ref()
            .map(|session| session.sas.clone())
    }

    /// The pending (SAS-matched, awaiting touch) pairing, if any.
    #[must_use]
    pub fn pending_pair(&self) -> Option<PendingPair> {
        self.pending.lock().expect("pending mutex poisoned").clone()
    }

    /// Mark that an XX handshake has started (loader: "Exchanging keys…").
    pub fn begin_handshake(&self) {
        *self.handshaking.lock().expect("handshaking mutex poisoned") = true;
        *self
            .pairing_error
            .lock()
            .expect("pairing_error mutex poisoned") = false;
        self.mark_dirty();
    }

    /// Record a completed XX handshake and the SAS to display.
    pub fn set_pairing_sas(&self, ha_static_key: &str, sas: &str) {
        *self.session.lock().expect("session mutex poisoned") = Some(PairingSession {
            ha_static_key: ha_static_key.to_owned(),
            sas: sas.to_owned(),
        });
        *self.handshaking.lock().expect("handshaking mutex poisoned") = false;
        log::info!("pairing handshake complete; SAS ready");
        self.mark_dirty();
    }

    /// Record the authenticated pair request (SAS matched in Home Assistant).
    pub fn accept_pair_request(&self, ha_id: &str, ha_name: &str) {
        *self.pending.lock().expect("pending mutex poisoned") = Some(PendingPair {
            ha_id: ha_id.to_owned(),
            ha_name: ha_name.to_owned(),
        });
        log::info!("pair request accepted; awaiting on-panel confirmation");
        self.mark_dirty();
    }

    /// Record that the current pairing attempt failed; the panel shows the one
    /// generic error screen until the user starts over.
    pub fn fail_pairing(&self) {
        if !self.pairing_open(Instant::now()) {
            return;
        }
        *self
            .pairing_error
            .lock()
            .expect("pairing_error mutex poisoned") = true;
        self.reset_attempt_but_keep_error();
        log::warn!("pairing attempt failed");
        self.mark_dirty();
    }

    /// Clear pending/session/handshaking without clearing the error flag.
    fn reset_attempt_but_keep_error(&self) {
        *self.pending.lock().expect("pending mutex poisoned") = None;
        *self.session.lock().expect("session mutex poisoned") = None;
        *self.handshaking.lock().expect("handshaking mutex poisoned") = false;
    }

    /// Confirm the pending pairing on the panel: persist the instance with Home
    /// Assistant's static key, select it if nothing is selected, close the window.
    pub fn confirm_pairing(&self) -> Result<Option<PairedInstance>> {
        let Some(pending) = self.pending_pair() else {
            return Ok(None);
        };
        let Some(session) = self.session.lock().expect("session mutex poisoned").clone() else {
            return Ok(None);
        };
        let instance = new_instance(
            &pending.ha_id,
            &pending.ha_name,
            session.ha_static_key,
            None,
        );

        {
            let mut store = self.store.lock().expect("store mutex poisoned");
            store.upsert(instance.clone());
            if store.selected().is_none() {
                store.set_selected(Some(instance.ha_id.clone()));
            }
        }
        self.cancel_pairing();
        log::info!("paired with {} ({})", instance.ha_name, instance.ha_id);
        Ok(Some(instance))
    }

    /// Decline the pending pairing and return to idle ("Cancel").
    pub fn reject_pairing(&self) {
        *self.rejection.lock().expect("rejection mutex poisoned") = Some(PairRejection::Declined);
        self.cancel_pairing();
        log::info!("pairing declined on the panel");
    }

    /// Take the reason the last pending pairing ended, if it was recorded.
    ///
    /// A window that simply lapses leaves this `None`; the WebSocket server
    /// treats that as [`PairRejection::TimedOut`].
    #[must_use]
    pub fn take_rejection(&self) -> Option<PairRejection> {
        self.rejection
            .lock()
            .expect("rejection mutex poisoned")
            .take()
    }

    /// Seconds until `ip` may attempt another handshake, if it is locked out.
    #[must_use]
    pub fn handshake_retry_after(&self, ip: std::net::IpAddr, now: Instant) -> Option<u64> {
        self.pairing
            .lock()
            .expect("pairing mutex poisoned")
            .retry_after(ip, now)
    }

    /// Record a failed handshake attempt from `ip`, returning the lockout
    /// duration if this attempt tripped it.
    pub fn record_handshake_failure(&self, ip: std::net::IpAddr, now: Instant) -> Option<u64> {
        self.pairing
            .lock()
            .expect("pairing mutex poisoned")
            .record_failure(ip, now)
    }

    /// Authenticate a peer by the static public key it presented. Returns the
    /// paired instance if the key is known.
    #[must_use]
    pub fn authenticate_by_static_key(&self, key_hex: &str) -> Option<PairedInstance> {
        self.store
            .lock()
            .expect("store mutex poisoned")
            .instances()
            .iter()
            .find(|i| i.ha_static_key == key_hex)
            .cloned()
    }

    /// Forget one instance (or all). Re-selects a remaining instance if the
    /// selected one disappears.
    pub fn unpair(&self, id: Option<&str>, all: bool) -> Result<usize> {
        let removed: Vec<String> = {
            let mut store = self.store.lock().expect("store mutex poisoned");
            let targets: Vec<String> = if all {
                store.instances().iter().map(|i| i.ha_id.clone()).collect()
            } else {
                id.map(str::to_owned).into_iter().collect()
            };
            targets
                .into_iter()
                .filter(|target| store.remove(target))
                .collect()
        };
        for ha_id in &removed {
            self.state.remove_instance(ha_id);
            self.ready
                .lock()
                .expect("ready mutex poisoned")
                .remove(ha_id);
            self.connected
                .lock()
                .expect("connected mutex poisoned")
                .remove(ha_id);
        }
        self.mark_dirty();
        Ok(removed.len())
    }

    /// Choose which paired instance drives the display.
    pub fn select(&self, id: &str) -> Result<bool> {
        let selected = self.store.lock().expect("store mutex poisoned").select(id);
        if selected {
            log::info!("display control moved to {id}");
            self.mark_dirty();
        }
        Ok(selected)
    }

    /// Store one dashboard value pushed by an authenticated instance.
    pub fn set_value(&self, ha_id: &str, key: &str, value: &str) {
        self.state.set_value(ha_id, key, value);
        self.persist.set_value(ha_id, key, value);
        self.mark_ready(ha_id);
        self.mark_dirty();
    }

    /// Replace an authenticated instance's whole dashboard state.
    pub fn set_values(&self, ha_id: &str, values: BTreeMap<String, String>) {
        self.state.set_values(ha_id, values.clone());
        self.persist.replace_values(ha_id, &values);
        self.mark_ready(ha_id);
        self.mark_dirty();
    }

    /// The dashboard values stored for an instance.
    #[must_use]
    pub fn values_for(&self, ha_id: &str) -> BTreeMap<String, String> {
        self.state.values(ha_id)
    }

    /// The values of the instance currently on the panel.
    #[must_use]
    pub fn visible_values(&self) -> BTreeMap<String, String> {
        let ha_id = self
            .store
            .lock()
            .expect("store mutex poisoned")
            .selected()
            .map(str::to_owned);
        ha_id.map(|id| self.state.values(&id)).unwrap_or_default()
    }

    /// Full paired instance record, including its static public key, if paired.
    #[must_use]
    pub fn paired_instance(&self, ha_id: &str) -> Option<PairedInstance> {
        self.store
            .lock()
            .expect("store mutex poisoned")
            .instance(ha_id)
            .cloned()
    }

    /// Record that an instance connected/disconnected. The first connection
    /// marks it ready, which is what ends the splash screen.
    pub fn set_connected(&self, ha_id: &str, connected: bool) {
        {
            let mut set = self.connected.lock().expect("connected mutex poisoned");
            if connected {
                set.insert(ha_id.to_owned());
            } else {
                set.remove(ha_id);
            }
        }
        if connected {
            self.mark_ready(ha_id);
        }
        self.mark_dirty();
    }

    /// Mark an instance as heard-from. Sticky for the process lifetime so a
    /// dropped socket does not flash the splash screen back.
    fn mark_ready(&self, ha_id: &str) {
        self.ready
            .lock()
            .expect("ready mutex poisoned")
            .insert(ha_id.to_owned());
    }

    /// Whether the instance has been heard from since boot.
    #[must_use]
    fn instance_ready(&self, ha_id: &str) -> bool {
        self.ready
            .lock()
            .expect("ready mutex poisoned")
            .contains(ha_id)
    }

    /// Whether any paired instance currently holds a WebSocket.
    #[must_use]
    pub fn any_connected(&self) -> bool {
        !self
            .connected
            .lock()
            .expect("connected mutex poisoned")
            .is_empty()
    }

    /// Compute the screen to show at `now`.
    #[must_use]
    pub fn tick(&self, now: Instant) -> Screen {
        if self.pairing_open(now) {
            if *self
                .pairing_error
                .lock()
                .expect("pairing_error mutex poisoned")
            {
                return Screen::PairingError;
            }
            if let Some(pending) = self.pending_pair() {
                return Screen::Confirm {
                    ha_name: pending.ha_name,
                };
            }
            if let Some(sas) = self
                .session
                .lock()
                .expect("session mutex poisoned")
                .as_ref()
                .map(|session| session.sas.clone())
            {
                return Screen::PairingCode { sas };
            }
            if *self.handshaking.lock().expect("handshaking mutex poisoned") {
                return Screen::PairingHandshake {
                    name: self.identity.name.clone(),
                };
            }
            return Screen::PairingWaiting {
                name: self.identity.name.clone(),
            };
        }

        // The window has lapsed: drop any half-finished attempt so the next one
        // starts clean, then fall through to the steady-state screen.
        if self.pending_pair().is_some()
            || self
                .session
                .lock()
                .expect("session mutex poisoned")
                .is_some()
        {
            self.reset_attempt();
        }

        let selected = {
            let store = self.store.lock().expect("store mutex poisoned");
            if !store.is_paired() {
                return Screen::Idle;
            }
            store.selected().map(str::to_owned)
        };

        if selected
            .as_deref()
            .is_some_and(|id| self.instance_ready(id))
        {
            return Screen::Dashboard {
                values: self.visible_values(),
            };
        }

        // Paired but nothing heard from Home Assistant yet.
        Screen::Splash {
            name: self.identity.name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{snapshot_with, NoopPersistence};
    use std::net::Ipv4Addr;

    fn runtime() -> Runtime {
        let identity = DeviceIdentity::generate("Test", "0.0.0").unwrap();
        Runtime::new(snapshot_with(identity), Arc::new(NoopPersistence))
    }

    fn ip(n: u8) -> std::net::IpAddr {
        std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 0, n))
    }

    /// Drive a full pairing: arm, XX complete with a SAS, then HA names itself.
    fn pair(rt: &Runtime, id: &str, name: &str, key: &str) {
        rt.arm_pairing().unwrap();
        rt.begin_handshake();
        rt.set_pairing_sas(key, "12345678");
        rt.accept_pair_request(id, name);
        rt.confirm_pairing().unwrap();
    }

    #[test]
    fn pairing_shows_loader_sas_confirm_then_selects_first_instance() {
        let rt = runtime();
        let now = Instant::now();
        rt.arm_pairing().unwrap();

        // Window open, nothing yet: the waiting loader.
        assert!(matches!(rt.tick(now), Screen::PairingWaiting { .. }));

        // XX in flight: the handshake status variant.
        rt.begin_handshake();
        assert!(matches!(rt.tick(now), Screen::PairingHandshake { .. }));

        // XX done: the SAS, and only then.
        rt.set_pairing_sas(&"ab".repeat(32), "12345678");
        assert!(matches!(
            rt.tick(now),
            Screen::PairingCode { sas } if sas == "12345678"
        ));
        assert_eq!(rt.pairing_sas(now).as_deref(), Some("12345678"));

        // SAS matched in HA and it named itself: the confirmation.
        rt.accept_pair_request("ha1", "My home");
        assert!(matches!(
            rt.tick(now),
            Screen::Confirm { ha_name } if ha_name == "My home"
        ));

        let instance = rt.confirm_pairing().unwrap().unwrap();
        assert_eq!(instance.ha_id, "ha1");
        assert_eq!(instance.ha_static_key, "ab".repeat(32));
        // Paired but not yet heard from: the panel holds on the splash.
        assert!(matches!(rt.tick(now), Screen::Splash { .. }));
        rt.set_connected("ha1", true);
        assert!(matches!(rt.tick(now), Screen::Dashboard { .. }));

        let status = rt.status();
        assert_eq!(status.selected.as_deref(), Some("ha1"));
        assert_eq!(status.instances.len(), 1);
    }

    #[test]
    fn handshake_failure_shows_the_generic_error_until_rearm() {
        let rt = runtime();
        let now = Instant::now();
        rt.arm_pairing().unwrap();
        rt.begin_handshake();
        rt.fail_pairing();
        assert_eq!(rt.tick(now), Screen::PairingError);

        // Re-arming the window clears the error and returns to the loader.
        rt.arm_pairing().unwrap();
        assert!(matches!(rt.tick(now), Screen::PairingWaiting { .. }));
    }

    #[test]
    fn decline_returns_to_idle() {
        let rt = runtime();
        let now = Instant::now();
        rt.arm_pairing().unwrap();
        rt.set_pairing_sas(&"ab".repeat(32), "12345678");
        rt.accept_pair_request("ha1", "Home");
        assert!(matches!(rt.tick(now), Screen::Confirm { .. }));

        rt.reject_pairing();
        assert!(rt.pending_pair().is_none());
        assert_eq!(rt.take_rejection(), Some(PairRejection::Declined));
        assert!(!rt.pairing_open(Instant::now()));
        assert_eq!(rt.tick(now), Screen::Idle);
    }

    #[test]
    fn expired_window_returns_to_idle() {
        let rt = runtime();
        let start = Instant::now();
        rt.arm_pairing().unwrap();
        let later = start + crate::pairing::WINDOW + std::time::Duration::from_secs(1);
        assert_eq!(rt.tick(later), Screen::Idle);
    }

    #[test]
    fn authenticate_by_static_key_matches_only_paired_keys() {
        let rt = runtime();
        let key = "cd".repeat(32);
        pair(&rt, "ha1", "Home", &key);
        assert_eq!(rt.authenticate_by_static_key(&key).unwrap().ha_id, "ha1");
        assert!(rt.authenticate_by_static_key(&"ef".repeat(32)).is_none());
    }

    #[test]
    fn handshake_attempts_are_rate_limited_per_address() {
        let rt = runtime();
        let now = Instant::now();
        rt.arm_pairing().unwrap();
        for _ in 0..crate::pairing::MAX_FAILURES_PER_IP {
            rt.record_handshake_failure(ip(7), now);
        }
        assert!(rt.handshake_retry_after(ip(7), now).is_some());
        // A different address is unaffected; the lockout expires.
        assert!(rt.handshake_retry_after(ip(8), now).is_none());
        let later = now + crate::pairing::LOCKOUT + std::time::Duration::from_secs(1);
        assert!(rt.handshake_retry_after(ip(7), later).is_none());
    }

    #[test]
    fn dashboard_shows_selected_instance_values_and_unpair_returns_to_idle() {
        let rt = runtime();
        let now = Instant::now();
        for (id, name) in [("ha1", "Home"), ("ha2", "Dev")] {
            pair(&rt, id, name, &format!("{id:0>64}"));
        }
        assert_eq!(rt.status().instances.len(), 2);

        rt.set_value("ha1", "text", "alpha");
        rt.set_value("ha2", "text", "beta");
        assert!(matches!(
            rt.tick(now),
            Screen::Dashboard { values } if values["text"] == "alpha"
        ));

        assert!(rt.select("ha2").unwrap());
        assert!(matches!(
            rt.tick(now),
            Screen::Dashboard { values } if values["text"] == "beta"
        ));

        rt.unpair(None, true).unwrap();
        assert_eq!(rt.tick(now), Screen::Idle);
    }

    #[test]
    fn splash_shows_until_home_assistant_is_heard_from() {
        let rt = runtime();
        let now = Instant::now();
        pair(&rt, "ha1", "Home", &"ab".repeat(32));

        // Paired, but quiet: the splash, not an empty dashboard.
        assert!(matches!(rt.tick(now), Screen::Splash { .. }));

        // A pushed value also ends the splash, even before a connect.
        rt.set_value("ha1", "text", "hi");
        assert!(matches!(
            rt.tick(now),
            Screen::Dashboard { values } if values["text"] == "hi"
        ));

        // Ready is sticky: a disconnect does not bring the splash back.
        rt.set_connected("ha1", false);
        assert!(matches!(rt.tick(now), Screen::Dashboard { .. }));
    }
}
