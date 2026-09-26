// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 MusiThang <viktor.laszlo92@protonmail.com>

//! The carrier under a GFSK burst: where it starts and how it moves, read as
//! the Bluetooth test suites define them, from whatever bits were sent.
//!
//! **The suites' definitions.** The initial carrier f0 is the mean
//! frequency from the centre of the first preamble bit to the centre of the
//! bit after the preamble: 4 bits on BR (RF.TS.p35 RF/TRM/CA/BV-08-C d), 8 on
//! LE (RF-PHY.TS.4.2.1 TP/TRM-LE/CA/BV-06-C step 4). The drift readings fk
//! are the mean frequency over every ten bits from the second payload bit
//! (RF/TRM/CA/BV-09-C e, TP/TRM-LE/CA/BV-06-C step 6). The preamble
//! alternates, so its mean is the carrier as it stands.
//!
//! **Ten bits of traffic are not ten bits of `1010`.** The suites send an
//! alternation for the drift test, and five whole periods of one average to
//! the carrier exactly. Whitened traffic does not balance over ten bits: six
//! ones and four zeros put a BR block's mean some 30 kHz off the carrier,
//! which the suites' definition would call drift. So each bit's own
//! modulation is taken out before the blocks are averaged ([`by_bit`]): a
//! bit's mean reading depends on the bit and its two neighbours and nothing
//! further (a symbol two bits away moves it by about 1e-8 of the deviation,
//! `deviation::suite_readings` has the figure), so the burst itself shows
//! what each of the eight three-bit contexts adds, as the mean over every bit
//! with that context. What is left of a bit's reading is the carrier under
//! it. No pulse shape is assumed: the transmitter's own is what is measured.
//! `signal::net::conformance` holds the blocks this gives on random traffic
//! to the carrier the reference transmitter was given.

use super::deviation::READINGS_PER_BIT;

/// The mean of `at` over bit `k`, at [`READINGS_PER_BIT`] evenly spaced
/// instants: the suites' reading of one bit.
fn bit_mean(at: &impl Fn(f64) -> f32, k: usize) -> f64 {
    let n = READINGS_PER_BIT;
    (0..n)
        .map(|j| at(k as f64 + (j as f64 + 0.5) / n as f64) as f64)
        .sum::<f64>()
        / n as f64
}

/// The initial carrier f0: the mean of `at` (the frequency at `x` bit
/// periods from the start of bit 0) from the centre of bit `first` to the
/// centre of bit `first + preamble_bits`, read [`READINGS_PER_BIT`] times a
/// bit. The suites integrate; this is the same integral, sampled finely.
pub fn initial(at: impl Fn(f64) -> f32, first: usize, preamble_bits: usize) -> f64 {
    let n = READINGS_PER_BIT * preamble_bits;
    let start = first as f64 + 0.5;
    (0..n)
        .map(|j| at(start + (j as f64 + 0.5) / READINGS_PER_BIT as f64) as f64)
        .sum::<f64>()
        / n as f64
}

