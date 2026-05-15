//! Additional risk rules that can be composed into the RiskGate.
//!
//! This module provides helpers for computing exposure, drawdown,
//! and correlated-risk checks that are too specific for the core gate.

use std::collections::HashMap;

/// Compute the total notional exposure for a set of positions grouped by a key.
pub fn aggregate_exposure<T: std::hash::Hash + Eq + Clone>(
    positions: &[(T, f64)],
) -> HashMap<T, f64> {
    let mut map = HashMap::new();
    for (key, notional) in positions {
        *map.entry(key.clone()).or_insert(0.0) += notional;
    }
    map
}

/// Check if adding `new_notional` to an existing group would exceed `max_pct` of `capital`.
pub fn would_exceed_cap(
    current: f64,
    new_notional: f64,
    capital: f64,
    max_pct: f64,
) -> bool {
    if capital <= 0.0 {
        return true;
    }
    (current + new_notional) / capital > max_pct
}

/// Simple drawdown check: reject if current portfolio drawdown from peak exceeds threshold.
pub fn check_drawdown(current_value: f64, peak_value: f64, max_drawdown_pct: f64) -> bool {
    if peak_value <= 0.0 {
        return false;
    }
    let dd = (peak_value - current_value) / peak_value;
    dd > max_drawdown_pct
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_would_exceed_cap() {
        assert!(would_exceed_cap(8000.0, 3000.0, 100_000.0, 0.10));
        assert!(!would_exceed_cap(5000.0, 1000.0, 100_000.0, 0.10));
    }

    #[test]
    fn test_check_drawdown() {
        assert!(check_drawdown(80_000.0, 100_000.0, 0.15));
        assert!(!check_drawdown(95_000.0, 100_000.0, 0.15));
    }
}
