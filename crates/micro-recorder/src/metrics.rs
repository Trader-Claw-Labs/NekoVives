//! Live microstructure state + metric computation.
//!
//! Folds the raw event stream into a rolling state, then on a fixed cadence emits
//! a `MetricSnapshot` with the indicators from the HFT microstructure literature:
//!
//!   OBI   — Order Book Imbalance (static): (Qbid − Qask)/(Qbid + Qask), L1 and L5.
//!   OFI   — Order Flow Imbalance (dynamic): net change in best-level depth between
//!           successive book updates (Cont et al.), accumulated over a window.
//!   CVD   — Cumulative Volume Delta: running sum of signed taker volume.
//!   VAMP  — Volume-Adjusted Mid Price: mid weighted by opposite-side depth.
//!   basis — perp/spot − chainlink, in bps (the resolving-price divergence).
//!   liquidations — signed forced-liq notional in the trailing window.
//!
//! All raw events are still written verbatim by the writer, so any of these can be
//! recomputed offline with a different window/definition.

use std::collections::VecDeque;

use serde::Serialize;

use crate::types::{BookSnapshot, LiqPrint, TradePrint};

/// Trailing windows (ms) used by the metric computations.
const CVD_HIST_MS: i64 = 300_000; // keep 5 min of trades for windowed CVD slices
const OFI_WINDOW_MS: i64 = 5_000;
const LIQ_WINDOW_MS: i64 = 60_000;

#[derive(Serialize, Default, Clone)]
pub struct MetricSnapshot {
    pub ts_ms: i64,

    // ── Spot venue ──────────────────────────────────────────────────────────
    pub spot_mid: f64,
    pub spot_obi_l1: f64,
    pub spot_obi_l5: f64,
    pub spot_vamp: f64,
    pub spot_ofi_5s: f64,

    // ── Binance perp venue (depth only on geo-restricted IPs) ─────────────────
    pub perp_mid: f64,
    pub perp_obi_l1: f64,
    pub perp_obi_l5: f64,
    pub perp_vamp: f64,
    pub perp_ofi_5s: f64,

    // ── Bybit perp venue (primary derivatives venue: trades+liq+funding) ──────
    pub bybit_mid: f64,
    pub bybit_obi_l1: f64,
    pub bybit_obi_l5: f64,
    pub bybit_vamp: f64,
    pub bybit_ofi_5s: f64,

    // ── Mark / funding (whichever derivatives venue delivers them) ────────────
    pub mark_price: f64,
    pub index_price: f64,
    pub funding_rate: f64,

    // ── Flow: CVD per venue. `cvd_total`/`*_5s`/`*_15s` track Binance perp for
    //    backward-compat; `*_spot` track Binance spot; `*_bybit` track Bybit perp
    //    (the venue that actually delivers the trade tape on restricted IPs). ──
    pub cvd_total: f64,
    pub cvd_5s: f64,
    pub cvd_15s: f64,
    pub cvd_total_spot: f64,
    pub cvd_5s_spot: f64,
    pub cvd_15s_spot: f64,
    pub cvd_total_bybit: f64,
    pub cvd_5s_bybit: f64,
    pub cvd_15s_bybit: f64,
    pub trade_count_total: u64,
    pub trade_count_spot: u64,
    pub trade_count_bybit: u64,

    // ── Liquidations (Binance @forceOrder and/or Bybit allLiquidation) ────────
    /// Signed liq notional in the last 60s: +long-liq, −short-liq.
    pub liq_notional_60s: f64,
    pub liq_long_notional_60s: f64,
    pub liq_short_notional_60s: f64,
    pub liq_count_total: u64,

    // ── Oracle / basis ─────────────────────────────────────────────────────────
    pub chainlink: f64,
    /// (derivatives_mid − chainlink)/chainlink × 1e4, where derivatives_mid is the
    /// Binance perp mid if available, else the Bybit perp mid.
    pub basis_bps: f64,
    pub oracle_age_ms: i64,

