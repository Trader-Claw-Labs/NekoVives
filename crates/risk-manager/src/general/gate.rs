//! General trading risk gate.

use crate::general::correlation::CorrelationMatrix;
use crate::general::sizing;
use crate::general::state::{FillRecord, PortfolioState};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for the general trading risk gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingRiskConfig {
    pub daily_loss_limit_pct: f64,
    pub drawdown_hard_limit_pct: f64,
    pub drawdown_soft_limit_pct: f64,
    pub risk_per_trade_pct: f64,
    pub correlation_threshold: f64,
    pub max_correlated_exposure_pct: f64,
    pub max_strategy_exposure_pct: f64,
    pub max_memecoin_exposure_pct: f64,
    pub min_notional_usd: f64,
}

impl Default for TradingRiskConfig {
    fn default() -> Self {
        Self {
            daily_loss_limit_pct: 0.04,
            drawdown_hard_limit_pct: 0.20,
            drawdown_soft_limit_pct: 0.15,
            risk_per_trade_pct: 0.01,
            correlation_threshold: 0.80,
            max_correlated_exposure_pct: 0.40,
            max_strategy_exposure_pct: 0.25,
            max_memecoin_exposure_pct: 0.15,
            min_notional_usd: 10.0,
        }
    }
}

/// An order request to be evaluated.
#[derive(Debug, Clone)]
pub struct OrderRequest {
    pub symbol: String,
    pub strategy_id: String,
    pub side: String,
    pub proposed_size_usd: f64,
    pub stop_distance_atr: f64,
    pub atr14: f64,
    pub is_memecoin: bool,
}

/// Context for evaluating an order.
#[derive(Debug, Clone)]
pub struct OrderContext {
    pub current_price: f64,
}

/// An approved order with potentially adjusted size.
#[derive(Debug, Clone)]
pub struct ApprovedOrder {
    pub symbol: String,
    pub strategy_id: String,
    pub side: String,
    pub approved_size_usd: f64,
    pub is_memecoin: bool,
}

/// Reasons why an order can be rejected.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RiskRejection {
    #[error("daily loss limit reached")]
    DailyLossLimit,
    #[error("drawdown gate triggered")]
    DrawdownLimit,
    #[error("correlation cap exceeded")]
    CorrelationLimit,
    #[error("strategy cap exceeded: {0}")]
    StrategyCap(String),
    #[error("memecoin cap exceeded")]
    MemecoinCap,
    #[error("size below minimum notional")]
    BelowMinNotional,
    #[error("system halted (manual)")]
    Halted,
}

/// Centralized risk gate for general trading.
#[derive(Debug)]
pub struct TradingRiskGate {
    config: TradingRiskConfig,
    state: parking_lot::RwLock<PortfolioState>,
    correlation: parking_lot::Mutex<CorrelationMatrix>,
}

impl TradingRiskGate {
    pub fn new(config: TradingRiskConfig, capital: f64) -> Self {
        Self {
            config,
            state: parking_lot::RwLock::new(PortfolioState::new(capital)),
            correlation: parking_lot::Mutex::new(CorrelationMatrix::with_crypto_defaults()),
        }
    }

    pub fn with_defaults(capital: f64) -> Self {
        Self::new(TradingRiskConfig::default(), capital)
    }

    // ── Kill-switch ────────────────────────────────────────────────────

    pub fn halt_all(&self) {
        self.state.write().halted = true;
        tracing::warn!("[risk] SYSTEM HALTED manually");
    }

    pub fn resume_all(&self) {
        self.state.write().halted = false;
        tracing::info!("[risk] SYSTEM RESUMED");
    }

    pub fn is_halted(&self) -> bool {
        self.state.read().halted
    }

    // ── State updates ──────────────────────────────────────────────────

    pub fn record_fill(&self, fill: &FillRecord) {
        let mut state = self.state.write();
        state.record_fill(fill);
    }

    pub fn update_market(&self, prices: &HashMap<String, f64>) {
        let mut state = self.state.write();
        let equity = prices
            .iter()
            .map(|(sym, price)| {
                let pos_value: f64 = state
                    .positions
                    .iter()
                    .filter(|p| &p.symbol == sym)
                    .map(|p| p.size_usd * price / p.entry_price)
                    .sum();
                pos_value
            })
            .sum::<f64>()
            + state.total_capital;
        state.update_equity(equity);
    }

    pub fn set_correlation(&self, a: &str, b: &str, corr: f64) {
        self.correlation.lock().set(a, b, corr);
    }

    // ── Order approval ─────────────────────────────────────────────────

