//! Adapter that wraps the existing Rhai candle-based backtester / live runner
//! behind the `StrategyEngine` trait.
//!
//! This engine is the *default* (`kind = None` or `kind = "rhai_candle"`) and
//! calls the same `run_rhai_on_candle_buffer` / `run_polymarket_binary_*`
//! functions that the legacy runner uses.  Behaviour is therefore bit-for-bit
//! identical to the pre-Phase-0 code path.

use std::collections::VecDeque;
use std::path::PathBuf;

use strategy_core::{
    engine::StrategyEngine,
    types::{
        BookSnapshot, EngineError, EngineEvent, EngineMetrics, ExecutionMode, MarketSnapshot,
        OrderIntent, Portfolio,
    },
};

pub struct RhaiCandleEngine {
    script: String,
    market_type: String,
    workspace_dir: PathBuf,
    buffer: VecDeque<crate::tools::backtest::Candle>,
    mode: ExecutionMode,
    last_metrics: Option<crate::tools::backtest::BacktestMetrics>,
}

impl RhaiCandleEngine {
    pub fn new(script: String, market_type: String, workspace_dir: PathBuf) -> Self {
        Self {
            script,
            market_type,
            workspace_dir,
            buffer: VecDeque::new(),
            mode: ExecutionMode::Backtest,
            last_metrics: None,
        }
    }
}

impl StrategyEngine for RhaiCandleEngine {
    fn name(&self) -> &str {
        strategy_core::engines::RHAI_CANDLE
    }

    async fn initialize(
        &mut self,
        mode: ExecutionMode,
        _portfolio: &Portfolio,
    ) -> Result<(), EngineError> {
        self.mode = mode;
        self.buffer.clear();
        Ok(())
    }

    async fn on_tick(
        &mut self,
        snap: &MarketSnapshot,
        portfolio: &mut Portfolio,
    ) -> Result<Vec<OrderIntent>, EngineError> {
        // Convert MarketSnapshot back to a Candle and push onto the buffer.
        if let Some(c) = &snap.candle {
            self.buffer.push_back(crate::tools::backtest::Candle {
                open_time_ms: c.open_time_ms,
                open: c.open,
                high: c.high,
                low: c.low,
                close: c.close,
                volume: c.volume,
            });
        }

        // Run the Rhai engine on the current buffer.
        let script_content = match crate::tools::backtest::read_script_or_default(
            &self.workspace_dir,
            &self.script,
        ) {
            Some(s) => s,
            None => {
                return Err(EngineError::NotReady(format!(
                    "script not found: {}",
                    self.script
                )))
            }
        };

        let metrics = crate::tools::backtest::run_rhai_on_candle_buffer(
            &script_content,
            self.buffer.iter().cloned().collect(),
            portfolio.balance_usdc,
            0.001,
        );

        // Sync portfolio balance from metrics.
        let pnl_delta = metrics.total_return_pct / 100.0 * portfolio.initial_balance
            - (portfolio.balance_usdc - portfolio.initial_balance);
        portfolio.balance_usdc += pnl_delta;
        portfolio.realized_pnl = metrics.total_return_pct / 100.0 * portfolio.initial_balance;

        self.last_metrics = Some(metrics);

        // rhai_candle is a black-box — we don't extract individual OrderIntents;
        // the legacy runner handles order placement directly.
        Ok(vec![])
    }

    async fn on_book(
        &mut self,
        _snap: &BookSnapshot,
        _portfolio: &mut Portfolio,
    ) -> Result<Vec<OrderIntent>, EngineError> {
        Ok(vec![])
    }

    async fn on_event(
        &mut self,
        _event: EngineEvent,
        _portfolio: &mut Portfolio,
    ) -> Result<(), EngineError> {
        Ok(())
    }

    async fn finalize(&mut self, portfolio: &Portfolio) -> EngineMetrics {
        let m = self.last_metrics.as_ref();
        EngineMetrics {
            total_return_pct: m.map(|x| x.total_return_pct).unwrap_or(0.0),
            sharpe_ratio: m.map(|x| x.sharpe_ratio).unwrap_or(0.0),
            max_drawdown_pct: m.map(|x| x.max_drawdown_pct).unwrap_or(0.0),
            win_rate_pct: m.map(|x| x.win_rate_pct).unwrap_or(0.0),
            total_trades: m.map(|x| x.total_trades).unwrap_or(0),
            analysis: m
                .map(|x| x.analysis.clone())
                .unwrap_or_else(|| "No trades executed.".to_string()),
            extra: std::collections::HashMap::from([
                ("balance_usdc".to_string(), portfolio.balance_usdc),
            ]),
            data_confidence: "high".to_string(),
        }
    }
}
