//! Stable device identity and the high-entropy mDNS instance name.
//!
//! The instance name is what Home Assistant remembers as the device identity
//! and what the integration re-resolves over mDNS when the device's IP changes;
//! the opaque `id` is what travels in the zeroconf TXT record. Both are
//! generated once on first boot and persisted, so the pairing survives reboots
//! and DHCP changes.
//!
//! Neither value contains the pairing code; see [`crate::pairing`].

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Number of random bytes behind the opaque device id.
const ID_BYTES: usize = 8;
/// Number of random bytes used for the human-readable instance name.
const NAME_BYTES: usize = 4;

/// Everything Home Assistant needs to recognise and re-find this device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIdentity {
    /// Opaque, stable identifier advertised in the TXT `id` key.
    pub id: String,
    /// High-entropy mDNS instance name (`screensight-<hex>`), remembered by HA.
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
        let suffix = crate::random::hex(NAME_BYTES).context("generating instance name")?;
        Ok(Self {
            id,
            name: format!("screensight-{suffix}"),
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
        assert!(a.name.starts_with("screensight-"));
        assert_eq!(a.name.len(), "screensight-".len() + NAME_BYTES * 2);
        assert_eq!(a.model, "Screensight Studio");
        assert_eq!(a.version, "0.1.0");

        // Entropy: two independent draws must not collide.
        assert_ne!(a.id, b.id);
        assert_ne!(a.name, b.name);
    }
}