    pub fn approve_order(
        &self,
        order: &OrderRequest,
        _ctx: &OrderContext,
    ) -> Result<ApprovedOrder, RiskRejection> {
        let mut state = self.state.write();
        state.reset_daily_if_needed();

        // 1. Halted
        if state.halted {
            return Err(RiskRejection::Halted);
        }

        // 2. Daily loss limit
        if state.daily_pnl_pct < -self.config.daily_loss_limit_pct {
            state.halted = true;
            tracing::warn!(
                "[risk] daily loss limit triggered: {:.2}%",
                state.daily_pnl_pct * 100.0
            );
            return Err(RiskRejection::DailyLossLimit);
        }

        // 3. Drawdown hard limit
        if state.drawdown_pct > self.config.drawdown_hard_limit_pct {
            state.halted = true;
            tracing::warn!(
                "[risk] drawdown hard limit triggered: {:.2}%",
                state.drawdown_pct * 100.0
            );
            return Err(RiskRejection::DrawdownLimit);
        }

        // 4. ATR-based sizing
        let sized = sizing::size_by_atr(
            state.total_capital,
            self.config.risk_per_trade_pct,
            order.stop_distance_atr,
            order.atr14,
            self.config.min_notional_usd,
        )
        .ok_or(RiskRejection::BelowMinNotional)?;

        // 5. Drawdown soft limit → halve size
        let approved_size = sizing::apply_drawdown_adjustment(
            sized,
            state.drawdown_pct,
            self.config.drawdown_soft_limit_pct,
            self.config.drawdown_hard_limit_pct,
        );
        if approved_size <= 0.0 {
            return Err(RiskRejection::DrawdownLimit);
        }

        // Cap at proposed size (don't oversize)
        let approved_size = approved_size.min(order.proposed_size_usd);

        // 6. Correlation limit
        let new_pct = state.correlated_exposure_pct(&order.symbol, self.config.correlation_threshold)
            + (approved_size / state.total_capital);
        if new_pct > self.config.max_correlated_exposure_pct {
            return Err(RiskRejection::CorrelationLimit);
        }

        // 7. Per-strategy cap
        let strategy_pct = state.strategy_exposure_pct(&order.strategy_id)
            + (approved_size / state.total_capital);
        if strategy_pct > self.config.max_strategy_exposure_pct {
            return Err(RiskRejection::StrategyCap(order.strategy_id.clone()));
        }

        // 8. Memecoin cap
        if order.is_memecoin {
            let mem_pct = state.memecoin_exposure_pct()
                + (approved_size / state.total_capital);
            if mem_pct > self.config.max_memecoin_exposure_pct {
                return Err(RiskRejection::MemecoinCap);
            }
        }

        tracing::info!(
            "[risk] approved order {} {} {:.2} USD (strategy: {})",
            order.side,
            order.symbol,
            approved_size,
            order.strategy_id
        );

        Ok(ApprovedOrder {
            symbol: order.symbol.clone(),
            strategy_id: order.strategy_id.clone(),
            side: order.side.clone(),
            approved_size_usd: approved_size,
            is_memecoin: order.is_memecoin,
        })
    }

    // ── Queries ────────────────────────────────────────────────────────

    pub fn status(&self) -> GateStatus {
        let state = self.state.read();
        GateStatus {
            daily_pnl_pct: state.daily_pnl_pct,
            drawdown_pct: state.drawdown_pct,
            halted: state.halted,
            total_positions: state.positions.len(),
            total_capital: state.total_capital,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GateStatus {
    pub daily_pnl_pct: f64,
    pub drawdown_pct: f64,
    pub halted: bool,
    pub total_positions: usize,
    pub total_capital: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::general::state::PositionRecord;

    #[test]
    fn test_halt_prevents_orders() {
        let gate = TradingRiskGate::with_defaults(100_000.0);
        gate.halt_all();
        assert!(gate.is_halted());

        let order = OrderRequest {
            symbol: "BTC".into(),
            strategy_id: "breakout".into(),
            side: "buy".into(),
            proposed_size_usd: 1000.0,
            stop_distance_atr: 2.0,
            atr14: 100.0,
            is_memecoin: false,
        };
        let ctx = OrderContext { current_price: 50_000.0 };

        assert!(matches!(
            gate.approve_order(&order, &ctx),
            Err(RiskRejection::Halted)
        ));
    }

    #[test]
    fn test_daily_loss_limit() {
        let gate = TradingRiskGate::with_defaults(100_000.0);
        gate.record_fill(&FillRecord {
            symbol: "BTC".into(),
            strategy_id: "breakout".into(),
            side: "sell".into(),
            size_usd: 5000.0,
            price: 50_000.0,
            pnl_realized: -5000.0,
            is_memecoin: false,
            timestamp: chrono::Utc::now(),
        });

        let order = OrderRequest {
            symbol: "BTC".into(),
            strategy_id: "breakout".into(),
            side: "buy".into(),
            proposed_size_usd: 1000.0,
            stop_distance_atr: 2.0,
            atr14: 100.0,
            is_memecoin: false,
        };
        let ctx = OrderContext { current_price: 50_000.0 };

        assert!(matches!(
            gate.approve_order(&order, &ctx),
            Err(RiskRejection::DailyLossLimit)
        ));
    }

    #[test]
    fn test_strategy_cap() {
        let gate = TradingRiskGate::with_defaults(100_000.0);
        // Seed a position that pushes strategy exposure near the cap
        {
            let mut state = gate.state.write();
            state.add_position(PositionRecord {
                symbol: "BTC".into(),
                strategy_id: "breakout".into(),
                side: "long".into(),
                size_usd: 26_000.0,
                entry_price: 50_000.0,
                opened_at: chrono::Utc::now(),
                is_memecoin: false,
            });
        }

        let order = OrderRequest {
            symbol: "ETH".into(),
            strategy_id: "breakout".into(),
            side: "buy".into(),
            proposed_size_usd: 5000.0,
            stop_distance_atr: 2.0,
            atr14: 10.0,
            is_memecoin: false,
        };
        let ctx = OrderContext { current_price: 3000.0 };

        assert!(matches!(
            gate.approve_order(&order, &ctx),
            Err(RiskRejection::StrategyCap(_))
        ));
    }

    #[test]
    fn test_approve_valid_order() {
        let gate = TradingRiskGate::with_defaults(100_000.0);
        let order = OrderRequest {
            symbol: "BTC".into(),
            strategy_id: "breakout".into(),
            side: "buy".into(),
            proposed_size_usd: 1000.0,
            stop_distance_atr: 2.0,
            atr14: 10.0,
            is_memecoin: false,
        };
        let ctx = OrderContext { current_price: 50_000.0 };

        let approved = gate.approve_order(&order, &ctx).unwrap();
        assert_eq!(approved.symbol, "BTC");
        assert!(approved.approved_size_usd > 0.0);
    }
}
