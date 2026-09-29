// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 MusiThang <viktor.laszlo92@protonmail.com>

//! The MODULATION and CARRIER rows of a classic piconet, drawn for one side.
//!
//! **One side, not one piconet.** The Piconets panel draws them for every
//! member's headers pooled; the Piconet view's bench draws them twice, the
//! master's and the slave's, under one heading. So the rows take the
//! accumulator they read (`piconet::Deviation`, `piconet::Carrier`) rather
//! than the piconet, and the heading, the footnote and the reason a side is
//! not measured stay with the caller, which knows what it counted.
//!
//! **One set of limits.** The BR limits and the resolutions a reading must
//! beat are here once, so a figure is judged alike wherever it is drawn
//! (rule 5).

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::signal::bt::piconet::{Carrier, Deviation};
use crate::signal::dsp::uncertainty::Uncertain;
use crate::state::SdrMetrics;
use crate::ui::widgets::limit::{Limit, LimitRow, RowWidths};
use crate::ui::widgets::reading::Reading;

/// Label width of a field row, the Piconets panel's detail block's and the
/// bench's.
pub(super) const LABEL_W: usize = 9;

/// BR's modulation index band, **read from the Core Specification 5.4,
/// Vol 2, Part A, 3.1.1** on the SIG's own site this session: "The
/// Modulation index shall be between 0.28 and 0.35" (GFSK, BT = 0.5,
/// 1 Msym/s).
pub(super) const BR_INDEX: Limit = Limit::Band {
    low: 0.28,
    high: 0.35,
};

/// The same band as a deviation: `h = 2 * delta_f / 1 Msym/s`, so 140 to
/// 175 kHz. Derived from [`BR_INDEX`], not a second figure from the text.
pub(super) const BR_DELTA_F1_KHZ: Limit = Limit::Band {
    low: 140.0,
    high: 175.0,
};

/// The same section: "the minimum frequency deviation, Fmin ... which
/// corresponds to 1010 sequence shall be no smaller than ±80% of the
/// frequency deviation (fd) ... which corresponds to a 00001111 sequence".
/// The text states it for the minimum; what is shown against it here is the
/// ratio of the means, which is what a header's symbols support, and the
/// row is labelled so.
pub(super) const BR_RATIO: Limit = Limit::Min(0.8);

/// Resolutions a reading must beat before it prints, as the BLE rows'
/// (`ble_detail`): a fraction of the band each limit states.
pub(super) const INDEX_RESOLUTION: f64 = 0.02;
pub(super) const DELTA_F1_RESOLUTION_KHZ: f64 = 10.0;
pub(super) const RATIO_RESOLUTION: f64 = 0.1;

/// The initial carrier's limit, **read from the Core Specification 5.4,
/// Vol 2, Part A, 3.1.3**: "The transmitted initial center frequency shall
/// be within ±75 kHz from Fc."
pub(super) const F0_LIMIT_KHZ: Limit = Limit::Band {
    low: -75.0,
    high: 75.0,
};

/// The drift's limit, **read from the same section's Table 3.3**: ±25 kHz
/// for a one-slot packet, ±40 kHz for three and five slots. What is read
/// here is the access code and header, which every packet type must keep
/// within 40 of its f0; a one-slot packet's 25 is over its whole length,
/// which a header cannot show. So 40 is what a reading is held to, and a
/// reading over it is a packet over its limit whatever its type.
pub(super) const DRIFT_LIMIT_KHZ: Limit = Limit::Band {
    low: -40.0,
    high: 40.0,
};

/// **The same table**: "Maximum drift rate 400 Hz/µs", allowed "anywhere in
/// a packet".
pub(super) const DRIFT_RATE_LIMIT: Limit = Limit::Band {
    low: -400.0,
    high: 400.0,
};

/// As the BLE rows' (`ble_detail`).
pub(super) const F0_RESOLUTION_KHZ: f64 = 10.0;
pub(super) const DRIFT_RESOLUTION_KHZ: f64 = 10.0;
pub(super) const DRIFT_RATE_RESOLUTION: f64 = 80.0;

/// Slot jitter's limit, **read from the Core Specification 5.4, Vol 2,
/// Part B, 2.2.5**: "The instantaneous timing shall not deviate more than
/// 1 μs from the average timing."
pub(super) const JITTER_LIMIT_US: Limit = Limit::Max(1.0);

/// A jitter reading must beat this before it prints: a quarter of the
/// limit, which a few dozen hits reach.
pub(super) const JITTER_RESOLUTION_US: f64 = 0.25;

/// The slot clock's limit, **read from the Core Specification 5.4, Vol 2,
/// Part B, 2.2.5**: "the average timing of packet transmission shall not
/// drift faster than 20 ppm relative to the ideal slot timing of 625 μs".
pub(super) const CLOCK_LIMIT_PPM: Limit = Limit::Band {
    low: -20.0,
    high: 20.0,
};

/// A clock reading must beat this before it prints: a twentieth of the
/// limit, which a minute of hits passes by far.
pub(super) const CLOCK_RESOLUTION_PPM: f64 = 1.0;

