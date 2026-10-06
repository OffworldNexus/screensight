//! Durable device state: paired Home Assistant instances, the selected one, and
//! their dashboard values.
//!
//! The runtime keeps all of this in memory and treats it as authoritative; the
//! [`Persistence`] seam mirrors every write to storage. Production uses SQLite
//! through SeaORM via a background writer ([`ChannelPersistence`]); tests use
//! [`NoopPersistence`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use crate::identity::{DeviceIdentity, DeviceKeys};

/// One Home Assistant instance the device has been paired with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedInstance {
    /// Home Assistant's own instance/entry identifier.
    pub ha_id: String,
    /// Friendly name shown on the panel during confirmation.
    pub ha_name: String,
    /// Home Assistant's static Noise public key, hex-encoded. Presented at
    /// reconnect (Noise IK) to authenticate the peer.
    pub ha_static_key: String,
    /// Last address we saw this instance at, for diagnostics only.
    pub last_ip: Option<String>,
    /// Unix seconds at which pairing completed.
    pub paired_at_unix: u64,
}

/// Everything loaded from storage at startup.
pub struct Snapshot {
    /// Stable device identity (generated on first boot).
    pub identity: DeviceIdentity,
    /// The device's long-lived Noise static keypair.
    pub keys: DeviceKeys,
    /// Paired instances.
    pub instances: Vec<PairedInstance>,
    /// `ha_id` of the instance allowed to drive the display.
    pub selected: Option<String>,
    /// Per-instance dashboard values.
    pub values: std::collections::HashMap<String, BTreeMap<String, String>>,
}

/// Durably stores writes coming from the in-memory runtime.
///
/// Implementations are fire-and-forget: they must never block the caller (the
/// GPUI panel writes from its own thread).
pub trait Persistence: Send + Sync {
    /// Insert or replace an instance.
    fn upsert_instance(&self, instance: &PairedInstance);
    /// Remove an instance and all of its values.
    fn remove_instance(&self, ha_id: &str);
    /// Record which instance drives the display.
    fn set_selected(&self, ha_id: Option<&str>);
    /// Set one dashboard value.
    fn set_value(&self, ha_id: &str, key: &str, value: &str);
    /// Replace an instance's whole dashboard state.
    fn replace_values(&self, ha_id: &str, values: &BTreeMap<String, String>);
}

/// A write for the background SQLite writer.
#[derive(Debug)]
pub enum PersistCommand {
    /// Insert or replace an instance.
    UpsertInstance(PairedInstance),
    /// Remove an instance and its values.
    RemoveInstance(String),
    /// Record the selected instance.
    SetSelected(Option<String>),
    /// Set one value.
    SetValue {
        ha_id: String,
        key: String,
        value: String,
    },
    /// Replace an instance's values.
    ReplaceValues {
        ha_id: String,
        values: BTreeMap<String, String>,
    },
}

/// Persistence that hands writes to the background SQLite writer.
pub struct ChannelPersistence {
    tx: UnboundedSender<PersistCommand>,
}

impl ChannelPersistence {
    /// Wrap the writer's command channel.
    #[must_use]
    pub fn new(tx: UnboundedSender<PersistCommand>) -> Self {
        Self { tx }
    }
}

impl Persistence for ChannelPersistence {
    fn upsert_instance(&self, instance: &PairedInstance) {
        let _ = self
            .tx
            .send(PersistCommand::UpsertInstance(instance.clone()));
    }

    fn remove_instance(&self, ha_id: &str) {
        let _ = self
            .tx
            .send(PersistCommand::RemoveInstance(ha_id.to_owned()));
    }

    fn set_selected(&self, ha_id: Option<&str>) {
        let _ = self
            .tx
            .send(PersistCommand::SetSelected(ha_id.map(str::to_owned)));
    }

    fn set_value(&self, ha_id: &str, key: &str, value: &str) {
        let _ = self.tx.send(PersistCommand::SetValue {
            ha_id: ha_id.to_owned(),
            key: key.to_owned(),
            value: value.to_owned(),
        });
    }

    fn replace_values(&self, ha_id: &str, values: &BTreeMap<String, String>) {
        let _ = self.tx.send(PersistCommand::ReplaceValues {
            ha_id: ha_id.to_owned(),
            values: values.clone(),
        });
    }
}

/// Persistence that discards everything (tests, headless runs without a DB).
pub struct NoopPersistence;

impl Persistence for NoopPersistence {
    fn upsert_instance(&self, _instance: &PairedInstance) {}
    fn remove_instance(&self, _ha_id: &str) {}
    fn set_selected(&self, _ha_id: Option<&str>) {}
    fn set_value(&self, _ha_id: &str, _key: &str, _value: &str) {}
    fn replace_values(&self, _ha_id: &str, _values: &BTreeMap<String, String>) {}
}

