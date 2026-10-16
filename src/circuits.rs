//! The Boolean circuits of Algorithms 2 and 3 in Hemenway, Lu, Ostrovsky, Welser (2016),
//! written once against swanky's [`FancyBinary`]. The same code is garbled by the dealer,
//! evaluated under garbling by party 1, and run in plaintext by the tests.
//!
//! Hardwired: the gate, the operand width w and the output width r (independent; r > w,
//! r = w and r < w all work). Inputs, least significant bit first: Alice's share `x0` and
//! Bob's share `x1` (w wires each), and Alice's random mask `R` (r wires).
//!
//! ```text
//! Algorithm 2 (< 0)                         Algorithm 3 (>> c)
//!   x <- x0 + x1 (mod 2^w)                    x <- x0 + x1 (mod 2^w)
//!   b <- 1{x < 0}  (the sign wire of x)       y <- x >> c  in r bits (sign wire repeated, c low wires dropped)
//!   z1 = b + R (mod 2^r) -> Bob               z1 = y + R (mod 2^r) -> Bob
//! ```
//!
//! Alice keeps `z0 = -R`, so `z0 + z1 = f(x) mod 2^r`. The sign test and the shift are pure
//! wiring. Each addition is a ripple-carry adder with one AND per bit, and the carry out of the
//! top bit is never computed. Both gates cost `(w-1) + (r-1)` AND gates, `2(n-1)` for
//! r = w = n. XOR and NOT are free.

use crate::gates::{GateKind, GateRings};
use crate::Error;
use fancy_traits::{Circuit, CircuitInputMapper, CircuitOutputMapper, FancyBinary};
use swanky_channel::Channel;

/// One share-converted gate from Z_{2^w} to Z_{2^r}: Algorithm 2 or 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShareConverted {
    kind: GateKind,
    in_bits: u32,
    out_bits: u32,
}

impl ShareConverted {
    pub fn new(kind: GateKind, rings: GateRings) -> Result<ShareConverted, Error> {
        let w = rings.input.bits();
        if let GateKind::Ars { shift } = kind {
            if shift >= w {
                return Err(Error::new(format!("shift must be in 0..{w} for a {w}-bit operand, got {shift}")));
            }
        }
        Ok(ShareConverted { kind, in_bits: w, out_bits: rings.output.bits() })
    }

    /// Input wires: `x0` and `x1` (w each), then `R` (r).
    pub fn ninputs(&self) -> usize {
        2 * self.in_bits as usize + self.out_bits as usize
    }

    /// Number of AND gates, the only gates that cost ciphertexts.
    pub fn and_count(&self) -> usize {
        (self.in_bits as usize - 1) + (self.out_bits as usize - 1)
    }
}

impl<F: FancyBinary> Circuit<F> for ShareConverted {
    type Input = Vec<F::Item>;
    type Output = Vec<F::Item>;

    // does the computation of the sgn(x) or the x >> c
    fn execute(&self, f: &mut F, inputs: Vec<F::Item>, ch: &mut Channel) -> swanky_error::Result<Vec<F::Item>> {
        let (w, out) = (self.in_bits as usize, self.out_bits as usize);
        assert_eq!(inputs.len(), 2 * w + out, "expected x0 and x1 ({w} wires each) and R ({out} wires)");
        let (x0, rest) = inputs.split_at(w);
        let (x1, r) = rest.split_at(w);
        let x = add(f, x0, x1, ch)?; //first line of Algorithm 2 and 3, over w bits
        match self.kind {
            GateKind::Lt0 => increment(f, r, &x[w - 1], ch), // sign(x) adding the sign bit to r, over r bits
            GateKind::Ars { shift } => {
                let s = shift as usize;
                // drop low shift bits; past the top the sign wire repeats (sign extension to r bits)
                let y: Vec<F::Item> = (0..out).map(|i| x[(i + s).min(w - 1)].clone()).collect();
                add(f, &y, r, ch) // add y and R
            }
        }
    }
}

impl<F: FancyBinary> CircuitInputMapper<F> for ShareConverted {
    fn map(&self, inputs: Vec<F::Item>) -> Vec<F::Item> {
        inputs
    }

    fn ninputs(&self) -> usize {
        ShareConverted::ninputs(self)
    }

    fn modulus(&self, _: usize) -> u16 {
        2
    }
}

impl<F: FancyBinary> CircuitOutputMapper<F> for ShareConverted {
    fn flatten(output: Vec<F::Item>) -> Vec<F::Item> {
        output
    }
}

/// `a + b mod 2^n`: ripple carry, with the carry out of bit i computed as
/// `maj(a_i, b_i, c_i) = c_i ^ ((a_i ^ c_i) & (b_i ^ c_i))`, one AND. n-1 ANDs in total.
fn add<F: FancyBinary>(f: &mut F, a: &[F::Item], b: &[F::Item], ch: &mut Channel) -> swanky_error::Result<Vec<F::Item>> {
    let n = a.len();
    let mut sum = Vec::with_capacity(n);
    let mut carry: Option<F::Item> = None;
    for i in 0..n {
        let ab = f.xor(&a[i], &b[i]);
        let last = i + 1 == n;
        match carry.take() {
            None => {
                sum.push(ab);
                if !last {
                    carry = Some(f.and(&a[i], &b[i], ch)?);
                }
            }
            Some(c) => {
                sum.push(f.xor(&ab, &c));
                if !last {
                    let ac = f.xor(&a[i], &c);
                    let bc = f.xor(&b[i], &c);
                    let t = f.and(&ac, &bc, ch)?;
                    carry = Some(f.xor(&t, &c));
                }
            }
        }
    }
    Ok(sum)
}

/// `a + bit mod 2^n` for a single wire `bit`: a half-adder chain, n-1 ANDs.
fn increment<F: FancyBinary>(f: &mut F, a: &[F::Item], bit: &F::Item, ch: &mut Channel) -> swanky_error::Result<Vec<F::Item>> {
    let n = a.len();
    let mut sum = Vec::with_capacity(n);
    let mut carry = bit.clone();
    for i in 0..n {
        sum.push(f.xor(&a[i], &carry));
        if i + 1 < n {
            carry = f.and(&a[i], &carry, ch)?;
        }
    }
    Ok(sum)
}
