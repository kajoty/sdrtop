// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 MusiThang <viktor.laszlo92@protonmail.com>

//! `NetBtBenchPanel` - one classic piconet's measurement bench, the master
//! and the slave side by side.
//!
//! **Two devices, one instrument.** A piconet is two ends of one link, and
//! every figure the Piconets panel pools is here once for each end: the
//! master's column in the piconet's own colour, the slave's in the ordinary
//! ink, under one heading per section with the limit it is held to. The
//! rows are the Piconets panel's own (`sections`), drawn for one side, so a
//! figure reads the same wherever it stands (rule 5).
//!
//! **Only what a header placed.** A packet goes to a side once its header
//! was read at one clock (`piconet::Direction`); the rest are counted as
//! not yet placed and kept out of both columns, never guessed onto one
//! (rule 2).
//!
//! **Timing by side, on the piconet's grid.** The grid is fitted to every
//! member's packets (`signal::bt::slots`), so it belongs to neither end.
//! Each side's jitter is its packets' scatter about their own average, as
//! Core 5.4 Vol 2 Part B 2.2.5 states jitter, and how far the slave's
//! packets sit from the master's is the difference of the two averages,
//! which the grid's own placement cancels out of.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::bt_packets::selected;
use super::bt_piconets::{silence, uap_text};
use super::sections::{
    channel_runs, clock_of, header_lines, index_of, residual_histogram, BR_DELTA_F1_KHZ, BR_INDEX,
    BR_RATIO, CLOCK_LIMIT_PPM, CLOCK_RESOLUTION_PPM, DELTA_F1_RESOLUTION_KHZ, DRIFT_LIMIT_KHZ,
    DRIFT_RATE_LIMIT, DRIFT_RATE_RESOLUTION, DRIFT_RESOLUTION_KHZ, F0_LIMIT_KHZ, F0_RESOLUTION_KHZ,
    INDEX_RESOLUTION, JITTER_LIMIT_US, JITTER_RESOLUTION_US, RATIO_RESOLUTION,
};
use crate::signal::bt::header::PacketType;
use crate::signal::bt::piconet::{Direction, HeaderRead, Kind, PayloadVerdict, Piconet, Side};
use crate::signal::bt::slots::{spread, SlotRefusal, Spread, MIN_HITS};
use crate::signal::dsp::uncertainty::Uncertain;
use crate::state::{Provenance, SdrMetrics};
use crate::ui::panel::{FeedSpan, Panel, PanelChrome, Staleness};
use crate::ui::widgets::limit::LimitRow;
use crate::ui::widgets::reading::Reading;

pub struct NetBtBenchPanel;

/// Columns between the master's column and the slave's.
const GAP: usize = 1;

/// `line` cut to `width` columns, span by span, so a row too wide for its
/// column never runs into the next.
fn fit(line: Line<'static>, width: usize) -> Line<'static> {
    let mut left = width;
    let mut spans = Vec::new();
    for span in line.spans {
        if left == 0 {
            break;
        }
        let n = span.content.chars().count();
        if n <= left {
            left -= n;
            spans.push(span);
        } else {
            let cut: String = span.content.chars().take(left).collect();
            spans.push(Span::styled(cut, span.style));
            left = 0;
        }
    }
    Line::from(spans)
}

/// A quiet line, for what is not measured and why.
fn quiet(text: String, theme: &crate::Theme) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {text}"),
        Style::default().fg(theme.stale),
    ))
}

/// A note under a section: what its readings rest on, in the label ink.
fn footnote(text: &str, iw: usize, theme: &crate::Theme) -> Vec<Line<'static>> {
    crate::ui::chrome::wrap(text, iw.saturating_sub(1), 2)
        .into_iter()
        .map(|chunk| {
            Line::from(Span::styled(
                format!(" {chunk}"),
                Style::default().fg(theme.label),
            ))
        })
        .collect()
}

/// A field row across the whole bench: its label, a value, and what the
/// value is.
fn field(label: &str, value: String, note: &str, theme: &crate::Theme) -> Line<'static> {
    Line::from(vec![
        crate::ui::chrome::field(label, PAIR_LABEL, theme),
        Span::raw(" "),
        Span::styled(value, Style::default().fg(theme.value)),
        Span::styled(note.to_string(), Style::default().fg(theme.label)),
    ])
}

/// The longest label a paired row carries, `Drift worst`.
const PAIR_LABEL: usize = 11;

/// The widest a bar is drawn, so a wide bench does not stretch a gauge
/// into a line nobody reads end to end.
const BAR_MAX: usize = 32;

/// The bench's columns: the labels once, then the master's and the
/// slave's, a gap between.
#[derive(Clone, Copy)]
struct Columns {
    side: usize,
    /// Too narrow for two columns: each side's reading on a line of its
    /// own, marked with its arrow, and no bars.
    stacked: bool,
    master: Color,
    slave: Color,
}

impl Columns {
    const LABEL: usize = PAIR_LABEL + 2;

    /// The narrowest a side's column is drawn at: a reading, its
    /// uncertainty and its unit, `167.03 ±0.03 kHz`, with room to spare.
    const SIDE_MIN: usize = 26;

    fn of(iw: usize, master: Color, slave: Color) -> Self {
        let side = iw.saturating_sub(Self::LABEL + GAP) / 2;
        let stacked = side < Self::SIDE_MIN;
        Self {
            side: if stacked {
                iw.saturating_sub(Self::LABEL + 2)
            } else {
                side
            },
            stacked,
            master,
            slave,
        }
    }

    fn bar(self) -> usize {
        self.side.saturating_sub(1).min(BAR_MAX)
    }
}

/// One side's cell in a paired row.
enum Cell<'a> {
    /// Held to a limit: the reading, and its bar on the line below.
    Judged(LimitRow<'a>),
    /// A reading with nothing it can be held to yet (a relative offset).
    Plain(Reading<'a>),
    /// Plain text, a count.
    Text(String),
    /// Nothing on this side, and why, briefly.
    Missing(&'static str),
    /// A trace of the side's newest packets, oldest on the left, in the
    /// side's colour, with the span it is scaled to.
    Trend(String, String, Color),
}

impl Cell<'_> {
    fn spans(&self, theme: &crate::Theme) -> Vec<Span<'static>> {
        match self {
            Cell::Judged(row) => {
                let mut spans = row.reading_spans(theme);
                // The value wears amber or red only when it is not safely
                // inside, as in the packet list; the bar says the rest.
                if let (Some(c), Some(first)) = (row.flag_colour(theme), spans.first_mut()) {
                    first.style = first.style.fg(c);
                }
                spans
            }
            Cell::Plain(r) => r.spans(theme),
            Cell::Text(t) => vec![Span::styled(t.clone(), Style::default().fg(theme.value))],
            Cell::Missing(why) => vec![Span::styled(
                format!("— {why}"),
                Style::default().fg(theme.stale),
            )],
            Cell::Trend(trace, span, colour) => vec![
                Span::styled(trace.clone(), Style::default().fg(*colour)),
                Span::styled(format!("  {span}"), Style::default().fg(theme.label)),
            ],
        }
    }
}