    // ── Polymarket (active window) ─────────────────────────────────────────────
    pub pm_up_bid: f64,
    pub pm_up_ask: f64,
    pub pm_down_bid: f64,
    pub pm_down_ask: f64,
    pub window_secs_left: i64,
}

/// A single CEX venue's rolling state.
#[derive(Default)]
struct VenueState {
    book: BookSnapshot,
    prev_best_bid: Option<(f64, f64)>, // (price, qty) for OFI
    prev_best_ask: Option<(f64, f64)>,
    /// (ts_ms, signed_ofi_delta) for windowed OFI.
    ofi_hist: VecDeque<(i64, f64)>,
}

impl VenueState {
    fn on_book(&mut self, mut b: BookSnapshot) {
        b.normalize();
        // ── OFI: Cont/Kukanov/Stoikov best-level flow ──────────────────────────
        // ΔOFI = e_bid − e_ask, where e is the signed depth change at the best
        // quote accounting for price moves (new better quote = full add; worse =
        // full removal; same price = delta size).
        let new_bid = b.best_bid().map(|l| (l.price, l.qty));
        let new_ask = b.best_ask().map(|l| (l.price, l.qty));
        let e_bid = bid_flow(self.prev_best_bid, new_bid);
        let e_ask = ask_flow(self.prev_best_ask, new_ask);
        let ofi = e_bid - e_ask;
        if ofi != 0.0 {
            self.ofi_hist.push_back((b.ts_ms, ofi));
        }
        let cutoff = b.ts_ms - CVD_HIST_MS;
        while self.ofi_hist.front().is_some_and(|x| x.0 < cutoff) {
            self.ofi_hist.pop_front();
        }
        self.prev_best_bid = new_bid;
        self.prev_best_ask = new_ask;
        self.book = b;
    }

    fn obi(&self, levels: usize) -> f64 {
        let bid: f64 = self.book.bids.iter().take(levels).map(|l| l.qty).sum();
        let ask: f64 = self.book.asks.iter().take(levels).map(|l| l.qty).sum();
        let tot = bid + ask;
        if tot < f64::EPSILON {
            0.0
        } else {
            (bid - ask) / tot
        }
    }

    /// Volume-Adjusted Mid Price: weight each side by the OPPOSITE side's size,
    /// so a heavy bid pulls the fair price UP toward the ask.
    fn vamp(&self) -> f64 {
        match (self.book.best_bid(), self.book.best_ask()) {
            (Some(b), Some(a)) => {
                let tot = b.qty + a.qty;
                if tot < f64::EPSILON {
                    (b.price + a.price) / 2.0
                } else {
                    (b.price * a.qty + a.price * b.qty) / tot
                }
            }
            _ => 0.0,
        }
    }

    fn ofi_window(&self, now_ms: i64, window_ms: i64) -> f64 {
        let cutoff = now_ms - window_ms;
        self.ofi_hist
            .iter()
            .filter(|(ts, _)| *ts >= cutoff)
            .map(|(_, v)| *v)
            .sum()
    }
}

/// Best-bid flow term for OFI.
fn bid_flow(prev: Option<(f64, f64)>, new: Option<(f64, f64)>) -> f64 {
    match (prev, new) {
        (Some((pp, pq)), Some((np, nq))) => {
            if np > pp {
                nq // price improved → all new size is added demand
            } else if np < pp {
                -pq // best bid pulled back → all old size is removed demand
            } else {
                nq - pq // same level → net size change
            }
        }
        (None, Some((_, nq))) => nq,
        (Some((_, pq)), None) => -pq,
        (None, None) => 0.0,
    }
}

