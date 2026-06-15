//! Native 3-leg edge validation — the trusted "is this edge real, or luck/artifact?"
//! check. Ports `scripts/ml/edge_validator.py`. Runs on a runner's OFFICIAL-resolution
//! trades; only declares EDGE when all three independent legs pass.
//!
//!   LEG 1 — bootstrap 95% CI on EV/trade (fee-aware). PASS iff CI lower bound > 0.
//!   LEG 2 — random-outcome null: a skill-less bettor at the same prices. PASS iff the
//!           real EV beats that null at p < 0.05.
//!   LEG 3 — shuffled-outcome null: permute win/loss labels. PASS iff p < 0.05.

use serde::Serialize;

const CRYPTO_FEE: f64 = 0.018;

fn fee(p: f64) -> f64 {
    CRYPTO_FEE * p * (1.0 - p)
}

/// Uniform random index in `0..n` (free-function RNG — API-stable across rand versions).
fn rand_idx(n: usize) -> usize {
    ((rand::random::<f64>() * n as f64) as usize).min(n.saturating_sub(1))
}

/// EV per $1 stake for a binary bet entered at `entry`, net of the crypto taker fee.
fn ev1(entry: f64, won: bool) -> f64 {
    if won {
        (1.0 / entry) * (1.0 - fee(entry)) - 1.0
    } else {
        -1.0
    }
}

#[derive(Serialize, Default)]
pub struct ValidationResult {
    pub n: usize,
    pub wr_pct: f64,
    pub break_even_pct: f64,
    pub ev_per_trade_pct: f64,
    pub ci_lo: f64,
    pub ci_hi: f64,
    pub leg1_pass: bool,
    pub p_random: f64,
    pub leg2_pass: bool,
    pub p_shuffle: f64,
    pub leg3_pass: bool,
    /// Leg 4 — calibration null: a bettor that wins each trade with probability equal
    /// to the ENTRY PRICE (the fair value the price implies). Unlike Leg 2 (50/50),
    /// this works at constant price — it asks "does your selection beat the price's own
    /// implied probability?". p_calib < 0.05 → your edge is over fair value, not just
    /// over a coin flip. (Added after an external quant review showed Leg 3 is blind to
    /// constant-price edge.)
    #[serde(default)]
    pub p_calib: f64,
    #[serde(default)]
    pub leg4_pass: bool,
    pub verdict: String, // "EDGE" | "NO_EDGE" | "INSUFFICIENT"
    pub note: String,
}

/// True if a trade's `side` label denotes a BINARY bet the validator can use.
/// Engines disagree on the label: clob_1hz/clob_events emit `bet_yes`/`bet_no`,
/// the strategy-core engine paths emit `yes`/`no`, and the on_candle path
/// (archive_candles / polymarket_binary) emits `{yes|no}_{win|loss}`. Missing
/// any of these (the cause of BUG-2) silently zeroed the validator on the most-
/// used on_candle path. Crypto buy/sell sides are NOT binary → excluded.
pub fn is_binary_side(side: &str) -> bool {
    matches!(side, "bet_yes" | "bet_no" | "yes" | "no")
        || side.starts_with("yes_")
        || side.starts_with("no_")
}

/// Extract `(entries, wons)` from a backtest's trades for the validator, across
/// ALL engine side-label conventions. `won` is `pnl > 0.0`. Entry prices outside
/// (0.01, 0.99) are dropped (non-binary / degenerate).
pub fn extract_binary_trades(
    trades: &[crate::tools::backtest::AllTrade],
) -> (Vec<f64>, Vec<bool>) {
    let mut entries = Vec::new();
    let mut wons = Vec::new();
    for t in trades {
        if is_binary_side(&t.side) && t.price > 0.01 && t.price < 0.99 {
            entries.push(t.price);
            wons.push(t.pnl > 0.0);
        }
    }
    (entries, wons)
}

