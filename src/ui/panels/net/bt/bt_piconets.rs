// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 MusiThang <viktor.laszlo92@protonmail.com>

//! `NetBtPiconetsPanel` - the classic preset's roster: one row per piconet
//! heard, and the selected one spelled out below (net-ux-polish-plan 6.1).
//!
//! **A roster of piconets, not of devices.** A LAP is the master's lower
//! address part and every member of its piconet sends it, so the title and
//! every label say piconet (`signal::bt::piconet` has the reasoning).
//!
//! **The UAP column says how far the narrowing got**, in the words
//! `net_bt_hops` already uses: one value once it is down to one, the number
//! of candidates left while a header alone cannot choose
//! (`signal::bt::header::PiconetClock` has the measured floor of two), and a
//! dash before any header of it has been decoded. The detail block says
//! which of those it is in a sentence.
//!
//! Replaces `net_bt_census`, which only ever said that nothing was decoded.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::sections::{
    self, channel_runs, F0_RESOLUTION_KHZ, INDEX_RESOLUTION, JITTER_RESOLUTION_US, LABEL_W,
};
use crate::signal::bt::piconet::{ordered, Inquiry, Kind, Piconet, DCI};
use crate::state::SdrMetrics;
use crate::ui::panel::{FeedSpan, Panel, PanelChrome, Staleness};
use crate::ui::widgets::reading::Reading;
use crate::ui::widgets::table::{
    columns_that_fit, grow_to_contents, header, row, viewport_start, Align, Column, Sort,
};

pub struct NetBtPiconetsPanel;

const COLUMNS: &[Column] = &[
    Column {
        title: "LAP",
        // `● 0x5a3c71`: the hop panel's colour chip, then 24 bits.
        width: 10,
        align: Align::Left,
    },
    Column {
        title: "KIND",
        // `inquiry`, `piconet`, `paged` (`piconet::Kind::word`).
        width: 7,
        align: Align::Left,
    },
    Column {
        title: "LAST",
        width: 6,
        align: Align::Right,
    },
    Column {
        title: "HITS",
        width: 6,
        align: Align::Right,
    },
    Column {
        title: "CH",
        width: 3,
        align: Align::Right,
    },
    Column {
        title: "UAP",
        // `32 left`: one header leaves 32 (`header::PiconetClock`).
        width: 7,
        align: Align::Right,
    },
    Column {
        title: "FIRST",
        width: 6,
        align: Align::Right,
    },
];

/// The column the roster is ordered by: the most recently heard first.
const ORDERED_BY: usize = 2;

/// The colour chip a piconet wears here and on the hop panel: solid, so the
/// colour carries in any font (a braille block drew as faint dots).
pub(crate) const CHIP: char = '\u{25cf}';

/// Rows the table keeps before the detail block may take any.
const TABLE_KEEPS: usize = 3;

/// The same shape the census uses: a bench glances, it does not time.
fn ago(secs: u64) -> String {
    if secs < 90 {
        format!("{secs} s")
    } else {
        format!("{} min", secs / 60)
    }
}

/// The UAP cell: one value (as the address mode shows it,
/// `NetState::show_uap`), candidates left, or a dash before any header.
fn uap_cell(uaps: Option<&Vec<u8>>, net: &crate::state::NetState) -> String {
    match uaps.map(|u| u.as_slice()) {
        Some([one]) => net.show_uap(*one),
        Some(many) if !many.is_empty() => format!("{} left", many.len()),
        _ => "-".to_string(),
    }
}

