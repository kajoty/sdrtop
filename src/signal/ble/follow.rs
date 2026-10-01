// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 MusiThang <viktor.laszlo92@protonmail.com>

//! Following a BLE connection from its CONNECT_IND: when and where each of
//! its events will be, in the radio's own stream.
//!
//! Plain data in and out: a CONNECT_IND's parameters and the stream
//! position its packet ended at go in, the next event's channel, expected
//! anchor and listening window come out, and what was heard there goes back
//! in. No radio, no clock but the stream's.
//!
//! **The timing, from Core 5.4 Vol 6 Part B**, read on the SIG's site:
//! - 4.5.3: "the start of the first packet will be no earlier than
//!   transmitWindowDelay + transmitWindowOffset and no later than
//!   transmitWindowDelay + transmitWindowOffset + transmitWindowSize after
//!   the end of the packet containing the CONNECT_IND PDU", the delay 1.25 ms
//!   for a CONNECT_IND, the offset and size in 1.25 ms units.
//! - 4.5.4: "The first packet sent in the Connection State by the Central
//!   determines the anchor point for the first connection event, and
//!   therefore the timings of all future connection events"; 4.5.1: "The
//!   start of connection events are spaced regularly with an interval of
//!   connInterval", in 1.25 ms units.
//! - 4.2.4, window widening: the listener widens its window by the
//!   transmitter's clock accuracy times the time since it last synchronised,
//!   plus a fixed allowance, "16 when the sleep clock applies" in us.
//!
//! **Whose clocks.** The Central's accuracy is the SCA its CONNECT_IND
//! declares, at the top of its range (Table 2.11). This radio's own clock is
//! [`OWN_CLOCK_PPM`]: an assumption, not a reading, stated where it is used.
//! The anchors heard will measure the Central's clock against this radio's,
//! which is the measurement, not this window.
// Read only by its tests until the NET worker follows a connection.
#![allow(dead_code)]

use super::connect::{sca_ppm, ConnectIndData, Csa1, Csa2};
use super::Phy;

/// The accuracy this radio's own sample clock is assumed to keep, in ppm,
/// for the listening window only: an assumption, neither measured nor read
/// from a radio's data sheet. Too narrow a window loses events, too wide
/// costs only samples, so it errs wide; a frequency reference does not
/// narrow it yet.
pub const OWN_CLOCK_PPM: f64 = 20.0;

/// 4.2.4's fixed allowance when the sleep clock applies, us: the larger of
/// its two, since which clock the Central runs is not known here.
const WIDENING_FIXED_US: f64 = 16.0;

/// The CONNECT_IND's transmitWindowDelay, us (4.5.3).
const TRANSMIT_WINDOW_DELAY_US: f64 = 1250.0;

/// The unit of WinOffset, WinSize and Interval, us.
const UNIT_US: f64 = 1250.0;

/// The next event a follower should listen for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Due {
    pub counter: u16,
    /// Its data channel index.
    pub channel: u8,
    /// Where its anchor is expected, stream pairs, fractional.
    pub anchor_pair: f64,
    /// How far either side of it to listen, pairs (4.2.4).
    pub widening_pairs: f64,
    /// While no anchor has been heard, the transmit window the first one
    /// fell in (4.5.3): listen from `anchor_pair - widening_pairs` to
    /// `anchor_pair + window_pairs + widening_pairs`. Zero after.
    pub window_pairs: f64,
}

/// Which channel selection algorithm the connection uses, with its state.
#[derive(Clone, Debug)]
enum Hops {
    One(Csa1),
    Two(Csa2),
}

/// Where the timing is anchored: an event's counter and where its anchor
/// was heard, in pairs.
#[derive(Clone, Copy, Debug)]
struct Anchor {
    counter: u16,
    pair: f64,
}

/// One connection, followed.
#[derive(Clone, Debug)]
pub struct Connection {
    params: ConnectIndData,
    raw_rate: f64,
    hops: Hops,
    /// The next event not yet accounted for, and its channel.
    counter: u16,
    channel: u8,
    /// Where the CONNECT_IND ended, pairs: the origin of the transmit
    /// window, and the last synchronisation until an anchor is heard.
    connect_end: f64,
    anchor: Option<Anchor>,
    phy: Phy,
}