/// `spans` padded or cut to exactly `width` columns.
fn exactly(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let line = fit(Line::from(spans), width);
    let pad = width.saturating_sub(line.width());
    let mut out = line.spans;
    out.push(Span::raw(" ".repeat(pad)));
    out
}

/// A paired row: the label, the master's cell and the slave's; then, where
/// either is held to a limit, both bars on a line of their own under them.
fn pair(
    label: &str,
    master: Cell<'_>,
    slave: Cell<'_>,
    c: Columns,
    theme: &crate::Theme,
) -> Vec<Line<'static>> {
    let mut first = vec![
        crate::ui::chrome::field(label, PAIR_LABEL, theme),
        Span::raw(" "),
    ];
    if c.stacked {
        let arrow = |a: &str, colour| Span::styled(a.to_string(), Style::default().fg(colour));
        first.push(arrow("▶ ", c.master));
        first.extend(exactly(master.spans(theme), c.side));
        let mut second = vec![Span::raw(" ".repeat(Columns::LABEL)), arrow("◀ ", c.slave)];
        second.extend(exactly(slave.spans(theme), c.side));
        return vec![Line::from(first), Line::from(second)];
    }
    first.extend(exactly(master.spans(theme), c.side + GAP));
    first.extend(exactly(slave.spans(theme), c.side));
    let mut out = vec![Line::from(first)];
    let bar = |cell: &Cell<'_>| match cell {
        Cell::Judged(row) if c.bar() >= 8 => row.bar_spans(theme, c.bar()),
        _ => Vec::new(),
    };
    let (mb, sb) = (bar(&master), bar(&slave));
    if !mb.is_empty() || !sb.is_empty() {
        let mut second = vec![Span::raw(" ".repeat(Columns::LABEL))];
        second.extend(exactly(mb, c.side + GAP));
        second.extend(exactly(sb, c.side));
        out.push(Line::from(second));
    }
    out
}

/// What the UAP rests on: one value only a payload's CRC can choose (a
/// header leaves two, `header::PiconetClock`), the CRCs that pass under it
/// in the packets kept, or how far the narrowing got.
fn uap_lines(
    p: &Piconet,
    state: &SdrMetrics,
    iw: usize,
    theme: &crate::Theme,
) -> Vec<Line<'static>> {
    let net = &state.net;
    let text = match net.bt_uap.get(&p.lap).map(|u| u.as_slice()) {
        Some([_]) => {
            let passing = p
                .packets
                .iter()
                .filter(|k| k.payload == PayloadVerdict::Crc(true))
                .count();
            let under = match passing {
                0 => String::new(),
                1 => " · 1 CRC passes under it".to_string(),
                n => format!(" · {n} CRCs pass under it"),
            };
            let full = format!("{}, resolved by a payload CRC{under}", uap_text(p.lap, net));
            // Narrow, the short form: still what the value rests on.
            if full.chars().count() + Columns::LABEL > iw {
                format!("{}, by a payload CRC", uap_text(p.lap, net))
            } else {
                full
            }
        }
        Some([_, _]) => "2 left: an encrypted link resolves from a reconnect".to_string(),
        Some(many) if !many.is_empty() => {
            format!("{} left: each further header narrows them", many.len())
        }
        _ => "not narrowed: no header of it decoded yet".to_string(),
    };
    // Two rows where one is too narrow: what the value rests on is the
    // point of the line, and cut short it would say less than it knows.
    crate::ui::chrome::wrap(&text, iw.saturating_sub(Columns::LABEL), 2)
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| field(if i == 0 { "UAP" } else { "" }, chunk, "", theme))
        .collect()
}

/// The link's addresses and packet types in the packets kept, the most
/// sent first: `LT_ADDR 1 · POLL 262 · NULL 258 · DM3/2-DH3 20`.
fn traffic_lines(p: &Piconet, iw: usize, rows: usize, theme: &crate::Theme) -> Vec<Line<'static>> {
    let mut addrs: Vec<u8> = Vec::new();
    let mut types: Vec<(String, u64)> = Vec::new();
    for k in &p.packets {
        let Some(HeaderRead::Decoded(h)) = k.header else {
            continue;
        };
        if !addrs.contains(&h.lt_addr) {
            addrs.push(h.lt_addr);
        }
        let name = PacketType::from_code(h.packet_type.code()).shown();
        match types.iter_mut().find(|(n, _)| *n == name) {
            Some((_, c)) => *c += 1,
            None => types.push((name, 1)),
        }
    }
    if addrs.is_empty() {
        return vec![quiet("no header read at one clock yet".to_string(), theme)];
    }
    addrs.sort_unstable();
    types.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut text = format!(
        "LT_ADDR {}",
        addrs
            .iter()
            .map(|a| a.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    for (name, count) in types {
        text.push_str(&format!(" · {name} {count}"));
    }
    crate::ui::chrome::wrap(&text, iw.saturating_sub(1), rows)
        .into_iter()
        .map(|chunk| {
            Line::from(Span::styled(
                format!(" {chunk}"),
                Style::default().fg(theme.value),
            ))
        })
        .collect()
}

/// The most braille cells a trend takes, two points to a cell.
const TREND_CELLS: usize = 16;

/// The most packets a trend point averages. One header's reading is
/// noisy, and a trace of single readings is a solid block that shows the
/// noise and hides the drift; a point is the mean of a few consecutive
/// packets instead, so what moves is the side, not the scatter.
const TREND_GROUP: usize = 4;

/// A trend of `value` over one side's newest packets that have it: each
/// point the mean of consecutive packets, oldest on the left and the
/// newest at the right edge, scaled to the points' span, which is stated
/// beside it with `places` decimals. Missing below four points, where a
/// trace is a guess at a shape.
fn trend(
    p: &Piconet,
    direction: Direction,
    c: Columns,
    places: usize,
    value: impl Fn(&crate::signal::bt::piconet::BtPacket) -> Option<f64>,
) -> Cell<'static> {
    let cells = c.side.saturating_sub(16).min(TREND_CELLS);
    if cells < 4 {
        return Cell::Missing("no room for a trend");
    }
    let mut values: Vec<f64> = p
        .packets
        .iter()
        .filter(|k| k.direction == Some(direction))
        .filter_map(&value)
        .take(cells * 2 * TREND_GROUP)
        .collect();
    values.reverse();
    // As many packets a point as fill the trace, and no more than
    // TREND_GROUP; the oldest few that do not make a whole point are left.
    let group = values.len().div_ceil(cells * 2).clamp(1, TREND_GROUP);
    let skip = values.len() % group;
    let points: Vec<f64> = values[skip..]
        .chunks(group)
        .map(|g| g.iter().sum::<f64>() / g.len() as f64)
        .collect();
    if points.len() < 4 {
        return Cell::Missing("too few packets");
    }
    let (lo, hi) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    let data: Vec<f32> = points.iter().map(|v| *v as f32).collect();
    let colour = match direction {
        Direction::Master => c.master,
        Direction::Slave => c.slave,
    };
    Cell::Trend(
        // Exactly as many cells as there are points for, so the newest is
        // at the right edge.
        crate::ui::widgets::charts::mini_braille_line(&data, points.len().div_ceil(2)),
        format!("{lo:.places$}–{hi:.places$}"),
        colour,
    )
}

