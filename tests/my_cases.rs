//! Runs the cases in tests/my_cases.txt. Add your own lines there.

use gc_gates::{simulate, FixedPoint, GateKind, Ring};

const TRIALS: usize = 10;

#[test]
fn my_cases() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/my_cases.txt");
    let text = std::fs::read_to_string(path).unwrap();
    let mut failures = Vec::new();
    let mut count = 0;
    for (lineno, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        let at = format!("my_cases.txt:{}", lineno + 1);
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [bits, frac, shift, value] = fields[..] else {
            panic!("{at}: expected `bits frac shift value`, got `{line}`");
        };
        let num = |s: &str| s.parse::<u32>().unwrap_or_else(|_| panic!("{at}: `{s}` is not a number"));
        let ring = Ring::new(num(bits)).unwrap_or_else(|e| panic!("{at}: {e}"));
        let fp = FixedPoint::new(ring, num(frac)).unwrap_or_else(|e| panic!("{at}: {e}"));
        let x = fp.parse(value).unwrap_or_else(|e| panic!("{at}: {e}"));

        for kind in [GateKind::Lt0, GateKind::Ars { shift: num(shift) }] {
            let want = kind.reference(ring, x);
            for _ in 0..TRIALS {
                let got = simulate(kind, ring, x).unwrap_or_else(|e| panic!("{at}: {e}"));
                count += 1;
                if got != want {
                    failures.push(format!(
                        "{at}: {kind} on {value} (raw {}): got {}, want {}",
                        ring.to_signed(x),
                        ring.to_signed(got),
                        ring.to_signed(want)
                    ));
                    break;
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n"));
    println!("{count} gate evaluations passed");
}
