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
    pub verdict: String, // "EDGE" | "NO_EDGE" | "INSUFFICIENT"
    pub note: String,
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

    let l1 = ci_lo > 0.0;
    let l2 = p2 < 0.05;
    let l3 = p3 < 0.05;
    let edge = l1 && l2 && l3;

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
        leg3_pass: l3,
        verdict: if edge { "EDGE" } else { "NO_EDGE" }.to_string(),
        note: if edge {
            "Survives all 3 independent tests — worth a small real pilot.".to_string()
        } else {
            "Consistent with luck/fees — do NOT commit capital.".to_string()
        },
    }
}
