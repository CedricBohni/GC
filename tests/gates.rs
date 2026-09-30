use gc_gates::online::{eval_shared, reveal};
use gc_gates::transport::{Channel, LocalChannel, TcpChannel};
use gc_gates::gates::gen;
use gc_gates::{deal, simulate, GateKind, PartyKey, Ring};
use std::time::Duration;

fn kinds(ring: Ring) -> Vec<GateKind> {
    let mut k = vec![GateKind::Lt0];
    k.extend((0..ring.bits()).map(|shift| GateKind::Ars { shift }));
    k
}

fn check(kind: GateKind, ring: Ring, x: u128) {
    let got = simulate(kind, ring, x).unwrap();
    let want = kind.reference(ring, x);
    assert_eq!(
        got,
        want,
        "{kind} on {}-bit ring, x = {} (raw {x}): got {}, want {}",
        ring.bits(),
        ring.to_signed(x),
        ring.to_signed(got),
        ring.to_signed(want)
    );
}

/// Every input, every shift, several fresh garblings and share splits each, for n = 1..=8.
#[test]
fn exhaustive_small_rings() {
    for n in 1..=8 {
        let ring = Ring::new(n).unwrap();
        for kind in kinds(ring) {
            for x in 0..=ring.mask() {
                for _ in 0..3 {
                    check(kind, ring, x);
                }
            }
        }
    }
}

#[test]
fn edge_values_wide_rings() {
    for n in [9, 16, 31, 32, 33, 63, 64, 65, 100, 127, 128] {
        let ring = Ring::new(n).unwrap();
        let (lo, hi) = ring.signed_range();
        let edges = [lo, lo + 1, lo / 2, -2, -1, 0, 1, 2, hi / 2, hi - 1, hi];
        let shifts = [0, 1, n / 2, n - 2, n - 1];
        for &v in &edges {
            let x = ring.from_signed(v);
            check(GateKind::Lt0, ring, x);
            for &shift in &shifts {
                check(GateKind::Ars { shift }, ring, x);
            }
        }
    }
}

#[test]
fn random_values_all_widths() {
    for n in 1..=128 {
        let ring = Ring::new(n).unwrap();
        for _ in 0..20 {
            let x = ring.random();
            check(GateKind::Lt0, ring, x);
            check(GateKind::Ars { shift: rand::random_range(0..n) }, ring, x);
        }
    }
}

/// Random shares rarely carry far. Pick share splits and masks that ripple a carry through
/// every bit of both adders, for every width.
#[test]
fn worst_case_carries_all_widths() {
    for n in 1..=128 {
        let ring = Ring::new(n).unwrap();
        let m = ring.mask();
        let (lo, hi) = ring.signed_range();
        let xs = [lo, lo + 1, -1, 0, 1, hi].map(|v| ring.from_signed(v));
        let x0s = [0, 1, m, m >> 1, m - (m >> 1), m ^ 1];
        let mut kinds = vec![GateKind::Lt0];
        kinds.extend([0, 1, n / 2, n.saturating_sub(2), n - 1].map(|shift| GateKind::Ars { shift }));
        kinds.retain(|k| !matches!(k, GateKind::Ars { shift } if *shift >= n));
        for &kind in &kinds {
            for x in xs {
                for x0 in x0s.map(|v| ring.reduce(v)) {
                    let x1 = ring.sub(x, x0);
                    let (alice, bob) = gen(kind, ring).unwrap();
                    let (msg, z0) = alice.respond(x0, bob.request(x1));
                    let got = ring.add(z0, bob.finish(x1, &msg).unwrap());
                    assert_eq!(got, kind.reference(ring, x), "{kind} n={n} x={x} x0={x0}");
                }
            }
        }
    }
}

#[test]
fn invalid_parameters_are_rejected() {
    let ring = Ring::new(16).unwrap();
    assert!(deal(GateKind::Ars { shift: 16 }, ring, 1).is_err());
}

fn party<C: Channel>(ring: Ring, keys: Vec<PartyKey>, xs: Vec<u128>, mut ch: C) -> (Vec<u128>, Vec<u128>) {
    let shares = eval_shared(keys, &xs, &mut ch).unwrap();
    let out = reveal(ring, &shares, &mut ch).unwrap();
    (shares, out)
}

fn run_parties<C: Channel + Send + 'static>(kind: GateKind, ring: Ring, xs: &[u128], ch0: C, ch1: C) -> Vec<u128> {
    let (k0, k1) = deal(kind, ring, xs.len()).unwrap();
    let (x0, x1): (Vec<u128>, Vec<u128>) = xs.iter().map(|&x| ring.share(x)).unzip();
    let h1 = std::thread::spawn(move || party(ring, k1, x1, ch1));
    let (s0, out0) = party(ring, k0, x0, ch0);
    let (s1, out1) = h1.join().unwrap();
    assert_eq!(out0, out1, "both parties must reveal the same outputs");
    let sums: Vec<u128> = s0.iter().zip(&s1).map(|(&a, &b)| ring.add(a, b)).collect();
    assert_eq!(sums, out0);
    out0
}

fn tcp_pair() -> (TcpChannel, TcpChannel) {
    let addr = format!("127.0.0.1:{}", 20000 + rand::random_range(0..20000));
    let a1 = addr.clone();
    let t = std::thread::spawn(move || TcpChannel::connect(a1.as_str(), Duration::from_secs(10)).unwrap());
    let c0 = TcpChannel::listen(addr.as_str()).unwrap();
    (c0, t.join().unwrap())
}

#[test]
fn online_protocol_local_and_tcp() {
    for &use_tcp in &[false, true] {
        for (n, shift) in [(16, 8), (64, 20), (128, 64)] {
            let ring = Ring::new(n).unwrap();
            let xs: Vec<u128> = (0..50).map(|_| ring.random()).collect();
            for kind in [GateKind::Lt0, GateKind::Ars { shift }] {
                let out = if use_tcp {
                    let (c0, c1) = tcp_pair();
                    run_parties(kind, ring, &xs, c0, c1)
                } else {
                    let (c0, c1) = LocalChannel::pair();
                    run_parties(kind, ring, &xs, c0, c1)
                };
                let want: Vec<u128> = xs.iter().map(|&x| kind.reference(ring, x)).collect();
                assert_eq!(out, want, "{kind} n={n} tcp={use_tcp}");
            }
        }
    }
}

#[test]
fn keys_roundtrip_through_serialization() {
    let ring = Ring::new(32).unwrap();
    let (k0, k1) = deal(GateKind::Ars { shift: 7 }, ring, 3).unwrap();
    let k0: Vec<PartyKey> = bincode::deserialize(&bincode::serialize(&k0).unwrap()).unwrap();
    let k1: Vec<PartyKey> = bincode::deserialize(&bincode::serialize(&k1).unwrap()).unwrap();
    let x = ring.from_signed(-1000);
    for (a, b) in k0.into_iter().zip(k1) {
        let (PartyKey::Garbler(a), PartyKey::Evaluator(b)) = (a, b) else {
            panic!("party 0 must hold garbler keys and party 1 evaluator keys");
        };
        let (x0, x1) = ring.share(x);
        let (msg, z0) = a.respond(x0, b.request(x1));
        let y = ring.add(z0, b.finish(x1, &msg).unwrap());
        assert_eq!(ring.to_signed(y), -1000 >> 7);
    }
}
