//! Gates whose operand ring Z_2^w and output ring Z_2^r differ: r > w, r = w and r < w.

use fancy_plaintext::{Dummy, DummyVal};
use gc_gates::circuits::ShareConverted;
use gc_gates::gates::{gen, EvaluatorKey};
use gc_gates::online::eval_shared;
use gc_gates::ring::bit;
use gc_gates::transport::LocalChannel;
use gc_gates::{deal, simulate, GateKind, GateRings, PartyKey, Ring, I256, U256};
use std::collections::HashSet;

mod common;
use common::bigint_reference_widths;

fn rings(w: u32, r: u32) -> GateRings {
    GateRings { input: Ring::new(w).unwrap(), output: Ring::new(r).unwrap() }
}

/// The shapes the satellite-collision evaluator uses, plus the extremes.
const SHAPES: [(u32, u32); 8] = [(26, 149), (27, 150), (116, 64), (53, 122), (151, 151), (1, 256), (256, 1), (256, 256)];

fn all_kinds(w: u32) -> Vec<GateKind> {
    std::iter::once(GateKind::Lt0).chain((0..w).map(|shift| GateKind::Ars { shift })).collect()
}

fn check(kind: GateKind, g: GateRings, x: U256) {
    let (w, r) = (g.input.bits(), g.output.bits());
    let want = bigint_reference_widths(kind, w, r, x);
    assert_eq!(kind.reference(g, x), want, "GateKind::reference disagrees with BigInt: {kind} w={w} r={r} x={x}");
    let got = simulate(kind, g, x).unwrap();
    assert_eq!(got, want, "{kind} w={w} r={r} x={} (raw {x}): got {got}, want {want}", g.input.to_signed(x));
}

/// Every w in 1..=6, r in 1..=8, every x, both gates, every shift, fresh keys each time.
#[test]
fn exhaustive_small_widths() {
    for w in 1..=6 {
        for r in 1..=8 {
            let g = rings(w, r);
            for kind in all_kinds(w) {
                for x in (0..=g.input.mask().as_u128()).map(U256::new) {
                    for _ in 0..3 {
                        check(kind, g, x);
                    }
                }
            }
        }
    }
}

/// The circuit alone, in plaintext: every x0, x1 and R for small mixed widths.
#[test]
fn plaintext_exhaustive_mixed_widths() {
    for w in 1..=4 {
        for r in 1..=5 {
            let g = rings(w, r);
            for kind in all_kinds(w) {
                let circuit = ShareConverted::new(kind, g).unwrap();
                let all = |ring: Ring| (0..=ring.mask().as_u128()).map(U256::new);
                for x0 in all(g.input) {
                    for x1 in all(g.input) {
                        for mask in all(g.output) {
                            let wires = [(x0, w), (x1, w), (mask, r)]
                                .iter()
                                .flat_map(|&(v, n)| (0..n).map(move |j| DummyVal::new_bool(bit(v, j))))
                                .collect();
                            let out = Dummy::eval(&circuit, wires).unwrap();
                            assert_eq!(out.len(), r as usize);
                            let z1 = out.iter().enumerate().fold(U256::ZERO, |z, (i, b)| z | (U256::from(b.val()) << i as u32));
                            let want = g.output.add(kind.reference(g, g.input.add(x0, x1)), mask);
                            assert_eq!(z1, want, "{kind} w={w} r={r} x0={x0} x1={x1} R={mask}");
                        }
                    }
                }
            }
        }
    }
}

/// The evaluator's shapes on 0, ±1, the extremes and random values.
#[test]
fn evaluator_shapes_edge_and_random_values() {
    for (w, r) in SHAPES {
        let g = rings(w, r);
        let (lo, hi) = g.input.signed_range();
        let edges = [lo, lo + 1, I256::MINUS_ONE, I256::ZERO, I256::ONE, hi - 1, hi].map(|v| g.input.from_signed(v));
        let randoms = (0..10).map(|_| g.input.random());
        let mut kinds = vec![GateKind::Lt0];
        let shifts = [0, 1, w / 2, w.saturating_sub(2), w - 1, rand::random_range(0..w)];
        kinds.extend(shifts.into_iter().filter(|&s| s < w).map(|shift| GateKind::Ars { shift }));
        // `s >= r` is allowed while `s < w`: the result is the sign extension.
        if r < w {
            kinds.push(GateKind::Ars { shift: r });
        }
        for x in edges.into_iter().chain(randoms) {
            for &kind in &kinds {
                check(kind, g, x);
            }
        }
    }
}

