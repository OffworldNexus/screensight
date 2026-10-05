//! Shared daemon runtime: identity, pairing window, per-instance display state
//! and persistence, behind interior mutability so the control socket, the
//! WebSocket server and the renderer can all share one `Arc<Runtime>`.
//!
//! The runtime is deliberately free of async and GPU types: it is the
//! testable core that every transport drives.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::pairing::{PairingManager, SubmitOutcome};
use crate::protocol::{PairedSummary, StatusReport};
use crate::state::Displays;
use crate::store::{now_unix, PairedInstance, Store};

/// A pairing whose code was accepted and is waiting for the on-panel
/// "Yes · pair this display" / "Not my home" decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingPair {
    pub ha_id: String,
    pub ha_name: String,
}

/// What the panel should show right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Never paired and no window open.
    Unpaired,
    /// A pairing window is open, waiting for a Home Assistant to submit a code.
    Pairing { code: String },
    /// Code accepted; waiting for the user to confirm on the touch panel.
    Confirm { code: String, ha_name: String },
    /// Paired: show the selected instance's text (or a rest state).
    Display { text: Option<String> },
    /// The pairing window just expired without success.
    Timeout,
    /// No Home Assistant discovered and none paired.
    NoHome,
}

/// How long the transient [`Screen::Timeout`] lingers before returning to rest.
const TIMEOUT_LINGER: Duration = Duration::from_secs(20);

/// Shared runtime state.
pub struct Runtime {
    store: Mutex<Store>,
    state_dir: PathBuf,
    pairing: Mutex<PairingManager>,
    pending: Mutex<Option<PendingPair>>,
    displays: Displays,
    connected: Mutex<HashSet<String>>,
    last_timeout: Mutex<Option<Instant>>,
    was_pairing: Mutex<bool>,
}

impl Runtime {
    /// Load (or create) persisted state and build the runtime.
    pub fn load(state_dir: PathBuf, model: &str, version: &str) -> Result<Self> {
        let store = Store::load_or_init(&state_dir, model, version)?;
        let displays = Displays::new();
        displays.set_selected(store.selected.clone());
        Ok(Self {
            store: Mutex::new(store),
            state_dir,
            pairing: Mutex::new(PairingManager::new()),
            pending: Mutex::new(None),
            displays,
            connected: Mutex::new(HashSet::new()),
            last_timeout: Mutex::new(None),
            was_pairing: Mutex::new(false),
        })
    }

    /// Device identity accessor.
    #[must_use]
    pub fn status(&self) -> StatusReport {
        // Query the window before taking the store lock so we never nest locks.
        let pairing_code = self.pairing_code(Instant::now());
        let pairing_open = pairing_code.is_some();
        let store = self.store.lock().expect("store mutex poisoned");
        let instances = store
            .instances
            .iter()
            .map(|i| PairedSummary {
                ha_id: i.ha_id.clone(),
                ha_name: i.ha_name.clone(),
                selected: store.selected.as_deref() == Some(i.ha_id.as_str()),
            })
            .collect();
        StatusReport {
            id: store.identity.id.clone(),
            name: store.identity.name.clone(),
            model: store.identity.model.clone(),
            version: store.identity.version.clone(),
            pairing: pairing_open,
            pairing_code,
            instances,
            selected: store.selected.clone(),
        }
    }

    /// Identity getters used by mDNS advertisement.
    #[must_use]
    pub fn identity(&self) -> crate::identity::DeviceIdentity {
        self.store
            .lock()
            .expect("store mutex poisoned")
            .identity
            .clone()
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
        *self.last_timeout.lock().expect("timeout mutex poisoned") = None;
        log::info!("pairing window opened");
        Ok(code)
    }