/// The carrier under each of `bits`, from its mean reading less what its
/// three-bit context adds, as the burst shows it; `None` for the two end
/// bits, which have no context, and for every bit unless both settled
/// contexts (`000` and `111`) occur, since the carrier is placed midway
/// between them.
///
/// **Fitted together with a straight-line carrier.** A context's mean is
/// taken at its members' mean time, and a carrier that drifts puts that
/// mean where the drift stood then: some 200 Hz on a 4 kHz drift over 200
/// bits, left in every bit of that context. So the readings are fitted as a
/// linear carrier plus one offset per context, by least squares (alternating
/// the two, which converges to it), and a bit's carrier is its reading less
/// its context's offset: a linear drift then leaves nothing behind, and a
/// curved one only its bend.
pub fn by_bit(bits: &[bool], at: impl Fn(f64) -> f32) -> Vec<Option<f64>> {
    const ROUNDS: usize = 12;
    let n = bits.len();
    let context =
        |k: usize| (bits[k - 1] as usize) << 2 | (bits[k] as usize) << 1 | bits[k + 1] as usize;
    let inner: Vec<(usize, usize, f64)> = (1..n.saturating_sub(1))
        .map(|k| (k, context(k), bit_mean(&at, k)))
        .collect();
    let counts = inner.iter().fold([0usize; 8], |mut c, &(_, ctx, _)| {
        c[ctx] += 1;
        c
    });
    if counts[0b000] == 0 || counts[0b111] == 0 || inner.len() < 3 {
        return vec![None; n];
    }
    let mut offset = [0.0f64; 8];
    let (mut c0, mut c1) = (0.0f64, 0.0f64);
    for _ in 0..ROUNDS {
        // The contexts' offsets, less the line.
        let mut sums = [0.0f64; 8];
        for &(k, ctx, m) in &inner {
            sums[ctx] += m - (c0 + c1 * k as f64);
        }
        for c in 0..8 {
            if counts[c] > 0 {
                offset[c] = sums[c] / counts[c] as f64;
            }
        }
        // The line, less the contexts' offsets.
        let (mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
        for &(k, ctx, m) in &inner {
            let (x, y) = (k as f64, m - offset[ctx]);
            sx += x;
            sy += y;
            sxx += x * x;
            sxy += x * y;
        }
        let count = inner.len() as f64;
        c1 = (count * sxy - sx * sy) / (count * sxx - sx * sx);
        c0 = (sy - c1 * sx) / count;
    }
    // The carrier sits midway between the settled ones and zeros.
    let shift = (offset[0b000] + offset[0b111]) / 2.0;
    let mut out = vec![None; n];
    for &(k, ctx, m) in &inner {
        out[k] = Some(m - offset[ctx] + shift);
    }
    out
}

/// The suites' drift readings over `carrier` (from [`by_bit`]): the mean of
/// every whole block of ten bits from `from`, up to `to`. A block missing a
/// bit's carrier is skipped rather than averaged over nine.
pub fn ten_bit_blocks(carrier: &[Option<f64>], from: usize, to: usize) -> Vec<f64> {
    let mut out = Vec::new();
    let mut k = from;
    while k + 10 <= to.min(carrier.len()) {
        let block: Option<Vec<f64>> = carrier[k..k + 10].iter().copied().collect();
        if let Some(b) = block {
            out.push(b.iter().sum::<f64>() / 10.0);
        }
        k += 10;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trace of carrier plus a constant step per bit value: every bit's
    /// carrier comes back as the carrier, whatever the balance of ones and
    /// zeros in a block, and a drifting carrier is followed.
    #[test]
    fn each_bit_gives_the_carrier_under_it() {
        let mut rng = crate::signal::dsp::testkit::Rng::new(4);
        let bits: Vec<bool> = (0..200).map(|_| rng.next_u64() & 1 == 1).collect();
        let carrier = |x: f64| 10_000.0 + 20.0 * x;
        let at = |x: f64| {
            let k = (x.floor() as usize).min(bits.len() - 1);
            (carrier(x) + if bits[k] { 150_000.0 } else { -150_000.0 }) as f32
        };
        let got = by_bit(&bits, at);
        assert!(got[0].is_none() && got[199].is_none());
        for (k, c) in got.iter().enumerate().skip(1).take(198) {
            let want = carrier(k as f64 + 0.5);
            assert!((c.unwrap() - want).abs() < 1.0, "{k}: {c:?} vs {want}");
        }
        let blocks = ten_bit_blocks(&got, 1, 199);
        assert_eq!(blocks.len(), 19);
        assert!((initial(at, 0, 4) - carrier(2.5)).abs() < 150_000.0);
    }

    /// Without both settled contexts there is nothing to place the carrier
    /// between: refused, not guessed.
    #[test]
    fn no_settled_bits_no_carrier() {
        let bits: Vec<bool> = (0..50).map(|i| i % 2 == 0).collect();
        assert!(by_bit(&bits, |_| 0.0).iter().all(Option::is_none));
    }
}
