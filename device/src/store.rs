//! Persisted device state: identity, paired Home Assistant instances, selection.
//!
//! The file lives at `$SCREENSIGHT_STATE_DIR/state.json`
//! (default `/var/lib/screensight/`). Writes are atomic (temp file + rename) so
//! a power cut can never leave a half-written token store behind.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::identity::DeviceIdentity;

/// One Home Assistant instance the device has been paired with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedInstance {
    /// Home Assistant's own instance/entry identifier.
    pub ha_id: String,
    /// Friendly name shown on the panel during confirmation.
    pub ha_name: String,
    /// Long-lived bearer token minted at pairing time.
    pub token: String,
    /// Last address we saw this instance at, for diagnostics only.
    #[serde(default)]
    pub last_ip: Option<String>,
    /// Unix seconds at which pairing completed.
    pub paired_at_unix: u64,
}

/// The whole persisted document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Store {
    /// Stable identity, generated on first boot.
    pub identity: DeviceIdentity,
    /// Paired Home Assistant instances (any number; multi-pair).
    #[serde(default)]
    pub instances: Vec<PairedInstance>,
    /// `ha_id` of the instance currently allowed to drive the display.
    #[serde(default)]
    pub selected: Option<String>,
}

/// Directory holding `state.json`, honouring `SCREENSIGHT_STATE_DIR`.
pub fn state_dir() -> PathBuf {
    std::env::var_os("SCREENSIGHT_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/screensight"))
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join("state.json")
}

impl Store {
    /// Load `state.json`, generating and persisting a fresh identity if absent.
    ///
    /// `model`/`version` only matter when the identity is created.
    pub fn load_or_init(dir: &Path, model: &str, version: &str) -> Result<Self> {
        let path = state_path(dir);
        if path.exists() {
            let raw =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let store: Store = serde_json::from_str(&raw)
                .with_context(|| format!("parsing {}", path.display()))?;
            return Ok(store);
        }

        let store = Store {
            identity: DeviceIdentity::generate(model, version)?,
            instances: Vec::new(),
            selected: None,
        };
        store.save(dir)?;
        Ok(store)
    }

    /// Atomically persist the store.
    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let final_path = state_path(dir);
        let tmp_path = dir.join("state.json.tmp");
        let json = serde_json::to_vec_pretty(self).context("serialising state")?;
        fs::write(&tmp_path, &json).with_context(|| format!("writing {}", tmp_path.display()))?;
        fs::rename(&tmp_path, &final_path)
            .with_context(|| format!("renaming into {}", final_path.display()))?;
        Ok(())
    }

    /// Look up a paired instance by id.
    #[must_use]
    pub fn instance(&self, ha_id: &str) -> Option<&PairedInstance> {
        self.instances.iter().find(|i| i.ha_id == ha_id)
    }

    /// The currently selected instance, if any (and still paired).
    #[must_use]
    pub fn selected_instance(&self) -> Option<&PairedInstance> {
        self.selected.as_deref().and_then(|id| self.instance(id))
    }

    /// Whether any instance is paired.
    #[must_use]
    pub fn is_paired(&self) -> bool {
        !self.instances.is_empty()
    }

    /// Insert or replace an instance by `ha_id`.
    pub fn upsert_instance(&mut self, instance: PairedInstance) {
        if let Some(existing) = self
            .instances
            .iter_mut()
            .find(|i| i.ha_id == instance.ha_id)
        {
            *existing = instance;
        } else {
            self.instances.push(instance);
        }
    }

    /// Remove an instance; clears the selection if it pointed at it.
    ///
    /// Returns `true` if an instance was removed.
    pub fn remove_instance(&mut self, ha_id: &str) -> bool {
        let before = self.instances.len();
        self.instances.retain(|i| i.ha_id != ha_id);
        if self.selected.as_deref() == Some(ha_id) {
            self.selected = None;
        }
        self.instances.len() != before
    }

    /// Point the display at `ha_id`. Returns `false` if it is not paired.
    pub fn select(&mut self, ha_id: &str) -> bool {
        if self.instance(ha_id).is_some() {
            self.selected = Some(ha_id.to_owned());
            true
        } else {
            false
        }
    }
}

/// Current unix time in seconds.
#[must_use]
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "screensight-store-{tag}-{}-{}",
            std::process::id(),
            now_unix()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn instance(id: &str) -> PairedInstance {
        PairedInstance {
            ha_id: id.to_owned(),
            ha_name: format!("Home {id}"),
            token: "deadbeef".to_owned(),
            last_ip: Some("10.0.0.2".to_owned()),
            paired_at_unix: now_unix(),
        }
    }

    #[test]
    fn init_creates_and_reloads_stable_identity() {
        let dir = tmpdir("init");
        let first = Store::load_or_init(&dir, "Screensight Studio", "0.1.0").unwrap();
        let second = Store::load_or_init(&dir, "ignored", "ignored").unwrap();
        assert_eq!(first.identity.id, second.identity.id);
        assert_eq!(first.identity.name, second.identity.name);
        assert_eq!(second.identity.model, "Screensight Studio");
        assert!(!second.is_paired());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_select_and_remove() {
        let dir = tmpdir("crud");
        let mut store = Store::load_or_init(&dir, "m", "v").unwrap();
        store.upsert_instance(instance("a"));
        store.upsert_instance(instance("b"));
        assert!(store.is_paired());
        assert_eq!(store.instances.len(), 2);

        // Upsert replaces rather than duplicates.
        let mut replacement = instance("a");
        replacement.ha_name = "Renamed".to_owned();
        store.upsert_instance(replacement);
        assert_eq!(store.instances.len(), 2);
        assert_eq!(store.instance("a").unwrap().ha_name, "Renamed");

        assert!(store.select("b"));
        assert!(!store.select("missing"));
        assert_eq!(store.selected_instance().unwrap().ha_id, "b");

        assert!(store.remove_instance("b"));
        assert!(store.selected.is_none());
        assert!(!store.remove_instance("b"));

        // Round-trips through disk.
        store.save(&dir).unwrap();
        let reloaded = Store::load_or_init(&dir, "m", "v").unwrap();
        assert_eq!(reloaded.instances.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