    /// Close the pairing window without pairing.
    pub fn cancel_pairing(&self) {
        self.pairing
            .lock()
            .expect("pairing mutex poisoned")
            .cancel();
        *self.pending.lock().expect("pending mutex poisoned") = None;
        log::info!("pairing window closed");
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
        let instance = PairedInstance {
            ha_id: pending.ha_id.clone(),
            ha_name: pending.ha_name.clone(),
            token,
            last_ip: None,
            paired_at_unix: now_unix(),
        };

        let mut store = self.store.lock().expect("store mutex poisoned");
        store.upsert_instance(instance.clone());
        if store.selected.is_none() {
            store.selected = Some(instance.ha_id.clone());
        }
        let selected = store.selected.clone();
        store.save(&self.state_dir)?;
        drop(store);

        self.displays.set_selected(selected);
        self.cancel_pairing();
        log::info!("paired with {} ({})", instance.ha_name, instance.ha_id);
        Ok(Some(instance))
    }

    /// Decline the pending pairing and re-arm a fresh window (the design's
    /// "Not my home").
    pub fn reject_pairing(&self) -> Result<()> {
        *self.pending.lock().expect("pending mutex poisoned") = None;
        let now = Instant::now();
        self.pairing
            .lock()
            .expect("pairing mutex poisoned")
            .arm(now)?;
        Ok(())
    }

    /// Check the token presented on a WebSocket upgrade. Returns the instance.
    #[must_use]
    pub fn authenticate(&self, token: &str) -> Option<PairedInstance> {
        self.store
            .lock()
            .expect("store mutex poisoned")
            .instances
            .iter()
            .find(|i| i.token == token)
            .cloned()
    }

    /// Forget one instance (or all). Re-selects a remaining instance if the
    /// selected one disappears.
    pub fn unpair(&self, id: Option<&str>, all: bool) -> Result<usize> {
        let mut store = self.store.lock().expect("store mutex poisoned");
        let removed = if all {
            let n = store.instances.len();
            store.instances.clear();
            store.selected = None;
            n
        } else if let Some(id) = id {
            usize::from(store.remove_instance(id))
        } else {
            0
        };
        if store.selected.is_none() {
            store.selected = store.instances.first().map(|i| i.ha_id.clone());
        }
        let selected = store.selected.clone();
        store.save(&self.state_dir)?;
        drop(store);
        self.displays.set_selected(selected);
        Ok(removed)
    }

    /// Choose which paired instance drives the display.
    pub fn select(&self, id: &str) -> Result<bool> {
        let mut store = self.store.lock().expect("store mutex poisoned");
        if !store.select(id) {
            return Ok(false);
        }
        let selected = store.selected.clone();
        store.save(&self.state_dir)?;
        drop(store);
        self.displays.set_selected(selected);
        log::info!("display control moved to {id}");
        Ok(true)
    }

    /// Store text pushed by an authenticated instance.
    pub fn set_text(&self, ha_id: &str, text: Option<String>) {
        self.displays.set_text(ha_id, text);
    }