/// The UAP as a sentence, for the detail block.
fn uap_sentence(uaps: Option<&Vec<u8>>, net: &crate::state::NetState) -> String {
    let masked = net.address_display == crate::state::AddressDisplay::Masked;
    match uaps.map(|u| u.as_slice()) {
        Some([one]) => net.show_uap(*one),
        // Listed while a reader can take them in; a first header leaves 32,
        // and 32 values are a wall, not a reading. Masked, two candidates
        // are half an address byte away from one, so none are listed.
        Some(many) if (2..=4).contains(&many.len()) && !masked => format!(
            "{} candidates ({}); a header alone does not choose",
            many.len(),
            many.iter()
                .map(|u| format!("{u:#04x}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Some(many) if !many.is_empty() => format!(
            "{} candidates; each further header narrows them",
            many.len()
        ),
        _ => "not narrowed: no header of it decoded yet".to_string(),
    }
}

/// A LAP as the roster and the hop lanes name it, padded to one width:
/// `NetState::show_lap`, so an inquiry code by its abbreviation, a masked
/// one by its roster number, every other in hex.
pub(crate) fn lap_name(lap: u32, net: &crate::state::NetState) -> String {
    format!("{:<8}", net.show_lap(lap))
}

fn cells(p: &Piconet, state: &SdrMetrics, now: std::time::Instant) -> Vec<String> {
    let since = |t: std::time::Instant| ago(now.saturating_duration_since(t).as_secs());
    vec![
        format!("{CHIP} {}", lap_name(p.lap, &state.net)),
        p.kind().word().to_string(),
        since(p.last_seen),
        p.hits.to_string(),
        p.channels_hit().to_string(),
        uap_text(p.lap, &state.net),
        since(p.first_seen),
    ]
}

/// A LAP's UAP as the roster's column states it, and the packet list's
/// title with it.
pub(super) fn uap_text(lap: u32, net: &crate::state::NetState) -> String {
    match Inquiry::of(lap) {
        // Fixed by the specification, not narrowed from anything.
        Some(_) => "DCI".to_string(),
        None => uap_cell(net.bt_uap.get(&lap), net),
    }
}

/// What the section's classic panels say with no piconet to show: which of
/// the three silences it is (refused, not running, listening with nothing
/// heard), each in its own words. `None` once anything has been heard.
pub(super) fn silence(
    state: &SdrMetrics,
    width: usize,
    theme: &crate::Theme,
) -> Option<Vec<Line<'static>>> {
    if !state.net.bt_piconets.is_empty() {
        return None;
    }
    let note = |text: &str, ink| {
        crate::ui::chrome::wrap(text, width, 4)
            .into_iter()
            .map(move |chunk| Line::from(Span::styled(chunk, Style::default().fg(ink))))
    };
    let mut lines: Vec<Line<'static>> = Vec::new();
    match &state.net.bt_refused {
        Some(reason) => {
            lines.extend(note("not watching", theme.stale));
            lines.extend(note(reason, theme.label));
        }
        None if state.net.bt_channels_watched.is_empty() => {
            lines.extend(note("no classic receiver running", theme.stale));
        }
        None => lines.extend(note(
            &format!(
                "watching {} channels - no piconet heard yet",
                state.net.bt_channels_watched.len()
            ),
            theme.stale,
        )),
    }
    Some(lines)
}

/// How the hits are spaced, in words (`Piconet::pace`), for the rows it can
/// say something about: an inquiry code, and a LAP no header has followed.
/// A piconet with headers is a piconet, and its pace would only repeat it.
/// A fact about timing only; the name it may earn is decided elsewhere.
fn pace_text(p: &Piconet) -> Option<String> {
    let pace = p.pace;
    if pace.close == 0 || (Inquiry::of(p.lap).is_none() && p.headers.captured > 0) {
        return None;
    }
    Some(if pace.is_half_slot() {
        format!(
            "{} of {} close spacings on odd half slots: the 3200/s pace of inquiry and paging",
            pace.odd_half, pace.close
        )
    } else if pace.odd_half == 0 && pace.whole > 0 {
        format!(
            "{} close spacings, all whole slots, as a piconet's are",
            pace.close
        )
    } else {
        format!(
            "{} close spacings, {} on odd half slots: not enough to say",
            pace.close, pace.odd_half
        )
    })
}

fn detail(
    p: &Piconet,
    state: &SdrMetrics,
    now: std::time::Instant,
    iw: usize,
    theme: &crate::Theme,
) -> Vec<Line<'static>> {
    let field = |label: &str, value: String| {
        Line::from(vec![
            crate::ui::chrome::field(label, LABEL_W, theme),
            Span::styled(value, Style::default().fg(theme.value)),
        ])
    };
    let since =
        |t: std::time::Instant| format!("{} ago", ago(now.saturating_duration_since(t).as_secs()));
    let kind = p.kind();
    let mut out = vec![
        crate::ui::chrome::section(
            match kind {
                Kind::Inquiry(_) => "inquiry",
                Kind::Paged => "page",
                Kind::Piconet => "piconet",
            },
            "",
            iw,
            theme,
        ),
        field(
            "LAP",
            match kind {
                Kind::Inquiry(i) => format!("{:#08x}  {}, no one's address", p.lap, i.short()),
                Kind::Paged => format!(
                    "{}  the paged device's, not a master's",
                    state.net.show_lap(p.lap)
                ),
                Kind::Piconet => format!("{}  the master's", state.net.show_lap(p.lap)),
            },
        ),
        field("hits", p.hits.to_string()),
        field(
            "heard",
            format!("first {}, last {}", since(p.first_seen), since(p.last_seen)),
        ),
    ];
    let room = iw.saturating_sub(LABEL_W + 1);
    for (label, text) in [
        (
            "channels",
            // Of 79, not of the channels watched now: in SURVEY a piconet
            // was heard wherever the survey stood at the time.
            format!(
                "{} of 79: {}",
                p.channels_hit(),
                channel_runs(p.channel_mask())
            ),
        ),
        match kind {
            Kind::Inquiry(i) => ("meaning", i.meaning().to_string()),
            Kind::Paged => (
                "meaning",
                "someone is calling the device this LAP belongs to".to_string(),
            ),
            Kind::Piconet => (
                "UAP",
                uap_sentence(state.net.bt_uap.get(&p.lap), &state.net),
            ),
        },
    ]
    .into_iter()
    .chain(pace_text(p).map(|t| ("pace", t)))
    {
        for (i, chunk) in crate::ui::chrome::wrap(&text, room, 3)
            .into_iter()
            .enumerate()
        {
            out.push(field(if i == 0 { label } else { "" }, chunk));
        }
    }
    out
}

