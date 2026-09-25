//! Two-party garbled-circuit gates on additive shares, in the dealer model (1 dealer +
//! 2 semi-honest parties), following Hemenway, Lu, Ostrovsky, Welser: "High-precision Secure
//! Computation of Satellite Collision Probabilities", SCN 2016.
//!
//! Gates, over Z_{2^n} for any 1 <= n <= 128 on signed (two's complement) values:
//! - `lt0`: `1{x < 0}`, share-converted Less-Than-Zero (Algorithm 2),
//! - `ars`: arithmetic right shift by a public `s`, share-converted shift-right-by-constant
//!   (Algorithm 3).
//!
//! Roles:
//! - the dealer calls [`gates::deal`] and sends each party its `Vec<PartyKey>` (serde),
//! - each party runs [`online::eval_shared`] over a [`transport::Channel`].
//!
//! [`simulate`] runs dealer and both parties in-process, for tests.

pub mod circuits;
pub mod fixed;
pub mod gates;
pub mod online;
pub mod ring;
pub mod transport;

pub use fixed::FixedPoint;
pub use gates::{deal, GateKind, PartyKey};
pub use ring::Ring;

#[derive(Debug)]
pub struct Error(String);

impl Error {
    pub fn new(msg: impl Into<String>) -> Error {
        Error(msg.into())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error(format!("I/O error: {e}"))
    }
}

impl From<bincode::Error> for Error {
    fn from(e: bincode::Error) -> Error {
        Error(format!("serialization error: {e}"))
    }
}

impl From<swanky_error::Error> for Error {
    fn from(e: swanky_error::Error) -> Error {
        Error(format!("garbled circuit error: {e}"))
    }
}

/// Run one gate on the ring element `x` with fresh keys: the dealer deals, `x` is split into
/// random additive shares, both parties run the online phase, and the output is reconstructed.
pub fn simulate(kind: GateKind, ring: Ring, x: u128) -> Result<u128, Error> {
    let (alice, bob) = gates::gen(kind, ring)?;
    let (x0, x1) = ring.share(x);
    let w = bob.request(x1);
    let (msg, z0) = alice.respond(x0, w);
    let z1 = bob.finish(x1, &msg)?;
    Ok(ring.add(z0, z1))
}
