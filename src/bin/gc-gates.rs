//! Command-line front end. Run without arguments for usage.

use gc_gates::online::{eval_shared, reveal};
use gc_gates::transport::TcpChannel;
use gc_gates::circuits::ShareConverted;
use gc_gates::{deal, simulate, Error, FixedPoint, GateKind, PartyKey, Ring};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

const USAGE: &str = "\
gc-gates: 2PC garbled-circuit gates (less-than-zero, arithmetic right shift) on
additive shares over Z_{2^n}

Check gates on your own numbers (local simulation, fresh keys every trial):
  gc-gates check --bits N --frac F [--shift S] [--trials T] VALUE...
      VALUE is a decimal (e.g. -3.25, encoded as round(v * 2^F)) or raw:<signed int>.
      --shift defaults to F. --trials defaults to 20.

Distributed run in separate terminals (dealer, then party 0 = garbler, party 1 = evaluator):
  gc-gates deal  --gate lt0|ars --bits N [--shift S] [--count K] --out DIR
      Writes DIR/party0.key and DIR/party1.key (K gate instances each, default 1).
  gc-gates share --bits N --frac F VALUE...
      Input owner: split values into additive shares, one comma list per party.
  gc-gates party --id 0 --key DIR/party0.key --input SHARES --listen ADDR [--reveal] [--frac F]
  gc-gates party --id 1 --key DIR/party1.key --input SHARES --connect ADDR [--reveal] [--frac F]
      Runs the online phase and prints this party's output shares. With --reveal both
      parties exchange shares and print the plaintext outputs (as fixed-point with F).
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check") => Opts::parse(&args[1..]).and_then(|o| cmd_check(&o)),
        Some("deal") => Opts::parse(&args[1..]).and_then(|o| cmd_deal(&o)),
        Some("share") => Opts::parse(&args[1..]).and_then(|o| cmd_share(&o)),
        Some("party") => Opts::parse(&args[1..]).and_then(|o| cmd_party(&o)),
        _ => {
            eprint!("{USAGE}");
            std::process::exit(2);
        }
    };
    match result {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    }
}

/// `--flag value` options, `--flag` switches and positional values.
struct Opts {
    flags: HashMap<String, Option<String>>,
    values: Vec<String>,
}

const SWITCHES: &[&str] = &["reveal"];

impl Opts {
    fn parse(args: &[String]) -> Result<Opts, Error> {
        let mut flags = HashMap::new();
        let mut values = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.strip_prefix("--") {
                // `--` ends the options; everything after it is a value.
                Some("") => values.extend(it.by_ref().cloned()),
                Some(name) if SWITCHES.contains(&name) => {
                    flags.insert(name.to_string(), None);
                }
                Some(name) => {
                    let v = it.next().ok_or_else(|| Error::new(format!("--{name} needs a value")))?;
                    flags.insert(name.to_string(), Some(v.clone()));
                }
                // Negative numbers are values, not flags: `check ... -3.25`.
                None => values.push(a.clone()),
            }
        }
        Ok(Opts { flags, values })
    }

    fn str(&self, name: &str) -> Result<&str, Error> {
        self.opt_str(name).ok_or_else(|| Error::new(format!("missing --{name}")))
    }

    fn opt_str(&self, name: &str) -> Option<&str> {
        self.flags.get(name).and_then(|v| v.as_deref())
    }

    fn num(&self, name: &str) -> Result<u32, Error> {
        parse_u32(name, self.str(name)?)
    }

    fn opt_num(&self, name: &str) -> Result<Option<u32>, Error> {
        self.opt_str(name).map(|v| parse_u32(name, v)).transpose()
    }

    fn has(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    fn ring(&self) -> Result<Ring, Error> {
        Ring::new(self.num("bits")?)
    }
}

fn parse_u32(name: &str, v: &str) -> Result<u32, Error> {
    v.parse().map_err(|_| Error::new(format!("--{name}: `{v}` is not a non-negative integer")))
}

