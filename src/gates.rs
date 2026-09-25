//! Garbled-circuit gates and the dealer, following the dealer-assisted protocol of
//! Hemenway, Lu, Ostrovsky, Welser (2016), §5, for the circuits of Algorithms 2 and 3.
//!
//! Party 0 is Alice (garbler role), party 1 is Bob (evaluator). For each gate instance the
//! dealer:
//! - garbles [`ShareConverted`] with fresh randomness (half-gates, free-XOR, via swanky),
//! - samples Alice's mask `R`, gives Alice `z0 = -R`, and puts the active labels of `R` next
//!   to the garbled circuit (they do not depend on any input, so no one has to send them later),
//! - gives Alice both labels of every `x0` and `x1` wire,
//! - pregenerates one random OT per `x1` wire (§4.1.1): Alice gets `(m0, m1)`, Bob gets a
//!   random choice bit `c` and `m_c`.
//!
//! Online, one round trip (three steps):
//! 1. Bob sends `w = c XOR x1` ([`EvaluatorKey::request`]),
//! 2. Alice sends the labels of her `x0` bits and, for each `x1` wire, `(q0, q1) =
//!    (s0 ^ m_w, s1 ^ m_{1^w})` where `s0, s1` are the wire's labels ([`GarblerKey::respond`]),
//! 3. Bob takes `q_{x1} ^ m_c`, the label of his bit, evaluates the circuit, and decodes
//!    `z1 = f(x0 + x1) + R` ([`EvaluatorKey::finish`]).
//!
//! Every key is single-use: evaluating a garbled circuit twice leaks.

use crate::circuits::ShareConverted;
use crate::ring::{bit, Ring};
use crate::Error;
use fancy_garbling::classic::{GarbledCircuit, OutputMapping};
use fancy_garbling::{WireLabel, WireMod2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateKind {
    /// `1{x < 0}` (Algorithm 2).
    Lt0,
    /// `x >>_A shift` (Algorithm 3).
    Ars { shift: u32 },
}

impl GateKind {
    pub fn parse(name: &str, shift: Option<u32>) -> Result<GateKind, Error> {
        match (name, shift) {
            ("lt0", _) => Ok(GateKind::Lt0),
            ("ars", Some(shift)) => Ok(GateKind::Ars { shift }),
            ("ars", None) => Err(Error::new("gate `ars` needs --shift")),
            _ => Err(Error::new(format!("unknown gate `{name}` (expected lt0 or ars)"))),
        }
    }

    /// Plaintext reference: what the gate computes on the ring element `x`.
    pub fn reference(&self, ring: Ring, x: u128) -> u128 {
        match self {
            GateKind::Lt0 => ring.msb(x) as u128,
            GateKind::Ars { shift } => ring.from_signed(ring.to_signed(x) >> shift),
        }
    }
}

impl std::fmt::Display for GateKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GateKind::Lt0 => write!(f, "lt0"),
            GateKind::Ars { shift } => write!(f, "ars(s={shift})"),
        }
    }
}

/// A wire label as the 128-bit value that travels over the channel.
type Label = u128;

fn to_label(w: &WireMod2) -> Label {
    u128::from_le_bytes(w.to_repr().into())
}

fn from_label(l: Label) -> WireMod2 {
    WireMod2::from_repr(l.to_le_bytes().into(), 2)
}

/// Alice's (party 0's) material for one gate instance.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GarblerKey {
    ring: Ring,
    kind: GateKind,
    /// `[label of 0, label of 1]` for each `x0` wire, LSB first.
    x0_labels: Vec<[Label; 2]>,
    /// The same for each `x1` wire.
    x1_labels: Vec<[Label; 2]>,
    /// Random OT sender strings `(m0, m1)`, one pair per `x1` wire.
    ot: Vec<[Label; 2]>,
    /// `-R`.
    z_share: u128,
}

/// Bob's (party 1's) material for one gate instance.
#[derive(Serialize, Deserialize)]
pub struct EvaluatorKey {
    ring: Ring,
    kind: GateKind,
    gc: GarbledCircuit,
    outputs: OutputMapping,
    /// Active labels of the `R` wires.
    r_labels: Vec<Label>,
    /// Random OT choice bits `c`, bit j for `x1` wire j.
    ot_choice: u128,
    /// `m_c` for each `x1` wire.
    ot_labels: Vec<Label>,
}