/// In-memory paired-instance registry that mirrors writes to persistence.
pub struct Store {
    instances: Vec<PairedInstance>,
    selected: Option<String>,
    persist: Arc<dyn Persistence>,
}

impl Store {
    /// Build the registry from a loaded snapshot.
    #[must_use]
    pub fn new(
        instances: Vec<PairedInstance>,
        selected: Option<String>,
        persist: Arc<dyn Persistence>,
    ) -> Self {
        Self {
            instances,
            selected,
            persist,
        }
    }

    /// All paired instances.
    #[must_use]
    pub fn instances(&self) -> &[PairedInstance] {
        &self.instances
    }

    /// The selected instance id, if any.
    #[must_use]
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Look up an instance by id.
    #[must_use]
    pub fn instance(&self, ha_id: &str) -> Option<&PairedInstance> {
        self.instances.iter().find(|i| i.ha_id == ha_id)
    }

    /// Whether any instance is paired.
    #[must_use]
    pub fn is_paired(&self) -> bool {
        !self.instances.is_empty()
    }

    /// Insert or replace an instance by `ha_id`.
    pub fn upsert(&mut self, instance: PairedInstance) {
        if let Some(existing) = self
            .instances
            .iter_mut()
            .find(|i| i.ha_id == instance.ha_id)
        {
            *existing = instance.clone();
        } else {
            self.instances.push(instance.clone());
        }
        self.persist.upsert_instance(&instance);
    }

    /// Remove an instance; re-selects a remaining one if the selected vanished.
    ///
    /// Returns `true` if an instance was removed.
    pub fn remove(&mut self, ha_id: &str) -> bool {
        let before = self.instances.len();
        self.instances.retain(|i| i.ha_id != ha_id);
        if self.instances.len() == before {
            return false;
        }
        self.persist.remove_instance(ha_id);
        if self.selected.as_deref() == Some(ha_id) {
            self.selected = self.instances.first().map(|i| i.ha_id.clone());
            self.persist.set_selected(self.selected.as_deref());
        }
        true
    }

    /// Point the display at `ha_id`. Returns `false` if it is not paired.
    pub fn select(&mut self, ha_id: &str) -> bool {
        if self.instance(ha_id).is_none() {
            return false;
        }
        self.set_selected(Some(ha_id.to_owned()));
        true
    }

    /// Replace the selection (used after pairing).
    pub fn set_selected(&mut self, ha_id: Option<String>) {
        self.selected = ha_id;
        self.persist.set_selected(self.selected.as_deref());
    }
}

/// The default state directory, honouring `SCREENSIGHT_STATE_DIR`.
#[must_use]
pub fn state_dir() -> PathBuf {
    std::env::var_os("SCREENSIGHT_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/screensight"))
}

/// The SQLite database path inside `dir`.
#[must_use]
pub fn db_path(dir: &Path) -> PathBuf {
    dir.join("screensight.db")
}

/// Current unix time in seconds.
#[must_use]
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Build an instance record for a completed pairing.
#[must_use]
pub fn new_instance(
    ha_id: &str,
    ha_name: &str,
    ha_static_key: String,
    last_ip: Option<String>,
) -> PairedInstance {
    PairedInstance {
        ha_id: ha_id.to_owned(),
        ha_name: ha_name.to_owned(),
        ha_static_key,
        last_ip,
        paired_at_unix: now_unix(),
    }
}

/// Build a snapshot with a freshly generated identity and static keypair (tests
/// and first boot).
#[must_use]
pub fn snapshot_with(identity: DeviceIdentity) -> Snapshot {
    Snapshot {
        identity,
        keys: DeviceKeys::generate().expect("OS randomness for the test device keypair"),
        instances: Vec::new(),
        selected: None,
        values: std::collections::HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(id: &str) -> PairedInstance {
        PairedInstance {
            ha_id: id.to_owned(),
            ha_name: format!("Home {id}"),
            ha_static_key: "deadbeef".to_owned(),
            last_ip: Some("10.0.0.2".to_owned()),
            paired_at_unix: now_unix(),
        }
    }

    fn store() -> Store {
        Store::new(Vec::new(), None, Arc::new(NoopPersistence))
    }

    #[test]
    fn upsert_select_and_remove() {
        let mut store = store();
        store.upsert(instance("a"));
        store.upsert(instance("b"));
        assert!(store.is_paired());
        assert_eq!(store.instances().len(), 2);

        let mut replacement = instance("a");
        replacement.ha_name = "Renamed".to_owned();
        store.upsert(replacement);
        assert_eq!(store.instances().len(), 2);
        assert_eq!(store.instance("a").unwrap().ha_name, "Renamed");

        assert!(store.select("b"));
        assert!(!store.select("missing"));
        assert_eq!(store.selected(), Some("b"));

        assert!(store.remove("b"));
        assert_eq!(store.selected(), Some("a"));
        assert!(!store.remove("b"));
    }
}