/// Best-ask flow term for OFI (mirror of bid).
fn ask_flow(prev: Option<(f64, f64)>, new: Option<(f64, f64)>) -> f64 {
    match (prev, new) {
        (Some((pp, pq)), Some((np, nq))) => {
            if np < pp {
                nq // ask improved (lower) → new supply added
            } else if np > pp {
                -pq // ask pulled up → old supply removed
            } else {
                nq - pq
            }
        }
        (None, Some((_, nq))) => nq,
        (Some((_, pq)), None) => -pq,
        (None, None) => 0.0,
    }
}

/// Polymarket per-token top-of-book (maintained from snapshot + deltas).
#[derive(Default, Clone)]
pub struct PmToken {
    pub best_bid: f64,
    pub best_ask: f64,
}

#[derive(Default)]
pub struct RecorderState {
    spot: VenueState,
    perp: VenueState,
    bybit: VenueState,

    // Flow tape (Binance perp).
    trades: VecDeque<TradePrint>,
    cvd_total: f64,
    trade_count_total: u64,

    // Flow tape (Binance spot) — cross-check / fallback for the perp tape.
    trades_spot: VecDeque<TradePrint>,
    cvd_total_spot: f64,
    trade_count_spot: u64,

    // Flow tape (Bybit perp) — primary tape on Binance-restricted IPs.
    trades_bybit: VecDeque<TradePrint>,
    cvd_total_bybit: f64,
    trade_count_bybit: u64,

    // Liquidations (perp).
    liqs: VecDeque<LiqPrint>,
    liq_count_total: u64,

    // Mark / funding.
    mark_price: f64,
    index_price: f64,
    funding_rate: f64,

    // Oracle.
    chainlink: f64,
    chainlink_ts_ms: i64,

    // Polymarket active window.
    pub up_token: String,
    pub down_token: String,
    pub window_close_ms: i64,
    pm_up: PmToken,
    pm_down: PmToken,
}

impl RecorderState {
    pub fn on_trade(&mut self, src: &str, t: TradePrint) {
        // CVD per venue: perp is the price-discovery venue, spot is the fallback /
        // cross-check (e.g. when a network blocks the futures trade stream).
        match src {
            crate::types::SRC_BINANCE_PERP => {
                self.cvd_total += t.signed_qty();
                self.trade_count_total += 1;
                self.trades.push_back(t);
                let cutoff = t.ts_ms - CVD_HIST_MS;
                while self.trades.front().is_some_and(|x| x.ts_ms < cutoff) {
                    self.trades.pop_front();
                }
            }
            crate::types::SRC_BINANCE_SPOT => {
                self.cvd_total_spot += t.signed_qty();
                self.trade_count_spot += 1;
                self.trades_spot.push_back(t);
                let cutoff = t.ts_ms - CVD_HIST_MS;
                while self.trades_spot.front().is_some_and(|x| x.ts_ms < cutoff) {
                    self.trades_spot.pop_front();
                }
            }
            crate::types::SRC_BYBIT_PERP => {
                self.cvd_total_bybit += t.signed_qty();
                self.trade_count_bybit += 1;
                self.trades_bybit.push_back(t);
                let cutoff = t.ts_ms - CVD_HIST_MS;
                while self.trades_bybit.front().is_some_and(|x| x.ts_ms < cutoff) {
                    self.trades_bybit.pop_front();
                }
            }
            _ => {}
        }
    }

    pub fn on_book(&mut self, src: &str, b: BookSnapshot) {
        match src {
            crate::types::SRC_BINANCE_SPOT => self.spot.on_book(b),
            crate::types::SRC_BINANCE_PERP => self.perp.on_book(b),
            crate::types::SRC_BYBIT_PERP => self.bybit.on_book(b),
            _ => {}
        }
    }

    pub fn on_liq(&mut self, l: LiqPrint) {
        self.liq_count_total += 1;
        self.liqs.push_back(l);
        let cutoff = l.ts_ms - CVD_HIST_MS;
        while self.liqs.front().is_some_and(|x| x.ts_ms < cutoff) {
            self.liqs.pop_front();
        }
    }

