//! Pairing window and per-address rate limiting for the Noise XX handshake.
//!
//! Under OFF-220 the device no longer generates or verifies a code: the pairing
//! secret is the Noise XX handshake and the SAS derived from it (compared on the
//! Home Assistant side), not a value the device could leak. What remains here is
//! the *window* — pairing is only possible while it is open — and the per-IP
//! lockout that keeps unauthenticated handshake attempts from exhausting device
//! resources.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

/// How long a freshly armed window stays open.
pub const WINDOW: Duration = Duration::from_secs(180);
/// Failed handshake attempts from one address before it is locked out.
pub const MAX_FAILURES_PER_IP: u32 = 5;
/// How long an address is locked out after too many failures.
pub const LOCKOUT: Duration = Duration::from_secs(30);

/// Per-address failure bookkeeping.
#[derive(Debug, Default)]
struct Failures {
    count: u32,
    locked_until: Option<Instant>,
}

/// Manages the pairing window and its per-address attempt budget.
#[derive(Debug, Default)]
pub struct PairingManager {
    opened_at: Option<Instant>,
    failures: HashMap<IpAddr, Failures>,
}

impl PairingManager {
    /// A manager with no open window.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Open (or re-arm) the window and clear the attempt budget.
    pub fn arm(&mut self, now: Instant) {
        self.opened_at = Some(now);
        self.failures.clear();
    }

    /// Close the window.
    pub fn cancel(&mut self) {
        self.opened_at = None;
    }

    /// Whether a window is open at `now`.
    #[must_use]
    pub fn is_open(&self, now: Instant) -> bool {
        self.opened_at
            .is_some_and(|opened| now.saturating_duration_since(opened) < WINDOW)
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

    /// Record a failed handshake attempt from `ip`.
    ///
    /// Returns `Some(seconds)` when this attempt trips (or extends) the lockout,
    /// otherwise `None`.
    pub fn record_failure(&mut self, ip: IpAddr, now: Instant) -> Option<u64> {
        let failures = self.failures.entry(ip).or_default();
        failures.count += 1;
        if failures.count >= MAX_FAILURES_PER_IP {
            failures.count = 0;
            failures.locked_until = Some(now + LOCKOUT);
            return Some(LOCKOUT.as_secs());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(n: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, n))
    }

    #[test]
    fn arm_opens_window_and_expires() {
        let now = Instant::now();
        let mut mgr = PairingManager::new();
        assert!(!mgr.is_open(now));
        mgr.arm(now);
        assert!(mgr.is_open(now));
        assert!(!mgr.is_open(now + WINDOW + Duration::from_secs(1)));
    }

    #[test]
    fn repeated_failures_lock_out_per_address() {
        let now = Instant::now();
        let mut mgr = PairingManager::new();
        mgr.arm(now);

        for _ in 0..MAX_FAILURES_PER_IP - 1 {
            assert_eq!(mgr.record_failure(ip(1), now), None);
        }
        // The last failure trips the lockout.
        assert_eq!(mgr.record_failure(ip(1), now), Some(LOCKOUT.as_secs()));
        assert!(mgr.retry_after(ip(1), now).is_some());
        // A different address is unaffected.
        assert!(mgr.retry_after(ip(2), now).is_none());
        // The lockout expires.
        assert!(mgr
            .retry_after(ip(1), now + LOCKOUT + Duration::from_secs(1))
            .is_none());
    }

    #[test]
    fn cancel_and_rearm_clear_the_window_and_budget() {
        let now = Instant::now();
        let mut mgr = PairingManager::new();
        mgr.arm(now);
        for _ in 0..MAX_FAILURES_PER_IP {
            mgr.record_failure(ip(1), now);
        }
        mgr.cancel();
        assert!(!mgr.is_open(now));
        mgr.arm(now);
        assert!(mgr.is_open(now));
        assert!(mgr.retry_after(ip(1), now).is_none());
    }
}
