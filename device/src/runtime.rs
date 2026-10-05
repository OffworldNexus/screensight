//! Shared daemon runtime: identity, the four-state screen machine, per-instance
//! dashboard state and persistence, behind interior mutability so the control
//! socket, the WebSocket server and the renderer can all share one `Arc<Runtime>`.
//!
//! The runtime is deliberately free of async and GPU types: it is the testable
//! core that every transport drives. A [`Notify`] wakes the panel loop whenever
//! anything visible changes.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use tokio::sync::Notify;

use crate::identity::DeviceIdentity;
use crate::pairing::{PairingManager, SubmitOutcome};
use crate::protocol::{PairRejection, PairedSummary, StatusReport};
use crate::state::StateManager;
use crate::store::{new_instance, PairedInstance, Persistence, Snapshot, Store};

/// A pairing whose code was accepted and is waiting for the on-panel
/// "Pair" / "Cancel" decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingPair {
    /// Home Assistant instance id.
    pub ha_id: String,
    /// Friendly name shown on the confirm screen.
    pub ha_name: String,
}

/// What the panel should show right now. The device has exactly four faces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Paired, but Home Assistant has not been heard from since boot: the
    /// device is still coming up and waiting for its first state.
    Splash { name: String },
    /// Not paired and no pairing window open.
    Idle,
    /// A pairing window is open, waiting for Home Assistant to submit the code.
    Pairing { code: String, name: String },
    /// The code was accepted; waiting for the user to approve on the panel.
    Confirm { ha_name: String },
    /// Paired: the dashboard, driven by the selected instance's values.
    Dashboard { values: BTreeMap<String, String> },
}

/// Shared runtime state.
pub struct Runtime {
    identity: DeviceIdentity,
    store: Mutex<Store>,
    persist: Arc<dyn Persistence>,
    pairing: Mutex<PairingManager>,
    pending: Mutex<Option<PendingPair>>,
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
            store: Mutex::new(store),
            persist,
            pairing: Mutex::new(PairingManager::new()),
            pending: Mutex::new(None),
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

