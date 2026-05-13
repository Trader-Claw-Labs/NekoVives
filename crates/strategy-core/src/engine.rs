//! The `StrategyEngine` trait — the single contract every engine must fulfil.

use crate::types::{
    EngineError, EngineEvent, EngineMetrics, ExecutionMode, MarketSnapshot, OrderIntent, Portfolio,
};

/// Core trait implemented by every trading engine in Neko Vives.
///
/// # Lifecycle
/// ```text
/// initialize() → [on_tick | on_book]* → on_event* → finalize()
/// ```
///
/// The runner calls `on_tick` for candle-driven engines and `on_book` for
/// order-book-driven engines (arb, minting).  An engine may implement both
/// if it needs hybrid data.
pub trait StrategyEngine: Send + Sync {
    /// Human-readable name (e.g. "arb_binary", "minting_mm", "rhai_candle").
    fn name(&self) -> &str;

    /// Validate config and warm up internal state before the first tick.
    /// Called once by the runner before any market data is fed in.
    fn initialize(
        &mut self,
        mode: ExecutionMode,
        portfolio: &Portfolio,
    ) -> impl std::future::Future<Output = Result<(), EngineError>> + Send;

    /// Called on every new closed candle (candle-driven engines).
    /// Returns zero or more order intents.
    fn on_tick(
        &mut self,
        snap: &MarketSnapshot,
        portfolio: &mut Portfolio,
    ) -> impl std::future::Future<Output = Result<Vec<OrderIntent>, EngineError>> + Send;

    /// Called on every order book update (book-driven engines, e.g. arb).
    /// Default: no-op.
    fn on_book(
        &mut self,
        snap: &crate::types::BookSnapshot,
        portfolio: &mut Portfolio,
    ) -> impl std::future::Future<Output = Result<Vec<OrderIntent>, EngineError>> + Send {
        let _ = (snap, portfolio);
        async { Ok(vec![]) }
    }

    /// Receives execution feedback (fills, partial fills, resolutions, etc.).
    fn on_event(
        &mut self,
        event: EngineEvent,
        portfolio: &mut Portfolio,
    ) -> impl std::future::Future<Output = Result<(), EngineError>> + Send;

    /// Called once at the end of a backtest run or when a runner is stopped.
    /// Returns final performance metrics.
    fn finalize(
        &mut self,
        portfolio: &Portfolio,
    ) -> impl std::future::Future<Output = EngineMetrics> + Send;
}

/// Trait object-safe wrapper for boxed engines.
/// AFIT (`impl Future`) is not object-safe, so this thin wrapper uses
/// `Box<dyn StrategyEngine>` ergonomics via blanket impl where needed.
/// For now engines are stored as generics; this marker helps type-erase when
/// the runner eventually holds heterogeneous engine collections.
pub trait DynStrategyEngine: Send + Sync {
    fn engine_name(&self) -> &str;
}

impl<T: StrategyEngine> DynStrategyEngine for T {
    fn engine_name(&self) -> &str {
        self.name()
    }
}
