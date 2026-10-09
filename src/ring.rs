//! The ring Z_{2^n} for 1 <= n <= 256, with elements stored in a [`U256`].
//!
//! Elements are always kept reduced, i.e. in `[0, 2^n)`. Signed values use two's complement:
//! `x` represents `x - 2^n` when its MSB is set.

use crate::Error;
pub use ethnum::{I256, U256};
use serde::{Deserialize, Serialize};

pub const MAX_BITS: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ring {
    bits: u32,
}

impl Ring {
    pub fn new(bits: u32) -> Result<Ring, Error> {
        if bits == 0 || bits > MAX_BITS {
            return Err(Error::new(format!("ring bit width must be in 1..={MAX_BITS}, got {bits}")));
        }
        Ok(Ring { bits })
    }

    pub fn bits(&self) -> u32 {
        self.bits
    }

    pub fn mask(&self) -> U256 {
        low_mask(self.bits)
    }

    pub fn reduce(&self, x: U256) -> U256 {
        x & self.mask()
    }

    pub fn add(&self, a: U256, b: U256) -> U256 {
        self.reduce(a.wrapping_add(b))
    }

    pub fn sub(&self, a: U256, b: U256) -> U256 {
        self.reduce(a.wrapping_sub(b))
    }

    pub fn neg(&self, a: U256) -> U256 {
        self.reduce(a.wrapping_neg())
    }

    pub fn mul(&self, a: U256, b: U256) -> U256 {
        self.reduce(a.wrapping_mul(b))
    }

    pub fn msb(&self, x: U256) -> bool {
        bit(x, self.bits - 1)
    }

    /// Smallest and largest signed value representable in n bits.
    pub fn signed_range(&self) -> (I256, I256) {
        if self.bits == MAX_BITS {
            (I256::MIN, I256::MAX)
        } else {
            (-(I256::ONE << (self.bits - 1)), (I256::ONE << (self.bits - 1)) - 1)
        }
    }

    /// Two's complement encoding of `x` (reduced mod 2^n; out-of-range values wrap).
    pub fn from_signed(&self, x: I256) -> U256 {
        self.reduce(x.as_u256())
    }

    pub fn to_signed(&self, x: U256) -> I256 {
        let x = self.reduce(x);
        if self.msb(x) {
            (x | !self.mask()).as_i256()
        } else {
            x.as_i256()
        }
    }

    pub fn random(&self) -> U256 {
        self.reduce(U256::from_words(rand::random(), rand::random()))
    }

    /// Split `x` into two uniformly random additive shares.
    pub fn share(&self, x: U256) -> (U256, U256) {
        let s0 = self.random();
        (s0, self.sub(x, s0))
    }

    /// 128-bit words per element on the channel: 1 for n <= 128, 2 above.
    pub fn words(&self) -> usize {
        if self.bits <= 128 {
            1
        } else {
            2
        }
    }

    /// `x` as [`Ring::words`] 128-bit words, low word first.
    pub fn to_words(&self, x: U256) -> Vec<u128> {
        let (hi, lo) = self.reduce(x).into_words();
        if self.words() == 1 {
            vec![lo]
        } else {
            vec![lo, hi]
        }
    }

    /// Inverse of [`Ring::to_words`]. Fails on a wrong length or a value of `2^n` or more.
    pub fn from_words(&self, words: &[u128]) -> Result<U256, Error> {
        let x = match *words {
            [lo] if self.words() == 1 => U256::new(lo),
            [lo, hi] if self.words() == 2 => U256::from_words(hi, lo),
            _ => {
                return Err(Error::new(format!(
                    "expected {} word(s) per {}-bit ring element, got {}",
                    self.words(),
                    self.bits,
                    words.len()
                )))
            }
        };
        if x & !self.mask() != 0 {
            return Err(Error::new(format!("value exceeds the {}-bit ring", self.bits)));
        }
        Ok(x)
    }
}

/// `2^k - 1` for 0 <= k <= 256.
pub fn low_mask(k: u32) -> U256 {
    if k >= 256 {
        U256::MAX
    } else {
        (U256::ONE << k) - 1
    }
}

/// Bit `i` of `x`.
pub fn bit(x: U256, i: u32) -> bool {
    (x >> i) & 1 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_roundtrip_extremes() {
        for bits in [1, 2, 7, 8, 63, 64, 65, 127, 128, 129, 200, 255, 256] {
            let r = Ring::new(bits).unwrap();
            let (lo, hi) = r.signed_range();
            for v in [lo, lo + 1, I256::MINUS_ONE, I256::ZERO, hi - 1, hi] {
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
        assert!(Ring::new(257).is_err());
    }

    #[test]
    fn words_roundtrip() {
        for bits in [1, 64, 128, 129, 200, 256] {
            let r = Ring::new(bits).unwrap();
            for x in [U256::ZERO, r.mask(), r.random()] {
                let w = r.to_words(x);
                assert_eq!(w.len(), r.words());
                assert_eq!(r.from_words(&w).unwrap(), x);
            }
        }
        let r = Ring::new(130).unwrap();
        assert!(r.from_words(&[0, 4]).is_err()); // bit 130 set
        assert!(r.from_words(&[0]).is_err()); // too short
    }
}
