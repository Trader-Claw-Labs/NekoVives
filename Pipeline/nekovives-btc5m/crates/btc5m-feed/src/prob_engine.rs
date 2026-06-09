//! Probability + decision layer.
//!
//! Two ways to estimate P(Up at close):
//!   1. `logistic_baseline` — a transparent, tunable fallback that needs no model.
//!   2. A Rhai script (feature `rhai`) so you can hot-swap the calibrated model's
//!      decision logic exactly like your other NekoVives strategies.
//!
//! The decision gate is where the *fee* lives. Crypto is Polymarket's most
//! expensive category, and this is a taker strategy, so the fee is not a footnote
//! — it is the difference between +EV and donating. We never cross the spread
//! unless `p_model - ask - fee(p) - slippage > min_edge`.

use crate::features::Features;
use crate::types::{PmBook, Side};

/// Polymarket per-share taker fee model (Global), as of mid-2026.
/// fee_per_share = fee_rate * p * (1 - p), peaking at p = 0.5.
/// Crypto category peaks near $0.018/share => fee_rate ≈ 0.072.
/// Makers pay 0 (and may earn a rebate). VERIFY before going live; rates change.
pub const CRYPTO_FEE_RATE: f64 = 0.072;

#[inline]
pub fn taker_fee_per_share(p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    CRYPTO_FEE_RATE * p * (1.0 - p)
}

/// Transparent logistic baseline. Weights are starting points to be replaced by
/// the trained+calibrated model; keep them as a sanity fallback.
#[derive(Debug, Clone)]
pub struct LogisticWeights {
    pub w_dist: f64,
    pub w_flow: f64,
    pub w_mom: f64,
    pub w_imb: f64,
    /// Time-decay: amplify `dist` as the close approaches (less future variance).
    pub time_gamma: f64,
}

impl Default for LogisticWeights {
    fn default() -> Self {
        // Distance dominates; everything else nudges. Tune via backtest.
        Self {
            w_dist: 0.18,
            w_flow: 0.6,
            w_mom: 0.04,
            w_imb: 0.4,
            time_gamma: 0.6,
        }
    }
}

#[inline]
fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// P(Up at close) from the baseline. Returns a probability in (0, 1).
pub fn logistic_baseline(f: &Features, w: &LogisticWeights) -> f64 {
    // More weight on distance as time runs out: factor in [1, ~1/secs].
    let urgency = 1.0 + w.time_gamma * (300.0 / (f.secs_left + 5.0)).ln().max(0.0);
    let score = w.w_dist * f.dist_bps * urgency
        + w.w_flow * f.flow_15s
        + w.w_mom * f.mom_15s_bps
        + w.w_imb * f.book_imbalance_l5;
    sigmoid(score)
}

/// A concrete trade the gate decided to take.
#[derive(Debug, Clone)]
pub struct Decision {
    pub side: Side,
    pub limit_price: f64,
    pub stake_usdc: f64,
    pub p_model: f64,
    pub edge_net: f64,
}

/// Hard gate parameters. These mirror the risk-manager's circuit breakers; keep
/// them in one place so the strategy can't silently override them.
#[derive(Debug, Clone)]
pub struct GateParams {
    /// Minimum NET edge (after fee + slippage) to act, in probability units.
    pub min_edge: f64,
    /// Max spread we tolerate (probability units, i.e. cents/100).
    pub max_spread: f64,
    /// Assumed adverse slippage between signal and fill, probability units.
    pub slippage: f64,
    /// Don't trade with fewer than this many seconds left (no time to fill).
    pub min_secs_left: f64,
    /// Don't trade earlier than this many seconds left (too much uncertainty)…
    pub max_secs_left: f64,
    /// …unless the net edge clears this much (rare, very mispriced books).
    pub max_secs_left_override_edge: f64,
    pub max_stake_usdc: f64,
    /// Edge at which we deploy `max_stake_usdc`; smaller edges scale down.
    pub ref_edge: f64,
}

impl Default for GateParams {
    fn default() -> Self {
        Self {
            min_edge: 0.04,
            max_spread: 0.04,
            slippage: 0.02,
            min_secs_left: 5.0,
            max_secs_left: 240.0,
            max_secs_left_override_edge: 0.15,
            max_stake_usdc: 25.0,
            ref_edge: 0.15,
        }
    }
}

/// Decide whether to take a side. Returns `None` to sit out.
///
/// `p_up` is the model's calibrated P(Up at close). We evaluate BOTH sides
/// (buying Up at its ask, or Down at its ask) and take the better net edge.
pub fn gate(
    f: &Features,
    p_up: f64,
    up: &PmBook,
    down: &PmBook,
    g: &GateParams,
) -> Option<Decision> {
    if f.secs_left < g.min_secs_left {
        return None;
    }
    let allow_early = f.secs_left <= g.max_secs_left;

    let candidates = [
        (Side::Up, p_up, up),
        (Side::Down, 1.0 - p_up, down),
    ];

    let mut best: Option<Decision> = None;
    for (side, p, book) in candidates {
        if book.spread() > g.max_spread || book.best_ask <= 0.0 || book.best_ask >= 1.0 {
            continue;
        }
        let fee = taker_fee_per_share(book.best_ask);
        let edge_net = p - book.best_ask - fee - g.slippage;

        let pass = if allow_early {
            edge_net > g.min_edge
        } else {
            edge_net > g.max_secs_left_override_edge
        };
        if !pass {
            continue;
        }

        let scale = (edge_net / g.ref_edge).clamp(0.0, 1.0);
        let stake = g.max_stake_usdc * scale;
        let d = Decision {
            side,
            limit_price: book.best_ask, // IOC at the ask
            stake_usdc: stake,
            p_model: p,
            edge_net,
        };
        best = match best {
            Some(b) if b.edge_net >= d.edge_net => Some(b),
            _ => Some(d),
        };
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(ask: f64, bid: f64) -> PmBook {
        PmBook { ts_ms: 0, best_bid: bid, best_ask: ask, ask_size: 100.0, bid_size: 100.0 }
    }

    #[test]
    fn fee_peaks_at_half() {
        assert!(taker_fee_per_share(0.5) > taker_fee_per_share(0.85));
        assert!((taker_fee_per_share(0.5) - 0.018).abs() < 1e-6);
    }

    #[test]
    fn gate_takes_clear_edge_late() {
        let mut f = Features::default();
        f.secs_left = 20.0;
        // Model says Up 0.80, Up ask is 0.60 => big edge even after fee+slip.
        let d = gate(&f, 0.80, &book(0.60, 0.58), &book(0.45, 0.40), &GateParams::default());
        assert!(matches!(d, Some(ref dec) if dec.side == Side::Up));
    }

    #[test]
    fn gate_sits_out_when_priced_in() {
        let mut f = Features::default();
        f.secs_left = 20.0;
        // Model 0.62, ask 0.61 => edge < fee+slippage. Sit.
        let d = gate(&f, 0.62, &book(0.61, 0.59), &book(0.41, 0.39), &GateParams::default());
        assert!(d.is_none());
    }
}
