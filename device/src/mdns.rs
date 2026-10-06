//! mDNS/DNS-SD advertisement via Avahi over D-Bus.
//!
//! Avahi already owns UDP/5353 on the device, so rather than run a second
//! responder (which would fight it) we ask Avahi to publish a `_screensight._tcp`
//! service with our TXT records. The pairing secret is the Noise handshake and
//! its derived SAS, so the TXT payload carries only the device's *public* static
//! key (`key=`) and a `noise=1` marker — never the SAS or any private key.
//!
//! When the pairing window opens/closes the `pairing=1` key must appear or
//! disappear; Avahi has no reliable in-place TXT edit across versions, so we
//! free the entry group and publish a fresh one. That is cheap and rare.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use zbus::zvariant::OwnedObjectPath;

use crate::identity::DeviceIdentity;
use crate::runtime::Runtime;

/// Avahi's well-known D-Bus name.
const AVAHI: &str = "org.freedesktop.Avahi";
/// The Avahi server object.
const SERVER_PATH: &str = "/";
/// The Avahi server interface (`EntryGroupNew`).
const SERVER_IFACE: &str = "org.freedesktop.Avahi.Server";
/// The per-service entry-group interface.
const GROUP_IFACE: &str = "org.freedesktop.Avahi.EntryGroup";
/// Avahi's `AVAHI_IF_UNSPEC` / `AVAHI_PROTO_UNSPEC` sentinels.
const IF_UNSPEC: i32 = -1;
const PROTO_UNSPEC: i32 = -1;

/// DNS-SD service type Home Assistant discovers on.
pub const SERVICE_TYPE: &str = "_screensight._tcp";
/// Protocol version advertised in TXT `api`.
pub const API_VERSION: u32 = 1;
/// How often we reconcile the advertised `pairing` flag.
const RECONCILE: Duration = Duration::from_secs(2);

/// Build the Avahi TXT payload (`aay`) for the current state.
///
/// The keys are `id`, `model`, `api`, `version`, the device's static public key
/// (`key`, public by definition) and the Noise marker (`noise=1`), plus
/// `pairing=1` while the window is open. The SAS, any private key and any
/// pairing secret are never advertised.
#[must_use]
pub fn txt_records(identity: &DeviceIdentity, public_key_hex: &str, pairing: bool) -> Vec<Vec<u8>> {
    let mut records = vec![
        ("id", identity.id.clone()),
        ("model", identity.model.clone()),
        ("api", API_VERSION.to_string()),
        ("version", identity.version.clone()),
        ("noise", "1".to_owned()),
        ("key", public_key_hex.to_owned()),
    ];
    if pairing {
        records.push(("pairing", "1".to_owned()));
    }
    records
        .into_iter()
        .map(|(key, value)| format!("{key}={value}").into_bytes())
        .collect()
}

/// Publish the service and keep the `pairing` flag in sync forever.
///
/// Returns an error only if Avahi cannot be reached at startup; failures to
/// publish later are logged and retried, never fatal.
pub async fn advertise(runtime: Arc<Runtime>, port: u16) -> Result<()> {
    let connection = zbus::Connection::system()
        .await
        .context("connecting to the system D-Bus (is avahi-daemon running?)")?;
    let server = zbus::proxy::Builder::new(&connection)
        .destination(AVAHI)?
        .path(SERVER_PATH)?
        .interface(SERVER_IFACE)?
        .build()
        .await?;

    let mut published: Option<(OwnedObjectPath, bool)> = None;
    loop {
        let pairing = runtime.pairing_open(Instant::now());
        if published.as_ref().map(|(_, flag)| *flag) != Some(pairing) {
            if let Some((path, _)) = published.take() {
                free_group(&connection, &path).await;
            }
            match publish_group(&connection, &server, &runtime, port, pairing).await {
                Ok(path) => {
                    log::info!(
                        "advertising {} as {}:{} (pairing={})",
                        SERVICE_TYPE,
                        runtime.identity().name,
                        port,
                        pairing
                    );
                    published = Some((path, pairing));
                }
                Err(err) => log::warn!("mDNS publish failed (will retry): {err:#}"),
            }
        }
        tokio::time::sleep(RECONCILE).await;
    }
}

