# gc-gates

Two-party garbled-circuit gates on additive shares in the dealer model (1 dealer + 2 semi-honest
parties), after Hemenway, Lu, Ostrovsky and Welser, *High-precision Secure Computation of
Satellite Collision Probabilities* ([SCN 2016](https://eprint.iacr.org/2016/319)). Works over
Z_{2^n} for any 1 ≤ n ≤ 256, on signed (two's complement) fixed-point values with any number of
fractional bits. This is the garbled-circuit counterpart of `../FSS`: same gates, same
command-line interface, same library shape.

| Gate | Computes | Paper | Garbled circuit | Online |
|---|---|---|---|---|
| `lt0` | `1{x < 0}` | Algorithm 2 (share-converted Less-Than-Zero) | 2(n−1) AND gates | 1 round trip |
| `ars` | `x >>_A s` = `floor(x / 2^s)`, exact | Algorithm 3 (share-converted shift-right-by-constant) | 2(n−1) AND gates | 1 round trip |

Inputs and outputs are additive shares in Z_{2^n}. Party 0 is Alice (the garbler role), and
party 1 is Bob (the evaluator).

## Toolchain

The garbling library ([swanky](https://github.com/GaloisInc/swanky)) needs Rust 1.98. The
version is pinned in `rust-toolchain.toml`. On this machine `/usr/bin/cargo` and
`/usr/bin/rustc` are 1.75 and come first on `PATH`, and the rustup proxy still calls the system
`rustc`. Put the pinned toolchain first before running anything:

```sh
rustup toolchain install 1.98.0 --profile minimal     # once
export PATH="$(rustup run 1.98.0 rustc --print sysroot)/bin:$PATH"
```

## Your own numbers

```sh
cargo run --release -- check --bits 16 --frac 8 -- -3.25 7.5 raw:-32768
cargo run --release -- check --bits 128 --frac 64 --shift 64 --trials 100 -- -123456.789
cargo run --release -- check --bits 256 --frac 128 -- -123456789012345678901234567890.0625
```

A value is either a decimal, encoded as `round(v · 2^frac)`, or `raw:<int>`, the signed ring
integer itself. `--shift` defaults to `--frac`. Each trial uses a freshly garbled circuit and
fresh random shares, and the result is compared with the plaintext computation. The command
exits non-zero if any trial fails.

To keep cases as regression tests, add lines to [tests/my_cases.txt](tests/my_cases.txt)
(`bits frac shift value`) and run `cargo test --test my_cases`.

## Distributed run (separate terminals)

In every terminal, put the pinned toolchain first (see [Toolchain](#toolchain)) and set
`B=target/release/gc-gates`. This computes `x >> 128` on a 256-bit ring with 128 fractional
bits, for the inputs −1234.5678 and 3.75.

```sh
# terminal 1: dealer and input owner
$B deal  --gate ars --bits 256 --shift 128 --count 2 --out keys
$B share --bits 256 --frac 128 --out shares -- -1234.5678 3.75
# terminal 2: party 0 (garbler)
$B party --id 0 --key keys/party0.key --input-file shares/party0.shares --listen 127.0.0.1:7000 --reveal --frac 0
# terminal 3: party 1 (evaluator)
$B party --id 1 --key keys/party1.key --input-file shares/party1.shares --connect 127.0.0.1:7000 --reveal --frac 0
```

Both parties print `-1235` and `3`, the inputs rounded down.

- `deal` writes `keys/party0.key` and `keys/party1.key`, and `share --out` writes
  `shares/party0.shares` and `shares/party1.shares`. All four are secrets, readable only by you
  (mode 0600), and `keys/` and `shares/` are in `.gitignore`. `--count` must equal the number of
  values shared.
- A shares file names its ring width and party, so passing the wrong one to `party` stops with
  an error before the key is used.
- Without `--out`, `share` prints the two lists instead; pass a list with `--input <list>` in
  place of `--input-file`.
- `--frac` on `party` only affects printing: set it to the input's fractional bits minus the
  shift (here 128 − 128 = 0).

Without `--reveal`, each party prints only its output shares. Use this when a later gate
consumes the output. The parties can be started in either order.

Keys are single-use. Before connecting, `party` renames its key file to `<key>.used`, so a
second run on the same file fails instead of reusing the garbled circuit. Deal fresh keys for
every run.

## Library use

```rust
use gc_gates::{deal, GateKind, Ring, online::eval_shared, transport::TcpChannel};

let ring = Ring::new(64)?;
let (keys0, keys1) = deal(GateKind::Ars { shift: 16 }, ring, 1000)?;   // dealer; keys are serde types
// party b, holding additive shares x_b of its inputs:
let y_b = eval_shared(keys_b, &x_b, &mut channel)?;                   // shares of the outputs; consumes the keys
```

`gates::gen` returns one instance's `(GarblerKey, EvaluatorKey)`. Their three protocol steps
(`EvaluatorKey::request`, `GarblerKey::respond`, `EvaluatorKey::finish`) are public, so you can
use them in your own protocol. `circuits::ShareConverted` is the Boolean circuit itself. Every
key is single-use, so `respond`, `finish` and `eval_shared` take their keys by value. A reused
garbler key would hand Bob both labels of some `x0` wire, and their XOR is the global offset
Delta, which lets him decode every wire.

## Protocol

The circuits are Algorithms 2 and 3 of the paper. Inside the circuit, the two shares are added
back together, the gate is applied, and the result is masked with Alice's random `R`:

```text
Algorithm 2 (< 0)                         Algorithm 3 (>> c)
  x <- x0 + x1 (mod 2^n)                    x <- x0 + x1 (mod 2^n)
  b <- 1{x < 0}  (sign wire)                y <- x >> c  (sign wire duplicated, c low wires dropped)
  z1 = b + R (mod 2^n) -> Bob               z1 = y + R (mod 2^n) -> Bob
  Alice keeps z0 = -R                       Alice keeps z0 = -R
```

The sign test and the shift are wiring only. Each addition is a ripple-carry adder with one AND
per bit and no carry out of the top bit. `lt0`'s second addition adds a single bit, so it is a
half-adder chain. Both gates cost 2(n−1) ANDs: 254 at n = 128, 510 at n = 256.

The key material follows the paper's dealer-assisted variant (§5): the dealer garbles the
circuit and sends the garbled circuit to Bob and the input wire labels to Alice. It also
pregenerates one random OT per bit of Bob's share (§4.1.1): Alice gets `(m0, m1)`, and Bob gets
`(c, m_c)`. Online, over one round trip:

1. Bob sends `w = c ⊕ x1` (n bits, one ring element per gate),
2. Alice sends the labels of her `x0` bits, and for each wire of Bob's
   `(q0, q1) = (s0 ⊕ m_w, s1 ⊕ m_{1⊕w})`, where `s0` and `s1` are that wire's labels,
3. Bob computes `q_{x1} ⊕ m_c`, the label of his bit, then evaluates and decodes `z1`.

### Deviations from the paper

- **Half-gates and free-XOR.** The paper garbles with JustGarble (fixed-key AES) without free-XOR
  or half-gates (§4.1.3). fancy-garbling uses both. XOR and NOT are free, and an AND costs two
  ciphertexts instead of four.
- **The dealer samples `R`.** In the paper, Alice provides `R` as a circuit input. Here the
  dealer samples `R` on Alice's behalf and gives her `z0 = −R`. The active labels of `R` do not
  depend on any input, so they go in Bob's key next to the garbled circuit instead of being sent
  online. The distribution is the same.

### Cost at n = 128, per gate

| | FSS (`../FSS`) | GC (this project) |
|---|---|---|
| party 0 key | 4.4 KB (`lt0`), 8.8 KB (`ars`) | 12.3 KB |
| party 1 key | same as party 0 | 21.5 KB (garbled circuit: 764 blocks of 16 B) |
| online traffic | 16 B each way, then local DCF evaluation | 16 B Bob → Alice, 6 KB Alice → Bob |

At n = 256 the GC keys are 24.7 KB (party 0) and 43.0 KB (party 1, 1532 blocks), and the
online traffic is 32 B Bob → Alice and 12 KB Alice → Bob. Key and message sizes grow linearly
in n.

### Rings wider than 128 bits

Ring elements are 256-bit integers (`U256`/`I256` from the
[`ethnum`](https://crates.io/crates/ethnum) crate), as in the FSS project. The circuit is
generic in n, and wire labels stay 128 bits for every width, since they set the security level,
not the ring. On the channel a ring element (Bob's OT correction `w`, a revealed share) is one
128-bit word for n ≤ 128 and two above, so rings up to 128 bits send exactly what they did
before. In key files, `R` and the OT choice bits are stored without leading zero bytes.

## Design notes

**Library: [swanky](https://github.com/GaloisInc/swanky)'s `fancy-garbling`** (Galois, MIT),
pinned to commit `e0f4a62`. Reasons:

- It is Rust, so it drops into the same crate structure, serde key files, TCP transport and CLI
  as the FSS project.
- `fancy_garbling::classic` garbles *statically*: `GarbledCircuit::garble` returns the input
  encoder, the garbled circuit and the output mapping as separate, serializable values, and
  `eval` runs later on labels. That is exactly the paper's split between the dealer, Alice and
  Bob. Most GC libraries only garble while streaming to a live evaluator.
- Circuits are written once against the `FancyBinary` trait. The same `ShareConverted` code is
  garbled by the dealer, evaluated by Bob, and run in plaintext by `fancy-plaintext` in the tests.
- It implements half-gates with free-XOR over a fixed-key-AES correlation-robust hash.

Alternatives considered:

- **EMP-toolkit (`emp-sh2pc`)** is the fastest option, but it is C++ (FFI and a CMake build) and
  garbles while streaming to the evaluator, with no dealer split.
- **mpz (TLSNotary)** is Rust, but async, API-unstable and built around TLSNotary's DEAP
  protocol.
- **tandem and polytune** are maliciously secure multi-party engines with their own fixed
  protocol, far from the paper's semi-honest two-party setting.
- **A hand-rolled garbler** like the one in `../Secure-Satellite-Collision-Avoidance` would work,
  but it adds about 1,400 lines of cryptographic code to maintain.

Costs of this choice: swanky is not on crates.io (it is a git dependency, recorded in
`Cargo.lock`), it needs Rust 1.98, and it is research software.

## Tests

`cargo test` covers:
- the circuits in plaintext: every `(x0, x1, R)` for n = 1..5 and every shift, plus random values
  for every n up to 256 (`tests/circuits.rs`),
- the garbled circuit size: 2 ciphertexts per AND and 2(n−1) ANDs,
- the full gates, as in the FSS project: exhaustive for n = 1..8 (every input and every shift,
  with fresh garblings), edge values and random values for every n up to 256, and share splits
  that push a carry through every bit of both adders for every n up to 256. Expected values come
  from BigInt arithmetic (`tests/common/mod.rs`, shared with the FSS project) that shares no code
  with the library,
- the online protocol over in-process channels and TCP, up to n = 256,
- key serialization,
- the security properties of each party's view (`tests/security.rs`): Bob cannot use the OT label
  he did not choose, Bob's output is masked by `R`, and Alice's view of the OT is uniform,
- `tests/my_cases.txt`.

The tests were checked against deliberately broken gates to confirm that they fail. The broken
versions were a wrong adder carry, a wrong increment carry, a wrong sign-extension wire, an OT
that ignores `w`, and `R = 0`. The `R = 0` version still computes correct outputs, so only
`tests/security.rs` catches it.