    pub fn on_mark(&mut self, mark: f64, index: f64, funding: f64) {
        self.mark_price = mark;
        self.index_price = index;
        self.funding_rate = funding;
    }

    pub fn on_oracle(&mut self, ts_ms: i64, price: f64) {
        self.chainlink = price;
        self.chainlink_ts_ms = ts_ms;
    }

    pub fn set_window(&mut self, up: String, down: String, close_ms: i64) {
        self.up_token = up;
        self.down_token = down;
        self.window_close_ms = close_ms;
    }

    /// Apply a Polymarket price_change delta's best_bid/best_ask to the token book.
    pub fn on_pm_change(&mut self, asset_id: &str, best_bid: f64, best_ask: f64) {
        let tok = if asset_id == self.up_token {
            Some(&mut self.pm_up)
        } else if asset_id == self.down_token {
            Some(&mut self.pm_down)
        } else {
            None
        };
        if let Some(tok) = tok {
            if best_bid > 0.0 {
                tok.best_bid = best_bid;
            }
            if best_ask > 0.0 {
                tok.best_ask = best_ask;
            }
        }
    }

    /// Apply a Polymarket book snapshot's best levels to the token book.
    pub fn on_pm_book(&mut self, asset_id: &str, best_bid: f64, best_ask: f64) {
        let tok = if asset_id == self.up_token {
            Some(&mut self.pm_up)
        } else if asset_id == self.down_token {
            Some(&mut self.pm_down)
        } else {
            None
        };
        if let Some(tok) = tok {
            tok.best_bid = best_bid;
            tok.best_ask = best_ask;
        }
    }

    fn cvd_window(trades: &VecDeque<TradePrint>, now_ms: i64, window_ms: i64) -> f64 {
        let cutoff = now_ms - window_ms;
        trades
            .iter()
            .filter(|t| t.ts_ms >= cutoff)
            .map(|t| t.signed_qty())
            .sum()
    }