async fn publish_group(
    connection: &zbus::Connection,
    server: &zbus::Proxy<'_>,
    runtime: &Runtime,
    port: u16,
    pairing: bool,
) -> Result<OwnedObjectPath> {
    let path: OwnedObjectPath = server
        .call("EntryGroupNew", &())
        .await
        .context("calling EntryGroupNew")?;
    let group: zbus::Proxy<'_> = zbus::proxy::Builder::new(connection)
        .destination(AVAHI)?
        .path(path.as_str())?
        .interface(GROUP_IFACE)?
        .build()
        .await?;

    let identity = runtime.identity();
    let txt = txt_records(&identity, &runtime.public_key_hex(), pairing);
    let _: () = group
        .call(
            "AddService",
            &(
                IF_UNSPEC,
                PROTO_UNSPEC,
                0u32,
                identity.name.as_str(),
                SERVICE_TYPE,
                "",
                "",
                port,
                txt,
            ),
        )
        .await
        .context("calling EntryGroup.AddService")?;
    let _: () = group
        .call("Commit", &())
        .await
        .context("calling EntryGroup.Commit")?;
    Ok(path)
}

async fn free_group(connection: &zbus::Connection, path: &OwnedObjectPath) {
    let Ok(group) = zbus::proxy::Builder::new(connection)
        .destination(AVAHI)
        .and_then(|b| b.path(path.as_str()))
        .and_then(|b| b.interface(GROUP_IFACE))
    else {
        return;
    };
    let Ok(group): Result<zbus::Proxy<'_>, _> = group.build().await else {
        return;
    };
    let _: Result<(), _> = group.call("Free", &()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> DeviceIdentity {
        DeviceIdentity {
            id: "675725a50745891c".to_owned(),
            name: "screensight-17d98b22".to_owned(),
            model: "Screensight Studio".to_owned(),
            version: "0.1.0".to_owned(),
        }
    }

    fn keys(records: &[Vec<u8>]) -> Vec<String> {
        records
            .iter()
            .map(|r| String::from_utf8(r.clone()).unwrap())
            .collect()
    }

    fn public_key() -> String {
        "ab".repeat(32)
    }

    #[test]
    fn pairing_flag_present_only_while_open() {
        let open = keys(&txt_records(&identity(), &public_key(), true));
        assert!(open.iter().any(|r| r == "pairing=1"));
        let closed = keys(&txt_records(&identity(), &public_key(), false));
        assert!(!closed.iter().any(|r| r.starts_with("pairing=")));
    }

    #[test]
    fn advertises_noise_marker_and_public_key() {
        let records = keys(&txt_records(&identity(), &public_key(), false));
        assert!(records.iter().any(|r| r == "noise=1"));
        assert!(records
            .iter()
            .any(|r| r == &format!("key={}", public_key())));
        // api stays 1 until 1.0; only `noise=1` is added for compatibility.
        assert!(records.iter().any(|r| r == "api=1"));
    }

    #[test]
    fn txt_contains_only_allowlisted_keys() {
        let records = txt_records(&identity(), &public_key(), true);
        for record in keys(&records) {
            let key = record.split('=').next().unwrap();
            assert!(
                ["id", "model", "api", "version", "noise", "key", "pairing"].contains(&key),
                "unexpected TXT key: {key}"
            );
        }
    }

    #[test]
    fn pairing_secret_never_appears_in_txt() {
        // The TXT builder is never handed the SAS, so this asserts the
        // allowlist holds even for a representative panel code.
        let sas = "93704101";
        let records = txt_records(&identity(), &public_key(), true);
        for record in keys(&records) {
            assert!(!record.contains(sas), "SAS leaked into TXT: {record}");
        }
    }
}
