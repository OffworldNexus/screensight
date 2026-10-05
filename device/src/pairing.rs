//! Pairing window: 6-digit code, constant-time verification, rate limiting.
//!
//! The code is generated on the device, drawn on the panel and verified on the
//! device; it is never written to the mDNS TXT record (see [`crate::mdns`]) and
//! never leaves the device except in the hash-free form the user typed into
//! Home Assistant. Failed attempts are rate-limited per source address.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use anyhow::Result;
use subtle::ConstantTimeEq;

use crate::random;

/// Number of digits in a pairing code.
pub const CODE_DIGITS: usize = 6;
/// Pairing codes are drawn uniformly from `0..CODE_MODULUS`.
pub const CODE_MODULUS: u32 = 1_000_000;
/// How long a freshly armed window stays open.
pub const WINDOW: Duration = Duration::from_secs(180);
/// Failed attempts from one address before it is locked out.
pub const MAX_FAILURES_PER_IP: u32 = 5;
/// How long an address is locked out after too many failures.
pub const LOCKOUT: Duration = Duration::from_secs(30);
/// Bytes of entropy behind a pairing token.
pub const TOKEN_BYTES: usize = 32;

/// Per-address failure bookkeeping.
#[derive(Debug, Default)]
struct Failures {
    count: u32,
    locked_until: Option<Instant>,
}

/// The result of submitting a pairing code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// Code accepted; the device is waiting for the on-panel confirmation.
    Pending,
    /// Code rejected; `attempts_left` before this address is locked out.
    InvalidCode { attempts_left: u32 },
    /// This address is locked out for `retry_after_secs`.
    RateLimited { retry_after_secs: u64 },
    /// No window is open (or it expired).
    WindowClosed,
}

/// Manages the pairing code and its lifetime.
#[derive(Debug, Default)]
pub struct PairingManager {
    code: Option<String>,
    opened_at: Option<Instant>,
    failures: HashMap<IpAddr, Failures>,
}

impl PairingManager {
    /// A manager with no open window.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Open (or re-arm) the window: generate a fresh code and reset limits.
    ///
    /// Returns the new code so the caller can draw it on the panel.
    pub fn arm(&mut self, now: Instant) -> Result<String> {
        let value = random::u32_below(CODE_MODULUS)?;
        let code = format!("{value:0width$}", width = CODE_DIGITS);
        self.code = Some(code.clone());
        self.opened_at = Some(now);
        self.failures.clear();
        Ok(code)
    }

    /// Close the window and forget the code.
    pub fn cancel(&mut self) {
        self.code = None;
        self.opened_at = None;
    }

    /// Whether a window is open at `now`.
    #[must_use]
    pub fn is_open(&self, now: Instant) -> bool {
        match (self.code.as_ref(), self.opened_at) {
            (Some(_), Some(opened)) => now.saturating_duration_since(opened) < WINDOW,
            _ => false,
        }
    }

    /// The current code, if a window is still open (for drawing on the panel).
    #[must_use]
    pub fn code(&self, now: Instant) -> Option<&str> {
        if self.is_open(now) {
            self.code.as_deref()
        } else {
            None
        }
    }

    /// Seconds until `ip` may retry, if it is currently locked out.
    #[must_use]
    pub fn retry_after(&self, ip: IpAddr, now: Instant) -> Option<u64> {
        let until = self.failures.get(&ip).and_then(|f| f.locked_until)?;
        if until > now {
            Some(until.saturating_duration_since(now).as_secs().max(1))
        } else {
            None
        }
    }

