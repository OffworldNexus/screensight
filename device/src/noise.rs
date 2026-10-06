//! Noise Protocol transport for the Home Assistant link.
//!
//! The device is always the Noise **responder**: Home Assistant dials `/ws` and
//! initiates. Pairing uses `Noise_XX` (no prior keys; both static keys are
//! exchanged and mutually authenticated by the handshake). Steady state uses
//! `Noise_IK` (Home Assistant already knows the device's static key, so the
//! device authenticates in one round trip and identifies the peer by the
//! initiator's static key).
//!
//! Every Noise message travels as exactly one WebSocket **binary** frame; the
//! application JSON lives encrypted inside. The pattern is negotiated by the
//! `Sec-WebSocket-Protocol` subprotocol before the upgrade (see
//! [`crate::server`]) rather than by a framing byte.
//!
//! The pairing SAS is derived from the final Noise handshake hash, which binds
//! the ephemeral keys on both legs: an active MITM that terminates and
//! re-originates the handshake necessarily produces a different SAS on each leg,
//! so the panel and Home Assistant disagree and pairing fails.

use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use snow::params::NoiseParams;
use snow::{Builder, HandshakeState, Keypair, TransportState};

/// Protocol name for first-contact pairing.
pub const XX_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_SHA256";
/// Protocol name for remembered-key reconnects.
pub const IK_PATTERN: &str = "Noise_IK_25519_ChaChaPoly_SHA256";

/// Subprotocol token Home Assistant sends to request XX pairing.
pub const SUBPROTOCOL_XX: &str = "screensight.noise.xx";
/// Subprotocol token Home Assistant sends to request IK steady state.
pub const SUBPROTOCOL_IK: &str = "screensight.noise.ik";

/// Domain-separation prefix mixed into the handshake hash before the SAS digit
/// extraction, so the same hash can never be reused as a code elsewhere.
const SAS_DOMAIN: &[u8] = b"screensight/pairing-sas/v1";
/// Number of decimal digits in a SAS.
pub const SAS_DIGITS: usize = 8;
/// SAS values are drawn uniformly from `0..SAS_MODULUS`.
pub const SAS_MODULUS: u32 = 100_000_000;

/// How long a peer may stall a single handshake step before it is dropped.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Largest handshake message we will ever need to buffer. XX/IK 25519 messages
/// are well under 200 bytes; this is generous head-room.
const MAX_HANDSHAKE_MSG: usize = 1024;
/// Largest transport message: the Noise protocol's own maximum.
const MAX_TRANSPORT_MSG: usize = 65535;

/// Which Noise pattern a connection negotiated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// First contact: neither side knows the other's static key.
    Xx,
    /// Steady state: Home Assistant already knows the device's static key.
    Ik,
}

impl Mode {
    /// Parse the subprotocol token's mode, if it is one we speak.
    #[must_use]
    pub fn from_subprotocol(token: &str) -> Option<Self> {
        match token {
            SUBPROTOCOL_XX => Some(Self::Xx),
            SUBPROTOCOL_IK => Some(Self::Ik),
            _ => None,
        }
    }

    /// The subprotocol token for this mode.
    #[must_use]
    pub fn subprotocol(self) -> &'static str {
        match self {
            Self::Xx => SUBPROTOCOL_XX,
            Self::Ik => SUBPROTOCOL_IK,
        }
    }

    /// The Noise parameter string for this mode.
    #[must_use]
    pub fn params(self) -> NoiseParams {
        let name = match self {
            Self::Xx => XX_PATTERN,
            Self::Ik => IK_PATTERN,
        };
        NoiseParams::from_str(name).expect("built-in Noise pattern names are valid")
    }
}

/// Generate a fresh X25519 static keypair for Noise.
pub fn generate_keypair() -> Result<Keypair> {
    Builder::new(Mode::Xx.params())
        .generate_keypair()
        .context("generating a Noise static keypair")
}

