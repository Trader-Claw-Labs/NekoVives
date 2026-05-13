//! strategy-core — Abstract contract for all Neko Vives trading engines.
//!
//! Every engine (rhai_candle, arb_binary, minting_mm, …) implements
//! [`StrategyEngine`].  The runner dispatches to the correct impl based on
//! `RunnerConfig.kind`.  This crate has zero knowledge of Rhai, CLOB, or
//! Polymarket — it only defines types and the trait.

pub mod types;
pub mod engine;
pub mod engines;

pub use types::*;
pub use engine::StrategyEngine;