impl Connection {
    /// From a CONNECT_IND whose packet ended at `end_pair` in a stream of
    /// `raw_rate` pairs a second, `ch_sel` its header's ChSel bit (set:
    /// Channel Selection Algorithm #2). `None` for a channel map with no
    /// used channel, which no connection can hop on.
    pub fn new(c: &ConnectIndData, ch_sel: bool, end_pair: f64, raw_rate: f64) -> Option<Self> {
        let hops = if ch_sel {
            Hops::Two(Csa2::new(c.access_address, c.channel_map)?)
        } else {
            Hops::One(Csa1::new(c.hop_increment, c.channel_map)?)
        };
        let mut this = Self {
            params: *c,
            raw_rate,
            hops,
            counter: 0,
            channel: 0,
            connect_end: end_pair,
            anchor: None,
            // A legacy CONNECT_IND is sent on LE 1M, and the connection
            // starts on the PHY it was made on.
            phy: Phy::OneM,
        };
        this.channel = this.hop();
        Some(this)
    }

    pub fn access_address(&self) -> u32 {
        self.params.access_address
    }

    pub fn crc_init(&self) -> u32 {
        self.params.crc_init
    }

    pub fn phy(&self) -> Phy {
        self.phy
    }

    /// The event `self.counter`'s channel, advancing CSA #1's state.
    fn hop(&mut self) -> u8 {
        match &mut self.hops {
            Hops::One(csa) => csa.next(),
            Hops::Two(csa) => csa.channel(self.counter).0,
        }
    }

    fn pairs(&self, us: f64) -> f64 {
        us * 1e-6 * self.raw_rate
    }

    fn interval_pairs(&self) -> f64 {
        self.pairs(self.params.interval as f64 * UNIT_US)
    }

    /// The next event to listen for.
    pub fn next_due(&self) -> Due {
        let interval = self.interval_pairs();
        let (anchor_pair, window_pairs, since) = match self.anchor {
            Some(a) => {
                let at = a.pair + (self.counter.wrapping_sub(a.counter)) as f64 * interval;
                (at, 0.0, at - a.pair)
            }
            None => {
                let start = self.connect_end
                    + self
                        .pairs(TRANSMIT_WINDOW_DELAY_US + self.params.win_offset as f64 * UNIT_US);
                let at = start + self.counter as f64 * interval;
                let window = self.pairs(self.params.win_size as f64 * UNIT_US);
                (at, window, at + window - self.connect_end)
            }
        };
        let ppm = sca_ppm(self.params.sca).1 as f64 + OWN_CLOCK_PPM;
        Due {
            counter: self.counter,
            channel: self.channel,
            anchor_pair,
            widening_pairs: ppm * 1e-6 * since + self.pairs(WIDENING_FIXED_US),
            window_pairs,
        }
    }

    /// The event due was heard, its anchor at `anchor_pair`: the timing is
    /// fixed from it, and the next event is due.
    pub fn heard(&mut self, anchor_pair: f64) {
        self.anchor = Some(Anchor {
            counter: self.counter,
            pair: anchor_pair,
        });
        self.advance();
    }

    /// The event due passed without its anchor heard: the next is due, on
    /// the timing as it was.
    pub fn missed(&mut self) {
        self.advance();
    }

