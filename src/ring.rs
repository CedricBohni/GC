//! The ring Z_{2^n} for 1 <= n <= 128, with elements stored in a `u128`.
//!
//! Elements are always kept reduced, i.e. in `[0, 2^n)`. Signed values use two's complement:
//! `x` represents `x - 2^n` when its MSB is set.

use crate::Error;
use serde::{Deserialize, Serialize};

pub const MAX_BITS: u32 = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ring {
    bits: u32,
}

impl Ring {
    pub fn new(bits: u32) -> Result<Ring, Error> {
        if bits == 0 || bits > MAX_BITS {
            return Err(Error::new(format!("ring bit width must be in 1..=128, got {bits}")));
        }
        Ok(Ring { bits })
    }

    pub fn bits(&self) -> u32 {
        self.bits
    }

    pub fn mask(&self) -> u128 {
        low_mask(self.bits)
    }

    pub fn reduce(&self, x: u128) -> u128 {
        x & self.mask()
    }

    pub fn add(&self, a: u128, b: u128) -> u128 {
        self.reduce(a.wrapping_add(b))
    }

    pub fn sub(&self, a: u128, b: u128) -> u128 {
        self.reduce(a.wrapping_sub(b))
    }

    pub fn neg(&self, a: u128) -> u128 {
        self.reduce(a.wrapping_neg())
    }

    pub fn mul(&self, a: u128, b: u128) -> u128 {
        self.reduce(a.wrapping_mul(b))
    }

    pub fn msb(&self, x: u128) -> bool {
        bit(x, self.bits - 1)
    }

    /// Smallest and largest signed value representable in n bits.
    pub fn signed_range(&self) -> (i128, i128) {
        if self.bits == 128 {
            (i128::MIN, i128::MAX)
        } else {
            (-(1i128 << (self.bits - 1)), (1i128 << (self.bits - 1)) - 1)
        }
    }

    /// Two's complement encoding of `x` (reduced mod 2^n; out-of-range values wrap).
    pub fn from_signed(&self, x: i128) -> u128 {
        self.reduce(x as u128)
    }

    pub fn to_signed(&self, x: u128) -> i128 {
        let x = self.reduce(x);
        if self.msb(x) {
            (x | !self.mask()) as i128
        } else {
            x as i128
        }
    }

    pub fn random(&self) -> u128 {
        self.reduce(rand::random::<u128>())
    }

    /// Split `x` into two uniformly random additive shares.
    pub fn share(&self, x: u128) -> (u128, u128) {
        let s0 = self.random();
        (s0, self.sub(x, s0))
    }
}

/// `2^k - 1` for 0 <= k <= 128.
pub fn low_mask(k: u32) -> u128 {
    if k >= 128 {
        u128::MAX
    } else {
        (1u128 << k) - 1
    }
}

/// Bit `i` of `x`.
pub fn bit(x: u128, i: u32) -> bool {
    (x >> i) & 1 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_roundtrip_extremes() {
        for bits in [1, 2, 7, 8, 63, 64, 65, 127, 128] {
            let r = Ring::new(bits).unwrap();
            let (lo, hi) = r.signed_range();
            for v in [lo, lo + 1, -1, 0, hi - 1, hi] {
                if v < lo || v > hi {
                    continue;
                }
                assert_eq!(r.to_signed(r.from_signed(v)), v, "bits={bits} v={v}");
            }
        }
    }

    #[test]
    fn rejects_bad_widths() {
        assert!(Ring::new(0).is_err());
        assert!(Ring::new(129).is_err());
    }
}
