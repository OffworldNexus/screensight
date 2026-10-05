//! Stable device identity and the human-friendly mDNS name.
//!
//! The name is a randomly drawn adjective+noun pair ("Brave Otter") shown on
//! the pairing screen and advertised over mDNS, so Home Assistant discovers the
//! device under that name. The opaque `id` is what travels in the zeroconf TXT
//! record and identifies the device durably. Both are generated once on first
//! boot and persisted, so the pairing survives reboots and DHCP changes.
//!
//! Neither value contains the pairing code; see [`crate::pairing`].

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Number of random bytes behind the opaque device id.
const ID_BYTES: usize = 8;

/// Everything Home Assistant needs to recognise and re-find this device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIdentity {
    /// Opaque, stable identifier advertised in the TXT `id` key.
    pub id: String,
    /// Human-friendly name ("Brave Otter"), remembered by Home Assistant.
    pub name: String,
    /// Human-facing model advertised in TXT `model`.
    pub model: String,
    /// Daemon version advertised in TXT `version`.
    pub version: String,
}

impl DeviceIdentity {
    /// Generate a fresh identity with OS-provided randomness.
    ///
    /// `model` and `version` are supplied by the caller so the library stays
    /// free of environment/build assumptions.
    ///
    /// Returns an error if the OS random source is unavailable.
    pub fn generate(model: impl Into<String>, version: impl Into<String>) -> Result<Self> {
        let id = crate::random::hex(ID_BYTES).context("generating device id")?;
        let name = crate::names::generate().context("generating device name")?;
        Ok(Self {
            id,
            name,
            model: model.into(),
            version: version.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_stable_shaped_identity() {
        let a = DeviceIdentity::generate("Screensight Studio", "0.1.0").unwrap();
        let b = DeviceIdentity::generate("Screensight Studio", "0.1.0").unwrap();

        assert_eq!(a.id.len(), ID_BYTES * 2);
        assert!(a.id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!crate::names::is_legacy(&a.name));
        assert_eq!(a.name.split(' ').count(), 2);
        assert_eq!(a.model, "Screensight Studio");
        assert_eq!(a.version, "0.1.0");

        // Entropy: two independent ids must not collide. Names are drawn from a
        // small word list, so a repeat is possible and deliberately not asserted.
        assert_ne!(a.id, b.id);
    }
}
