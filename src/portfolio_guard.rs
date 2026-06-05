//! Global portfolio guard — stops all live runners when wallet drops >X% from baseline.

use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PortfolioGuardConfig {
    /// Stop all live runners if wallet drops this fraction from the recorded baseline.
    /// 0.5 = stop when down 50%. 0.0 = disabled.
    pub max_loss_pct: f64,
    /// USDC balance when this guard was first activated (set on first live trade).
    pub baseline_usdc: f64,
}

pub struct PortfolioGuard {
    config: Arc<RwLock<PortfolioGuardConfig>>,
}

impl PortfolioGuard {
    pub fn new(max_loss_pct: f64) -> Self {
        Self {
            config: Arc::new(RwLock::new(PortfolioGuardConfig {
                max_loss_pct,
                baseline_usdc: 0.0,
            })),
        }
    }

    pub fn set_baseline(&self, usdc: f64) {
        let mut cfg = self.config.write().unwrap();
        if cfg.baseline_usdc <= 0.0 {
            cfg.baseline_usdc = usdc;
            tracing::info!("[PORTFOLIO_GUARD] Baseline set: ${:.2}", usdc);
        }
    }

    /// Check if current balance breaches the loss threshold.
    /// Returns true if all live runners should be stopped.
    pub fn check(&self, current_usdc: f64) -> bool {
        let cfg = self.config.read().unwrap();
        if cfg.max_loss_pct <= 0.0 || cfg.baseline_usdc <= 0.0 || current_usdc <= 0.0 {
            return false;
        }
        let loss_pct = (cfg.baseline_usdc - current_usdc) / cfg.baseline_usdc;
        if loss_pct >= cfg.max_loss_pct {
            tracing::error!(
                "[PORTFOLIO_GUARD] BREACH: wallet=${:.2} down {:.1}% from baseline=${:.2} (threshold={:.0}%)",
                current_usdc, loss_pct * 100.0, cfg.baseline_usdc, cfg.max_loss_pct * 100.0
            );
            return true;
        }
        false
    }

    pub fn status(&self) -> PortfolioGuardConfig {
        self.config.read().unwrap().clone()
    }
}
