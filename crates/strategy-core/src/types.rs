//! Shared value types used across all strategy engines.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Execution mode ────────────────────────────────────────────────────────────

/// Which execution environment the engine is running in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Replay historical data.  No orders are placed.
    Backtest,
    /// Use live market data but place no real orders; track simulated P&L.
    DryRun,
    /// Place real orders on the exchange/CLOB.
    Live,
}

impl ExecutionMode {
    pub fn is_live(&self) -> bool {
        matches!(self, ExecutionMode::Live)
    }
    pub fn places_real_orders(&self) -> bool {
        matches!(self, ExecutionMode::Live)
    }
}

impl std::fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionMode::Backtest => write!(f, "backtest"),
            ExecutionMode::DryRun => write!(f, "dryrun"),
            ExecutionMode::Live => write!(f, "live"),
        }
    }
}

// ── Market snapshot ───────────────────────────────────────────────────────────

/// A single OHLCV candle — mirrors the existing `backtest::Candle`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandleSnap {
    pub open_time_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Top-of-book quote for a single token (YES or NO).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookLevel {
    /// Best ask (lowest price someone is willing to sell).
    pub best_ask: f64,
    /// Best bid (highest price someone is willing to buy).
    pub best_bid: f64,
    /// Liquidity available at best ask (USD).
    pub ask_depth_usd: f64,
    /// Liquidity available at best bid (USD).
    pub bid_depth_usd: f64,
}

/// Combined snapshot of both sides of a binary market.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookSnapshot {
    pub market_id: String,
    pub slug: String,
    pub yes: BookLevel,
    pub no: BookLevel,
    pub timestamp: DateTime<Utc>,
    /// Extra key/value indicators from the engine context.
    #[serde(default)]
    pub meta: HashMap<String, f64>,
}

impl BookSnapshot {
    /// Sum of best asks for YES and NO.
    pub fn ask_sum(&self) -> f64 {
        self.yes.best_ask + self.no.best_ask
    }
    /// Mid-price of YES token.
    pub fn yes_mid(&self) -> f64 {
        (self.yes.best_ask + self.yes.best_bid) / 2.0
    }
    /// Mid-price of NO token.
    pub fn no_mid(&self) -> f64 {
        (self.no.best_ask + self.no.best_bid) / 2.0
    }
    /// Edge available for binary arbitrage (1.0 - ask_sum - fee).
    pub fn arb_edge(&self, fee_pct: f64) -> f64 {
        1.0 - self.ask_sum() - fee_pct
    }
}

/// Generic market snapshot that engines receive on every tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSnapshot {
    pub market_id: String,
    pub slug: String,
    pub candle: Option<CandleSnap>,
    pub book: Option<BookSnapshot>,
    pub timestamp: DateTime<Utc>,
}

// ── Portfolio ─────────────────────────────────────────────────────────────────

/// Engine-local portfolio state (updated by the runner on every fill).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Portfolio {
    /// Available USDC balance.
    pub balance_usdc: f64,
    /// Initial USDC deposited (used for return calculations).
    pub initial_balance: f64,
    /// Open positions keyed by `token_id`.
    pub positions: HashMap<String, Position>,
    /// Cumulative realised P&L.
    pub realized_pnl: f64,
}

impl Portfolio {
    pub fn new(initial: f64) -> Self {
        Self {
            balance_usdc: initial,
            initial_balance: initial,
            positions: HashMap::new(),
            realized_pnl: 0.0,
        }
    }

    pub fn total_return_pct(&self) -> f64 {
        if self.initial_balance == 0.0 {
            return 0.0;
        }
        (self.balance_usdc + self.unrealized_value() - self.initial_balance)
            / self.initial_balance
            * 100.0
    }

    pub fn unrealized_value(&self) -> f64 {
        self.positions.values().map(|p| p.unrealized_pnl).sum()
    }
}

/// A single open position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub token_id: String,
    pub side: Side,
    pub size: f64,
    pub entry_price: f64,
    pub current_price: f64,
    pub unrealized_pnl: f64,
    pub opened_at: DateTime<Utc>,
}

// ── Order intent ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Yes,
    No,
}

/// An engine's request to place, cancel, or mint/merge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OrderIntent {
    /// Buy `size_usd` worth of `token_id` at up to `limit_price`.
    Buy {
        token_id: String,
        side: Side,
        size_usd: f64,
        /// None = market order.
        limit_price: Option<f64>,
    },
    /// Sell `size_usd` worth of `token_id`.
    Sell {
        token_id: String,
        side: Side,
        size_usd: f64,
        limit_price: Option<f64>,
    },
    /// Mint YES+NO from USDC via CTF contract.
    Mint {
        market_slug: String,
        amount_usdc: f64,
        collateral: String,
    },
    /// Merge YES+NO back to USDC.
    Merge {
        market_slug: String,
        amount_usdc: f64,
        collateral: String,
    },
    /// Cancel a pending order.
    Cancel { order_id: String },
    /// No action this tick.
    Hold,
}

// ── Engine events ─────────────────────────────────────────────────────────────

/// Events that flows back from the runner to the engine after execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineEvent {
    OrderFilled {
        order_id: String,
        token_id: String,
        side: Side,
        price: f64,
        size: f64,
        fee: f64,
        timestamp: DateTime<Utc>,
    },
    OrderCancelled {
        order_id: String,
    },
    PartialFill {
        order_id: String,
        token_id: String,
        side: Side,
        filled_size: f64,
        remaining_size: f64,
    },
    MarketResolved {
        market_id: String,
        winning_side: Side,
        timestamp: DateTime<Utc>,
    },
    /// Mint confirmed on-chain.
    MintConfirmed {
        condition_id: String,
        amount_usdc: f64,
        market_slug: String,
    },
    /// Merge confirmed on-chain.
    MergeConfirmed {
        condition_id: String,
        amount_usdc: f64,
        market_slug: String,
    },
}

// ── Metrics ───────────────────────────────────────────────────────────────────

/// Standard performance metrics all engines must produce.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EngineMetrics {
    pub total_return_pct: f64,
    pub sharpe_ratio: f64,
    pub max_drawdown_pct: f64,
    pub win_rate_pct: f64,
    pub total_trades: u32,
    pub analysis: String,
    /// Engine-specific extra values.
    #[serde(default)]
    pub extra: HashMap<String, f64>,
    /// Confidence in backtest data: "high" | "medium" | "low"
    #[serde(default)]
    pub data_confidence: String,
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("insufficient balance: need {need:.2} USDC, have {have:.2}")]
    InsufficientBalance { need: f64, have: f64 },

    #[error("market not found: {0}")]
    MarketNotFound(String),

    #[error("execution error: {0}")]
    Execution(#[from] anyhow::Error),

    #[error("partial fill: {order_id} — only {filled}/{total} filled")]
    PartialFill { order_id: String, filled: f64, total: f64 },

    #[error("engine not ready: {0}")]
    NotReady(String),
}
