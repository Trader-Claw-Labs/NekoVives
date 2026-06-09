//! Feature engineering.
//!
//! These are the inputs the literature actually supports at this horizon:
//! distance-to-beat (in bps), order-flow imbalance, signed trade flow,
//! micro-momentum, realized volatility and time remaining. NOT candles / RSI.
//!
//! IMPORTANT: this Rust implementation is the *live* path. `ml/features.py`
//! mirrors it exactly for training. If you change a formula here, change it
//! there too, or your model trains on features it never sees live.

use serde::{Deserialize, Serialize};

use crate::state::Snapshot;
use crate::types::Trade;

/// One feature vector, aligned 1:1 with `ml/features.py::FEATURE_ORDER`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Features {
    /// Signed distance of the resolving (Chainlink) price from price-to-beat, in bps.
    /// Positive => currently "Up". This is the single strongest input.
    pub dist_bps: f64,
    /// Same distance but using lead-venue spot, to expose basis vs Chainlink.
    pub dist_bps_spot: f64,
    /// Basis between spot and chainlink in bps (spot - chainlink). Large |basis|
    /// near the boundary is the classic blow-up; the model learns to distrust it.
    pub basis_bps: f64,
    /// Top-level order-flow imbalance from the lead book in [-1, 1].
    pub book_imbalance: f64,
    /// Multi-level (depth-weighted) imbalance in [-1, 1].
    pub book_imbalance_l5: f64,
    /// Signed taker volume over the last 15s, normalized by total volume in [-1, 1].
    pub flow_15s: f64,
    /// Signed taker volume over the last 5s in [-1, 1].
    pub flow_5s: f64,
    /// Micro-momentum: spot return over last 15s, in bps.
    pub mom_15s_bps: f64,
    /// Realized volatility over the trailing 60s, in bps (std of 1s returns * sqrt(n)).
    pub rv_60s_bps: f64,
    /// Seconds remaining in the window.
    pub secs_left: f64,
}

impl Features {
    /// Flatten into the canonical order LightGBM/sklearn expects.
    pub fn to_array(&self) -> [f64; 10] {
        [
            self.dist_bps,
            self.dist_bps_spot,
            self.basis_bps,
            self.book_imbalance,
            self.book_imbalance_l5,
            self.flow_15s,
            self.flow_5s,
            self.mom_15s_bps,
            self.rv_60s_bps,
            self.secs_left,
        ]
    }
}

#[inline]
fn bps(numer: f64, denom: f64) -> f64 {
    if denom.abs() < f64::EPSILON {
        0.0
    } else {
        (numer / denom) * 10_000.0
    }
}

/// Depth-weighted order-flow imbalance over the first `levels` book levels.
/// Returns a value in [-1, 1]: +1 fully bid-heavy, -1 fully ask-heavy.
fn imbalance(book: &crate::types::BookTop, levels: usize) -> f64 {
    let bid: f64 = book.bids.iter().take(levels).map(|l| l.qty).sum();
    let ask: f64 = book.asks.iter().take(levels).map(|l| l.qty).sum();
    let tot = bid + ask;
    if tot < f64::EPSILON {
        0.0
    } else {
        (bid - ask) / tot
    }
}

/// Signed flow ratio over a slice of trades: sum(signed_qty) / sum(|qty|).
fn signed_flow(trades: &[Trade], now_ms: i64, window_ms: i64) -> f64 {
    let cutoff = now_ms - window_ms;
    let (mut signed, mut total) = (0.0f64, 0.0f64);
    for t in trades.iter().filter(|t| t.ts_ms >= cutoff) {
        signed += t.signed_qty();
        total += t.qty;
    }
    if total < f64::EPSILON {
        0.0
    } else {
        signed / total
    }
}

/// Realized vol in bps from 1-second bucketed returns of the trade tape.
fn realized_vol_bps(trades: &[Trade], now_ms: i64, window_ms: i64) -> f64 {
    let cutoff = now_ms - window_ms;
    // Bucket last price per second.
    let mut buckets: Vec<(i64, f64)> = Vec::new();
    for t in trades.iter().filter(|t| t.ts_ms >= cutoff) {
        let sec = t.ts_ms / 1000;
        match buckets.last_mut() {
            Some(last) if last.0 == sec => last.1 = t.price,
            _ => buckets.push((sec, t.price)),
        }
    }
    if buckets.len() < 3 {
        return 0.0;
    }
    let rets: Vec<f64> = buckets
        .windows(2)
        .map(|w| (w[1].1 / w[0].1).ln())
        .collect();
    let mean = rets.iter().sum::<f64>() / rets.len() as f64;
    let var = rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / rets.len() as f64;
    var.sqrt() * (rets.len() as f64).sqrt() * 10_000.0
}

/// Compute the full feature vector from a snapshot + recent history.
pub fn compute(snap: &Snapshot, trades: &[Trade]) -> Option<Features> {
    let w = snap.window.as_ref()?;
    let ptb = w.price_to_beat?; // no features until we have the resolving open price
    let now = snap.now_ms;

    let mom_start = trades
        .iter()
        .find(|t| t.ts_ms >= now - 15_000)
        .map(|t| t.price)
        .unwrap_or(snap.spot);

    Some(Features {
        dist_bps: bps(snap.chainlink - ptb, ptb),
        dist_bps_spot: bps(snap.spot - ptb, ptb),
        basis_bps: bps(snap.spot - snap.chainlink, snap.chainlink),
        book_imbalance: imbalance(&snap.book, 1),
        book_imbalance_l5: imbalance(&snap.book, 5),
        flow_15s: signed_flow(trades, now, 15_000),
        flow_5s: signed_flow(trades, now, 5_000),
        mom_15s_bps: bps(snap.spot - mom_start, mom_start),
        rv_60s_bps: realized_vol_bps(trades, now, 60_000),
        secs_left: w.seconds_remaining(now),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BookTop, Level};

    #[test]
    fn imbalance_bounds() {
        let mut b = BookTop::default();
        b.bids = vec![Level { price: 100.0, qty: 9.0 }];
        b.asks = vec![Level { price: 101.0, qty: 1.0 }];
        let i = imbalance(&b, 1);
        assert!((i - 0.8).abs() < 1e-9);
    }

    #[test]
    fn signed_flow_sign() {
        let now = 100_000;
        let trades = vec![
            Trade { ts_ms: now - 1000, price: 100.0, qty: 3.0, buyer_is_maker: false }, // +3 buy
            Trade { ts_ms: now - 500, price: 100.0, qty: 1.0, buyer_is_maker: true },   // -1 sell
        ];
        let f = signed_flow(&trades, now, 5_000);
        assert!((f - 0.5).abs() < 1e-9); // (3-1)/4
    }
}