/// Lower-case hex encoding, used to persist keys in SQLite and advertise the
/// device's public key over mDNS.
#[must_use]
pub fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(out, "{byte:02x}").expect("writing to a String cannot fail");
    }
    out
}

/// Decode a lower-case hex string back to bytes.
pub fn from_hex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        anyhow::bail!("hex string has an odd length");
    }
    (0..text.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&text[i..i + 2], 16)
                .with_context(|| format!("invalid hex byte at offset {i}"))
        })
        .collect()
}

/// Derive the 8-digit pairing SAS from the final Noise handshake hash.
///
/// `SHA256(SAS_DOMAIN || handshake_hash)` is mapped uniformly into
/// `0..SAS_MODULUS` by rejection sampling over big-endian 4-byte windows: a
/// window at or above the largest multiple of the modulus below `2^32` is
/// skipped. This avoids modulo bias while staying trivially portable to
/// `hashlib`/`noiseprotocol` on the Home Assistant side.
#[must_use]
pub fn sas_from_handshake_hash(handshake_hash: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(SAS_DOMAIN);
    hasher.update(handshake_hash);
    let digest = hasher.finalize();

    // `u32::MAX / SAS_MODULUS * SAS_MODULUS` — the largest unbiased multiple.
    let limit = (u32::MAX / SAS_MODULUS) * SAS_MODULUS;
    for window in digest.as_slice().as_chunks::<4>().0 {
        let value = u32::from_be_bytes([window[0], window[1], window[2], window[3]]);
        if value < limit {
            return format!("{:0width$}", value % SAS_MODULUS, width = SAS_DIGITS);
        }
    }
    // 8 windows all landing in the rejected (2.2%) slice is ~10^-68: unreachable
    // in practice, but still produce a well-formed code rather than panicking.
    let value = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    format!("{:0width$}", value % SAS_MODULUS, width = SAS_DIGITS)
}

/// One side of a Noise handshake, before transport mode is entered.
pub struct Handshake {
    state: HandshakeState,
    mode: Mode,
}

impl Handshake {
    /// Build a responder handshake with our static private key.
    ///
    /// The peer's static key is learned during the handshake (XX) or from the
    /// first message (IK); it is not required up front.
    pub fn responder(mode: Mode, local_private_key: &[u8]) -> Result<Self> {
        let state = Builder::new(mode.params())
            .local_private_key(local_private_key)
            .context("setting the Noise static private key")?
            .build_responder()
            .context("building the Noise responder")?;
        Ok(Self { state, mode })
    }

    /// Build an initiator handshake. Used by the integration tests and mirrors
    /// what Home Assistant does with `noiseprotocol`.
    pub fn initiator(
        mode: Mode,
        local_private_key: &[u8],
        remote_public_key: Option<&[u8]>,
    ) -> Result<Self> {
        let mut builder = Builder::new(mode.params())
            .local_private_key(local_private_key)
            .context("setting the Noise static private key")?;
        if let Some(remote) = remote_public_key {
            builder = builder
                .remote_public_key(remote)
                .context("setting the Noise remote static key")?;
        }
        let state = builder
            .build_initiator()
            .context("building the Noise initiator")?;
        Ok(Self { state, mode })
    }

    /// Which pattern this handshake is running.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Read one handshake message from the peer, returning any plaintext.
    pub fn read(&mut self, message: &[u8]) -> Result<Vec<u8>> {
        let mut payload = vec![0u8; MAX_HANDSHAKE_MSG];
        let len = self
            .state
            .read_message(message, &mut payload)
            .context("reading a Noise handshake message")?;
        payload.truncate(len);
        Ok(payload)
    }

    /// Write the next handshake message, optionally carrying `payload`.
    pub fn write(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
        let mut message = vec![0u8; MAX_HANDSHAKE_MSG];
        let len = self
            .state
            .write_message(payload, &mut message)
            .context("writing a Noise handshake message")?;
        message.truncate(len);
        Ok(message)
    }