/// The MODULATION rows: each side's index, df1 and df2/df1 against the BR
/// band, the same limits and resolutions the Piconets panel holds them to.
fn modulation_lines(
    p: &Piconet,
    c: Columns,
    iw: usize,
    trends: bool,
    theme: &crate::Theme,
) -> Vec<Line<'static>> {
    let (m, s) = (&p.headers.sides.master, &p.headers.sides.slave);
    let index = |side: &Side| match index_of(&side.deviation) {
        Some(i) => Cell::Judged(LimitRow::new(
            "",
            Reading::new(i, "", INDEX_RESOLUTION),
            BR_INDEX,
        )),
        None => Cell::Missing("not measured"),
    };
    let df1 = |side: &Side| match side.deviation.settled.mean() {
        Some(d) => Cell::Judged(LimitRow::new(
            "",
            Reading::new(d.scale(0.001), "kHz", DELTA_F1_RESOLUTION_KHZ),
            BR_DELTA_F1_KHZ,
        )),
        None => Cell::Missing("not measured"),
    };
    let ratio = |side: &Side| {
        let d = &side.deviation;
        match (d.alternating.mean(), d.settled.mean()) {
            (Some(df2), Some(df1)) => Cell::Judged(LimitRow::new(
                "",
                Reading::new(df2.ratio(&df1), "", RATIO_RESOLUTION),
                BR_RATIO,
            )),
            _ => Cell::Missing("no alternating bits"),
        }
    };
    let mut out = pair("Mod index", index(m), index(s), c, theme);
    out.extend(pair("df1 avg", df1(m), df1(s), c, theme));
    out.extend(pair("df2/df1", ratio(m), ratio(s), c, theme));
    if trends {
        let index =
            |k: &crate::signal::bt::piconet::BtPacket| index_of(&k.deviation).map(|i| i.value());
        out.extend(pair(
            "Mod trend",
            trend(p, Direction::Master, c, 3, index),
            trend(p, Direction::Slave, c, 3, index),
            c,
            theme,
        ));
    }
    // What the readings rest on, every member's, as the Classic view
    // always said it; and the headers a busy neighbour kept from being
    // read (`signal::net::measure`), said wherever the figures stand.
    let d = &p.headers.deviation;
    out.extend(footnote(
        &format!(
            "{} settled and {} alternating bits from {} headers, every member's, \
             read as the test suite defines them",
            d.settled.n, d.alternating.n, p.headers.captured
        ),
        iw,
        theme,
    ));
    if d.neighbour_busy > 0 {
        out.push(quiet(
            format!(
                "{} headers not read: the next channel was busy at the time",
                d.neighbour_busy
            ),
            theme,
        ));
    }
    out
}

/// The CARRIER rows: each side's f0, held to its limit only once a
/// reference makes it absolute, and its worst drift and drift rate.
fn carrier_lines(
    p: &Piconet,
    state: &SdrMetrics,
    c: Columns,
    iw: usize,
    trends: bool,
    theme: &crate::Theme,
) -> Vec<Line<'static>> {
    let (m, s) = (&p.headers.sides.master, &p.headers.sides.slave);
    let now = std::time::Instant::now();
    let mut relative = false;
    let mut f0 = |side: &Side| {
        let (Some(ppm), Some(mhz)) = (side.carrier.f0_ppm.mean(), side.carrier.channel_mhz.mean())
        else {
            return Cell::Missing("not measured");
        };
        let (ppm, provenance) = state.radio.corrected_ppm(ppm, now);
        let khz = ppm.scale(mhz.value() / 1e3);
        if provenance == Provenance::Unreferenced {
            relative = true;
            Cell::Plain(Reading::new(khz, "kHz", F0_RESOLUTION_KHZ))
        } else {
            Cell::Judged(LimitRow::new(
                "",
                Reading::new(khz, "kHz", F0_RESOLUTION_KHZ),
                F0_LIMIT_KHZ,
            ))
        }
    };
    let (mf, sf) = (f0(m), f0(s));
    let drift = |side: &Side| match side.carrier.worst_drift_hz {
        Some(d) => Cell::Judged(LimitRow::new(
            "",
            Reading::new(d.scale(0.001), "kHz", DRIFT_RESOLUTION_KHZ),
            DRIFT_LIMIT_KHZ,
        )),
        None => Cell::Missing("not measured"),
    };
    let rate = |side: &Side| match side.carrier.worst_rate_hz_per_us {
        Some(r) => Cell::Judged(LimitRow::new(
            "",
            Reading::new(r, "Hz/us", DRIFT_RATE_RESOLUTION),
            DRIFT_RATE_LIMIT,
        )),
        None => Cell::Missing("not measured"),
    };
    let mut out = pair("f0", mf, sf, c, theme);
    if relative {
        out.push(Line::from(vec![
            Span::raw(" ".repeat(Columns::LABEL)),
            Span::styled(
                "relative to our own oscillator".to_string(),
                Style::default().fg(theme.label),
            ),
        ]));
    }
    // Each packet's own f0, in kHz of its channel, corrected as the row is.
    let f0_of = |k: &crate::signal::bt::piconet::BtPacket| {
        let hz = crate::signal::bt::channel::centre_hz(k.channel)?;
        let (ppm, _) = state.radio.corrected_ppm(k.f0_ppm?, now);
        Some(ppm.value() * hz as f64 / 1e9)
    };
    if trends {
        out.extend(pair(
            "f0 trend",
            trend(p, Direction::Master, c, 1, f0_of),
            trend(p, Direction::Slave, c, 1, f0_of),
            c,
            theme,
        ));
    }
    out.extend(pair("Drift worst", drift(m), drift(s), c, theme));
    out.extend(pair("Rate worst", rate(m), rate(s), c, theme));
    let n = p.headers.carrier.f0_ppm.n;
    if n > 0 {
        out.extend(footnote(
            &format!("from {n} headers' access code and header, as the test suite defines them"),
            iw,
            theme,
        ));
    }
    out
}

