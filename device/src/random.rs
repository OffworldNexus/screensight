//! OS-randomness helpers used for identity, pairing codes and tokens.

use anyhow::Result;

/// Draw `n` bytes from the OS entropy pool and hex-encode them.
pub fn hex(n: usize) -> Result<String> {
    let mut buf = vec![0u8; n];
    fill(&mut buf)?;
    let mut out = String::with_capacity(n * 2);
    for byte in buf {
        use std::fmt::Write as _;
        write!(out, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(out)
}

/// Uniformly draw a `u32` in `0..modulus` using rejection sampling, so no
/// modulo bias creeps into the 6-digit pairing code.
pub fn u32_below(modulus: u32) -> Result<u32> {
    assert!(modulus > 0, "modulus must be positive");
    // Largest multiple of `modulus` that fits in a u32; values at or above it
    // are rejected and redrawn.
    let limit = (u32::MAX / modulus) * modulus;
    loop {
        let mut buf = [0u8; 4];
        fill(&mut buf)?;
        let value = u32::from_le_bytes(buf);
        if value < limit {
            return Ok(value % modulus);
        }
    }
}

/// Wrap `getrandom` so the error type never leaks across the crate boundary.
fn fill(buf: &mut [u8]) -> Result<()> {
    getrandom::fill(buf).map_err(|err| anyhow::anyhow!("OS random source: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_fixed_width_and_random() {
        let a = hex(8).unwrap();
        let b = hex(8).unwrap();
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn u32_below_stays_in_range() {
        for _ in 0..1000 {
            assert!(u32_below(1_000_000).unwrap() < 1_000_000);
        }
    }
}