/// A piconet summed up: its LAP, its UAP in words, its readings on one
/// line, and where the rest of it is. The sections themselves are the
/// Piconet view's bench, one side each, so this view stays the overview it
/// is: who is here, and where they hop.
///
/// **The readings every member's, pooled**, as the Classic view always
/// showed them: the index and f0 from every header, and the rms of every
/// member's residuals from the slot grid, which is not a jitter: the two
/// sides' offset from each other is in it (the bench has each side's). Each to the places its uncertainty gives it, and a dash until it
/// can be stated; the ± is the bench's, where there is room for it.
fn summary(p: &Piconet, state: &SdrMetrics, iw: usize, theme: &crate::Theme) -> Vec<Line<'static>> {
    let field = |label: &str, value: String| {
        Line::from(vec![
            crate::ui::chrome::field(label, LABEL_W, theme),
            Span::styled(value, Style::default().fg(theme.value)),
        ])
    };
    let mut out = vec![
        crate::ui::chrome::section("piconet", "", iw, theme),
        field(
            "LAP",
            format!("{}  the master's", state.net.show_lap(p.lap)),
        ),
    ];
    let uap = uap_sentence(state.net.bt_uap.get(&p.lap), &state.net);
    for (i, chunk) in crate::ui::chrome::wrap(&uap, iw.saturating_sub(LABEL_W + 1), 2)
        .into_iter()
        .enumerate()
    {
        out.push(field(if i == 0 { "UAP" } else { "" }, chunk));
    }
    let value = |r: Option<Reading>| {
        r.and_then(|r| r.value_text())
            .unwrap_or_else(|| "—".to_string())
    };
    let h = &p.headers;
    let index =
        value(sections::index_of(&h.deviation).map(|i| Reading::new(i, "", INDEX_RESOLUTION)));
    let f0 = value(
        h.carrier
            .f0_ppm
            .mean()
            .zip(h.carrier.channel_mhz.mean())
            .map(|(ppm, mhz)| {
                let (ppm, _) = state.radio.corrected_ppm(ppm, std::time::Instant::now());
                Reading::new(ppm.scale(mhz.value() / 1e3), "kHz", F0_RESOLUTION_KHZ)
            }),
    );
    let rms = value(
        p.slots
            .as_ref()
            .and_then(|s| s.as_ref().ok())
            .map(|f| Reading::new(f.rms_us, "us", JITTER_RESOLUTION_US)),
    );
    let dot = || Span::styled(" · ".to_string(), Style::default().fg(theme.label));
    let unit = |u: &str| Span::styled(u.to_string(), Style::default().fg(theme.label));
    let ink = |t: String| Span::styled(t, Style::default().fg(theme.value));
    out.push(Line::from(vec![
        unit(" index "),
        ink(index),
        dot(),
        unit("f0 "),
        ink(f0),
        unit(" kHz"),
        dot(),
        unit("grid rms "),
        ink(rms),
        unit(" us"),
    ]));
    out.push(Line::from(Span::styled(
        " Enter: packet by packet, on NET 6".to_string(),
        Style::default().fg(theme.label),
    )));
    out
}

