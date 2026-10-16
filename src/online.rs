//! The online phase between the two computing parties, over any [`Channel`].
//!
//! Party 0 holds garbler keys, party 1 evaluator keys. With `x_b` its additive input share:
//! 1. party 1 sends its OT corrections (one input-ring element per instance, as
//!    [`Ring::words`] 128-bit words),
//! 2. party 0 sends its `x0` input labels and the OT replies (3w labels per instance), and
//!    outputs `-R`,
//! 3. party 1 evaluates the garbled circuits and outputs `f(x) + R`.
//!
//! Input shares are in each key's input ring, output shares in its output ring.
//!
//! One round trip, batched over all instances. [`Channel::exchange`] is symmetric, so each
//! step is one exchange in which the other party sends an empty message.
//!
//! [`eval_shared`] consumes the keys, since every key is single-use.

use crate::gates::{EvaluatorKey, GarblerKey, PartyKey};
use crate::ring::{Ring, U256};
use crate::transport::Channel;
use crate::Error;

/// Run the gates on additively shared inputs (in each key's input ring). Returns this party's
/// shares of the outputs (in each key's output ring).
/// ch being the channel used. Consumes the keys: each one is single-use.
pub fn eval_shared<C: Channel>(keys: Vec<PartyKey>, x_shares: &[U256], ch: &mut C) -> Result<Vec<U256>, Error> {
    if keys.len() != x_shares.len() {
        return Err(Error::new(format!("{} keys but {} input shares", keys.len(), x_shares.len())));
    }
    let x_shares: Vec<U256> = keys.iter().zip(x_shares).map(|(k, &x)| k.input_ring().reduce(x)).collect();
    
    // if party is 0 do garbler if 1 do evaluator
    match keys.first().map(PartyKey::party) {
        None => Ok(Vec::new()),
        Some(0) => garbler(garbler_keys(keys)?, &x_shares, ch),
        Some(_) => evaluator(evaluator_keys(keys)?, &x_shares, ch),
    }
}

// knows its keys, x0 and channel
fn garbler<C: Channel>(keys: Vec<GarblerKey>, x0: &[U256], ch: &mut C) -> Result<Vec<U256>, Error> {
    // get w from evaluator (w = x_1 xor c) x_1 is not reveiled
    let rings: Vec<Ring> = keys.iter().map(GarblerKey::input_ring).collect();
    let w = decode(&rings, &ch.exchange(&[])?).map_err(|e| Error::new(format!("peer's OT corrections: {e}")))?;
    
    // prepare output buffer
    let mut msg = Vec::with_capacity(keys.iter().map(|k| k.message_len()).sum());
    let mut out = Vec::with_capacity(keys.len());
    
    // for each k (keys of garbler), x (x_0 shares), w (OT correction)
    for ((k, &x), &w) in keys.into_iter().zip(x0).zip(&w) {
        // if w = 0:  e0 = s0 ⊕ m0,  e1 = s1 ⊕ m1
        // if w = 1:  e0 = s0 ⊕ m1,  e1 = s1 ⊕ m0
        let (m, z0) = k.respond(x, w);
        msg.extend(m);
        out.push(z0);
    }

    // send to evaluator for them to compute f(x_0 + x_1) + R
    ch.exchange(&msg)?;
    // the -R as output
    Ok(out)
}

fn evaluator<C: Channel>(keys: Vec<EvaluatorKey>, x1: &[U256], ch: &mut C) -> Result<Vec<U256>, Error> {
    // compute and send OT corrections w = c xor x1 for each gate (confiming future choice in OT)
    let rings: Vec<Ring> = keys.iter().map(EvaluatorKey::input_ring).collect();
    let w: Vec<U256> = keys.iter().zip(x1).map(|(k, &x)| k.request(x)).collect();
    ch.exchange(&encode(&rings, &w))?;

    // receive from garbler with every 3w values
    // [ x0 label, bit 0 … bit n-1 ]            n labels: active labels for x0
    // [ s0⊕m, s1⊕m' ] for bit 0 … bit n-1       2n labels: both x1 labels, each masked with an OT pad
    let msg = ch.exchange(&[])?;
    
    // check length
    let want: usize = keys.iter().map(|k| 3 * k.input_ring().bits() as usize).sum();
    if msg.len() != want {
        return Err(Error::new(format!("peer sent {} labels, expected {want}", msg.len())));
    }

    let mut rest = &msg[..];
    let mut out = Vec::with_capacity(keys.len());
    for (k, &x) in keys.into_iter().zip(x1) {
        // split into 3 w parts
        let (m, tail) = rest.split_at(3 * k.input_ring().bits() as usize);
        out.push(k.finish(x, m)?);
        rest = tail;
    }
    Ok(out)
}

/// retrieve keys of the garbler
fn garbler_keys(keys: Vec<PartyKey>) -> Result<Vec<GarblerKey>, Error> {
    keys.into_iter()
        .map(|k| match k {
            PartyKey::Garbler(g) => Ok(g),
            PartyKey::Evaluator(_) => Err(Error::new("key list mixes party 0 and party 1 keys")),
        })
        .collect()
}

/// retrieve keys of the evaluator
fn evaluator_keys(keys: Vec<PartyKey>) -> Result<Vec<EvaluatorKey>, Error> {
    keys.into_iter()
        .map(|k| match k {
            PartyKey::Evaluator(e) => Ok(e),
            PartyKey::Garbler(_) => Err(Error::new("key list mixes party 0 and party 1 keys")),
        })
        .collect()
}

/// Reveal additively shared values in `ring` to both parties.
pub fn reveal<C: Channel>(ring: Ring, shares: &[U256], ch: &mut C) -> Result<Vec<U256>, Error> {
    let rings = vec![ring; shares.len()];
    let theirs = decode(&rings, &ch.exchange(&encode(&rings, shares))?).map_err(|e| Error::new(format!("peer's shares: {e}")))?;
    Ok(shares.iter().zip(theirs).map(|(&a, b)| ring.add(a, b)).collect())
}

/// One ring element per ring, each as [`Ring::words`] 128-bit words.
fn encode(rings: &[Ring], xs: &[U256]) -> Vec<u128> {
    rings.iter().zip(xs).flat_map(|(r, &x)| r.to_words(x)).collect()
}

/// Inverse of [`encode`]. Fails unless `words` holds exactly one in-range element per ring.
fn decode(rings: &[Ring], words: &[u128]) -> Result<Vec<U256>, Error> {
    let want: usize = rings.iter().map(Ring::words).sum();
    if words.len() != want {
        return Err(Error::new(format!("got {} words, expected {want}", words.len())));
    }
    let mut rest = words;
    rings
        .iter()
        .map(|r| {
            let (w, tail) = rest.split_at(r.words());
            rest = tail;
            r.from_words(w)
        })
        .collect()
}
