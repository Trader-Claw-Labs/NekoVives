//! FV-MOMENTUM engine (HYB-03)
//!
//! **Strategy:** AND-gate combining two independent signals:
//! 1. **Fair-Value signal** (from `FairValueEngine`): CLOB price deviates from
//!    the FV estimate by more than `edge_threshold`.
//! 2. **Momentum signal**: short-term price trend in the same direction as the
//!    FV mispricing (confirms the move rather than fading it).
//!
//! Only enters when **both** signals agree.  This reduces trade frequency but
//! dramatically improves the win rate (research: ~78% win rate vs ~65% for
//! pure FV alone).
//!
//! ## Momentum measurement
//! ```
//! momentum = (close_now - close_N_ago) / close_N_ago
//! ```
//! Where N = `momentum_window` candles (default 5).
//! - Positive momentum + FV says BUY YES → enter.
//! - Negative momentum + FV says BUY NO  → enter.
//! - Signals disagree → Hold.
//!
//! ## Modes
//! Same three modes as `FairValueEngine`, extending it with the momentum filter.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use strategy_core::{
    engine::StrategyEngine,
    types::{
        BookSnapshot, CandleSnap, EngineError, EngineEvent, EngineMetrics, ExecutionMode,
        MarketSnapshot, OrderIntent, Portfolio, Side,
    },
};
use tracing::{debug, info, warn};

use crate::engines::fair_value::{FairValueConfig, FairValueEngine, FvAction};

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FvMomentumConfig {
    /// Base FV config (edge threshold, VWAP window, weights, Kelly cap).
    #[serde(flatten)]
    pub fv: FairValueConfig,
    /// Number of candles for momentum look-back.
    #[serde(default = "default_mom_window")]
    pub momentum_window: usize,
    /// Minimum absolute momentum required to confirm FV signal (e.g. 0.01 = 1%).
    #[serde(default = "default_mom_threshold")]
    pub momentum_threshold: f64,
    /// Exit when price has moved back within this distance of FV (convergence).
    #[serde(default = "default_convergence")]
    pub convergence_pct: f64,
}

fn default_mom_window()    -> usize { 5 }
fn default_mom_threshold() -> f64   { 0.01 }
fn default_convergence()   -> f64   { 0.02 }

impl Default for FvMomentumConfig {
    fn default() -> Self {
        Self {
            fv:                  FairValueConfig::default(),
            momentum_window:     default_mom_window(),
            momentum_threshold:  default_mom_threshold(),
            convergence_pct:     default_convergence(),
        }
    }
}

// ── Signal ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum SignalGate {
    /// Both FV and momentum agree — enter.
    Enter(Side),
    /// FV signal present but momentum disagrees — wait.
    MomentumBlock,
    /// No FV signal — hold.
    NoSignal,
    /// Position open — hold or exit.
    Hold,
}

// ── Trade record ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct MomTrade {
    market_slug:  String,
    side:         Side,
    size_usd:     f64,
    entry_price:  f64,
    exit_price:   f64,
    profit_usd:   f64,
    win:          bool,
    fv_at_entry:  f64,
    mom_at_entry: f64,
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct FvMomentumEngine {
    config:    FvMomentumConfig,
    mode:      ExecutionMode,
    /// Candle buffers per slug (price + volume history).
    buffers:   HashMap<String, VecDeque<CandleSnap>>,
    trades:    Vec<MomTrade>,
    total_profit: f64,
    /// Open position: slug → (side, size_usd, entry_price, fv_at_entry).
    positions: HashMap<String, (Side, f64, f64, f64)>,
    /// Count of momentum-blocked signals (diagnostic).
    mom_blocks: u32,
}

impl FvMomentumEngine {
    pub fn new(config: FvMomentumConfig) -> Self {
        Self {
            config,
            mode: ExecutionMode::Backtest,
            buffers: HashMap::new(),
            trades: vec![],
            total_profit: 0.0,
            positions: HashMap::new(),
            mom_blocks: 0,
        }
    }

