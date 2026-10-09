//! Plaintext reference for the gates, written with BigInt arithmetic only. It shares no code with
//! `Ring` or `GateKind::reference`, so a bug in the crate's signed conversions cannot cancel out.

use gc_gates::{GateKind, U256};
use num_bigint::BigInt;

/// `1{x < 0}` or `floor(x / 2^s)` for the n-bit two's complement value `x`, as a ring element.
pub fn bigint_reference(kind: GateKind, n: u32, x: U256) -> U256 {
    let modulus = BigInt::from(1u8) << n as usize;
    let unsigned: BigInt = x.to_string().parse().unwrap();
    assert!(unsigned < modulus, "x = {x} is not reduced mod 2^{n}");
    let signed = if unsigned >= &modulus >> 1usize { unsigned - &modulus } else { unsigned };
    let out = match kind {
        GateKind::Lt0 => BigInt::from((signed < BigInt::from(0)) as u8),
        GateKind::Ars { shift } => {
            let d = BigInt::from(1u8) << shift as usize;
            // Floor division: `/` truncates toward zero, so step down for inexact negatives.
            let q = &signed / &d;
            if &q * &d != signed && signed < BigInt::from(0) {
                q - 1
            } else {
                q
            }
        }
    };
    let reduced = ((out % &modulus) + &modulus) % &modulus;
    reduced.to_string().parse().unwrap()
}

#[test]
fn reference_spot_checks() {
    let u = |v: u128| U256::new(v);
    // 8-bit ring: 0xF3 = -13. -13 >> 2 = floor(-3.25) = -4 = 0xFC.
    assert_eq!(bigint_reference(GateKind::Ars { shift: 2 }, 8, u(0xF3)), u(0xFC));
    assert_eq!(bigint_reference(GateKind::Ars { shift: 2 }, 8, u(13)), u(3));
    assert_eq!(bigint_reference(GateKind::Ars { shift: 2 }, 8, u(0xF4)), u(0xFD)); // -12 >> 2 = -3, exact
    assert_eq!(bigint_reference(GateKind::Lt0, 8, u(0x80)), u(1));
    assert_eq!(bigint_reference(GateKind::Lt0, 8, u(0x7F)), u(0));
    // 256-bit ring: -1 >> 255 = -1, and MIN >> 255 = -1, MAX >> 255 = 0.
    assert_eq!(bigint_reference(GateKind::Ars { shift: 255 }, 256, U256::MAX), U256::MAX);
    assert_eq!(bigint_reference(GateKind::Ars { shift: 255 }, 256, U256::ONE << 255), U256::MAX);
    assert_eq!(bigint_reference(GateKind::Ars { shift: 255 }, 256, (U256::ONE << 255) - U256::ONE), U256::ZERO);
}