/// Share splits that ripple a carry through every bit of the operand adder.
#[test]
fn evaluator_shapes_worst_case_carries() {
    for (w, r) in SHAPES {
        let g = rings(w, r);
        let m = g.input.mask();
        let (lo, hi) = g.input.signed_range();
        let xs = [lo, I256::MINUS_ONE, I256::ZERO, I256::ONE, hi].map(|v| g.input.from_signed(v));
        let x0s = [U256::ZERO, U256::ONE, m, m >> 1, m ^ 1];
        for kind in [GateKind::Lt0, GateKind::Ars { shift: 0 }, GateKind::Ars { shift: w - 1 }] {
            for x in xs {
                for x0 in x0s.map(|v| g.input.reduce(v)) {
                    let x1 = g.input.sub(x, x0);
                    let (alice, bob) = gen(kind, g).unwrap();
                    let (msg, z0) = alice.respond(x0, bob.request(x1));
                    let got = g.output.add(z0, bob.finish(x1, &msg).unwrap());
                    assert_eq!(got, bigint_reference_widths(kind, w, r, x), "{kind} w={w} r={r} x={x} x0={x0}");
                }
            }
        }
    }
}

/// The cost model: (w-1) + (r-1) ANDs, 3w labels per message, r output wires, and the uniform
/// case unchanged.
#[test]
fn sizes_follow_both_widths() {
    for (w, r) in SHAPES.into_iter().chain([(1, 1), (2, 5), (5, 2), (16, 16)]) {
        let g = rings(w, r);
        for kind in [GateKind::Lt0, GateKind::Ars { shift: w / 2 }] {
            let circuit = ShareConverted::new(kind, g).unwrap();
            assert_eq!(circuit.ninputs(), 2 * w as usize + r as usize);
            assert_eq!(circuit.and_count(), (w as usize - 1) + (r as usize - 1));
            if w == r {
                assert_eq!(circuit.and_count(), 2 * (w as usize - 1), "uniform cost must not change");
            }
            let (alice, bob) = gen(kind, g).unwrap();
            assert_eq!(alice.message_len(), 3 * w as usize);
            assert_eq!(bob.garbled_blocks(), 2 * circuit.and_count() + 2 * r as usize, "{kind} w={w} r={r}");
            assert_eq!((alice.input_ring(), alice.output_ring()), (g.input, g.output));
            assert_eq!(bob.rings(), g);
        }
    }
}

/// Wrong message lengths and out-of-range shifts are rejected.
#[test]
fn bad_messages_and_shifts_are_rejected() {
    for (w, r) in [(26, 149), (116, 64)] {
        let g = rings(w, r);
        // s must be below w, whatever r is
        assert!(deal(GateKind::Ars { shift: w }, g, 1).is_err());
        assert!(deal(GateKind::Ars { shift: w - 1 }, g, 1).is_ok());
        for wrong in [3 * w as usize - 1, 3 * w as usize + 1, 2 * w as usize + r as usize, 3 * r as usize] {
            let (alice, bob) = gen(GateKind::Lt0, g).unwrap();
            let (x0, x1) = g.input.share(U256::ONE);
            let (mut msg, _) = alice.respond(x0, bob.request(x1));
            msg.resize(wrong, 0);
            assert!(bob.finish(x1, &msg).is_err(), "w={w} r={r}: accepted {wrong} labels");
        }
    }
}