    fn advance(&mut self) {
        self.counter = self.counter.wrapping_add(1);
        self.channel = self.hop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::ble::connect::{ConnectIndData, Csa1, Csa2};

    fn params(interval: u16, win_offset: u16, win_size: u8) -> ConnectIndData {
        ConnectIndData {
            init_a: [0; 6],
            adv_a: [0; 6],
            access_address: 0x5065_4b6a,
            crc_init: 0x3a_5b7c,
            win_size,
            win_offset,
            interval,
            latency: 0,
            timeout: 100,
            channel_map: (1u64 << 37) - 1,
            hop_increment: 7,
            sca: 0,
        }
    }

    /// The first event's window is the transmit window: 1.25 ms plus the
    /// offset after the CONNECT_IND ends, WinSize long (4.5.3).
    #[test]
    fn the_first_event_is_looked_for_across_the_transmit_window() {
        let rate = 20e6;
        let c = Connection::new(&params(6, 2, 3), false, 1_000_000.0, rate).unwrap();
        let d = c.next_due();
        assert_eq!(d.counter, 0);
        let expect = 1_000_000.0 + (1.25e-3 + 2.0 * 1.25e-3) * rate;
        assert!((d.anchor_pair - expect).abs() < 1.0, "{d:?}");
        assert!((d.window_pairs - 3.0 * 1.25e-3 * rate).abs() < 1.0, "{d:?}");
        assert_eq!(c.access_address(), 0x5065_4b6a);
        assert_eq!(c.crc_init(), 0x3a_5b7c);
        assert_eq!(c.phy(), Phy::OneM);
    }

    /// Channels follow the algorithm the CONNECT_IND names.
    #[test]
    fn channels_follow_the_named_algorithm() {
        let p = params(6, 0, 1);
        let mut c1 = Connection::new(&p, false, 0.0, 20e6).unwrap();
        let mut csa1 = Csa1::new(7, p.channel_map).unwrap();
        let mut c2 = Connection::new(&p, true, 0.0, 20e6).unwrap();
        let csa2 = Csa2::new(p.access_address, p.channel_map).unwrap();
        for k in 0..20u16 {
            assert_eq!(c1.next_due().counter, k);
            assert_eq!(c1.next_due().channel, csa1.next());
            assert_eq!(c2.next_due().channel, csa2.channel(k).0);
            c1.missed();
            c2.missed();
        }
    }

    /// An anchor heard fixes the next ones a whole number of intervals
    /// later; before any is heard, they are counted from the transmit
    /// window, which stays their uncertainty.
    #[test]
    fn anchors_follow_the_interval() {
        let rate = 20e6;
        let interval = 80.0 * 1.25e-3 * rate;
        let mut c = Connection::new(&params(80, 4, 2), false, 0.0, rate).unwrap();
        let first = c.next_due();
        c.missed();
        let second = c.next_due();
        assert!((second.anchor_pair - first.anchor_pair - interval).abs() < 1.0);
        assert!(
            (second.window_pairs - first.window_pairs).abs() < 1.0,
            "{second:?}"
        );
        let heard_at = second.anchor_pair + 1234.0;
        c.heard(heard_at);
        let third = c.next_due();
        assert_eq!(third.counter, 2);
        assert!(
            (third.anchor_pair - heard_at - interval).abs() < 1.0,
            "{third:?}"
        );
        assert_eq!(third.window_pairs, 0.0);
    }

    /// Unheard events widen the window by the clocks' accuracy over the
    /// time since the last anchor heard (4.2.4); a heard one resets it.
    #[test]
    fn the_window_widens_until_an_anchor_is_heard() {
        let rate = 20e6;
        let mut c = Connection::new(&params(80, 0, 1), false, 0.0, rate).unwrap();
        let first = c.next_due().anchor_pair;
        c.heard(first + 3.0);
        let w1 = c.next_due().widening_pairs;
        c.missed();
        let w2 = c.next_due().widening_pairs;
        assert!(w2 > w1, "{w1} then {w2}");
        let at = c.next_due().anchor_pair;
        c.heard(at);
        assert!((c.next_due().widening_pairs - w1).abs() < 1e-6);
        // SCA 0 is up to 500 ppm (Table 2.11), this radio's own 20: over
        // 100 ms that is 52 us, and 4.2.4's 16 us besides.
        assert!(
            (w1 / rate * 1e6 - (520e-6 * 0.1e6 + 16.0)).abs() < 0.01,
            "{w1}"
        );
    }

    /// A map with no used channel is no connection to follow.
    #[test]
    fn an_empty_channel_map_is_refused() {
        let mut p = params(6, 0, 1);
        p.channel_map = 0;
        assert!(Connection::new(&p, false, 0.0, 20e6).is_none());
    }
}