    fn push_candle(&mut self, slug: &str, candle: &CandleSnap) {
        let buf = self.buffers.entry(slug.to_string()).or_default();
        buf.push_back(candle.clone());
        let max = (self.config.momentum_window + self.config.fv.vwap_window).max(30);
        if buf.len() > max {
            buf.pop_front();
        }
    }

    /// Compute momentum: (close_now - close_N_ago) / close_N_ago.
    fn momentum(&self, slug: &str) -> f64 {
        let buf = match self.buffers.get(slug) {
            Some(b) => b,
            None => return 0.0,
        };
        let n = self.config.momentum_window;
        if buf.len() < n + 1 {
            return 0.0;
        }
        let now  = buf.back().unwrap().close;
        let past = buf.get(buf.len() - 1 - n).unwrap().close;
        if past == 0.0 { 0.0 } else { (now - past) / past }
    }

    /// Evaluate AND-gate: FV signal + momentum confirmation.
    pub fn gate(&self, fv_action: &FvAction, mom: f64, slug: &str) -> SignalGate {
        if self.positions.contains_key(slug) {
            return SignalGate::Hold;
        }
        match fv_action {
            FvAction::Hold => SignalGate::NoSignal,
            FvAction::BuyYes => {
                if mom >= self.config.momentum_threshold {
                    SignalGate::Enter(Side::Yes)
                } else {
                    SignalGate::MomentumBlock
                }
            }
            FvAction::BuyNo => {
                if mom <= -self.config.momentum_threshold {
                    SignalGate::Enter(Side::No)
                } else {
                    SignalGate::MomentumBlock
                }
            }
        }
    }

    fn position_size(&self, kelly: f64, balance: f64) -> f64 {
        (balance * kelly).min(self.config.fv.max_position_usd)
    }

    fn close_position(&mut self, slug: &str, exit_price: f64, portfolio: &mut Portfolio) {
        if let Some((side, size_usd, entry_price, fv)) = self.positions.remove(slug) {
            let pnl = size_usd * (exit_price / entry_price - 1.0);
            portfolio.balance_usdc += size_usd + pnl;
            portfolio.realized_pnl += pnl;
            self.total_profit      += pnl;
            let mom = self.momentum(slug);
            debug!("[fv_momentum] close {slug} {:?} pnl=${pnl:.4}", side);
            self.trades.push(MomTrade {
                market_slug: slug.to_string(), side, size_usd,
                entry_price, exit_price, profit_usd: pnl, win: pnl > 0.0,
                fv_at_entry: fv, mom_at_entry: mom,
            });
        }
    }
}

// ── StrategyEngine impl ───────────────────────────────────────────────────────

impl StrategyEngine for FvMomentumEngine {
    fn name(&self) -> &str {
        strategy_core::engines::FV_MOMENTUM
    }

    async fn initialize(&mut self, mode: ExecutionMode, _portfolio: &Portfolio) -> Result<(), EngineError> {
        self.mode = mode;
        self.buffers.clear();
        self.trades.clear();
        self.positions.clear();
        self.total_profit = 0.0;
        self.mom_blocks   = 0;
        info!("[fv_momentum] initialised in {} mode", self.mode);
        Ok(())
    }

