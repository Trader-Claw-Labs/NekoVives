//! General trading risk management module.
//!
//! Provides `TradingRiskGate` — a centralized risk gate for all non-copy-trading
//! order flows. Enforces: daily loss limit, drawdown gates, ATR sizing,
//! correlation caps, per-strategy caps, memecoin caps, and manual kill-switch.

pub mod correlation;
pub mod gate;
pub mod sizing;
pub mod state;

pub use gate::{
    ApprovedOrder, GateStatus, OrderContext, OrderRequest, RiskRejection, TradingRiskConfig,
    TradingRiskGate,
};
pub use sizing::{apply_drawdown_adjustment, size_by_atr};
pub use state::{FillRecord, PortfolioState, PositionRecord};
