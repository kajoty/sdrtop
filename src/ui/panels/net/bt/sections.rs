// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 MusiThang <viktor.laszlo92@protonmail.com>

//! What the Classic view and the Piconet view say alike about a piconet.
//!
//! **One set of limits.** The BR limits and the resolutions a reading must
//! beat are here once, so a figure is judged alike wherever it is drawn
//! (rule 5): the Classic view's summary line, the packet list's cells and
//! the bench's rows all read them from here.
//!
//! **The parts with one owner each.** The slot clock, the residual plot,
//! the HEADERS account and a piconet's channels as runs are drawn by one
//! function each, which ever panel shows them.

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::signal::bt::piconet::{Deviation, Piconet};
use crate::signal::dsp::uncertainty::Uncertain;
use crate::state::SdrMetrics;
use crate::ui::widgets::limit::Limit;

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

/// What a piconet's UAP rests on, in words, as the address mode shows it:
/// one value only a payload's CRC can choose (a header leaves two,
/// `header::PiconetClock`), with the CRCs that pass under it in the packets
/// kept; two candidates, where an encrypted link stays until a reconnect
/// sends a few packets in the clear; more, which further headers narrow; or
/// none narrowed yet. Candidates are listed while a reader can take them in
/// and never when masked, where two are half an address byte from one.
pub(super) fn uap_account(p: &Piconet, net: &crate::state::NetState) -> String {
    let masked = net.address_display == crate::state::AddressDisplay::Masked;
    let listed = |many: &[u8]| {
        if masked {
            String::new()
        } else {
            format!(
                " ({})",
                many.iter()
                    .map(|u| format!("{u:#04x}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    };
    match net.bt_uap.get(&p.lap).map(|u| u.as_slice()) {
        Some([one]) => {
            let passing = p
                .packets
                .iter()
                .filter(|k| k.payload == crate::signal::bt::piconet::PayloadVerdict::Crc(true))
                .count();
            let under = match passing {
                0 => String::new(),
                1 => " · 1 CRC passes under it".to_string(),
                n => format!(" · {n} CRCs pass under it"),
            };
            format!("{}, resolved by a payload CRC{under}", net.show_uap(*one))
        }
        Some(two @ [_, _]) => format!(
            "2 candidates{}: only a payload CRC chooses, which an encrypted link \
             gives only at a reconnect",
            listed(two)
        ),
        Some(few) if (3..=4).contains(&few.len()) => format!(
            "{} candidates{}; each further header narrows them",
            few.len(),
            listed(few)
        ),
        Some(many) if !many.is_empty() => {
            format!(
                "{} candidates; each further header narrows them",
                many.len()
            )
        }
        _ => "not narrowed: no header of it decoded yet".to_string(),
    }
}

/// The channels a piconet was heard on, as runs: `2-5, 17, 40-41`.
pub(super) fn channel_runs(mask: u128) -> String {
    let mut runs = Vec::new();
    let mut ch = 0u8;
    while ch < 79 {
        if mask & (1 << ch) == 0 {
            ch += 1;
            continue;
        }
        let start = ch;
        while ch + 1 < 79 && mask & (1 << (ch + 1)) != 0 {
            ch += 1;
        }
        runs.push(if start == ch {
            start.to_string()
        } else {
            format!("{start}-{ch}")
        });
        ch += 1;
    }
    runs.join(", ")
}

/// The residual plot's reach either side of the grid, µs: past the 1 µs
/// limit, so a residual beyond it shows as one.
const PLOT_US: f64 = 1.5;
/// Its height in rows of eighth blocks.
const PLOT_ROWS: usize = 3;

/// Where each hit sat against the grid:
/// residuals from −[`PLOT_US`] to +[`PLOT_US`] in bars of the piconet's
/// own colour, the specification's ±1 µs (2.2.5) as `┊` rules in the
/// warning ink, zero as a dim one, and a tick and label row under them. The
/// numbers say how wide the spread is; the shape says whether it is one
/// spread or two, as when a peripheral answers a little late on every slot
/// and stands as its own hump. Returns the lines, empty where the width
/// cannot hold a readable plot, and how many residuals fell beyond it.
pub(super) fn residual_histogram(
    residuals: &[f32],
    iw: usize,
    colour: ratatui::style::Color,
    theme: &crate::Theme,
) -> (Vec<Line<'static>>, usize) {
    const EIGHTHS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let cols = iw.saturating_sub(4);
    let beyond = residuals
        .iter()
        .filter(|r| (r.abs() as f64) >= PLOT_US)
        .count();
    if cols < 15 {
        return (Vec::new(), beyond);
    }
    let col_of = |x: f64| {
        (((x + PLOT_US) / (2.0 * PLOT_US)) * cols as f64)
            .floor()
            .clamp(0.0, cols as f64 - 1.0) as usize
    };
    let mut bins = vec![0u32; cols];
    for &r in residuals.iter().filter(|r| (r.abs() as f64) < PLOT_US) {
        bins[col_of(r as f64)] += 1;
    }
    let most = bins.iter().copied().max().unwrap_or(0).max(1);
    let limits = [col_of(-1.0), col_of(1.0)];
    let zero = col_of(0.0);
    let mut out = Vec::with_capacity(PLOT_ROWS + 2);
    for row in 0..PLOT_ROWS {
        let base = (PLOT_ROWS - 1 - row) * 8;
        let mut spans = vec![Span::raw("  ")];
        for (c, &n) in bins.iter().enumerate() {
            let fill = ((n as f64 / most as f64 * (PLOT_ROWS * 8) as f64).round() as usize)
                .saturating_sub(base)
                .min(8);
            spans.push(if fill > 0 {
                Span::styled(EIGHTHS[fill].to_string(), Style::default().fg(colour))
            } else if limits.contains(&c) {
                Span::styled("\u{250a}", Style::default().fg(theme.status_warn))
            } else if c == zero {
                Span::styled("\u{250a}", Style::default().fg(theme.border_dim))
            } else {
                Span::raw(" ")
            });
        }
        out.push(Line::from(spans));
    }
    let ticks: String = (0..cols)
        .map(|c| {
            if limits.contains(&c) || c == zero {
                '\u{2534}'
            } else {
                '\u{2500}'
            }
        })
        .collect();
    out.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(ticks, Style::default().fg(theme.border_dim)),
    ]));
    let mut labels = vec![' '; cols];
    for (c, text) in [(limits[0], "-1"), (zero, "0"), (limits[1], "+1")] {
        let at = c.saturating_sub(text.len() / 2).min(cols - text.len());
        for (i, ch) in text.chars().enumerate() {
            labels[at + i] = ch;
        }
    }
    out.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            labels.into_iter().collect::<String>(),
            Style::default().fg(theme.label),
        ),
    ]));
    (out, beyond)
}

