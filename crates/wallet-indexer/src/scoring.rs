//! Wallet scoring engine implementing the WalletScore formula.

use serde::{Deserialize, Serialize};

/// Input metrics for a single wallet.
#[derive(Debug, Clone, Default)]
pub struct WalletMetrics {
    pub pnl_90d: f64,
    pub winrate: f64,
    pub trades_90d: i64,
    pub max_drawdown_pct: f64,
    pub sharpe_ratio: f64,
    pub cv_monthly_pnl: f64,
    pub unique_tickers: i64,
    pub capital_usd: f64,
}

/// Computed scores for a wallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletScore {
    pub address: String,
    pub venue: String,
    pub category: Option<String>,
    pub pnl_norm: f64,
    pub winrate_score: f64,
    pub drawdown_score: f64,
    pub sharpe_score: f64,
    pub consistency_score: f64,
    pub diversity_score: f64,
    pub total_score: f64,
}

/// Score a single wallet given its raw metrics and percentile context.
pub fn score_wallet(
    address: String,
    venue: String,
    category: Option<String>,
    metrics: &WalletMetrics,
    pnl_percentile: f64,
) -> WalletScore {
    // Hard filters already applied upstream; compute sub-scores here.
    let pnl_norm = pnl_percentile;

    let winrate_score = if metrics.trades_90d >= 50 {
        (metrics.winrate / 0.55).min(1.0)
    } else {
        0.0
    };

    let drawdown_score = 1.0 - (metrics.max_drawdown_pct / 0.40).min(1.0);

    let sharpe_score = (metrics.sharpe_ratio / 2.0).min(1.0);

    let consistency_score = (1.0 - metrics.cv_monthly_pnl).max(0.0);

    let diversity_score = (metrics.unique_tickers as f64 / 20.0).min(1.0);

    // Sub-scores are all normalized to 0–1; the weighted sum is therefore 0–1.
    // The rest of the system (dispatcher thresholds <65/>=80, RiskGate
    // min_leader_score_to_mirror=80, UI) works on a 0–100 scale, so scale up by
    // 100 here. Without this, every auto-scored wallet lands near ~50 at most and
    // was silently dropped as ScoreTooLow.
    let total_score_0_1 = 0.30 * pnl_norm
        + 0.20 * winrate_score
        + 0.15 * drawdown_score
        + 0.15 * sharpe_score
        + 0.10 * consistency_score
        + 0.10 * diversity_score;

    WalletScore {
        address,
        venue,
        category,
        pnl_norm,
        winrate_score,
        drawdown_score,
        sharpe_score,
        consistency_score,
        diversity_score,
        total_score: (total_score_0_1 * 100.0).clamp(0.0, 100.0),
    }
}

/// Filter out wallets that don't meet hard criteria before scoring.
pub fn passes_hard_filters(metrics: &WalletMetrics) -> bool {
    metrics.trades_90d >= 50
        && metrics.capital_usd >= 50_000.0
        && metrics.max_drawdown_pct <= 0.40
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_wallet() {
        let metrics = WalletMetrics {
            pnl_90d: 10000.0,
            winrate: 0.60,
            trades_90d: 100,
            max_drawdown_pct: 0.20,
            sharpe_ratio: 1.5,
            cv_monthly_pnl: 0.10,
            unique_tickers: 15,
            capital_usd: 100_000.0,
        };
        let score = score_wallet(
            "0xabc".into(),
            "polymarket".into(),
            Some("politics".into()),
            &metrics,
            0.80,
        );
        assert!(score.total_score > 0.0);
        assert!(score.total_score <= 100.0);
    }

    #[test]
    fn test_hard_filters_reject_small_capital() {
        let metrics = WalletMetrics {
            capital_usd: 10_000.0,
            trades_90d: 100,
            ..Default::default()
        };
        assert!(!passes_hard_filters(&metrics));
    }
}
