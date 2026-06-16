//! Engine adapters — bridge between `strategy-core` trait and the concrete
//! implementations living in this crate (or called via existing tools).

pub mod arb_binary;
pub mod arb_hedge;
pub mod fair_value;
pub mod fv_momentum;
pub mod minting_mm;
pub mod rewards_maker;
pub mod rewards_orchestrator;
pub mod rhai_candle;
pub mod rotation_compounder;
pub mod series_helper;
