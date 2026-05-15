//! Rolling correlation matrix for correlated-exposure checks.
//!
//! This is a simplified implementation that accepts pre-computed correlations.
//! In production, the matrix would be updated from historical price returns.

use std::collections::HashMap;

/// A correlation matrix keyed by (symbol_a, symbol_b) where a < b lexicographically.
#[derive(Debug, Clone, Default)]
pub struct CorrelationMatrix {
    data: HashMap<(String, String), f64>,
}

impl CorrelationMatrix {
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    pub fn set(&mut self, a: &str, b: &str, correlation: f64) {
        let key = if a < b {
            (a.to_string(), b.to_string())
        } else {
            (b.to_string(), a.to_string())
        };
        self.data.insert(key, correlation.clamp(-1.0, 1.0));
    }

    pub fn get(&self, a: &str, b: &str) -> Option<f64> {
        let key = if a < b {
            (a.to_string(), b.to_string())
        } else {
            (b.to_string(), a.to_string())
        };
        self.data.get(&key).copied()
    }

    pub fn is_correlated(&self, a: &str, b: &str, threshold: f64) -> bool {
        self.get(a, b).map(|c| c >= threshold).unwrap_or(false)
    }

    /// Bulk load default correlations for common crypto pairs.
    pub fn with_crypto_defaults() -> Self {
        let mut m = Self::new();
        // BTC-ETH strongly correlated in bull markets
        m.set("BTC", "ETH", 0.85);
        m.set("BTC", "SOL", 0.78);
        m.set("ETH", "SOL", 0.80);
        m.set("BTC", "AVAX", 0.72);
        m.set("ETH", "AVAX", 0.75);
        m.set("SOL", "AVAX", 0.70);
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_correlation_symmetric() {
        let mut m = CorrelationMatrix::new();
        m.set("BTC", "ETH", 0.85);
        assert_eq!(m.get("BTC", "ETH"), Some(0.85));
        assert_eq!(m.get("ETH", "BTC"), Some(0.85));
    }

    #[test]
    fn test_correlated_check() {
        let m = CorrelationMatrix::with_crypto_defaults();
        assert!(m.is_correlated("BTC", "ETH", 0.80));
        assert!(!m.is_correlated("BTC", "ETH", 0.90));
    }
}