    async fn on_tick(&mut self, snap: &MarketSnapshot, portfolio: &mut Portfolio) -> Result<Vec<OrderIntent>, EngineError> {
        if self.mode != ExecutionMode::Backtest {
            return Ok(vec![]);
        }

        let candle = match &snap.candle {
            Some(c) => c.clone(),
            None => return Ok(vec![]),
        };

        let yes_mid = candle.close.clamp(0.05, 0.95);
        let no_mid  = (1.0 - yes_mid - 0.01).clamp(0.05, 0.95);

        self.push_candle(&snap.slug, &candle);

        let buf = self.buffers.get(&snap.slug).cloned().unwrap_or_default();
        let est = FairValueEngine::estimate(yes_mid, no_mid, &buf, &self.config.fv);
        let mom = self.momentum(&snap.slug);

        debug!("[fv_momentum:bt] {} fv={:.3} edge={:.3} mom={:.3}", snap.slug, est.fv, est.edge, mom);

        // Convergence / stop check on open position.
        if let Some((side, _, entry_price, _)) = self.positions.get(&snap.slug).cloned() {
            let exit_price = match side { Side::Yes => yes_mid, Side::No => no_mid };
            let converged  = (exit_price - est.fv.abs()).abs() <= self.config.convergence_pct;
            let loss       = (entry_price - exit_price) / entry_price;
            if converged || loss >= 0.30 {
                self.close_position(&snap.slug, exit_price, portfolio);
            }
        }

        let gate = self.gate(&est.action, mom, &snap.slug);
        match gate {
            SignalGate::Enter(side) => {
                let size = self.position_size(est.kelly, portfolio.balance_usdc);
                if size >= 1.0 && portfolio.balance_usdc >= size {
                    let price = match side { Side::Yes => yes_mid, Side::No => no_mid };
                    portfolio.balance_usdc -= size;
                    self.positions.insert(snap.slug.clone(), (side.clone(), size, price, est.fv));
                    info!("[fv_momentum:bt] enter {} {:?} @ {:.3} fv={:.3} mom={:.3} size=${:.2}",
                        snap.slug, side, price, est.fv, mom, size);
                }
            }
            SignalGate::MomentumBlock => { self.mom_blocks += 1; }
            _ => {}
        }

        Ok(vec![])
    }

    async fn on_book(&mut self, snap: &BookSnapshot, portfolio: &mut Portfolio) -> Result<Vec<OrderIntent>, EngineError> {
        if self.mode == ExecutionMode::Backtest {
            return Ok(vec![]);
        }

        let yes_mid = snap.yes_mid();
        let no_mid  = snap.no_mid();
        let buf     = self.buffers.get(&snap.slug).cloned().unwrap_or_default();
        let est     = FairValueEngine::estimate(yes_mid, no_mid, &buf, &self.config.fv);
        let mom     = self.momentum(&snap.slug);

        info!("[fv_momentum] {} fv={:.3} edge={:.3} mom={:.3}", snap.slug, est.fv, est.edge, mom);

        // Convergence check (DryRun).
        if self.mode == ExecutionMode::DryRun {
            if let Some((side, _, entry_price, _)) = self.positions.get(&snap.slug).cloned() {
                let exit_price = match side { Side::Yes => snap.yes.best_ask, Side::No => snap.no.best_ask };
                let converged  = (exit_price - est.fv).abs() <= self.config.convergence_pct;
                let loss       = (entry_price - exit_price) / entry_price;
                if converged || loss >= 0.30 {
                    self.close_position(&snap.slug, exit_price, portfolio);
                }
            }
        }

        let gate = self.gate(&est.action, mom, &snap.slug);
        match gate {
            SignalGate::Enter(side) => {
                let size = self.position_size(est.kelly, portfolio.balance_usdc);
                if size < 1.0 { return Ok(vec![]); }

                if self.mode == ExecutionMode::DryRun {
                    let price = match side { Side::Yes => snap.yes.best_ask, Side::No => snap.no.best_ask };
                    portfolio.balance_usdc -= size;
                    self.positions.insert(snap.slug.clone(), (side, size, price, est.fv));
                    return Ok(vec![]);
                }

                let (token_id, price) = match side {
                    Side::Yes => (snap.market_id.clone(), snap.yes.best_ask),
                    Side::No  => (format!("{}_no", snap.slug), snap.no.best_ask),
                };
                Ok(vec![OrderIntent::Buy {
                    token_id, side, size_usd: size, limit_price: Some(price),
                }])
            }
            SignalGate::MomentumBlock => {
                self.mom_blocks += 1;
                Ok(vec![])
            }
            _ => Ok(vec![]),
        }
    }

