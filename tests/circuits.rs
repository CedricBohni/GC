//! The Boolean circuits of Algorithms 2 and 3, run in plaintext with swanky's `Dummy` backend,
//! so circuit bugs show up separately from garbling or protocol bugs.

use fancy_plaintext::{Dummy, DummyVal};
use gc_gates::circuits::ShareConverted;
use gc_gates::gates::gen;
use gc_gates::ring::bit;
use gc_gates::{GateKind, Ring};

/// z1 = f(x0 + x1) + R, straight from the circuit.
fn run(kind: GateKind, ring: Ring, x0: u128, x1: u128, r: u128) -> u128 {
    let circuit = ShareConverted::new(kind, ring).unwrap();
    let n = ring.bits();
    let wires = [x0, x1, r]
        .iter()
        .flat_map(|&v| (0..n).map(move |j| DummyVal::new_bool(bit(v, j))))
        .collect();
    let out = Dummy::eval(&circuit, wires).unwrap();
    assert_eq!(out.len(), n as usize);
    out.iter().enumerate().fold(0, |z, (i, b)| z | ((b.val() as u128) << i))
}

fn check(kind: GateKind, ring: Ring, x0: u128, x1: u128, r: u128) {
    let want = ring.add(kind.reference(ring, ring.add(x0, x1)), r);
    assert_eq!(run(kind, ring, x0, x1, r), want, "{kind} n={} x0={x0} x1={x1} R={r}", ring.bits());
}

#[test]
fn plaintext_exhaustive_small_rings() {
    for n in 1..=5 {
        let ring = Ring::new(n).unwrap();
        let kinds = std::iter::once(GateKind::Lt0).chain((0..n).map(|shift| GateKind::Ars { shift }));
        for kind in kinds {
            for x0 in 0..=ring.mask() {
                for x1 in 0..=ring.mask() {
                    for r in 0..=ring.mask() {
                        check(kind, ring, x0, x1, r);
                    }
                }
            }
        }
    }
}

#[test]
fn plaintext_random_all_widths() {
    for n in 1..=128 {
        let ring = Ring::new(n).unwrap();
        for _ in 0..20 {
            let (x0, x1, r) = (ring.random(), ring.random(), ring.random());
            check(GateKind::Lt0, ring, x0, x1, r);
            check(GateKind::Ars { shift: rand::random_range(0..n) }, ring, x0, x1, r);
        }
    }
}

/// Half-gates: two ciphertexts per AND, 2(n-1) ANDs per gate, plus the two blocks per output
/// wire that fancy-garbling writes for decoding.
#[test]
fn garbled_size_matches_and_count() {
    for n in [1, 2, 3, 16, 64, 127, 128] {
        let ring = Ring::new(n).unwrap();
        for kind in [GateKind::Lt0, GateKind::Ars { shift: n / 2 }] {
            let circuit = ShareConverted::new(kind, ring).unwrap();
            assert_eq!(circuit.and_count(), 2 * (n as usize - 1));
            let (_, bob) = gen(kind, ring).unwrap();
            assert_eq!(bob.garbled_blocks(), 2 * circuit.and_count() + 2 * n as usize, "{kind} n={n}");
        }
    }
}