/// One online batch whose keys all have different rings.
#[test]
fn online_batch_with_mixed_rings() {
    let mut k0 = Vec::new();
    let mut k1 = Vec::new();
    let mut cases = Vec::new();
    for (w, r) in SHAPES {
        let g = rings(w, r);
        for kind in [GateKind::Lt0, GateKind::Ars { shift: w / 3 }] {
            let (a, b) = deal(kind, g, 2).unwrap();
            k0.extend(a);
            k1.extend(b);
            cases.extend([(kind, g, g.input.from_signed(I256::new(-7))), (kind, g, g.input.random())]);
        }
    }
    let (x0, x1): (Vec<U256>, Vec<U256>) = cases.iter().map(|&(_, g, x)| g.input.share(x)).unzip();
    let (mut c0, mut c1) = LocalChannel::pair();
    let h = std::thread::spawn(move || eval_shared(k0, &x0, &mut c0).unwrap());
    let z1 = eval_shared(k1, &x1, &mut c1).unwrap();
    let z0 = h.join().unwrap();
    for (i, &(kind, g, x)) in cases.iter().enumerate() {
        let got = g.output.add(z0[i], z1[i]);
        assert_eq!(got, bigint_reference_widths(kind, g.input.bits(), g.output.bits(), x), "case {i}: {kind} {g:?}");
    }
}

/// Keys with different widths survive serde.
#[test]
fn mixed_keys_roundtrip_through_serialization() {
    for (w, r) in [(26, 149), (116, 64)] {
        let g = rings(w, r);
        let (k0, k1) = deal(GateKind::Ars { shift: 3 }, g, 2).unwrap();
        let k0: Vec<PartyKey> = bincode::deserialize(&bincode::serialize(&k0).unwrap()).unwrap();
        let k1: Vec<PartyKey> = bincode::deserialize(&bincode::serialize(&k1).unwrap()).unwrap();
        for (a, b) in k0.into_iter().zip(k1) {
            assert_eq!((a.rings(), b.rings()), (g, g));
            let (PartyKey::Garbler(a), PartyKey::Evaluator(b)) = (a, b) else {
                panic!("party 0 must hold garbler keys and party 1 evaluator keys");
            };
            let x = g.input.from_signed(I256::new(-1000));
            let (x0, x1) = g.input.share(x);
            let (msg, z0) = a.respond(x0, b.request(x1));
            let y = g.output.add(z0, b.finish(x1, &msg).unwrap());
            assert_eq!(g.output.to_signed(y), I256::new(-1000 >> 3), "w={w} r={r}");
        }
    }
}

/// Bob's output share stays uniform in the (possibly wider) output ring, and he still gets
/// exactly one label per `x1` wire.
#[test]
fn security_with_mixed_widths() {
    for (w, r) in [(26, 149), (116, 64), (1, 256)] {
        let g = rings(w, r);
        let x = g.input.from_signed(I256::MINUS_ONE);
        for kind in [GateKind::Lt0, GateKind::Ars { shift: 0 }] {
            let mut seen = HashSet::new();
            let mut high = 0u32;
            for _ in 0..200 {
                let (alice, bob) = gen(kind, g).unwrap();
                let (x0, x1) = g.input.share(x);
                let (msg, z0) = alice.respond(x0, bob.request(x1));
                let z1 = bob.finish(x1, &msg).unwrap();
                assert_eq!(g.output.add(z0, z1), kind.reference(g, x));
                high += bit(z1, r - 1) as u32;
                seen.insert(z1);
            }
            assert_eq!(seen.len(), 200, "{kind} w={w} r={r}: Bob's output share repeats across fresh keys");
            // the top output bit is a fair coin too (6 standard deviations either way)
            assert!((58..142).contains(&high), "{kind} w={w} r={r}: top bit of z1 set {high}/200 times");
        }
        for j in 0..w {
            let (alice, bob) = gen(GateKind::Lt0, g).unwrap();
            let (x0, x1) = g.input.share(x);
            let copy: EvaluatorKey = bincode::deserialize(&bincode::serialize(&bob).unwrap()).unwrap();
            let (msg, _) = alice.respond(x0, bob.request(x1));
            assert!(copy.finish(x1, &msg).is_ok());
            assert!(bob.finish(x1 ^ (U256::ONE << j), &msg).is_err(), "w={w} r={r} wire {j}: other OT label opened");
        }
    }
}