    /// Whether the handshake has exchanged everything it needs.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.state.is_handshake_finished()
    }

    /// The peer's static public key, once the handshake has revealed it.
    #[must_use]
    pub fn peer_static(&self) -> Option<Vec<u8>> {
        self.state.get_remote_static().map(<[u8]>::to_vec)
    }

    /// Enter transport mode, producing the session and the pairing SAS.
    pub fn into_session(self) -> Result<NoiseSession> {
        let sas = sas_from_handshake_hash(self.state.get_handshake_hash());
        let peer_static = self.state.get_remote_static().map(<[u8]>::to_vec);
        let transport = self
            .state
            .into_transport_mode()
            .context("entering Noise transport mode")?;
        Ok(NoiseSession {
            transport,
            peer_static,
            sas,
        })
    }
}

/// An established Noise transport session.
pub struct NoiseSession {
    /// The symmetric transport state used to encrypt and decrypt frames.
    pub transport: TransportState,
    /// The initiator's static public key, if the pattern revealed it.
    pub peer_static: Option<Vec<u8>>,
    /// The pairing SAS derived from the handshake hash (8 digits).
    pub sas: String,
}

impl NoiseSession {
    /// Encrypt one application frame.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut message = vec![0u8; MAX_TRANSPORT_MSG];
        let len = self
            .transport
            .write_message(plaintext, &mut message)
            .context("encrypting a Noise transport frame")?;
        message.truncate(len);
        Ok(message)
    }

    /// Decrypt one application frame.
    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let mut payload = vec![0u8; MAX_TRANSPORT_MSG];
        let len = self
            .transport
            .read_message(ciphertext, &mut payload)
            .context("decrypting a Noise transport frame")?;
        payload.truncate(len);
        Ok(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a full XX handshake and returns both sessions.
    fn xx_pair() -> (NoiseSession, NoiseSession) {
        let device = generate_keypair().unwrap();
        let ha = generate_keypair().unwrap();

        let mut responder = Handshake::responder(Mode::Xx, &device.private).unwrap();
        let mut initiator = Handshake::initiator(Mode::Xx, &ha.private, None).unwrap();

        let m1 = initiator.write(&[]).unwrap();
        responder.read(&m1).unwrap();
        let m2 = responder.write(&[]).unwrap();
        initiator.read(&m2).unwrap();
        let m3 = initiator.write(&[]).unwrap();
        responder.read(&m3).unwrap();

        assert!(initiator.is_finished());
        assert!(responder.is_finished());
        (
            initiator.into_session().unwrap(),
            responder.into_session().unwrap(),
        )
    }

    #[test]
    fn xx_handshake_authenticates_both_static_keys() {
        let device = generate_keypair().unwrap();
        let ha = generate_keypair().unwrap();

        let mut responder = Handshake::responder(Mode::Xx, &device.private).unwrap();
        let mut initiator = Handshake::initiator(Mode::Xx, &ha.private, None).unwrap();

        let m1 = initiator.write(&[]).unwrap();
        responder.read(&m1).unwrap();
        let m2 = responder.write(&[]).unwrap();
        initiator.read(&m2).unwrap();
        let m3 = initiator.write(&[]).unwrap();
        responder.read(&m3).unwrap();

        assert_eq!(responder.peer_static().unwrap(), ha.public);
        assert_eq!(initiator.peer_static().unwrap(), device.public);
    }

    #[test]
    fn ik_handshake_authenticates_the_initiator() {
        let device = generate_keypair().unwrap();
        let ha = generate_keypair().unwrap();

        let mut responder = Handshake::responder(Mode::Ik, &device.private).unwrap();
        let mut initiator =
            Handshake::initiator(Mode::Ik, &ha.private, Some(&device.public)).unwrap();

        let m1 = initiator.write(&[]).unwrap();
        responder.read(&m1).unwrap();
        let m2 = responder.write(&[]).unwrap();
        initiator.read(&m2).unwrap();

        assert!(initiator.is_finished());
        assert!(responder.is_finished());
        assert_eq!(responder.peer_static().unwrap(), ha.public);

        let mut initiator = initiator.into_session().unwrap();
        let mut responder = responder.into_session().unwrap();

        // Transport frames round-trip and are not plaintext.
        let ciphertext = initiator.encrypt(b"{\"type\":\"ping\"}").unwrap();
        assert_ne!(ciphertext, b"{\"type\":\"ping\"}");
        let plaintext = responder.decrypt(&ciphertext).unwrap();
        assert_eq!(plaintext, b"{\"type\":\"ping\"}");
    }

    #[test]
    fn both_legs_derive_the_same_sas() {
        let (initiator, responder) = xx_pair();
        assert_eq!(initiator.sas, responder.sas);
        assert_eq!(initiator.sas.len(), SAS_DIGITS);
        assert!(initiator.sas.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn an_mitm_produces_mismatched_sas_values() {
        // A MITM terminates the XX handshake on each side: the panel legitimately
        // pairs with the MITM, while Home Assistant legitimately pairs with the
        // MITM too — but the two legs have different handshake hashes, so the SAS
        // the panel shows never matches what Home Assistant derived.
        let device = generate_keypair().unwrap();
        let ha = generate_keypair().unwrap();
        let mitm = generate_keypair().unwrap();

        // Leg 1: device <-> MITM (MITM acts as initiator).
        let mut device_side = Handshake::responder(Mode::Xx, &device.private).unwrap();
        let mut mitm_a = Handshake::initiator(Mode::Xx, &mitm.private, None).unwrap();
        let m1 = mitm_a.write(&[]).unwrap();
        device_side.read(&m1).unwrap();
        let m2 = device_side.write(&[]).unwrap();
        mitm_a.read(&m2).unwrap();
        let m3 = mitm_a.write(&[]).unwrap();
        device_side.read(&m3).unwrap();
        let device_leg = device_side.into_session().unwrap();

        // Leg 2: MITM <-> HA (MITM acts as responder).
        let mut mitm_b = Handshake::responder(Mode::Xx, &mitm.private).unwrap();
        let mut ha_side = Handshake::initiator(Mode::Xx, &ha.private, None).unwrap();
        let n1 = ha_side.write(&[]).unwrap();
        mitm_b.read(&n1).unwrap();
        let n2 = mitm_b.write(&[]).unwrap();
        ha_side.read(&n2).unwrap();
        let n3 = ha_side.write(&[]).unwrap();
        mitm_b.read(&n3).unwrap();
        let ha_leg = ha_side.into_session().unwrap();

        assert_ne!(device_leg.sas, ha_leg.sas);
    }

    #[test]
    fn sas_matches_the_shared_known_answer_vector() {
        // Locked in step with the Python `tests/test_noise.py` KAT; change only
        // together, or pairing silently diverges across languages.
        let hash: Vec<u8> = (0u8..32).collect();
        assert_eq!(sas_from_handshake_hash(&hash), "93704101");
    }

    #[test]
    fn sas_is_rejection_sampled_within_modulus() {
        for i in 0..2000u32 {
            let hash = i.to_be_bytes();
            let sas = sas_from_handshake_hash(&hash);
            assert_eq!(sas.len(), SAS_DIGITS);
            assert!(sas.parse::<u32>().unwrap() < SAS_MODULUS);
        }
    }

    #[test]
    fn generated_keypairs_are_32_bytes_and_distinct() {
        let a = generate_keypair().unwrap();
        let b = generate_keypair().unwrap();
        assert_eq!(a.public.len(), 32);
        assert_eq!(a.private.len(), 32);
        assert_ne!(a.public, b.public);
    }

    #[test]
    fn hex_round_trips() {
        let bytes: Vec<u8> = (0u8..=255).collect();
        let text = to_hex(&bytes);
        assert_eq!(text.len(), 512);
        assert_eq!(from_hex(&text).unwrap(), bytes);
        assert!(from_hex("abc").is_err());
        assert!(from_hex("zz").is_err());
    }
}