/// Each side's residuals on the piconet's grid: its packets on the stream
/// the grid was fitted on, read against it.
fn residuals(p: &Piconet, direction: Direction) -> Vec<f64> {
    let Some(Ok(fit)) = p.slots.as_ref() else {
        return Vec::new();
    };
    p.packets
        .iter()
        .filter(|k| k.stream == p.slots_stream && k.direction == Some(direction))
        .filter_map(|k| fit.residual_at(k.at_us))
        .collect()
}

/// The TIMING rows: the piconet's clock, each side's jitter about its own
/// average, the slave's place against the master's, and the packets of
/// each side.
fn timing_lines(
    p: &Piconet,
    state: &SdrMetrics,
    iw: usize,
    c: Columns,
    plots: bool,
    theme: &crate::Theme,
) -> Vec<Line<'static>> {
    let mut out = vec![crate::ui::chrome::section(
        "timing",
        "625 us slots: Core 5.4 Vol 2 B 2.2.5",
        iw,
        theme,
    )];
    match &p.slots {
        None => out.push(quiet("no hit timed yet".to_string(), theme)),
        Some(Err(SlotRefusal::Collecting { have, need })) => out.push(quiet(
            format!("collecting: {have} of {need} hits to fit a slot grid"),
            theme,
        )),
        Some(Err(SlotRefusal::NoGrid { hits })) => out.push(quiet(
            format!("no slot grid: {hits} hits do not line up at 625 us beyond chance"),
            theme,
        )),
        Some(Ok(f)) => {
            let (clock, judged) = clock_of(f, state);
            let mut line = vec![
                crate::ui::chrome::field("clock", PAIR_LABEL, theme),
                Span::raw(" "),
            ];
            if judged {
                let row = LimitRow::new(
                    "",
                    Reading::new(clock, "ppm", CLOCK_RESOLUTION_PPM),
                    CLOCK_LIMIT_PPM,
                );
                line.extend(Cell::Judged(row).spans(theme));
            } else {
                line.extend(Reading::new(clock, "ppm", CLOCK_RESOLUTION_PPM).spans(theme));
                line.push(Span::styled(
                    "  relative to our own oscillator".to_string(),
                    Style::default().fg(theme.label),
                ));
            }
            out.push(Line::from(line));

            let (master, slave) = (
                residuals(p, Direction::Master),
                residuals(p, Direction::Slave),
            );
            let (ms, ss) = (spread(&master), spread(&slave));
            let jitter = |s: Option<Spread>| match s {
                Some(s) => Cell::Judged(LimitRow::new(
                    "",
                    Reading::new(Uncertain::exact(s.max_us), "us", f64::INFINITY),
                    JITTER_LIMIT_US,
                )),
                None => Cell::Missing("collecting"),
            };
            let rms = |s: Option<Spread>, timed: usize| match s {
                Some(s) => Cell::Plain(Reading::new(s.rms_us, "us", JITTER_RESOLUTION_US)),
                None => Cell::Text(format!("{timed} of {MIN_HITS} timed")),
            };
            out.extend(pair("Jitter max", jitter(ms), jitter(ss), c, theme));
            out.extend(pair(
                "rms",
                rms(ms, master.len()),
                rms(ss, slave.len()),
                c,
                theme,
            ));
            // The grid is every member's, so each side's average carries
            // where the fit put it; their difference does not.
            if let (Some(m), Some(s)) = (ms, ss) {
                let d = s.mean_us.value() - m.mean_us.value();
                let sigma = s.mean_us.sigma().hypot(m.mean_us.sigma());
                let reading =
                    Reading::new(Uncertain::from_sigma(d, sigma), "us", JITTER_RESOLUTION_US);
                let text = reading.text();
                let signed = if d > 0.0 && reading.value_text().is_some() {
                    format!("+{text}")
                } else {
                    text
                };
                out.push(field(
                    "offset",
                    signed,
                    if d >= 0.0 {
                        "  the slave's packets after the master's"
                    } else {
                        "  the slave's packets before the master's"
                    },
                    theme,
                ));
            }
            // Every member's residuals as a shape: the numbers say how wide,
            // the shape whether it is one spread or two, as when a slave
            // answers a little late on every slot and stands as its own hump.
            let beyond = if plots {
                let (bars, beyond) = residual_histogram(&f.residuals_us, iw, c.master, theme);
                out.extend(bars);
                beyond
            } else {
                0
            };
            let span = crate::ui::widgets::timing_fmt::seconds_ms((f.span_us / 1e3) as u64);
            let beyond = if beyond > 0 {
                format!(", {beyond} beyond the plot's 1.5")
            } else {
                String::new()
            };
            out.extend(footnote(
                &format!(
                    "residual from the grid, us{beyond}: {} hits over {span}, every member's; \
                     hits dated to 0.25 us",
                    f.hits
                ),
                iw,
                theme,
            ));
        }
    }
    let sides = &p.headers.sides;
    out.extend(pair(
        "packets",
        Cell::Text(sides.master.packets.to_string()),
        Cell::Text(sides.slave.packets.to_string()),
        c,
        theme,
    ));
    if sides.unknown.packets > 0 {
        out.push(quiet(
            format!(
                "{} not yet placed: ID, or before the clock was known",
                sides.unknown.packets
            ),
            theme,
        ));
    }
    out
}

/// The column heads over the pairs, each side in its own ink.
fn heads(c: Columns) -> Line<'static> {
    let bold = |col: Color| Style::default().fg(col).add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::raw(" ".repeat(Columns::LABEL))];
    if c.stacked {
        spans.push(Span::styled("▶ MASTER", bold(c.master)));
        spans.push(Span::raw("  "));
        spans.push(Span::styled("◀ SLAVE", bold(c.slave)));
        return Line::from(spans);
    }
    let master = c.master;
    spans.extend(exactly(
        vec![Span::styled("MASTER ▶", bold(master))],
        c.side + GAP,
    ));
    spans.extend(exactly(
        vec![Span::styled("◀ SLAVE", bold(c.slave))],
        c.side,
    ));
    Line::from(spans)
}

