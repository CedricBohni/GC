//! The online phase between the two computing parties, over any [`Channel`].
//!
//! Party 0 holds garbler keys, party 1 evaluator keys. With `x_b` its additive input share:
//! 1. party 1 sends its OT corrections (one ring element per instance),
//! 2. party 0 sends its `x0` input labels and the OT replies, and outputs `-R`,
//! 3. party 1 evaluates the garbled circuits and outputs `f(x) + R`.
//!
//! One round trip, batched over all instances. [`Channel::exchange`] is symmetric, so each
//! step is one exchange in which the other party sends an empty message.

use crate::gates::{EvaluatorKey, GarblerKey, PartyKey};
use crate::transport::Channel;
use crate::Error;

/// Run the gates on additively shared inputs. Returns this party's shares of the outputs.
pub fn eval_shared<C: Channel>(keys: &[PartyKey], x_shares: &[u128], ch: &mut C) -> Result<Vec<u128>, Error> {
    if keys.len() != x_shares.len() {
        return Err(Error::new(format!("{} keys but {} input shares", keys.len(), x_shares.len())));
    }
    let x_shares: Vec<u128> = keys.iter().zip(x_shares).map(|(k, &x)| k.ring().reduce(x)).collect();
    match keys.first().map(PartyKey::party) {
        None => Ok(Vec::new()),
        Some(0) => garbler(&garbler_keys(keys)?, &x_shares, ch),
        Some(_) => evaluator(&evaluator_keys(keys)?, &x_shares, ch),
    }
}

fn garbler<C: Channel>(keys: &[&GarblerKey], x0: &[u128], ch: &mut C) -> Result<Vec<u128>, Error> {
    let w = ch.exchange(&[])?;
    if w.len() != keys.len() {
        return Err(Error::new(format!("peer sent {} OT corrections, expected {}", w.len(), keys.len())));
    }
    let mut msg = Vec::with_capacity(keys.iter().map(|k| k.message_len()).sum());
    let mut out = Vec::with_capacity(keys.len());
    for ((k, &x), &w) in keys.iter().zip(x0).zip(&w) {
        let (m, z0) = k.respond(x, w);
        msg.extend(m);
        out.push(z0);
    }
    ch.exchange(&msg)?;
    Ok(out)
}

fn evaluator<C: Channel>(keys: &[&EvaluatorKey], x1: &[u128], ch: &mut C) -> Result<Vec<u128>, Error> {
    let w: Vec<u128> = keys.iter().zip(x1).map(|(k, &x)| k.request(x)).collect();
    ch.exchange(&w)?;
    let msg = ch.exchange(&[])?;
    let want: usize = keys.iter().map(|k| 3 * k.ring().bits() as usize).sum();
    if msg.len() != want {
        return Err(Error::new(format!("peer sent {} labels, expected {want}", msg.len())));
    }
    let mut rest = &msg[..];
    let mut out = Vec::with_capacity(keys.len());
    for (k, &x) in keys.iter().zip(x1) {
        let (m, tail) = rest.split_at(3 * k.ring().bits() as usize);
        out.push(k.finish(x, m)?);
        rest = tail;
    }
    Ok(out)
}

fn garbler_keys(keys: &[PartyKey]) -> Result<Vec<&GarblerKey>, Error> {
    keys.iter()
        .map(|k| match k {
            PartyKey::Garbler(g) => Ok(g),
            PartyKey::Evaluator(_) => Err(Error::new("key list mixes party 0 and party 1 keys")),
        })
        .collect()
}

fn evaluator_keys(keys: &[PartyKey]) -> Result<Vec<&EvaluatorKey>, Error> {
    keys.iter()
        .map(|k| match k {
            PartyKey::Evaluator(e) => Ok(e),
            PartyKey::Garbler(_) => Err(Error::new("key list mixes party 0 and party 1 keys")),
        })
        .collect()
}

/// Reveal additively shared values to both parties.
pub fn reveal<C: Channel>(keys: &[PartyKey], shares: &[u128], ch: &mut C) -> Result<Vec<u128>, Error> {
    let theirs = ch.exchange(shares)?;
    if theirs.len() != shares.len() {
        return Err(Error::new(format!("peer sent {} values, expected {}", theirs.len(), shares.len())));
    }
    Ok(keys
        .iter()
        .zip(shares.iter().zip(theirs))
        .map(|(k, (&a, b))| k.ring().add(a, b))
        .collect())
}