/// What the piconet's headers say, under its own heading, marked as the
/// port it is: the header decode is `libbtbb`'s, checked on the air
/// against two devices whose addresses were read off them
/// (`signal::bt::header`, rule 1).
///
/// **Read only under one UAP.** Before the UAP is one value the heading
/// says so and how many headers are waiting; no type is guessed from a
/// candidate (rule 2). A header that did not decode under the resolved UAP
/// is counted beside the ones that did, because a rising count is how a
/// wrong resolution would show.
pub(super) fn header_lines(
    p: &Piconet,
    state: &SdrMetrics,
    iw: usize,
    theme: &crate::Theme,
) -> Vec<Line<'static>> {
    use crate::signal::bt::header::PacketType;
    let h = &p.headers;
    let field = |label: &str, value: String| {
        Line::from(vec![
            crate::ui::chrome::field(label, LABEL_W, theme),
            Span::styled(value, Style::default().fg(theme.value)),
        ])
    };
    let mut out = vec![crate::ui::chrome::section(
        "headers",
        "libbtbb port, checked on air",
        iw,
        theme,
    )];
    if h.captured == 0 {
        out.push(field(
            "captured",
            "none yet: no header followed a hit".to_string(),
        ));
        return out;
    }
    let uap = match state.net.bt_uap.get(&p.lap).map(|u| u.as_slice()) {
        Some([one]) => *one,
        other => {
            let n = other.map_or(0, |u| u.len());
            out.push(field(
                "captured",
                format!(
                    "{}, not read: UAP not resolved ({n} candidates)",
                    h.captured
                ),
            ));
            out.push(field("clock", clock_text(h.clock_hypotheses)));
            return out;
        }
    };
    let mut read = format!("{} of {} captured", h.decoded, h.captured);
    if h.undecoded > 0 {
        read.push_str(&format!(
            ", {} did not decode under {}",
            h.undecoded,
            state.net.show_uap(uap)
        ));
    }
    out.push(field("read", read));
    let mut mix: Vec<(u32, u8)> = (0..16u8)
        .map(|c| (h.types[c as usize], c))
        .filter(|(n, _)| *n > 0)
        .collect();
    mix.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let room = iw.saturating_sub(LABEL_W + 1);
    let types = if mix.is_empty() {
        "-".to_string()
    } else {
        mix.iter()
            .map(|(n, c)| format!("{} {n}", PacketType::from_code(*c).shown()))
            .collect::<Vec<_>>()
            .join(" \u{00b7} ")
    };
    for (i, chunk) in crate::ui::chrome::wrap(&types, room, 2)
        .into_iter()
        .enumerate()
    {
        out.push(field(if i == 0 { "types" } else { "" }, chunk));
    }
    let addrs: Vec<String> = (0..8u8)
        .filter(|a| h.lt_addrs & (1 << a) != 0)
        .map(|a| {
            if a == 0 {
                "0 (broadcast)".to_string()
            } else {
                a.to_string()
            }
        })
        .collect();
    out.push(field(
        "LT_ADDR",
        if addrs.is_empty() {
            "-".to_string()
        } else {
            addrs.join(", ")
        },
    ));
    out.push(field("clock", clock_text(h.clock_hypotheses)));
    out
}

