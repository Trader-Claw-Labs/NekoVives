//! Portfolio state tracking for the general trading risk gate.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single open position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionRecord {
    pub symbol: String,
    pub strategy_id: String,
    pub side: String,
    pub size_usd: f64,
    pub entry_price: f64,
    pub opened_at: chrono::DateTime<chrono::Utc>,
    pub is_memecoin: bool,
}

/// A recorded fill.
#[derive(Debug, Clone)]
pub struct FillRecord {
    pub symbol: String,
    pub strategy_id: String,
    pub side: String,
    pub size_usd: f64,
    pub price: f64,
    pub pnl_realized: f64,
    pub is_memecoin: bool,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Portfolio state tracked by the risk gate.
#[derive(Debug, Clone)]
pub struct PortfolioState {
    pub total_capital: f64,
    pub peak_equity: f64,
    pub current_equity: f64,
    pub daily_pnl_usd: f64,
    pub daily_pnl_pct: f64,
    pub drawdown_pct: f64,
    pub halted: bool,
    pub positions: Vec<PositionRecord>,
    pub fills_today: Vec<FillRecord>,
    pub last_reset_utc: chrono::NaiveDate,
    pub correlation_matrix: HashMap<(String, String), f64>,
}

impl PortfolioState {
    pub fn new(capital: f64) -> Self {
        Self {
            total_capital: capital,
            peak_equity: capital,
            current_equity: capital,
            daily_pnl_usd: 0.0,
            daily_pnl_pct: 0.0,
            drawdown_pct: 0.0,
            halted: false,
            positions: Vec::new(),
            fills_today: Vec::new(),
            last_reset_utc: chrono::Utc::now().date_naive(),
            correlation_matrix: HashMap::new(),
        }
    }

    pub fn reset_daily_if_needed(&mut self) {
        let today = chrono::Utc::now().date_naive();
        if self.last_reset_utc != today {
            self.daily_pnl_usd = 0.0;
            self.daily_pnl_pct = 0.0;
            self.fills_today.clear();
            self.last_reset_utc = today;
        }
    }

    pub fn update_equity(&mut self, new_equity: f64) {
        self.current_equity = new_equity;
        if new_equity > self.peak_equity {
            self.peak_equity = new_equity;
        }
        if self.peak_equity > 0.0 {
            self.drawdown_pct = (self.peak_equity - new_equity) / self.peak_equity;
        }
        self.daily_pnl_pct = self.daily_pnl_usd / self.total_capital;
    }

    pub fn record_fill(&mut self, fill: &FillRecord) {
        self.reset_daily_if_needed();
        self.daily_pnl_usd += fill.pnl_realized;
        if self.total_capital > 0.0 {
            self.daily_pnl_pct = self.daily_pnl_usd / self.total_capital;
        }
        self.fills_today.push(fill.clone());
    }

    pub fn strategy_exposure_pct(&self, strategy_id: &str) -> f64 {
        let exposure: f64 = self
            .positions
            .iter()
            .filter(|p| p.strategy_id == strategy_id)
            .map(|p| p.size_usd)
            .sum();
        if self.total_capital > 0.0 {
            exposure / self.total_capital
        } else {
            0.0
        }
    }

    pub fn memecoin_exposure_pct(&self) -> f64 {
        let exposure: f64 = self
            .positions
            .iter()
            .filter(|p| p.is_memecoin)
            .map(|p| p.size_usd)
            .sum();
        if self.total_capital > 0.0 {
            exposure / self.total_capital
        } else {
            0.0
        }
    }

    pub fn correlated_exposure_pct(&self, symbol: &str, threshold: f64) -> f64 {
        let mut total = 0.0;
        for pos in &self.positions {
            if pos.symbol == symbol {
                total += pos.size_usd;
                continue;
            }
            let key = if symbol < &pos.symbol {
                (symbol.to_string(), pos.symbol.clone())
            } else {
                (pos.symbol.clone(), symbol.to_string())
            };
            if let Some(&corr) = self.correlation_matrix.get(&key) {
                if corr >= threshold {
                    total += pos.size_usd;
                }
            }
        }
        if self.total_capital > 0.0 {
            total / self.total_capital
        } else {
            0.0
        }
    }

    pub fn add_position(&mut self, pos: PositionRecord) {
        self.positions.push(pos);
    }

    pub fn remove_position(&mut self, symbol: &str, strategy_id: &str) {
        self.positions
            .retain(|p| !(p.symbol == symbol && p.strategy_id == strategy_id));
    }
}