/// The selected LAP's detail within `budget` rows, or nothing where it does
/// not fit (the table's rows come first, as the census's do): a piconet's
/// summary, or an inquiry's or a page's account, which has nowhere else to
/// be, since neither is a piconet the Piconet view could open.
fn detail_within(
    p: &Piconet,
    state: &SdrMetrics,
    now: std::time::Instant,
    iw: usize,
    budget: usize,
    theme: &crate::Theme,
) -> Vec<Line<'static>> {
    let text = match p.kind() {
        Kind::Piconet => {
            let out = summary(p, state, iw, theme);
            return if out.len() > budget { Vec::new() } else { out };
        }
        Kind::Inquiry(_) => format!(
            "UAP fixed at the DCI, {DCI:#04x}. Every device inquiring sends this code, \
             so there is no one clock, modulation or header stream to read"
        ),
        Kind::Paged => format!(
            "ID packets only: no header after any of {} hits. A page is a caller's \
             ID packets, not a piconet, so there is no clock, modulation or header \
             stream of one to read",
            p.hits
        ),
    };
    let mut out = detail(p, state, now, iw, theme);
    if out.len() > budget {
        return Vec::new();
    }
    for chunk in crate::ui::chrome::wrap(&text, iw.saturating_sub(1), 3) {
        if out.len() < budget {
            out.push(Line::from(Span::styled(
                format!(" {chunk}"),
                Style::default().fg(theme.label),
            )));
        }
    }
    out
}