    /// Verify a submitted code, updating rate-limit state.
    pub fn submit(&mut self, ip: IpAddr, submitted: &str, now: Instant) -> SubmitOutcome {
        if !self.is_open(now) {
            return SubmitOutcome::WindowClosed;
        }
        if let Some(retry_after_secs) = self.retry_after(ip, now) {
            return SubmitOutcome::RateLimited { retry_after_secs };
        }

        let expected = self.code.as_deref().expect("open window has a code");
        if codes_equal(expected, submitted) {
            self.failures.remove(&ip);
            return SubmitOutcome::Pending;
        }

        let failures = self.failures.entry(ip).or_default();
        failures.count += 1;
        if failures.count >= MAX_FAILURES_PER_IP {
            failures.count = 0;
            failures.locked_until = Some(now + LOCKOUT);
            return SubmitOutcome::RateLimited {
                retry_after_secs: LOCKOUT.as_secs(),
            };
        }
        SubmitOutcome::InvalidCode {
            attempts_left: MAX_FAILURES_PER_IP - failures.count,
        }
    }

    /// Mint a long-lived token for a completed pairing.
    pub fn mint_token(&self) -> Result<String> {
        random::hex(TOKEN_BYTES)
    }
}

/// Constant-time string comparison for the pairing code.
fn codes_equal(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(n: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, n))
    }

    #[test]
    fn arm_opens_window_and_code_has_six_digits() {
        let now = Instant::now();
        let mut mgr = PairingManager::new();
        assert!(!mgr.is_open(now));
        let code = mgr.arm(now).unwrap();
        assert_eq!(code.len(), CODE_DIGITS);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
        assert!(mgr.is_open(now));
        assert_eq!(mgr.code(now), Some(code.as_str()));
    }

    #[test]
    fn window_expires() {
        let now = Instant::now();
        let mut mgr = PairingManager::new();
        mgr.arm(now).unwrap();
        assert!(!mgr.is_open(now + WINDOW + Duration::from_secs(1)));
        assert_eq!(mgr.code(now + WINDOW + Duration::from_secs(1)), None);
    }

    #[test]
    fn correct_code_pends_then_token_is_minted() {
        let now = Instant::now();
        let mut mgr = PairingManager::new();
        let code = mgr.arm(now).unwrap();
        assert_eq!(mgr.submit(ip(1), &code, now), SubmitOutcome::Pending);
        let token = mgr.mint_token().unwrap();
        assert_eq!(token.len(), TOKEN_BYTES * 2);
    }

    #[test]
    fn wrong_code_is_counted_and_then_locked_out() {
        let now = Instant::now();
        let mut mgr = PairingManager::new();
        mgr.arm(now).unwrap();
        for expected_left in (1..MAX_FAILURES_PER_IP).rev() {
            assert_eq!(
                mgr.submit(ip(1), "000000", now),
                SubmitOutcome::InvalidCode {
                    attempts_left: expected_left
                }
            );
        }
        // One more failure trips the lockout.
        assert!(matches!(
            mgr.submit(ip(1), "000000", now),
            SubmitOutcome::RateLimited { .. }
        ));
        assert!(matches!(
            mgr.submit(ip(1), "999999", now),
            SubmitOutcome::RateLimited { .. }
        ));
        // A different address is unaffected.
        assert!(matches!(
            mgr.submit(ip(2), "000000", now),
            SubmitOutcome::InvalidCode { .. }
        ));
        // Lockout expires.
        assert!(matches!(
            mgr.submit(ip(1), "000000", now + LOCKOUT + Duration::from_secs(1)),
            SubmitOutcome::InvalidCode { .. }
        ));
    }

    #[test]
    fn submit_without_window_is_closed() {
        let now = Instant::now();
        let mut mgr = PairingManager::new();
        assert_eq!(
            mgr.submit(ip(1), "123456", now),
            SubmitOutcome::WindowClosed
        );
    }

    #[test]
    fn cancel_closes_window_and_rearm_clears_lockout() {
        let now = Instant::now();
        let mut mgr = PairingManager::new();
        mgr.arm(now).unwrap();
        for _ in 0..MAX_FAILURES_PER_IP {
            mgr.submit(ip(1), "000000", now);
        }
        mgr.cancel();
        assert!(!mgr.is_open(now));
        mgr.arm(now).unwrap();
        assert!(mgr.retry_after(ip(1), now).is_none());
    }
}