/// The whole bench for one piconet within `budget` rows: what it is and
/// what it carries, then each section in order while it fits whole. A
/// section left out is named on a last line, so a short panel says what a
/// taller one would show rather than stopping mid-section, as the Piconets
/// panel does.
fn bench(
    p: &Piconet,
    state: &SdrMetrics,
    iw: usize,
    budget: usize,
    theme: &crate::Theme,
) -> Vec<Line<'static>> {
    let mut out = uap_lines(p, state, iw, theme);
    // Neither an inquiry nor a page is a piconet: no two ends to set side
    // by side, as the Piconets panel says of them.
    if let Kind::Inquiry(_) | Kind::Paged = p.kind() {
        out.push(quiet(
            "not a piconet: no master and slave to set side by side".to_string(),
            theme,
        ));
        return out.into_iter().map(|l| fit(l, iw)).collect();
    }
    // The master in the piconet's own colour, its chip on the hop chart
    // and in the roster; the slave in the ordinary ink, which no piconet's
    // colour is, so the two never look alike.
    let master = state
        .net
        .bt_piconets
        .iter()
        .position(|q| q.lap == p.lap)
        .map_or(theme.value_hi, |k| theme.series_color(k));
    let c = Columns::of(iw, master, theme.value);
    // Narrow, one row of it: the types are the most sent first, so what a
    // row leaves off is the rarest.
    out.extend(traffic_lines(p, iw, if c.stacked { 1 } else { 2 }, theme));
    // Of 79, not of the channels watched now: in SURVEY a piconet was heard
    // wherever the survey stood at the time.
    let runs = format!(
        "{} of 79: {}",
        p.channels_hit(),
        channel_runs(p.channel_mask())
    );
    for (i, chunk) in crate::ui::chrome::wrap(&runs, iw.saturating_sub(Columns::LABEL), 2)
        .into_iter()
        .enumerate()
    {
        out.push(field(
            if i == 0 { "channels" } else { "" },
            chunk,
            "",
            theme,
        ));
    }

    // Each section as a whole and without its plots, the trends and the
    // residual shape: a plot is the first thing a short panel gives up, the
    // readings the last.
    let modulation = |trends| {
        let mut out = vec![
            crate::ui::chrome::section(
                "modulation",
                "BR limits: Core 5.4 Vol 2 A 3.1.1",
                iw,
                theme,
            ),
            heads(c),
        ];
        out.extend(modulation_lines(p, c, iw, trends, theme));
        out
    };
    let carrier = |trends| {
        let mut out = vec![crate::ui::chrome::section(
            "carrier",
            "BR limits: Core 5.4 Vol 2 A 3.1.3",
            iw,
            theme,
        )];
        out.extend(carrier_lines(p, state, c, iw, trends, theme));
        out
    };
    let timing = |plots| timing_lines(p, state, iw, c, plots, theme);
    let sections = [
        ("MODULATION", modulation(true), Some(modulation(false))),
        ("CARRIER", carrier(true), Some(carrier(false))),
        ("TIMING", timing(true), Some(timing(false))),
        ("HEADERS", header_lines(p, state, iw, theme), None),
    ];
    let last = sections.len() - 1;
    // The line naming what is left out may wrap on a narrow bench: it is
    // kept whole, so it is given the rows its longest form needs.
    let note = |left: &[&str]| format!("+ {} on a taller panel", left.join(", "));
    let note_rows = if note(&["MODULATION", "CARRIER", "TIMING", "HEADERS", "plots"]).len() + 1 > iw
    {
        2
    } else {
        1
    };
    let mut left_out = Vec::new();
    let mut plots_left = false;
    for (k, (name, whole, lean)) in sections.into_iter().enumerate() {
        // In order, leaving a row for the line that names what is left out,
        // unless this is the last section: whole, else without its trend,
        // else not at all, and nothing after a section left out.
        let reserve = if k < last { note_rows } else { 0 };
        let fits = |lines: &Vec<Line<'static>>| out.len() + lines.len() + reserve <= budget;
        if !left_out.is_empty() {
            left_out.push(name);
        } else if fits(&whole) {
            out.extend(whole);
        } else if let Some(lean) = lean.filter(|l| fits(l)) {
            out.extend(lean);
            plots_left = true;
        } else {
            left_out.push(name);
        }
    }
    if plots_left {
        left_out.push("plots");
    }
    if !left_out.is_empty() {
        let room = budget.saturating_sub(out.len()).min(note_rows);
        for chunk in crate::ui::chrome::wrap(&note(&left_out), iw.saturating_sub(1), room) {
            out.push(Line::from(Span::styled(
                format!(" {chunk}"),
                Style::default().fg(theme.label),
            )));
        }
    }
    out.into_iter().map(|l| fit(l, iw)).collect()
}

