//! Signed fixed-point encoding: a real `v` is stored as `round(v * 2^frac)` in two's complement
//! in Z_{2^n}. Parsing and printing are exact decimal conversions, so they work for all widths
//! up to 128 bits.

use crate::ring::Ring;
use crate::Error;
use num_bigint::BigInt;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedPoint {
    ring: Ring,
    frac: u32,
}

impl FixedPoint {
    pub fn new(ring: Ring, frac: u32) -> Result<FixedPoint, Error> {
        if frac > ring.bits() {
            return Err(Error::new(format!("frac ({frac}) cannot exceed the ring width ({})", ring.bits())));
        }
        Ok(FixedPoint { ring, frac })
    }

    pub fn ring(&self) -> Ring {
        self.ring
    }

    pub fn frac(&self) -> u32 {
        self.frac
    }

    /// Parse an input value. Accepts a plain decimal such as `-3.25` (no exponent notation),
    /// which is scaled by `2^frac` and rounded to nearest (ties away from zero), or `raw:<int>`,
    /// which is taken as the signed ring integer itself.
    pub fn parse(&self, s: &str) -> Result<u128, Error> {
        let s = s.trim();
        let scaled = match s.strip_prefix("raw:") {
            Some(raw) => raw
                .parse::<BigInt>()
                .map_err(|_| Error::new(format!("`{s}` is not an integer")))?,
            None => self.scale_decimal(s)?,
        };
        let (lo, hi) = self.ring.signed_range();
        if scaled < BigInt::from(lo) || scaled > BigInt::from(hi) {
            return Err(Error::new(format!(
                "`{s}` encodes to {scaled}, outside the signed {}-bit range [{lo}, {hi}]",
                self.ring.bits()
            )));
        }
        let v: i128 = scaled.to_string().parse().expect("in i128 range");
        Ok(self.ring.from_signed(v))
    }

    fn scale_decimal(&self, s: &str) -> Result<BigInt, Error> {
        let bad = || Error::new(format!("`{s}` is not a decimal number (use e.g. -3.25 or raw:-52)"));
        let (neg, body) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        let (int_part, frac_part) = body.split_once('.').unwrap_or((body, ""));
        if int_part.is_empty() && frac_part.is_empty() {
            return Err(bad());
        }
        if !int_part.chars().chain(frac_part.chars()).all(|c| c.is_ascii_digit()) {
            return Err(bad());
        }
        // v = digits / 10^k; scaled = round(digits * 2^frac / 10^k).
        let digits: BigInt = format!("0{int_part}{frac_part}").parse().map_err(|_| bad())?;
        let num = digits << self.frac as usize;
        let den = BigInt::from(10u32).pow(frac_part.len() as u32);
        let mut q = &num / &den;
        if (&num % &den) * 2u32 >= den {
            q += 1u32;
        }
        Ok(if neg { -q } else { q })
    }

    /// Exact decimal value of the ring element `x`.
    pub fn format(&self, x: u128) -> String {
        format_scaled(self.ring.to_signed(x), self.frac)
    }
}

/// Exact decimal string for `v / 2^frac`.
pub fn format_scaled(v: i128, frac: u32) -> String {
    let neg = v < 0;
    // v / 2^f = v * 5^f / 10^f, so the decimal expansion has at most f fractional digits.
    let digits = (BigInt::from(v.unsigned_abs()) * BigInt::from(5u32).pow(frac)).to_string();
    let f = frac as usize;
    let padded = format!("{digits:0>width$}", width = f + 1);
    let (int_part, frac_part) = padded.split_at(padded.len() - f);
    let frac_part = frac_part.trim_end_matches('0');
    let sign = if neg { "-" } else { "" };
    if frac_part.is_empty() {
        format!("{sign}{int_part}")
    } else {
        format!("{sign}{int_part}.{frac_part}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(bits: u32, frac: u32) -> FixedPoint {
        FixedPoint::new(Ring::new(bits).unwrap(), frac).unwrap()
    }

    #[test]
    fn parse_and_format() {
        let f = fp(16, 8);
        assert_eq!(f.ring().to_signed(f.parse("-3.25").unwrap()), -832);
        assert_eq!(f.format(f.parse("-3.25").unwrap()), "-3.25");
        assert_eq!(f.format(f.parse("raw:-1").unwrap()), "-0.00390625");
        assert_eq!(f.ring().to_signed(f.parse("0.001953125").unwrap()), 1); // exactly 0.5 LSB, rounds up
        assert_eq!(f.ring().to_signed(f.parse("-0.001953125").unwrap()), -1);
        assert!(f.parse("128").is_err());
        assert!(f.parse("-128").is_ok());
        assert!(f.parse("abc").is_err());
    }

    #[test]
    fn wide_rings_are_exact() {
        let f = fp(128, 100);
        let min = f.parse("raw:-170141183460469231731687303715884105728").unwrap();
        assert_eq!(f.ring().to_signed(min), i128::MIN);
        assert_eq!(f.format(f.parse("-0.75").unwrap()), "-0.75");
    }
}