impl Panel for NetBtPiconetsPanel {
    fn name(&self) -> &'static str {
        "net_bt_piconets"
    }

    fn min_size(&self) -> (u16, u16) {
        (30, 6)
    }

    fn focus_key(&self) -> Option<char> {
        // The command rail's letter too: no NET layout shows the rail
        // (`app::FocusKeys`).
        Some('c')
    }

    fn focus_bindings(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("↑↓", "select a piconet"),
            ("Enter", "packet by packet, on NET 6"),
        ]
    }

    fn chrome(&self, state: &SdrMetrics) -> PanelChrome {
        PanelChrome::new("Pi_conets")
            .stale_when(Staleness::NotStreaming)
            .shows_laps()
            // The TIMING block's clock error is an offset like a BLE one,
            // and worth what the reference makes it.
            .shows_offsets()
            .tag_if(true, state.net.mode.tag())
            // Hits and first sightings accumulate for the session, so a drop
            // at any point in it undercounts them.
            .counts_from_feed(FeedSpan::Session)
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

        // The three silences, each named (bar item 3).
        if let Some(lines) = silence(state, width, theme) {
            f.render_widget(Paragraph::new(lines), inner);
            return;
        }
        let roster = ordered(&state.net.bt_piconets);

        let now = std::time::Instant::now();
        // A column holds its widest cell whole (a long session's hit count).
        let all: Vec<Vec<String>> = roster.iter().map(|p| cells(p, state, now)).collect();
        let columns = grow_to_contents(COLUMNS, &all, &[]);
        let fit = columns_that_fit(&columns, width);
        let laps: Vec<u32> = roster.iter().map(|p| p.lap).collect();
        let cursor = state.net.bt_view.cursor(&laps);
        let height = inner.height as usize;

        // The detail block gives way to the table, as the census's does.
        // What the table keeps (header, its first rows, a gap) comes
        // first; the detail takes what is left.
        let budget = height.saturating_sub(2 + roster.len().min(TABLE_KEEPS));
        let extra = cursor
            .map(|i| detail_within(roster[i], state, now, width, budget, theme))
            .unwrap_or_default();

        let mut lines = vec![header(
            &columns,
            fit,
            Sort {
                column: ORDERED_BY,
                descending: false,
            },
            theme,
        )];
        // The rows the roster has, up to what the block leaves: the block
        // follows the last row rather than the foot of the panel, so a short
        // roster does not hold its detail a screen away from it.
        let body = height.saturating_sub(1 + extra.len()).min(roster.len());
        let start = viewport_start(
            state.net.bt_view.first_visible,
            cursor.unwrap_or(0),
            roster.len(),
            body,
        );
        for (i, p) in roster.iter().enumerate().skip(start).take(body) {
            let mut line = row(
                &columns,
                fit,
                &cells(p, state, now),
                Some(i) == cursor,
                theme,
            );
            // The chip wears the colour the scatter draws this piconet in:
            // its place in `bt_piconets`, the order first heard.
            let colour = state
                .net
                .bt_piconets
                .iter()
                .position(|q| q.lap == p.lap)
                .map(|k| theme.series_color(k));
            if let (Some(colour), Some(cell)) = (colour, line.spans.get(1).cloned()) {
                let text = cell.content.to_string();
                if let Some(rest) = text.strip_prefix(CHIP) {
                    line.spans.splice(
                        1..2,
                        [
                            Span::styled(CHIP.to_string(), cell.style.fg(colour)),
                            Span::styled(rest.to_string(), cell.style),
                        ],
                    );
                }
            }
            lines.push(line);
        }
        if !extra.is_empty() && lines.len() + extra.len() < height {
            lines.push(Line::from(""));
        }
        lines.extend(extra);
        f.render_widget(Paragraph::new(lines), inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::bt::piconet::observe;
    use crate::state::fixture::draw;
    use std::time::{Duration, Instant};

    fn heard() -> SdrMetrics {
        let mut m = SdrMetrics::fixture().streaming();
        m.net.bt_channels_watched = (0..20).collect();
        let t = Instant::now() - Duration::from_secs(30);
        for (i, ch) in [2u8, 3, 4, 5, 17].into_iter().enumerate() {
            observe(
                &mut m.net.bt_piconets,
                0x5a3c71,
                ch,
                t + Duration::from_secs(i as u64),
            );
        }
        observe(&mut m.net.bt_piconets, 0x123456, 9, Instant::now());
        m.net.bt_uap.insert(0x5a3c71, vec![0x4c, 0x9a]);
        m.net.bt_uap.insert(0x123456, vec![0x21]);
        m
    }

    /// The three silences are three sentences (bar item 3): refused, not
    /// running, and listening with nothing heard.
    #[test]
    fn an_empty_roster_says_which_silence_it_is() {
        let mut m = SdrMetrics::fixture().streaming();
        m.net.bt_refused = Some("no classic channel fits the view".to_string());
        let out = draw(NetBtPiconetsPanel, 50, 8, &m).join("\n");
        assert!(out.contains("not watching"), "{out}");
        assert!(out.contains("no classic channel fits"), "{out}");

        m.net.bt_refused = None;
        let out = draw(NetBtPiconetsPanel, 50, 8, &m).join("\n");
        assert!(out.contains("no classic receiver running"), "{out}");

        m.net.bt_channels_watched = vec![10, 20];
        let out = draw(NetBtPiconetsPanel, 50, 8, &m).join("\n");
        assert!(out.contains("watching 2 channels"), "{out}");
    }

    /// An inquiry code wears its own name and a UAP fixed by the
    /// specification, and its detail stops before the sections that would
    /// average every searching device into one.
    #[test]
    fn an_inquiry_code_is_named_and_not_read_as_a_piconet() {
        let mut m = heard();
        observe(&mut m.net.bt_piconets, 0x9E_8B33, 40, Instant::now());
        m.net.bt_view.selected = Some(0x9E_8B33);
        let out = draw(NetBtPiconetsPanel, 60, 24, &m);
        let text = out.join("\n");
        let row = out.iter().find(|l| l.contains("GIAC")).expect(&text);
        assert!(row.contains("DCI"), "{row}");
        assert!(!text.contains("0x9e8b33  the master's"), "{text}");
        assert!(text.contains("INQUIRY"), "{text}");
        assert!(text.contains("no one's address"), "{text}");
        assert!(text.contains("general inquiry"), "{text}");
        assert!(!text.contains("MODULATION"), "{text}");
        assert!(!text.contains("TIMING"), "{text}");
        assert!(!text.contains("HEADERS"), "{text}");
        // Its pace, when the worker has read it.
        let giac = m
            .net
            .bt_piconets
            .iter_mut()
            .find(|p| p.lap == 0x9E_8B33)
            .unwrap();
        giac.pace = crate::signal::bt::slots::Pace {
            close: 40,
            whole: 18,
            odd_half: 20,
        };
        let text = draw(NetBtPiconetsPanel, 60, 24, &m).join("\n");
        assert!(text.contains("pace"), "{text}");
        assert!(
            text.contains("20 of 40 close spacings on odd half"),
            "{text}"
        );
    }

    /// Masked, no LAP and no UAP value reaches the screen: a piconet is its
    /// roster number, a resolved UAP says it was found, and the detail's LAP
    /// line follows.
    #[test]
    fn masked_the_roster_shows_numbers_not_address_bits() {
        let mut m = heard();
        m.net.address_display = crate::state::AddressDisplay::Masked;
        m.net.bt_view.selected = Some(0x123456);
        let text = draw(NetBtPiconetsPanel, 70, 24, &m).join("\n");
        assert!(
            !text.contains("5a3c71") && !text.contains("123456"),
            "{text}"
        );
        assert!(!text.contains("0x21"), "{text}");
        assert!(text.contains("#2") && text.contains("found"), "{text}");
        assert!(text.contains("#2  the master's"), "{text}");
        assert!(text.contains("[MASKED"), "the frame says so: {text}");
        // In oui a LAP is shown as it is, and the frame claims nothing.
        m.net.address_display = crate::state::AddressDisplay::Oui;
        let text = draw(NetBtPiconetsPanel, 70, 24, &m).join("\n");
        assert!(
            text.contains("0x123456") && !text.contains("[OUI"),
            "{text}"
        );
    }

    /// The sort mark stands on the column the rows are ordered by, LAST,
    /// whatever columns are added before it.
    #[test]
    fn the_sort_mark_is_on_last() {
        assert_eq!(COLUMNS[ORDERED_BY].title, "LAST");
        let out = draw(NetBtPiconetsPanel, 70, 8, &heard());
        let head = out.iter().find(|l| l.contains("LAP")).unwrap();
        assert!(head.contains("LAST\u{25b4}"), "{head}");
    }

    /// A LAP with both signs of a page is named so, in the table and the
    /// detail, and its detail stops before the piconet's sections.
    #[test]
    fn a_page_is_named_and_not_read_as_a_piconet() {
        let mut m = heard();
        for _ in 0..20 {
            observe(&mut m.net.bt_piconets, 0x9a_0af4, 12, Instant::now());
        }
        let page = m
            .net
            .bt_piconets
            .iter_mut()
            .find(|p| p.lap == 0x9a_0af4)
            .unwrap();
        page.pace = crate::signal::bt::slots::Pace {
            close: 57,
            whole: 20,
            odd_half: 37,
        };
        m.net.bt_view.selected = Some(0x9a_0af4);
        let out = draw(NetBtPiconetsPanel, 70, 26, &m);
        let text = out.join("\n");
        let row = out
            .iter()
            .find(|l| l.contains("0x9a0af4") && l.contains('\u{25cf}'))
            .expect(&text);
        assert!(row.contains("paged"), "{row}");
        assert!(text.contains("PAGE"), "{text}");
        assert!(
            text.contains("the paged device's, not a master's"),
            "{text}"
        );
        assert!(text.contains("no header after any of 20 hits"), "{text}");
        assert!(!text.contains("MODULATION"), "{text}");
        // The piconets keep their word.
        let other = out.iter().find(|l| l.contains("0x123456")).expect(&text);
        assert!(other.contains("piconet"), "{other}");
    }

    /// **One row per piconet, the most recently heard first**, with its
    /// hits, how many channels, and how far its UAP has narrowed.
    #[test]
    fn each_piconet_gets_a_row_the_newest_first() {
        let out = draw(NetBtPiconetsPanel, 50, 8, &heard());
        let text = out.join("\n");
        let newest = text.find("0x123456").expect(&text);
        let older = text.find("0x5a3c71").expect(&text);
        assert!(newest < older, "{text}");
        let row = out.iter().find(|l| l.contains("0x5a3c71")).unwrap();
        assert!(row.contains("2 left"), "{row}");
        assert!(row.contains(" 5 "), "five hits: {row}");
        let row = out.iter().find(|l| l.contains("0x123456")).unwrap();
        assert!(row.contains("0x21"), "{row}");
    }

    /// **A first header leaves 32 candidates**, seen on the air: the cell
    /// holds `32 left` whole, and the detail says how many rather than
    /// listing a wall of values.
    #[test]
    fn many_candidates_are_counted_not_listed() {
        let mut m = heard();
        m.net
            .bt_uap
            .insert(0x5a3c71, (0..32).map(|i| i * 8 + 1).collect());
        m.net.bt_view.selected = Some(0x5a3c71);
        let out = draw(NetBtPiconetsPanel, 60, 16, &m).join("\n");
        assert!(out.contains("32 left"), "{out}");
        assert!(
            out.contains("32 candidates; each further header narrows them"),
            "{out}"
        );
        assert!(!out.contains("0x09"), "no list: {out}");
    }

    /// The detail follows the last row, not the foot of the panel: a short
    /// roster keeps its detail beside it.
    #[test]
    fn the_detail_follows_the_roster_rather_than_the_foot() {
        let mut m = heard();
        m.net.bt_view.selected = Some(0x5a3c71);
        let out = draw(NetBtPiconetsPanel, 60, 30, &m);
        let block = out.iter().position(|l| l.contains("PICONET")).unwrap();
        // The frame, the header, two rows, a gap.
        assert_eq!(block, 5, "{}", out.join("\n"));
    }

    #[test]
    fn it_fits_every_size_the_layout_can_hand_it() {
        let mut m = heard();
        m.net.bt_view.selected = Some(0x5a3c71);
        for w in 30..70u16 {
            for h in 6..24u16 {
                for s in [&m, &SdrMetrics::fixture()] {
                    for line in draw(NetBtPiconetsPanel, w, h, s) {
                        assert!(line.chars().count() <= w as usize, "{w}x{h}: {line:?}");
                    }
                }
            }
        }
    }

    /// **The selected piconet, summed up**: its LAP, its UAP in words, its
    /// readings on one line, and where the rest of it is. The sections live
    /// on NET 6's bench, one side each.
    #[test]
    fn the_selected_piconet_is_summed_up_and_points_at_net_6() {
        let mut m = heard();
        m.net.bt_view.selected = Some(0x5a3c71);
        let out = draw(NetBtPiconetsPanel, 60, 30, &m);
        let text = out.join("\n");
        let head = out.iter().position(|l| l.contains("PICONET")).expect(&text);
        let hint = out
            .iter()
            .position(|l| l.contains("Enter: packet by packet"))
            .expect(&text);
        assert!(
            hint - head <= 5,
            "three fields, the UAP on two rows at most, and the hint: {text}"
        );
        assert!(text.contains("the master's"), "{text}");
        assert!(text.contains("2 candidates (0x4c, 0x9a)"), "{text}");
        for gone in ["MODULATION", "CARRIER", "TIMING", "HEADERS"] {
            assert!(!text.contains(gone), "{gone}: {text}");
        }
        let none = draw(NetBtPiconetsPanel, 60, 16, &heard()).join("\n");
        assert!(!none.contains("PICONET"), "{none}");
    }

    /// The readings on one line, every member's pooled as the Classic view
    /// always showed them: the index, f0, and the rms from the slot grid,
    /// each dashed until it can be stated.
    #[test]
    fn the_summary_reads_on_one_line() {
        use crate::signal::bt::piconet::{observe_header, Deviation, HeaderRead};
        use crate::signal::dsp::deviation::Sums;
        let mut m = heard();
        m.net.bt_view.selected = Some(0x5a3c71);
        let line = |m: &SdrMetrics| {
            draw(NetBtPiconetsPanel, 70, 30, m)
                .into_iter()
                .find(|l| l.contains("index"))
                .unwrap_or_default()
        };
        let empty = line(&m);
        assert!(
            empty.contains("index —") && empty.contains("f0 —"),
            "{empty}"
        );

        let dev = Deviation {
            settled: Sums::of(&[158_000.0, 160_000.0, 162_000.0, 160_000.0]),
            alternating: Sums::default(),
            neighbour_busy: 0,
        };
        let carrier = crate::signal::bt::piconet::Carrier {
            f0_ppm: Sums::of(&[4.0, 6.0]),
            channel_mhz: Sums::of(&[2441.0, 2441.0]),
            worst_drift_hz: None,
            worst_rate_hz_per_us: None,
        };
        observe_header(
            &mut m.net.bt_piconets,
            0x5a3c71,
            HeaderRead::Unresolved,
            32,
            dev,
            carrier,
        );
        let times: Vec<f64> = (0..40u32)
            .map(|k| f64::from(k * 5) * crate::signal::bt::slots::SLOT_US)
            .collect();
        let p = m
            .net
            .bt_piconets
            .iter_mut()
            .find(|p| p.lap == 0x5a3c71)
            .unwrap();
        p.slots = Some(crate::signal::bt::slots::fit(&times));
        let full = line(&m);
        assert!(full.contains("index 0.32"), "{full}");
        assert!(full.contains("f0 12.2 kHz"), "{full}");
        assert!(full.contains("grid rms"), "{full}");
    }
}