/// The CLK1-6 hunt, in words: the whitening every header is read through
/// depends on it.
fn clock_text(hypotheses: u8) -> String {
    match hypotheses {
        0 => "CLK1-6 not tracked yet".to_string(),
        1 => "CLK1-6 found (1 of 64 hypotheses left)".to_string(),
        n => format!("CLK1-6: {n} of 64 hypotheses left"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The residuals as a shape**: each lands in its column, the
    /// ±1 µs limits and zero are ruled and labelled, and a residual past
    /// the plot is counted rather than dropped.
    #[test]
    fn the_residual_histogram_places_each_hit_and_the_limits() {
        let theme = crate::Theme::sdr();
        let colour = theme.series_color(1);
        // 40 columns across 3 us: 0.075 us each.
        let (lines, beyond) =
            residual_histogram(&[0.0, 0.01, 0.02, 0.9, -0.5, 2.0], 44, colour, &theme);
        assert_eq!(beyond, 1);
        assert_eq!(lines.len(), PLOT_ROWS + 2);
        let text = |l: &Line| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        };
        // The tallest bar (three residuals near zero) reaches the top row.
        let top = text(&lines[0]);
        assert_eq!(top.chars().nth(2 + 20), Some('\u{2588}'), "{top:?}");
        // Limits ruled at -1 and +1 (columns 6 and 33), labelled below.
        assert_eq!(top.chars().nth(2 + 6), Some('\u{250a}'), "{top:?}");
        assert_eq!(top.chars().nth(2 + 33), Some('\u{250a}'), "{top:?}");
        let labels = text(&lines[PLOT_ROWS + 1]);
        assert!(labels.contains("-1") && labels.contains("+1"), "{labels:?}");
        // The limit rules are in the warning ink, the bars in the colour.
        let limit_span = lines[0]
            .spans
            .iter()
            .find(|s| s.content == "\u{250a}")
            .unwrap();
        assert_eq!(limit_span.style.fg, Some(theme.status_warn));
        let bar = lines[2]
            .spans
            .iter()
            .find(|s| s.content != " " && s.content != "\u{250a}" && s.content != "  ")
            .unwrap();
        assert_eq!(bar.style.fg, Some(colour));
        // Too narrow for a readable plot: none, and still the count.
        assert_eq!(
            residual_histogram(&[3.0], 16, colour, &theme),
            (Vec::new(), 1)
        );
    }

    #[test]
    fn channel_runs_join_neighbours() {
        assert_eq!(channel_runs(0), "");
        assert_eq!(channel_runs(0b1111 << 2 | 1 << 17 | 1 << 78), "2-5, 17, 78");
    }
}