    pub fn snapshot(&self, now_ms: i64) -> MetricSnapshot {
        // Signed liquidation notional in the trailing window.
        let cutoff = now_ms - LIQ_WINDOW_MS;
        let mut liq_long = 0.0;
        let mut liq_short = 0.0;
        for l in self.liqs.iter().filter(|l| l.ts_ms >= cutoff) {
            let notional = l.price * l.qty;
            // buyer_is_maker==true => SELL liq => a LONG was force-closed.
            if l.buyer_is_maker {
                liq_long += notional;
            } else {
                liq_short += notional;
            }
        }

        let perp_mid = self.perp.book.mid().unwrap_or(0.0);
        let bybit_mid = self.bybit.book.mid().unwrap_or(0.0);
        // Prefer the Binance perp mid; fall back to Bybit, then the mark price.
        let deriv_mid = if perp_mid > 0.0 {
            perp_mid
        } else if bybit_mid > 0.0 {
            bybit_mid
        } else {
            self.mark_price
        };
        let basis_bps = if self.chainlink > 0.0 && deriv_mid > 0.0 {
            (deriv_mid - self.chainlink) / self.chainlink * 10_000.0
        } else {
            0.0
        };

        MetricSnapshot {
            ts_ms: now_ms,

            spot_mid: self.spot.book.mid().unwrap_or(0.0),
            spot_obi_l1: self.spot.obi(1),
            spot_obi_l5: self.spot.obi(5),
            spot_vamp: self.spot.vamp(),
            spot_ofi_5s: self.spot.ofi_window(now_ms, OFI_WINDOW_MS),

            perp_mid,
            perp_obi_l1: self.perp.obi(1),
            perp_obi_l5: self.perp.obi(5),
            perp_vamp: self.perp.vamp(),
            perp_ofi_5s: self.perp.ofi_window(now_ms, OFI_WINDOW_MS),

            bybit_mid,
            bybit_obi_l1: self.bybit.obi(1),
            bybit_obi_l5: self.bybit.obi(5),
            bybit_vamp: self.bybit.vamp(),
            bybit_ofi_5s: self.bybit.ofi_window(now_ms, OFI_WINDOW_MS),

            mark_price: self.mark_price,
            index_price: self.index_price,
            funding_rate: self.funding_rate,

            cvd_total: self.cvd_total,
            cvd_5s: Self::cvd_window(&self.trades, now_ms, 5_000),
            cvd_15s: Self::cvd_window(&self.trades, now_ms, 15_000),
            cvd_total_spot: self.cvd_total_spot,
            cvd_5s_spot: Self::cvd_window(&self.trades_spot, now_ms, 5_000),
            cvd_15s_spot: Self::cvd_window(&self.trades_spot, now_ms, 15_000),
            cvd_total_bybit: self.cvd_total_bybit,
            cvd_5s_bybit: Self::cvd_window(&self.trades_bybit, now_ms, 5_000),
            cvd_15s_bybit: Self::cvd_window(&self.trades_bybit, now_ms, 15_000),
            trade_count_total: self.trade_count_total,
            trade_count_spot: self.trade_count_spot,
            trade_count_bybit: self.trade_count_bybit,

            liq_notional_60s: liq_long - liq_short,
            liq_long_notional_60s: liq_long,
            liq_short_notional_60s: liq_short,
            liq_count_total: self.liq_count_total,

            chainlink: self.chainlink,
            basis_bps,
            oracle_age_ms: if self.chainlink_ts_ms > 0 {
                now_ms - self.chainlink_ts_ms
            } else {
                0
            },

            pm_up_bid: self.pm_up.best_bid,
            pm_up_ask: self.pm_up.best_ask,
            pm_down_bid: self.pm_down.best_bid,
            pm_down_ask: self.pm_down.best_ask,
            window_secs_left: if self.window_close_ms > 0 {
                ((self.window_close_ms - now_ms) / 1000).max(0)
            } else {
                0
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Level, SRC_BINANCE_PERP};

    fn book(bid_p: f64, bid_q: f64, ask_p: f64, ask_q: f64) -> BookSnapshot {
        BookSnapshot {
            ts_ms: 1000,
            bids: vec![Level { price: bid_p, qty: bid_q }],
            asks: vec![Level { price: ask_p, qty: ask_q }],
        }
    }

    #[test]
    fn obi_sign() {
        let mut v = VenueState::default();
        v.on_book(book(100.0, 9.0, 101.0, 1.0));
        assert!((v.obi(1) - 0.8).abs() < 1e-9);
    }

    #[test]
    fn vamp_pulls_toward_heavy_side() {
        let mut v = VenueState::default();
        // Heavy bid (9) vs thin ask (1): fair price should sit near the ask.
        v.on_book(book(100.0, 9.0, 101.0, 1.0));
        let vamp = v.vamp();
        assert!(vamp > 100.5, "vamp={vamp} should lean to the ask");
    }

    #[test]
    fn cvd_signs_taker_flow() {
        let mut s = RecorderState::default();
        s.on_trade(SRC_BINANCE_PERP, TradePrint { ts_ms: 1, price: 100.0, qty: 3.0, buyer_is_maker: false }); // +3
        s.on_trade(SRC_BINANCE_PERP, TradePrint { ts_ms: 2, price: 100.0, qty: 1.0, buyer_is_maker: true });  // -1
        assert!((s.cvd_total - 2.0).abs() < 1e-9);
    }

    #[test]
    fn ofi_best_bid_improve_positive() {
        let mut v = VenueState::default();
        v.on_book(book(100.0, 5.0, 101.0, 5.0));
        v.on_book(book(100.5, 4.0, 101.0, 5.0)); // bid improved → +demand
        assert!(v.ofi_window(2000, 5_000) > 0.0);
    }
}