    /// The text stored for an instance.
    #[must_use]
    pub fn text_for(&self, ha_id: &str) -> Option<String> {
        self.displays.text_for(ha_id)
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

    /// Record that an instance connected/disconnected.
    pub fn set_connected(&self, ha_id: &str, connected: bool) {
        let mut set = self.connected.lock().expect("connected mutex poisoned");
        if connected {
            set.insert(ha_id.to_owned());
        } else {
            set.remove(ha_id);
        }
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

    /// Frame version; the renderer rebuilds when this changes.
    #[must_use]
    pub fn display_version(&self) -> u64 {
        self.displays.version()
    }

    /// Advance time-based pairing state (window expiry) and return the screen.
    #[must_use]
    pub fn tick(&self, now: Instant) -> Screen {
        if let Some(pending) = self.pending_pair() {
            if let Some(code) = self.pairing_code(now) {
                return Screen::Confirm {
                    code,
                    ha_name: pending.ha_name,
                };
            }
            // Window expired while awaiting confirmation.
            *self.pending.lock().expect("pending mutex poisoned") = None;
        }

        let open = self.pairing_open(now);
        // Read and update the previous-window flag without holding its lock
        // while locking anything else.
        let previously_open = {
            let mut was = self.was_pairing.lock().expect("was_pairing mutex poisoned");
            let previous = *was;
            *was = open;
            previous
        };
        if previously_open && !open {
            let paired = self.store.lock().expect("store mutex poisoned").is_paired();
            if !paired {
                *self.last_timeout.lock().expect("timeout mutex poisoned") = Some(now);
            }
        }

        if open {
            let code = self.pairing_code(now).unwrap_or_default();
            return Screen::Pairing { code };
        }

        let paired = self.store.lock().expect("store mutex poisoned").is_paired();
        if paired {
            return Screen::Display {
                text: self.displays.visible_text(),
            };
        }

        let fresh_timeout = self
            .last_timeout
            .lock()
            .expect("timeout mutex poisoned")
            .map(|t| now.saturating_duration_since(t) < TIMEOUT_LINGER)
            .unwrap_or(false);
        if fresh_timeout {
            Screen::Timeout
        } else {
            Screen::NoHome
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn runtime(tag: &str) -> (Runtime, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "screensight-runtime-{tag}-{}-{}",
            std::process::id(),
            now_unix()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        (Runtime::load(dir.clone(), "Test", "0.0.0").unwrap(), dir)
    }

    #[test]
    fn pairing_confirm_persists_and_selects_first_instance() {
        let (rt, dir) = runtime("pair");
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
            Screen::Confirm { ha_name, .. } if ha_name == "My home"
        ));

        let instance = rt.confirm_pairing().unwrap().unwrap();
        assert_eq!(instance.ha_id, "ha1");
        assert_eq!(instance.token.len(), 64);
        assert!(!rt.pairing_open(Instant::now()));

        let status = rt.status();
        assert_eq!(status.selected.as_deref(), Some("ha1"));
        assert_eq!(status.instances.len(), 1);

        // Reloading from disk keeps the pairing.
        let reloaded = Runtime::load(dir.clone(), "Test", "0.0.0").unwrap();
        assert!(reloaded.status().instances.len() == 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_pair_keeps_per_instance_text_and_display_one() {
        let (rt, dir) = runtime("multi");
        let now = Instant::now();
        for (id, name) in [("ha1", "Home"), ("ha2", "Dev")] {
            let code = rt.arm_pairing().unwrap();
            let ip = std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
            assert_eq!(
                rt.submit_code(ip, &code, id, name, now),
                SubmitOutcome::Pending
            );
            rt.confirm_pairing().unwrap();
        }
        assert_eq!(rt.status().instances.len(), 2);

        // First pairing selected ha1; store both texts, only ha1 is shown.
        rt.set_text("ha1", Some("alpha".into()));
        rt.set_text("ha2", Some("beta".into()));
        assert_eq!(rt.text_for("ha1").as_deref(), Some("alpha"));
        assert_eq!(rt.text_for("ha2").as_deref(), Some("beta"));
        assert!(matches!(
            rt.tick(now),
            Screen::Display { text: Some(t) } if t == "alpha"
        ));

        // Selecting ha2 reveals its own text.
        assert!(rt.select("ha2").unwrap());
        assert!(matches!(
            rt.tick(now),
            Screen::Display { text: Some(t) } if t == "beta"
        ));

        // Unpairing the selected instance falls back to the remaining one.
        rt.unpair(Some("ha2"), false).unwrap();
        assert_eq!(rt.status().selected.as_deref(), Some("ha1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reject_rearms_and_wrong_code_is_not_pending() {
        let (rt, dir) = runtime("reject");
        let now = Instant::now();
        let code = rt.arm_pairing().unwrap();
        let ip = std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 0, 9));
        assert!(matches!(
            rt.submit_code(ip, "000000", "ha1", "Home", now),
            SubmitOutcome::InvalidCode { .. }
        ));
        assert!(rt.pending_pair().is_none());
        assert_eq!(
            rt.submit_code(ip, &code, "ha1", "Home", now),
            SubmitOutcome::Pending
        );
        rt.reject_pairing().unwrap();
        assert!(rt.pending_pair().is_none());
        // A fresh window is open again.
        assert!(rt.pairing_open(Instant::now()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