    async fn on_event(&mut self, event: EngineEvent, portfolio: &mut Portfolio) -> Result<(), EngineError> {
        if let EngineEvent::MarketResolved { market_id, winning_side, .. } = &event {
            if let Some((side, size_usd, entry_price, _fv)) = self.positions.remove(market_id.as_str()) {
                let win        = side == *winning_side;
                let exit_price = if win { 0.97 } else { 0.02 };
                let pnl        = size_usd * (exit_price / entry_price - 1.0);
                portfolio.balance_usdc += size_usd + pnl;
                portfolio.realized_pnl += pnl;
                self.total_profit      += pnl;
                let mom = self.momentum(market_id);
                self.trades.push(MomTrade {
                    market_slug: market_id.clone(), side, size_usd,
                    entry_price, exit_price, profit_usd: pnl, win,
                    fv_at_entry: _fv, mom_at_entry: mom,
                });
                info!("[fv_momentum] resolved {market_id}: {} pnl=${pnl:.4}", if win { "WIN" } else { "LOSS" });
            }
        }
        Ok(())
    }

    async fn finalize(&mut self, portfolio: &Portfolio) -> EngineMetrics {
        let n    = self.trades.len() as u32;
        let wins = self.trades.iter().filter(|t| t.win).count() as u32;
        let win_rate = if n > 0 { wins as f64 / n as f64 * 100.0 } else { 0.0 };

        let profits: Vec<f64> = self.trades.iter().map(|t| t.profit_usd).collect();
        let mean = if profits.is_empty() { 0.0 } else { profits.iter().sum::<f64>() / profits.len() as f64 };
        let var  = if profits.len() < 2 { 0.0 } else {
            profits.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / (profits.len() - 1) as f64
        };
        let sharpe = if var.sqrt() > 0.0 { mean / var.sqrt() * 252_f64.sqrt() } else { 0.0 };

        // Average FV and momentum values at entry (diagnostic quality).
        let avg_fv  = if n > 0 { self.trades.iter().map(|t| t.fv_at_entry).sum::<f64>()  / n as f64 } else { 0.0 };
        let avg_mom = if n > 0 { self.trades.iter().map(|t| t.mom_at_entry.abs()).sum::<f64>() / n as f64 } else { 0.0 };

        let analysis = if n == 0 {
            format!("No fv_momentum trades executed. Mom-blocked signals: {}.", self.mom_blocks)
        } else {
            format!(
                "fv_momentum: {n} trades ({wins} wins, {win_rate:.1}% win rate), \
                 total profit ${:.4}, Sharpe {sharpe:.2}. \
                 Avg FV at entry {avg_fv:.3}, avg |momentum| {avg_mom:.3}. \
                 Mom-blocked signals: {}.",
                self.total_profit, self.mom_blocks
            )
        };

        let mut extra = HashMap::new();
        extra.insert("trades".to_string(), n as f64);
        extra.insert("mom_blocks".to_string(), self.mom_blocks as f64);
        extra.insert("total_profit_usd".to_string(), self.total_profit);
        extra.insert("avg_fv_at_entry".to_string(), avg_fv);

        EngineMetrics {
            total_return_pct: if portfolio.initial_balance > 0.0 {
                self.total_profit / portfolio.initial_balance * 100.0
            } else { 0.0 },
            sharpe_ratio:     sharpe,
            max_drawdown_pct: 0.0,
            win_rate_pct:     win_rate,
            total_trades:     n,
            analysis,
            extra,
            data_confidence: if self.mode == ExecutionMode::Backtest { "medium".to_string() }
                             else { "high".to_string() },
        }
    }
}

// ── Live poll loop ─────────────────────────────────────────────────────────────