fn cmd_check(o: &Opts) -> Result<bool, Error> {
    let ring = o.ring()?;
    let fp = FixedPoint::new(ring, o.num("frac")?)?;
    let shift = o.opt_num("shift")?.unwrap_or(fp.frac());
    let trials = o.opt_num("trials")?.unwrap_or(20);
    if o.values.is_empty() {
        return Err(Error::new("give at least one VALUE"));
    }
    let xs = o.values.iter().map(|v| fp.parse(v)).collect::<Result<Vec<_>, _>>()?;
    let kinds = [GateKind::Lt0, GateKind::Ars { shift }];
    // Validate the shift before printing anything.
    ShareConverted::new(kinds[1], ring)?;

    println!("ring Z_2^{}, {} fractional bits, shift {shift}, {trials} trials per gate\n", ring.bits(), fp.frac());
    let mut all_ok = true;
    for (value, &x) in o.values.iter().zip(&xs) {
        println!("x = {value}  ->  encoded {} (raw {})", fp.format(x), ring.to_signed(x));
        for kind in kinds {
            let want = kind.reference(ring, x);
            let mut failed = 0;
            let mut last = want;
            for _ in 0..trials {
                let got = simulate(kind, ring, x)?;
                if got != want {
                    failed += 1;
                    last = got;
                }
            }
            all_ok &= failed == 0;
            let show = |y: u128| match kind {
                GateKind::Ars { .. } => format!("{} (raw {})", fp.format(y), ring.to_signed(y)),
                _ => ring.to_signed(y).to_string(),
            };
            let status = if failed == 0 {
                "PASS".to_string()
            } else {
                format!("FAIL {failed}/{trials}, e.g. got {}", show(last))
            };
            println!("  {:<10} expected {:<40} {status}", kind.to_string(), show(want));
        }
        println!();
    }
    println!("{}", if all_ok { "all checks passed" } else { "SOME CHECKS FAILED" });
    Ok(all_ok)
}

fn cmd_deal(o: &Opts) -> Result<bool, Error> {
    let ring = o.ring()?;
    let kind = GateKind::parse(o.str("gate")?, o.opt_num("shift")?)?;
    let count = o.opt_num("count")?.unwrap_or(1) as usize;
    let dir = PathBuf::from(o.str("out")?);
    let (k0, k1) = deal(kind, ring, count)?;
    std::fs::create_dir_all(&dir)?;
    for (id, keys) in [(0, &k0), (1, &k1)] {
        let path = dir.join(format!("party{id}.key"));
        std::fs::write(&path, bincode::serialize(keys)?)?;
        println!("wrote {} ({count} x {kind} on Z_2^{}, {} bytes)", path.display(), ring.bits(), std::fs::metadata(&path)?.len());
    }
    Ok(true)
}

fn cmd_share(o: &Opts) -> Result<bool, Error> {
    let ring = o.ring()?;
    let fp = FixedPoint::new(ring, o.num("frac")?)?;
    let xs = o.values.iter().map(|v| fp.parse(v)).collect::<Result<Vec<_>, _>>()?;
    let (s0, s1): (Vec<u128>, Vec<u128>) = xs.iter().map(|&x| ring.share(x)).unzip();
    let join = |v: &[u128]| v.iter().map(u128::to_string).collect::<Vec<_>>().join(",");
    println!("party 0 --input {}", join(&s0));
    println!("party 1 --input {}", join(&s1));
    Ok(true)
}

fn cmd_party(o: &Opts) -> Result<bool, Error> {
    let id = o.num("id")?;
    let keys: Vec<PartyKey> = bincode::deserialize(&std::fs::read(o.str("key")?)?)?;
    let first = keys.first().ok_or_else(|| Error::new("key file holds no gate instances"))?;
    if first.party() as u32 != id {
        return Err(Error::new(format!("key file is for party {}, not {id}", first.party())));
    }
    let ring = first.ring();
    let inputs = o
        .str("input")?
        .split(',')
        .map(|s| s.trim().parse::<u128>().map(|v| ring.reduce(v)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error::new("--input must be a comma-separated list of unsigned ring elements"))?;
    if inputs.len() != keys.len() {
        return Err(Error::new(format!("key file holds {} gate instances but --input has {} shares", keys.len(), inputs.len())));
    }

    let mut ch = match (id, o.opt_str("listen"), o.opt_str("connect")) {
        (0, Some(addr), None) => {
            println!("party 0: waiting for party 1 on {addr}");
            TcpChannel::listen(addr)?
        }
        (1, None, Some(addr)) => {
            println!("party 1: connecting to {addr}");
            TcpChannel::connect(addr, Duration::from_secs(60))?
        }
        _ => return Err(Error::new("party 0 needs --listen ADDR, party 1 needs --connect ADDR")),
    };
    let shares = eval_shared(&keys, &inputs, &mut ch)?;
    println!("party {id}: {} output share(s): {}", shares.len(), shares.iter().map(u128::to_string).collect::<Vec<_>>().join(","));
    if o.has("reveal") {
        let frac = o.opt_num("frac")?.unwrap_or(0);
        let fp = FixedPoint::new(ring, frac)?;
        let outs = reveal(&keys, &shares, &mut ch)?;
        for (k, y) in keys.iter().zip(outs) {
            let shown = match k.kind() {
                GateKind::Ars { .. } => format!("{} (raw {})", fp.format(y), ring.to_signed(y)),
                _ => ring.to_signed(y).to_string(),
            };
            println!("  {} -> {shown}", k.kind());
        }
    }
    Ok(true)
}