/// A piconet's slot clock from its fitted grid, the classic twin of the
/// census's crystal error: its slots run `rate_ppm` long on our clock, so
/// its clock runs that much slow, less our own oscillator's error, which a
/// reference takes out exactly as it does for a BLE offset
/// (`corrected_ppm`: the radio's LO and its sample clock come from one
/// crystal). With whether it is judged: only a reference makes it absolute,
/// and without one it is a reading, which the frame's [RELATIVE] says.
pub(super) fn clock_of(
    f: &crate::signal::bt::slots::SlotFit,
    state: &SdrMetrics,
) -> (Uncertain, bool) {
    let raw = Uncertain::from_sigma(-f.rate_ppm, f.rate_sigma_ppm);
    let (clock, provenance) = state.radio.corrected_ppm(raw, std::time::Instant::now());
    (clock, provenance != crate::state::Provenance::Unreferenced)
}

/// The modulation index a side's (or one packet's) settled readings give,
/// with its uncertainty: `h = 2 * df1 / 1 Msym/s`. `None` with fewer than
/// two readings.
pub(super) fn index_of(dev: &Deviation) -> Option<Uncertain> {
    dev.settled.mean().map(|df1| df1.scale(2.0 / 1e6))
}

/// The modulation rows of one side against the BR band, in the same
/// `widgets::limit` rows the BLE packet detail uses, so the two protocols'
/// transmitter quality reads alike: the index, df1, and df2/df1 once there
/// are alternating bits (a line saying so until then). `None` while there
/// are too few settled readings to measure; the caller says why, in its
/// own counts.
pub(super) fn modulation_rows(
    dev: &Deviation,
    iw: usize,
    theme: &crate::Theme,
) -> Option<Vec<Line<'static>>> {
    let df1 = dev.settled.mean()?;
    let index = index_of(dev)?;
    let mut rows = vec![
        LimitRow::new(
            "Mod index",
            Reading::new(index, "", INDEX_RESOLUTION),
            BR_INDEX,
        ),
        LimitRow::new(
            "df1 avg",
            Reading::new(df1.scale(0.001), "kHz", DELTA_F1_RESOLUTION_KHZ),
            BR_DELTA_F1_KHZ,
        ),
    ];
    let ratio = dev.alternating.mean().map(|df2| df2.ratio(&df1));
    if let Some(r) = ratio {
        rows.push(LimitRow::new(
            "df2/df1",
            Reading::new(r, "", RATIO_RESOLUTION),
            BR_RATIO,
        ));
    }
    let w = RowWidths::fit_within(&rows, iw);
    let mut out: Vec<Line<'static>> = rows.iter().map(|r| Line::from(r.spans(theme, w))).collect();
    if ratio.is_none() {
        out.push(Line::from(Span::styled(
            " df2/df1: fewer than two alternating bits yet".to_string(),
            Style::default().fg(theme.stale),
        )));
    }
    Some(out)
}

/// The carrier rows of one side: f0, and the worst header's drift and
/// drift rate, because the limits are on every packet. f0 is judged
/// against its limit only once a reference makes it absolute; until then
/// it is a plain field, relative to our own oscillator, as the clock row
/// is. `None` before two headers with enough blocks; the caller says so.
pub(super) fn carrier_rows(
    c: &Carrier,
    state: &SdrMetrics,
    iw: usize,
    theme: &crate::Theme,
) -> Option<Vec<Line<'static>>> {
    let (Some(f0), Some(mhz)) = (c.f0_ppm.mean(), c.channel_mhz.mean()) else {
        return None;
    };
    let now = std::time::Instant::now();
    let (ppm, provenance) = state.radio.corrected_ppm(f0, now);
    let khz = ppm.scale(mhz.value() / 1e3);
    let judged = provenance != crate::state::Provenance::Unreferenced;
    let mut rows = Vec::new();
    if judged {
        rows.push(LimitRow::new(
            "f0",
            Reading::new(khz, "kHz", F0_RESOLUTION_KHZ),
            F0_LIMIT_KHZ,
        ));
    }
    if let Some(d) = c.worst_drift_hz {
        rows.push(LimitRow::new(
            "Drift worst",
            Reading::new(d.scale(0.001), "kHz", DRIFT_RESOLUTION_KHZ),
            DRIFT_LIMIT_KHZ,
        ));
    }
    if let Some(r) = c.worst_rate_hz_per_us {
        rows.push(LimitRow::new(
            "Rate worst",
            Reading::new(r, "Hz/us", DRIFT_RATE_RESOLUTION),
            DRIFT_RATE_LIMIT,
        ));
    }
    let w = RowWidths::fit_within(&rows, iw);
    let mut out: Vec<Line<'static>> = rows.iter().map(|r| Line::from(r.spans(theme, w))).collect();
    if !judged {
        out.push(Line::from(vec![
            crate::ui::chrome::field("f0", LABEL_W, theme),
            Span::styled(
                Reading::new(khz, "kHz", F0_RESOLUTION_KHZ).text(),
                Style::default().fg(theme.value),
            ),
            Span::styled(
                "  relative to our own oscillator".to_string(),
                Style::default().fg(theme.label),
            ),
        ]));
    }
    Some(out)
}