/// Run the 3-leg validation. `entries` = the price actually paid per trade (the
/// settle/fill price), `wons` = official outcome. `iters` bootstrap/permutation samples.
pub fn validate(entries: &[f64], wons: &[bool], iters: usize) -> ValidationResult {
    let n = entries.len();
    if n < 30 {
        return ValidationResult {
            n,
            verdict: "INSUFFICIENT".to_string(),
            note: format!("n={n} — need ≥30 official-resolution trades for a verdict."),
            p_random: 1.0,
            p_shuffle: 1.0,
            ..Default::default()
        };
    }

    let real_ev: Vec<f64> = entries.iter().zip(wons).map(|(&e, &w)| ev1(e, w)).collect();
    let obs: f64 = real_ev.iter().sum::<f64>() / n as f64;
    let wr = wons.iter().filter(|&&w| w).count() as f64 / n as f64 * 100.0;
    let be = entries.iter().sum::<f64>() / n as f64 * 100.0;

    // LEG 1 — bootstrap CI on EV/trade
    let mut boot: Vec<f64> = (0..iters)
        .map(|_| {
            let s: f64 = (0..n).map(|_| real_ev[rand_idx(n)]).sum();
            s / n as f64
        })
        .collect();
    boot.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let ci_lo = boot[((iters as f64) * 0.025) as usize] * 100.0;
    let ci_hi = boot[(((iters as f64) * 0.975) as usize).min(iters - 1)] * 100.0;

    // LEG 2 — random-outcome null (skill-less bettor at the same prices)
    let ge2 = (0..iters)
        .filter(|_| {
            let m: f64 = entries.iter().map(|&e| ev1(e, rand::random::<f64>() < 0.5)).sum::<f64>() / n as f64;
            m >= obs
        })
        .count();
    let p2 = ge2 as f64 / iters as f64;

    // LEG 3 — shuffled-outcome null (Fisher-Yates permute of the real win/loss labels)
    let mut wv: Vec<bool> = wons.to_vec();
    let ge3 = (0..iters)
        .filter(|_| {
            for i in (1..n).rev() {
                let j = rand_idx(i + 1); // 0..=i
                wv.swap(i, j);
            }
            let m: f64 = entries.iter().zip(&wv).map(|(&e, &w)| ev1(e, w)).sum::<f64>() / n as f64;
            m >= obs
        })
        .count();
    let p3 = ge3 as f64 / iters as f64;

    // LEG 4 — calibration null: bettor wins each trade with prob = its ENTRY PRICE.
    // Works at constant price (unlike Leg 3): tests whether the selection beats the
    // fair value implied by the price itself.
    let ge4 = (0..iters)
        .filter(|_| {
            let m: f64 = entries.iter()
                .map(|&e| ev1(e, rand::random::<f64>() < e))
                .sum::<f64>() / n as f64;
            m >= obs
        })
        .count();
    let p4 = ge4 as f64 / iters as f64;

    // Is there enough price variance for Leg 3 (shuffle-null) to be meaningful? At
    // (near-)constant price the shuffle is mathematically blind (it always yields
    // p≈1.0 — the quant's valid point). Measure the std of entry prices; below a
    // small threshold, Leg 3 is SKIPPED rather than counted as a failure.
    let mean_e = entries.iter().sum::<f64>() / n as f64;
    let price_std = (entries.iter().map(|&e| (e - mean_e).powi(2)).sum::<f64>() / n as f64).sqrt();
    let leg3_applicable = price_std >= 0.05; // ≥5¢ spread of entry prices

    let l1 = ci_lo > 0.0;
    let l2 = p2 < 0.05;
    let l3 = p3 < 0.05;
    let l4 = p4 < 0.05;
    // EDGE requires Leg1 + Leg2 + Leg4 always; Leg3 only when price has variance.
    let edge = l1 && l2 && l4 && (!leg3_applicable || l3);

    let note = if edge {
        if leg3_applicable {
            "Survives all 4 tests (incl. shuffle + calibration null) — worth a small real pilot.".to_string()
        } else {
            "Constant-price strategy: passes Leg1/2/4 (calibration null); Leg3 skipped (no price \
             variance). Edge is over fair value, but a backtest can't tell signal from lookahead — \
             verify with walk-forward + a no-end-of-window-lookahead run before any capital.".to_string()
        }
    } else {
        "Consistent with luck/fees/lookahead — do NOT commit capital.".to_string()
    };

    ValidationResult {
        n,
        wr_pct: wr,
        break_even_pct: be,
        ev_per_trade_pct: obs * 100.0,
        ci_lo,
        ci_hi,
        leg1_pass: l1,
        p_random: p2,
        leg2_pass: l2,
        p_shuffle: p3,
        leg3_pass: if leg3_applicable { l3 } else { true }, // true = "not blocking"
        p_calib: p4,
        leg4_pass: l4,
        verdict: if edge { "EDGE" } else { "NO_EDGE" }.to_string(),
        note,
    }
}
