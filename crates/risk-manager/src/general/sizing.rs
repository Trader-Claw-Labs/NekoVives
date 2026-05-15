//! ATR-based position sizing helpers.

/// Compute risk-adjusted position size.
///
/// Formula: `size = (capital * risk_per_trade) / (stop_distance_atr * atr14)`
///
/// # Arguments
/// * `capital` — total account capital in USD
/// * `risk_per_trade` — fraction of capital to risk (e.g. 0.01 = 1%)
/// * `stop_distance_atr` — stop distance expressed as multiple of ATR
/// * `atr14` — current 14-period ATR value
/// * `min_notional` — floor below which the trade is rejected
///
/// Returns `None` if the computed size is below `min_notional`.
pub fn size_by_atr(
    capital: f64,
    risk_per_trade: f64,
    stop_distance_atr: f64,
    atr14: f64,
    min_notional: f64,
) -> Option<f64> {
    if capital <= 0.0 || risk_per_trade <= 0.0 || atr14 <= 0.0 {
        return None;
    }
    let stop_distance = stop_distance_atr * atr14;
    if stop_distance <= 0.0 {
        return None;
    }
    let size = (capital * risk_per_trade) / stop_distance;
    if size < min_notional {
        return None;
    }
    Some(size)
}

/// Apply a drawdown-based size reduction.
///
/// If drawdown > soft_limit, halve the size.
/// If drawdown > hard_limit, reject (return 0).
pub fn apply_drawdown_adjustment(
    size: f64,
    drawdown_pct: f64,
    soft_limit: f64,
    hard_limit: f64,
) -> f64 {
    if drawdown_pct >= hard_limit {
        return 0.0;
    }
    if drawdown_pct >= soft_limit {
        return size * 0.5;
    }
    size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_by_atr_basic() {
        // capital $100k, risk 1%, stop = 2x ATR, ATR = $100
        // size = (100_000 * 0.01) / (2.0 * 100) = 1000 / 200 = 5.0
        assert_eq!(
            size_by_atr(100_000.0, 0.01, 2.0, 100.0, 4.0),
            Some(5.0)
        );
    }

    #[test]
    fn test_size_by_atr_below_min() {
        // computed size = 5.0, min_notional = 10.0 → rejected
        assert_eq!(
            size_by_atr(100_000.0, 0.01, 2.0, 100.0, 10.0),
            None
        );
        // computed size = 5.0, min_notional = 4.0 → accepted
        assert_eq!(
            size_by_atr(100_000.0, 0.01, 2.0, 100.0, 4.0),
            Some(5.0)
        );
    }

    #[test]
    fn test_drawdown_adjustment() {
        assert_eq!(apply_drawdown_adjustment(100.0, 0.10, 0.15, 0.20), 100.0);
        assert_eq!(apply_drawdown_adjustment(100.0, 0.16, 0.15, 0.20), 50.0);
        assert_eq!(apply_drawdown_adjustment(100.0, 0.21, 0.15, 0.20), 0.0);
    }
}