    /// Device identity accessor.
    #[must_use]
    pub fn status(&self) -> StatusReport {
        let pairing_code = self.pairing_code(Instant::now());
        let pairing_open = pairing_code.is_some();
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
            pairing_code,
            instances,
            selected: store.selected().map(str::to_owned),
        }
    }

    /// Identity getter used by mDNS advertisement.
    #[must_use]
    pub fn identity(&self) -> DeviceIdentity {
        self.identity.clone()
    }

    /// Open (or re-arm) the pairing window, returning the new code.
    pub fn arm_pairing(&self) -> Result<String> {
        let now = Instant::now();
        let code = self
            .pairing
            .lock()
            .expect("pairing mutex poisoned")
            .arm(now)?;
        *self.pending.lock().expect("pending mutex poisoned") = None;
        *self.rejection.lock().expect("rejection mutex poisoned") = None;
        log::info!("pairing window opened");
        self.mark_dirty();
        Ok(code)
    }

    /// Close the pairing window without pairing, returning to idle.
    pub fn cancel_pairing(&self) {
        self.pairing
            .lock()
            .expect("pairing mutex poisoned")
            .cancel();
        *self.pending.lock().expect("pending mutex poisoned") = None;
        log::info!("pairing window closed");
        self.mark_dirty();
    }

    /// Whether the pairing window is open at `now`.
    #[must_use]
    pub fn pairing_open(&self, now: Instant) -> bool {
        self.pairing
            .lock()
            .expect("pairing mutex poisoned")
            .is_open(now)
    }

    /// The current pairing code, if the window is open.
    #[must_use]
    pub fn pairing_code(&self, now: Instant) -> Option<String> {
        self.pairing
            .lock()
            .expect("pairing mutex poisoned")
            .code(now)
            .map(str::to_owned)
    }

    /// The pending (code-accepted, awaiting touch) pairing, if any.
    #[must_use]
    pub fn pending_pair(&self) -> Option<PendingPair> {
        self.pending.lock().expect("pending mutex poisoned").clone()
    }

    /// Submit a pairing code. On success the pending pair is recorded.
    pub fn submit_code(
        &self,
        ip: std::net::IpAddr,
        code: &str,
        ha_id: &str,
        ha_name: &str,
        now: Instant,
    ) -> SubmitOutcome {
        let outcome = self
            .pairing
            .lock()
            .expect("pairing mutex poisoned")
            .submit(ip, code, now);
        if outcome == SubmitOutcome::Pending {
            *self.pending.lock().expect("pending mutex poisoned") = Some(PendingPair {
                ha_id: ha_id.to_owned(),
                ha_name: ha_name.to_owned(),
            });
            log::info!("pairing code accepted; awaiting on-panel confirmation");
            self.mark_dirty();
        }
        outcome
    }

    /// Confirm the pending pairing on the panel: mint a token, persist the
    /// instance, select it if nothing is selected, and close the window.
    pub fn confirm_pairing(&self) -> Result<Option<PairedInstance>> {
        let Some(pending) = self.pending_pair() else {
            return Ok(None);
        };
        let token = self
            .pairing
            .lock()
            .expect("pairing mutex poisoned")
            .mint_token()?;
        let instance = new_instance(&pending.ha_id, &pending.ha_name, token, None);

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
        *self.pending.lock().expect("pending mutex poisoned") = None;
        *self.rejection.lock().expect("rejection mutex poisoned") = Some(PairRejection::Declined);
        self.pairing
            .lock()
            .expect("pairing mutex poisoned")
            .cancel();
        log::info!("pairing declined on the panel");
        self.mark_dirty();
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

    /// Check the token presented on a WebSocket upgrade. Returns the instance.
    #[must_use]
    pub fn authenticate(&self, token: &str) -> Option<PairedInstance> {
        self.store
            .lock()
            .expect("store mutex poisoned")
            .instances()
            .iter()
            .find(|i| i.token == token)
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

    /// Full paired instance record (including token), if paired.
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
        if let Some(pending) = self.pending_pair() {
            if self.pairing_open(now) {
                return Screen::Confirm {
                    ha_name: pending.ha_name,
                };
            }
            // The window lapsed while awaiting confirmation.
            *self.pending.lock().expect("pending mutex poisoned") = None;
        }

        if self.pairing_open(now) {
            let code = self.pairing_code(now).unwrap_or_default();
            return Screen::Pairing {
                code,
                name: self.identity.name.clone(),
            };
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

    #[test]
    fn pairing_confirm_selects_first_instance() {
        let rt = runtime();
        let now = Instant::now();
        let code = rt.arm_pairing().unwrap();
        assert!(matches!(rt.tick(now), Screen::Pairing { .. }));

        let ip = std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));
        assert_eq!(
            rt.submit_code(ip, &code, "ha1", "My home", now),
            SubmitOutcome::Pending
        );
        assert!(matches!(
            rt.tick(now),
            Screen::Confirm { ha_name } if ha_name == "My home"
        ));

        let instance = rt.confirm_pairing().unwrap().unwrap();
        assert_eq!(instance.ha_id, "ha1");
        assert_eq!(instance.token.len(), 64);
        // Paired but not yet heard from: the panel holds on the splash.
        assert!(matches!(rt.tick(now), Screen::Splash { .. }));
        rt.set_connected("ha1", true);
        assert!(matches!(rt.tick(now), Screen::Dashboard { .. }));

        let status = rt.status();
        assert_eq!(status.selected.as_deref(), Some("ha1"));
        assert_eq!(status.instances.len(), 1);
    }

    #[test]
    fn decline_returns_to_idle() {
        let rt = runtime();
        let now = Instant::now();
        let code = rt.arm_pairing().unwrap();
        let ip = std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 0, 9));
        rt.submit_code(ip, &code, "ha1", "Home", now);
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
    fn wrong_code_is_not_pending() {
        let rt = runtime();
        let now = Instant::now();
        rt.arm_pairing().unwrap();
        let ip = std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 0, 9));
        assert!(matches!(
            rt.submit_code(ip, "000000", "ha1", "Home", now),
            SubmitOutcome::InvalidCode { .. }
        ));
        assert!(rt.pending_pair().is_none());
    }

    #[test]
    fn dashboard_shows_selected_instance_values_and_unpair_returns_to_idle() {
        let rt = runtime();
        let now = Instant::now();
        for (id, name) in [("ha1", "Home"), ("ha2", "Dev")] {
            let code = rt.arm_pairing().unwrap();
            let ip = std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
            rt.submit_code(ip, &code, id, name, now);
            rt.confirm_pairing().unwrap();
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
        let code = rt.arm_pairing().unwrap();
        let ip = std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        rt.submit_code(ip, &code, "ha1", "Home", now);
        rt.confirm_pairing().unwrap();

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