impl Panel for NetBtBenchPanel {
    fn name(&self) -> &'static str {
        "net_bt_bench"
    }

    fn min_size(&self) -> (u16, u16) {
        (30, 6)
    }

    fn focus_key(&self) -> Option<char> {
        // The Piconets panel's letter: the two are the Classic and the
        // Piconet view's accounts of a piconet, and never share a screen.
        Some('c')
    }

    fn focus_bindings(&self) -> &'static [(&'static str, &'static str)] {
        &[("← →", "the previous or next piconet")]
    }

    fn chrome(&self, state: &SdrMetrics) -> PanelChrome {
        let chrome = PanelChrome::new("Bench")
            .stale_when(Staleness::NotStreaming)
            .shows_laps()
            .shows_offsets()
            .tag_if(true, state.net.mode.tag())
            // The sides' readings and counts run for the session.
            .counts_from_feed(FeedSpan::Session);
        match selected(state) {
            Some(p) => chrome.suffix(format!(" {}", state.net.show_lap(p.lap))),
            None => chrome,
        }
    }

    fn render(
        &self,
        f: &mut Frame,
        inner: Rect,
        state: &SdrMetrics,
        theme: &crate::Theme,
        _focused: bool,
    ) {
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let width = inner.width as usize;
        if let Some(lines) = silence(state, width, theme) {
            f.render_widget(Paragraph::new(lines), inner);
            return;
        }
        let Some(p) = selected(state) else {
            let lines: Vec<Line> = crate::ui::chrome::wrap(
                "no piconet selected: select a piconet in NET 5 and press Enter",
                width,
                4,
            )
            .into_iter()
            .map(|c| Line::from(Span::styled(c, Style::default().fg(theme.stale))))
            .collect();
            f.render_widget(Paragraph::new(lines), inner);
            return;
        };
        let lines = bench(p, state, width, inner.height as usize, theme);
        f.render_widget(Paragraph::new(lines), inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::bt::header::{Header, PacketType};
    use crate::signal::bt::piconet::{
        observe, observe_packet, BtPacket, Carrier, Deviation, Direction, HeaderRead,
        PayloadVerdict,
    };
    use crate::state::fixture::draw;
    use std::time::Instant;

    const LAP: u32 = 0xc3_d318;

    fn heard() -> SdrMetrics {
        let mut m = SdrMetrics::fixture().streaming();
        m.net.bt_channels_watched = (60..80).collect();
        observe(&mut m.net.bt_piconets, LAP, 73, Instant::now());
        m.net.bt_uap.insert(LAP, vec![0x67]);
        m.net.bt_view.selected = Some(LAP);
        m
    }

    fn header(code: u8) -> HeaderRead {
        HeaderRead::Decoded(Header {
            lt_addr: 1,
            packet_type: PacketType::from_code(code),
            flags: 0,
            hec: 0,
            clk6: 0,
        })
    }

    fn packet(at_us: f64, code: u8, direction: Direction, payload: PayloadVerdict) -> BtPacket {
        BtPacket {
            seen: Instant::now(),
            at_us,
            stream: 1,
            channel: 73,
            header: Some(header(code)),
            direction: Some(direction),
            deviation: Deviation::default(),
            carrier: Carrier::default(),
            f0_ppm: None,
            payload,
        }
    }

    /// Settled readings around `df1_hz`, a little spread.
    fn around(df1_hz: f32) -> Deviation {
        let r: Vec<f32> = (0..24)
            .map(|k| df1_hz + (k % 3) as f32 * 1_000.0 - 1_000.0)
            .collect();
        Deviation::from_readings(&r, &[])
    }

    #[test]
    fn no_piconet_names_the_silence() {
        let mut m = SdrMetrics::fixture().streaming();
        m.net.bt_channels_watched = vec![10, 20];
        let out = draw(NetBtBenchPanel, 80, 10, &m).join("\n");
        assert!(out.contains("no piconet heard"), "{out}");

        let mut m = heard();
        m.net.bt_view.selected = None;
        let out = draw(NetBtBenchPanel, 80, 10, &m).join("\n");
        assert!(out.contains("select a piconet in NET 5"), "{out}");
    }

    /// Under one MODULATION heading, the master's column and the slave's,
    /// each with its own index; the link's addresses and packet types above.
    #[test]
    fn the_bench_puts_master_and_slave_side_by_side() {
        let mut m = heard();
        let sides = &mut m.net.bt_piconets[0].headers.sides;
        sides.master.deviation = around(167_000.0);
        sides.slave.deviation = around(159_000.0);
        for (k, (code, d)) in [
            (1, Direction::Master),
            (0, Direction::Slave),
            (1, Direction::Master),
        ]
        .into_iter()
        .enumerate()
        {
            let p = packet(k as f64 * 625.0, code, d, PayloadVerdict::NoPayload);
            observe_packet(&mut m.net.bt_piconets, LAP, p);
        }
        let out = draw(NetBtBenchPanel, 100, 50, &m);
        let text = out.join("\n");
        assert_eq!(text.matches("MODULATION").count(), 1, "{text}");
        let heads = out.iter().find(|l| l.contains("MASTER ▶")).expect(&text);
        let (m_at, s_at) = (
            heads.find("MASTER").unwrap(),
            heads.find("SLAVE").expect(heads),
        );
        assert!(m_at < s_at, "{heads}");
        let index = out.iter().find(|l| l.contains("0.334")).expect(&text);
        let (a, b) = (
            index.find("0.334").unwrap(),
            index.find("0.318").expect(index),
        );
        assert!(a < b, "the master on the left: {index}");
        let types = out.iter().find(|l| l.contains("LT_ADDR")).expect(&text);
        assert!(types.contains("LT_ADDR 1"), "{types}");
        assert!(types.contains("POLL 2"), "{types}");
        assert!(types.contains("NULL 1"), "{types}");
        assert!(
            types.find("POLL").unwrap() < types.find("NULL").unwrap(),
            "most first"
        );
    }

    /// A side with nothing measured says so in its own column, never a zero.
    #[test]
    fn a_side_with_nothing_yet_dashes() {
        let mut m = heard();
        m.net.bt_piconets[0].headers.sides.master.deviation = around(167_000.0);
        let out = draw(NetBtBenchPanel, 100, 50, &m);
        let text = out.join("\n");
        let heads = out.iter().position(|l| l.contains("MASTER ▶")).unwrap();
        let slave_at = out[heads].find("◀ SLAVE").unwrap();
        let slave_column: String = out[heads + 1..heads + 4]
            .iter()
            .map(|l| {
                l.chars()
                    .skip(out[heads][..slave_at].chars().count())
                    .collect::<String>()
            })
            .collect();
        assert!(slave_column.contains("not measured"), "{text}");
        assert!(
            !slave_column.contains("0."),
            "no figure on the slave's side: {text}"
        );
        assert!(text.contains("0.334"), "the master's still: {text}");
    }

    /// The UAP line says what the value rests on: one value only a payload's
    /// CRC can choose, and two is where an encrypted link stays.
    #[test]
    fn the_uap_line_says_how_it_was_reached() {
        let mut m = heard();
        observe_packet(
            &mut m.net.bt_piconets,
            LAP,
            packet(0.0, 3, Direction::Slave, PayloadVerdict::Crc(true)),
        );
        let text = draw(NetBtBenchPanel, 100, 50, &m).join("\n");
        assert!(text.contains("0x67, resolved by a payload CRC"), "{text}");
        assert!(text.contains("1 CRC passes under it"), "{text}");

        m.net.bt_uap.insert(LAP, vec![0x67, 0x9a]);
        let text = draw(NetBtBenchPanel, 100, 50, &m).join("\n");
        assert!(
            text.contains("2 left: an encrypted link resolves from a reconnect"),
            "{text}"
        );

        m.net.bt_uap.insert(LAP, vec![0x67]);
        m.net.address_display = crate::state::AddressDisplay::Masked;
        let text = draw(NetBtBenchPanel, 100, 50, &m).join("\n");
        assert!(!text.contains("0x67"), "{text}");
    }

    /// TIMING: the piconet's clock from its grid, each side's jitter about
    /// its own average, how far the slave's packets sit from the master's,
    /// and the packets of each side with the ones not yet placed.
    #[test]
    fn timing_reads_each_side_on_the_piconets_grid() {
        let mut m = heard();
        let mut times = Vec::new();
        let mut packets = Vec::new();
        for k in 0..24u32 {
            let master = k % 2 == 0;
            let wobble = if k % 4 < 2 { 0.2 } else { -0.2 };
            let t = 1_000.0 + k as f64 * 625.0 + if master { wobble } else { 2.0 + wobble };
            times.push(t);
            let d = if master {
                Direction::Master
            } else {
                Direction::Slave
            };
            packets.push(packet(t, 1 - k as u8 % 2, d, PayloadVerdict::NoPayload));
        }
        let p = &mut m.net.bt_piconets[0];
        p.slots = Some(crate::signal::bt::slots::fit(&times));
        p.slots_stream = 1;
        for k in packets {
            observe_packet(&mut m.net.bt_piconets, LAP, k);
        }
        let sides = &mut m.net.bt_piconets[0].headers.sides;
        sides.master.packets = 180;
        sides.slave.packets = 232;
        sides.unknown.packets = 2;
        let out = draw(NetBtBenchPanel, 100, 50, &m);
        let text = out.join("\n");
        assert!(text.contains("TIMING"), "{text}");
        let jitter = out.iter().find(|l| l.contains("Jitter max")).expect(&text);
        assert_eq!(jitter.matches(" us").count(), 2, "one a side: {jitter}");
        assert!(text.contains("clock"), "{text}");
        let offset = out
            .iter()
            .find(|l| l.contains("after the master"))
            .expect(&text);
        let value: f64 = offset
            .split_whitespace()
            .find(|w| w.starts_with('+'))
            .and_then(|w| w[1..].parse().ok())
            .expect(offset);
        assert!((value - 2.0).abs() < 0.1, "the slave 2 us behind: {offset}");
        let counts = out.iter().find(|l| l.contains("232")).expect(&text);
        assert!(counts.contains("180"), "{counts}");
        assert!(text.contains("2 not yet placed"), "{text}");
    }

    #[test]
    fn it_fits_every_size() {
        let mut m = heard();
        m.net.bt_piconets[0].headers.sides.master.deviation = around(167_000.0);
        for (w, h) in [(20, 5), (40, 8), (80, 30), (120, 50), (1, 1), (3, 2)] {
            draw(NetBtBenchPanel, w, h, &m);
            draw(NetBtBenchPanel, w, h, &SdrMetrics::fixture());
        }
    }

    /// Packets of one side with their own readings, oldest first, each
    /// `df1_hz(k)` and an f0 of `khz(k)` on channel 73.
    fn readings(
        m: &mut SdrMetrics,
        d: Direction,
        first_slot: u32,
        n: u32,
        df1_hz: impl Fn(u32) -> f32,
        khz: impl Fn(u32) -> f64,
    ) {
        for k in 0..n {
            let mut p = packet(
                (first_slot + 2 * k) as f64 * 625.0,
                1,
                d,
                PayloadVerdict::NoPayload,
            );
            p.deviation = around(df1_hz(k));
            p.f0_ppm = Some(crate::signal::dsp::uncertainty::Uncertain::from_sigma(
                khz(k) * 1e3 / 2_475e6 * 1e6,
                0.05,
            ));
            observe_packet(&mut m.net.bt_piconets, LAP, p);
        }
    }

    /// The braille characters of `line` from column `from` on, up to the
    /// first that is not one.
    fn braille(line: &str, from: usize) -> Vec<u32> {
        line.chars()
            .skip(from)
            .skip_while(|c| !('\u{2800}'..='\u{28ff}').contains(c))
            .take_while(|c| ('\u{2800}'..='\u{28ff}').contains(c))
            .map(|c| c as u32 - 0x2800)
            .collect()
    }

    /// A trend per side from its newest packets' own readings, oldest on
    /// the left, with the span it is scaled to beside it, so a trace that
    /// fills its cell is not read as a large change.
    #[test]
    fn the_trend_follows_the_ring() {
        let mut m = heard();
        // The master's index rises from 0.320 to 0.340; the slave's holds.
        readings(
            &mut m,
            Direction::Master,
            0,
            20,
            |k| 160_000.0 + k as f32 * 526.3,
            |_| 3.0,
        );
        readings(
            &mut m,
            Direction::Slave,
            1,
            20,
            |k| 159_000.0 + (k % 2) as f32 * 400.0,
            |k| -12.0 - k as f64 * 0.1,
        );
        let out = draw(NetBtBenchPanel, 100, 60, &m);
        let text = out.join("\n");
        let row = out.iter().find(|l| l.contains("Mod trend")).expect(&text);
        assert!(
            row.contains("0.320–0.340"),
            "the span it is scaled to: {row}"
        );
        let heads = out.iter().find(|l| l.contains("MASTER ▶")).unwrap();
        let slave_at = heads[..heads.find("◀ SLAVE").unwrap()].chars().count();
        let master = braille(row, 0);
        assert!(master.len() >= 8, "{row}");
        // Rising: the oldest at the bottom dot, the newest at the top.
        assert_ne!(master[0] & 0x40, 0, "{row}");
        assert_ne!(master[master.len() - 1] & 0x08, 0, "{row}");
        assert!(!braille(row, slave_at).is_empty(), "the slave's too: {row}");
        let f0 = out.iter().find(|l| l.contains("f0 trend")).expect(&text);
        assert!(f0.contains("-13.9–-12.0"), "{f0}");
    }

    /// Too narrow for two columns, each reading goes on a line of its own,
    /// marked with its side, the master's first.
    #[test]
    fn narrow_it_stacks() {
        let mut m = heard();
        let sides = &mut m.net.bt_piconets[0].headers.sides;
        sides.master.deviation = around(167_000.0);
        sides.slave.deviation = around(159_000.0);
        let out = draw(NetBtBenchPanel, 40, 60, &m);
        let text = out.join("\n");
        let master = out.iter().position(|l| l.contains("0.334")).expect(&text);
        let slave = out.iter().position(|l| l.contains("0.318")).expect(&text);
        assert_eq!(slave, master + 1, "one under the other: {text}");
        assert!(out[master].contains('▶'), "{text}");
        assert!(out[slave].contains('◀'), "{text}");
        assert!(!out[master].contains("0.318"), "{text}");
    }

    /// Shorter than every section, they give way from the bottom, TIMING
    /// first, and the last line names what a taller panel would show.
    #[test]
    fn short_it_gives_way() {
        let mut m = heard();
        let sides = &mut m.net.bt_piconets[0].headers.sides;
        sides.master.deviation = around(167_000.0);
        sides.slave.deviation = around(159_000.0);
        let tall = draw(NetBtBenchPanel, 100, 60, &m).join("\n");
        assert!(
            tall.contains("TIMING") && !tall.contains("taller panel"),
            "{tall}"
        );
        let out = draw(NetBtBenchPanel, 100, 18, &m);
        let text = out.join("\n");
        assert!(text.contains("MODULATION"), "{text}");
        assert!(!text.contains("├╴ TIMING"), "{text}");
        let last = out
            .iter()
            .rev()
            .find(|l| !l.trim_matches(['│', ' ', '╰', '─', '╯']).is_empty())
            .unwrap();
        assert!(
            last.contains("TIMING") && last.contains("on a taller panel"),
            "{text}"
        );
    }

    /// Short and narrow, a section keeps its readings and gives up its
    /// trend first, and the last line says the trends are on a taller panel.
    #[test]
    fn short_it_keeps_the_readings_before_the_plots() {
        let mut m = heard();
        let sides = &mut m.net.bt_piconets[0].headers.sides;
        sides.master.deviation = around(167_000.0);
        sides.slave.deviation = around(159_000.0);
        let out = draw(NetBtBenchPanel, 40, 17, &m);
        let text = out.join("\n");
        assert!(text.contains("Mod index"), "{text}");
        assert!(!text.contains("Mod trend"), "{text}");
        let note: String = out
            .iter()
            .rev()
            .skip(1)
            .take(2)
            .rev()
            .map(|l| l.trim_matches(['│', ' ']).to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(note.contains("plots on a taller panel"), "whole: {text}");
    }

    /// **Headers are read only under one UAP**: before, the section says
    /// how many wait and why; after, the read and the undecoded counts (a
    /// rising undecoded count is how a wrong UAP would show), the session's
    /// type mix and LT_ADDRs, and the clock hunt, under a heading that says
    /// it is a port.
    #[test]
    fn headers_are_read_only_once_the_uap_is_one_value() {
        use crate::signal::bt::piconet::observe_header;
        let mut m = heard();
        m.net.bt_uap.insert(LAP, vec![0x4c, 0x9a]);
        let at = |m: &SdrMetrics| draw(NetBtBenchPanel, 100, 80, m).join("\n");
        assert!(at(&m).contains("none yet"), "{}", at(&m));

        observe_header(
            &mut m.net.bt_piconets,
            LAP,
            HeaderRead::Unresolved,
            2,
            Default::default(),
            Default::default(),
        );
        let out = at(&m);
        assert!(
            out.contains("1, not read: UAP not resolved (2 candidates)"),
            "{out}"
        );

        m.net.bt_uap.insert(LAP, vec![0x4c]);
        let read = |t, a| {
            HeaderRead::Decoded(Header {
                lt_addr: a,
                packet_type: t,
                flags: 0,
                hec: 0,
                clk6: 0,
            })
        };
        for (t, a) in [
            (PacketType::Poll, 1),
            (PacketType::Poll, 1),
            (PacketType::Null, 0),
            (PacketType::Dh1, 2),
            (PacketType::Poll, 2),
        ] {
            observe_header(
                &mut m.net.bt_piconets,
                LAP,
                read(t, a),
                1,
                Default::default(),
                Default::default(),
            );
        }
        observe_header(
            &mut m.net.bt_piconets,
            LAP,
            HeaderRead::Undecoded,
            1,
            Default::default(),
            Default::default(),
        );
        let out = at(&m);
        assert!(
            out.contains("HEADERS") && out.contains("checked on air"),
            "{out}"
        );
        assert!(
            out.contains("5 of 7 captured, 1 did not decode under 0x4c"),
            "{out}"
        );
        assert!(
            out.contains("POLL 3 \u{00b7} NULL 1 \u{00b7} DH1/2-DH1 1"),
            "{out}"
        );
        assert!(out.contains("0 (broadcast), 1, 2"), "{out}");
        assert!(out.contains("CLK1-6 found"), "{out}");
    }

    /// What every reading rests on is said under it, as it was on the
    /// Classic view: the bits and headers the modulation is read from, the
    /// headers a busy neighbour kept from being read, and the headers the
    /// carrier is read from.
    #[test]
    fn the_readings_say_what_they_rest_on() {
        use crate::signal::bt::piconet::observe_header;
        use crate::signal::dsp::deviation::Sums;
        let mut m = heard();
        let dev = Deviation {
            settled: Sums::of(&[158_000.0, 160_000.0, 162_000.0, 160_000.0]),
            alternating: Sums::of(&[149_000.0, 151_000.0]),
            neighbour_busy: 2,
        };
        let carrier = Carrier {
            f0_ppm: Sums::of(&[4.0, 6.0]),
            channel_mhz: Sums::of(&[2441.0, 2441.0]),
            worst_drift_hz: None,
            worst_rate_hz_per_us: None,
        };
        observe_header(
            &mut m.net.bt_piconets,
            LAP,
            HeaderRead::Unresolved,
            32,
            dev,
            carrier,
        );
        let out = draw(NetBtBenchPanel, 100, 80, &m).join("\n");
        assert!(
            out.contains("4 settled and 2 alternating bits from 1 headers"),
            "{out}"
        );
        assert!(
            out.contains("2 headers not read: the next channel was busy"),
            "{out}"
        );
        assert!(
            out.contains("from 2 headers' access code and header"),
            "{out}"
        );
    }

    /// The piconet's clock from its slot grid: a reading relative to our
    /// own oscillator with no reference, and corrected once a reference
    /// makes it absolute.
    #[test]
    fn the_piconet_clock_is_relative_until_a_reference_judges_it() {
        let mut m = heard();
        let times: Vec<f64> = (0..40u32)
            .map(|k| f64::from(k * 5) * crate::signal::bt::slots::SLOT_US * (1.0 + 6e-6))
            .collect();
        m.net.bt_piconets[0].slots = Some(crate::signal::bt::slots::fit(&times));
        let out = draw(NetBtBenchPanel, 100, 80, &m);
        let clock = out
            .iter()
            .find(|l| l.starts_with("│ clock "))
            .unwrap()
            .clone();
        // Slots 6 ppm long: a clock 6 ppm slow.
        assert!(
            clock.contains("-6") && clock.contains("relative to our own oscillator"),
            "{clock}"
        );

        m.radio.reference = Some(crate::state::FrequencyReference {
            ppm: 2.0,
            sigma_ppm: 0.1,
            provenance: crate::state::Provenance::Traceable,
            source: "WWV 10 MHz".to_string(),
            at: std::time::Instant::now(),
            efficiency: None,
            trusted: None,
        });
        let out = draw(NetBtBenchPanel, 100, 80, &m);
        let clock = out.iter().find(|l| l.starts_with("│ clock ")).unwrap();
        assert!(
            clock.contains("-4") && !clock.contains("relative"),
            "corrected: {clock}"
        );
    }

    /// The residuals as a shape, every member's, with what they rest on:
    /// the plot that shows a slave answering late as a hump of its own.
    #[test]
    fn the_grid_residuals_are_plotted_with_what_they_rest_on() {
        let mut m = heard();
        let times: Vec<f64> = (0..60u32)
            .map(|k| f64::from(k) * 1e6 + if k % 2 == 0 { 0.0 } else { 0.3 })
            .collect();
        m.net.bt_piconets[0].slots = Some(crate::signal::bt::slots::fit(&times));
        let out = draw(NetBtBenchPanel, 100, 80, &m).join("\n");
        assert!(out.contains("residual from the grid"), "{out}");
        assert!(out.contains("60 hits over"), "{out}");
        assert!(out.contains("every member's"), "{out}");
        assert!(
            out.contains('\u{2588}') || out.contains('\u{2584}'),
            "a bar drawn: {out}"
        );
    }

    /// Where the piconet was heard, of all 79, as runs.
    #[test]
    fn the_channels_it_was_heard_on_are_runs() {
        let mut m = heard();
        for ch in [2u8, 3, 4, 5, 17] {
            observe(&mut m.net.bt_piconets, LAP, ch, Instant::now());
        }
        let out = draw(NetBtBenchPanel, 100, 80, &m).join("\n");
        assert!(out.contains("6 of 79: 2-5, 17, 73"), "{out}");
    }
}
