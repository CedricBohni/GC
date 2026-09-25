//! What each party sees. These guard the protocol plumbing around the garbling library: the
//! derandomised OT, the output mask R and Alice's view of Bob's input.

use gc_gates::gates::gen;
use gc_gates::{GateKind, Ring};
use std::collections::HashSet;

/// Bob gets exactly one label per `x1` wire from the OT. Using the other reply for any wire
/// gives a label the garbled circuit does not accept.
#[test]
fn evaluator_cannot_open_the_other_ot_label() {
    let ring = Ring::new(16).unwrap();
    for j in 0..16 {
        let (alice, bob) = gen(GateKind::Lt0, ring).unwrap();
        let (x0, x1) = ring.share(ring.from_signed(-5));
        let (msg, _) = alice.respond(x0, bob.request(x1));
        assert!(bob.finish(x1, &msg).is_ok());
        let flipped = x1 ^ (1 << j);
        assert!(bob.finish(flipped, &msg).is_err(), "wire {j}: Bob decoded with the label he did not choose");
    }
}

/// Bob's output is `f(x) + R`: over fresh keys for one fixed input, it must look uniform,
/// never the plain result.
#[test]
fn evaluator_output_is_masked() {
    let ring = Ring::new(64).unwrap();
    let x = ring.from_signed(-123_456);
    for kind in [GateKind::Lt0, GateKind::Ars { shift: 12 }] {
        let mut seen = HashSet::new();
        for _ in 0..200 {
            let (alice, bob) = gen(kind, ring).unwrap();
            let (x0, x1) = ring.share(x);
            let (msg, z0) = alice.respond(x0, bob.request(x1));
            let z1 = bob.finish(x1, &msg).unwrap();
            assert_eq!(ring.add(z0, z1), kind.reference(ring, x));
            seen.insert(z1);
        }
        assert_eq!(seen.len(), 200, "{kind}: Bob's output share repeats across fresh keys");
    }
}

/// Alice sees only `w = c XOR x1`. For a fixed `x1` every bit of `w` must be a fair coin.
#[test]
fn garbler_view_of_ot_is_uniform() {
    let ring = Ring::new(32).unwrap();
    for x1 in [0, ring.mask(), 0x5555_5555] {
        let mut ones = [0u32; 32];
        let trials = 400;
        for _ in 0..trials {
            let (_, bob) = gen(GateKind::Lt0, ring).unwrap();
            let w = bob.request(x1);
            for (j, c) in ones.iter_mut().enumerate() {
                *c += ((w >> j) & 1) as u32;
            }
        }
        for (j, &c) in ones.iter().enumerate() {
            let rate = c as f64 / trials as f64;
            assert!((0.38..0.62).contains(&rate), "x1={x1:#x}: bit {j} of w is 1 in {:.0}% of runs", rate * 100.0);
        }
    }
}
