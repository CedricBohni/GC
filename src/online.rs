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
/// ch being the channel used
pub fn eval_shared<C: Channel>(keys: &[PartyKey], x_shares: &[u128], ch: &mut C) -> Result<Vec<u128>, Error> {
    if keys.len() != x_shares.len() {
        return Err(Error::new(format!("{} keys but {} input shares", keys.len(), x_shares.len())));
    }
    let x_shares: Vec<u128> = keys.iter().zip(x_shares).map(|(k, &x)| k.ring().reduce(x)).collect();
    
    // if party is 0 do garbler if 1 do evaluator
    match keys.first().map(PartyKey::party) {
        None => Ok(Vec::new()),
        Some(0) => garbler(&garbler_keys(keys)?, &x_shares, ch),
        Some(_) => evaluator(&evaluator_keys(keys)?, &x_shares, ch),
    }
}

// knows its keys, x0 and channel
fn garbler<C: Channel>(keys: &[&GarblerKey], x0: &[u128], ch: &mut C) -> Result<Vec<u128>, Error> {
    // get w from evaluator (w = x_1 xor c) x_1 is not reveiled
    let w = ch.exchange(&[])?;
    if w.len() != keys.len() {
        return Err(Error::new(format!("peer sent {} OT corrections, expected {}", w.len(), keys.len())));
    }
    
    // prepare output buffer
    let mut msg = Vec::with_capacity(keys.iter().map(|k| k.message_len()).sum());
    let mut out = Vec::with_capacity(keys.len());
    
    // for each k (keys of garbler), x (x_0 shares), w (OT correction)
    for ((k, &x), &w) in keys.iter().zip(x0).zip(&w) {
        /// if w = 0:  e0 = s0 ⊕ m0,  e1 = s1 ⊕ m1
        /// if w = 1:  e0 = s0 ⊕ m1,  e1 = s1 ⊕ m0
        let (m, z0) = k.respond(x, w);
        msg.extend(m);
        out.push(z0);
    }

    // send to evaluator for them to compute f(x_0 + x_1) + R
    ch.exchange(&msg)?;
    // the -R as output
    Ok(out)
}

fn evaluator<C: Channel>(keys: &[&EvaluatorKey], x1: &[u128], ch: &mut C) -> Result<Vec<u128>, Error> {
    // compute and send OT corrections w = c xor x1 for each gate (confiming future choice in OT)
    let w: Vec<u128> = keys.iter().zip(x1).map(|(k, &x)| k.request(x)).collect();
    ch.exchange(&w)?;

    /// receive from garbler with every 3n values
    /// [ x0 label, bit 0 … bit n-1 ]            n labels: active labels for x0 (the line you asked about)
    /// [ s0⊕m, s1⊕m' ] for bit 0 … bit n-1       2n labels: both x1 labels, each masked with an OT pad
    let msg = ch.exchange(&[])?;
    
    // check length
    let want: usize = keys.iter().map(|k| 3 * k.ring().bits() as usize).sum();
    if msg.len() != want {
        return Err(Error::new(format!("peer sent {} labels, expected {want}", msg.len())));
    }

    let mut rest = &msg[..];
    let mut out = Vec::with_capacity(keys.len());
    for (k, &x) in keys.iter().zip(x1) {
        // split into 3 n parts
        let (m, tail) = rest.split_at(3 * k.ring().bits() as usize);
        // 
        out.push(k.finish(x, m)?);
        rest = tail;
    }
    Ok(out)
}

/// retrieve keys of the garbler
fn garbler_keys(keys: &[PartyKey]) -> Result<Vec<&GarblerKey>, Error> {
    keys.iter()
        .map(|k| match k {
            PartyKey::Garbler(g) => Ok(g),
            PartyKey::Evaluator(_) => Err(Error::new("key list mixes party 0 and party 1 keys")),
        })
        .collect()
}

/// retrieve keys of the evaluator
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