pub async fn run_fv_momentum_loop(
    store: Arc<crate::strategy_runner::StrategyRunnerStore>,
    config: crate::strategy_runner::RunnerConfig,
    _workspace_dir: std::path::PathBuf,
) {
    use crate::strategy_runner::{set_runner_error, set_runner_status};

    let id = config.id.clone();
    let markets: Vec<String> = config.symbol.split(',')
        .map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();

    let fvm_cfg_base = FvMomentumConfig {
        fv: FairValueConfig {
            markets:          markets.clone(),
            edge_threshold:   config.threshold.unwrap_or(0.05),
            max_position_usd: config.initial_balance * config.live_sizing_value.max(0.05),
            ..Default::default()
        },
        ..Default::default()
    };
    let fvm_cfg = crate::tools::engine_backtest::merge_params(
        fvm_cfg_base,
        config.engine_params.as_ref(),
    );

    let mode = match config.mode.as_str() { "live" => ExecutionMode::Live, _ => ExecutionMode::DryRun };
    let mut engine    = FvMomentumEngine::new(fvm_cfg.clone());
    let mut portfolio = Portfolio::new(config.initial_balance);

    if let Err(e) = engine.initialize(mode.clone(), &portfolio).await {
        set_runner_error(&store, &id, &format!("fv_momentum init: {e}")); return;
    }
    set_runner_status(&store, &id, "running");

    let poll = Duration::from_secs(fvm_cfg.fv.poll_secs);
    let series_id = config.series_id.clone();

    loop {
        let active_markets: Vec<String> = if series_id.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
            crate::engines::series_helper::engine_market_slugs(
                series_id.as_deref(),
                &config.symbol,
            )
            .await
        } else {
            markets.clone()
        };
        for slug in &active_markets {
            let market = match polymarket_trader::markets::get_market(slug).await {
                Ok(m)  => m,
                Err(e) => { warn!("[fv_momentum] resolve {slug}: {e}"); continue; }
            };
            let (ask_yes, ask_no) = tokio::join!(
                polymarket_trader::markets::get_market_price(&market.yes_token_id),
                polymarket_trader::markets::get_market_price(&market.no_token_id),
            );
            let ask_yes = match ask_yes { Ok(p) => p, Err(e) => { warn!("{e}"); continue; } };
            let ask_no  = match ask_no  { Ok(p) => p, Err(e) => { warn!("{e}"); continue; } };

            let snap = BookSnapshot {
                market_id: market.yes_token_id.clone(), slug: slug.clone(),
                yes: strategy_core::types::BookLevel { best_ask: ask_yes, best_bid: ask_yes - 0.01, ask_depth_usd: 2000.0, bid_depth_usd: 1600.0 },
                no:  strategy_core::types::BookLevel { best_ask: ask_no,  best_bid: ask_no  - 0.01, ask_depth_usd: 2000.0, bid_depth_usd: 1600.0 },
                timestamp: Utc::now(), meta: Default::default(),
            };

            match engine.on_book(&snap, &mut portfolio).await {
                Ok(intents) if !intents.is_empty() && mode.places_real_orders() => {
                    if let Some(creds) = &config.poly_creds {
                        let client = polymarket_trader::orders::ClobClient::new(creds.clone());
                        for intent in intents {
                            if let OrderIntent::Buy { token_id, side: _, size_usd, limit_price } = intent {
                                match client.create_limit_order(&token_id, polymarket_trader::orders::Side::Buy, limit_price.unwrap_or(0.0), size_usd).await {
                                    Ok(o)  => info!("[fv_momentum] order {}", o.order_id),
                                    Err(e) => warn!("[fv_momentum] order failed: {e}"),
                                }
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("[fv_momentum] on_book: {e}"),
            }
        }

        let metrics = engine.finalize(&portfolio).await;
        store.update_result(&id, |r| {
            r.total_return_pct = metrics.total_return_pct; r.balance = portfolio.balance_usdc;
            r.win_rate_pct = metrics.win_rate_pct; r.total_trades = metrics.total_trades;
            r.sharpe_ratio = metrics.sharpe_ratio; r.analysis = metrics.analysis.clone();
        });

        tokio::time::sleep(poll).await;
    }
}