/// Fresh keys for one gate instance.
pub fn gen(kind: GateKind, ring: Ring) -> Result<(GarblerKey, EvaluatorKey), Error> {
    let circuit = ShareConverted::new(kind, ring)?;
    let n = ring.bits() as usize;
    let (encoder, gc, outputs) = GarbledCircuit::garble::<WireMod2, _, _>(&circuit, rand::rng())?;
    let encode = |b: u16| encoder.encode_inputs(&vec![b; circuit.ninputs()]).iter().map(to_label).collect::<Vec<_>>();
    let (zeros, ones) = (encode(0), encode(1));
    let pair = |i: usize| [zeros[i], ones[i]];

    let r = ring.random();
    let r_labels = (0..n).map(|j| pair(2 * n + j)[bit(r, j as u32) as usize]).collect();
    let ot: Vec<[Label; 2]> = (0..n).map(|_| [rand::random(), rand::random()]).collect();
    let ot_choice = ring.random();
    let ot_labels = (0..n).map(|j| ot[j][bit(ot_choice, j as u32) as usize]).collect();
    Ok((
        GarblerKey {
            ring,
            kind,
            x0_labels: (0..n).map(pair).collect(),
            x1_labels: (n..2 * n).map(pair).collect(),
            ot,
            z_share: ring.neg(r),
        },
        EvaluatorKey { ring, kind, gc, outputs, r_labels, ot_choice, ot_labels },
    ))
}

impl GarblerKey {
    pub fn ring(&self) -> Ring {
        self.ring
    }

    pub fn kind(&self) -> GateKind {
        self.kind
    }

    /// Length of Alice's message per instance: n `x0` labels, then n OT pairs.
    pub fn message_len(&self) -> usize {
        3 * self.ring.bits() as usize
    }

    /// Step 2. Given Alice's share `x0` and Bob's OT correction `w`, returns her message to
    /// Bob and her output share `-R`.
    pub fn respond(&self, x0: u128, w: u128) -> (Vec<u128>, u128) {
        let n = self.ring.bits();
        let mut msg = Vec::with_capacity(self.message_len());
        msg.extend((0..n).map(|j| self.x0_labels[j as usize][bit(x0, j) as usize]));
        for j in 0..n {
            let [s0, s1] = self.x1_labels[j as usize];
            let [m0, m1] = self.ot[j as usize];
            let (mw, mw1) = if bit(w, j) { (m1, m0) } else { (m0, m1) };
            msg.push(s0 ^ mw);
            msg.push(s1 ^ mw1);
        }
        (msg, self.z_share)
    }
}

impl EvaluatorKey {
    pub fn ring(&self) -> Ring {
        self.ring
    }

    pub fn kind(&self) -> GateKind {
        self.kind
    }

    /// Step 1. Bob's OT correction `w = c XOR x1`, all n bits in one ring element.
    pub fn request(&self, x1: u128) -> u128 {
        self.ring.reduce(self.ot_choice ^ x1)
    }

    /// Step 3. Evaluates the garbled circuit on Alice's message and returns Bob's output
    /// share `z1 = f(x0 + x1) + R`.
    pub fn finish(&self, x1: u128, msg: &[u128]) -> Result<u128, Error> {
        let n = self.ring.bits() as usize;
        if msg.len() != 3 * n {
            return Err(Error::new(format!("garbler message has {} labels, expected {}", msg.len(), 3 * n)));
        }
        let (x0_labels, ot_msgs) = msg.split_at(n);
        let x1_labels = (0..n).map(|j| ot_msgs[2 * j + bit(x1, j as u32) as usize] ^ self.ot_labels[j]);
        let inputs: Vec<WireMod2> = x0_labels
            .iter()
            .copied()
            .chain(x1_labels)
            .chain(self.r_labels.iter().copied())
            .map(from_label)
            .collect();
        let circuit = ShareConverted::new(self.kind, self.ring)?;
        let bits = self.gc.eval(&circuit, inputs, &self.outputs)?;
        Ok(bits.iter().enumerate().fold(0, |z, (i, &b)| z | ((b as u128) << i)))
    }

    /// Size of the garbled circuit in 16-byte blocks.
    pub fn garbled_blocks(&self) -> usize {
        self.gc.size()
    }
}

/// Everything one party needs from the dealer for one gate instance.
#[derive(Serialize, Deserialize)]
pub enum PartyKey {
    Garbler(GarblerKey),
    Evaluator(EvaluatorKey),
}

impl PartyKey {
    pub fn party(&self) -> u8 {
        match self {
            PartyKey::Garbler(_) => 0,
            PartyKey::Evaluator(_) => 1,
        }
    }

    pub fn ring(&self) -> Ring {
        match self {
            PartyKey::Garbler(k) => k.ring(),
            PartyKey::Evaluator(k) => k.ring(),
        }
    }

    pub fn kind(&self) -> GateKind {
        match self {
            PartyKey::Garbler(k) => k.kind(),
            PartyKey::Evaluator(k) => k.kind(),
        }
    }
}

/// The dealer: fresh garbled circuits, masks and random OTs for `count` independent gate
/// instances. Party 0's keys come first.
pub fn deal(kind: GateKind, ring: Ring, count: usize) -> Result<(Vec<PartyKey>, Vec<PartyKey>), Error> {
    let mut keys0 = Vec::with_capacity(count);
    let mut keys1 = Vec::with_capacity(count);
    for _ in 0..count {
        let (g, e) = gen(kind, ring)?;
        keys0.push(PartyKey::Garbler(g));
        keys1.push(PartyKey::Evaluator(e));
    }
    Ok((keys0, keys1))
}
