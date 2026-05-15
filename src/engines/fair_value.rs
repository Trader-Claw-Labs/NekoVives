//! FAIR-VALUE engine (IA-03 / TRADE-04)
//!
//! **Strategy:** Estimate a market's "true" probability from multiple signals
//! (poll averages, base-rate calibration, volume-weighted price) and trade
//! whenever the CLOB price diverges from the estimate by more than a threshold.
//!
//! ## Fair-value estimation
//! ```
//! fv = w_price × price_mid + w_volume × volume_signal + w_calibration × calibrated
//! ```
//! - **price_mid**: (YES_mid + (1 - NO_mid)) / 2 — consensus from both sides.
//! - **volume_signal**: rolling 20-candle VWAP normalised to [0, 1].
//! - **calibrated**: isotonic-regression approximation. In Backtest, we use a
//!   simple logistic squeeze: `1 / (1 + exp(-k×(price - 0.5)))` where k=4.
//!
//! ## Trade logic
//! - If `price_yes < fv - edge_threshold` → BUY YES (market undervalues YES).
//! - If `price_yes > fv + edge_threshold` → BUY NO  (market overvalues YES).
//! - Sizing: Kelly with `p = fv`, `q = 1 - fv`, `b = 1/price - 1`.
//!
//! ## Modes
//! - **Backtest**: candle close as YES price; volume-weighted over rolling window.
//! - **DryRun / Live**: live best-ask prices; same FV formula.

use std::collections::VecDeque;
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
use std::collections::HashMap;

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FairValueConfig {
    /// Polymarket market slugs to evaluate.
    pub markets: Vec<String>,
    /// Minimum gap between CLOB price and FV estimate to enter (e.g. 0.05 = 5 cents).
    #[serde(default = "default_edge")]
    pub edge_threshold: f64,
    /// Rolling window for VWAP computation (number of candles).
    #[serde(default = "default_window")]
    pub vwap_window: usize,
    /// Weight on raw CLOB mid-price in FV formula.
    #[serde(default = "default_w_price")]
    pub w_price: f64,
    /// Weight on VWAP signal in FV formula.
    #[serde(default = "default_w_volume")]
    pub w_volume: f64,
    /// Weight on calibrated-probability signal in FV formula.
    #[serde(default = "default_w_cal")]
    pub w_calibration: f64,
    /// Kelly fraction cap (e.g. 0.25 = 25% of balance per trade).
    #[serde(default = "default_kelly_cap")]
    pub kelly_cap: f64,
    /// Maximum USD per position.
    #[serde(default = "default_max_pos")]
    pub max_position_usd: f64,
    /// Seconds between polls in DryRun/Live mode.
    #[serde(default = "default_poll")]
    pub poll_secs: u64,
}

fn default_edge()     -> f64  { 0.05 }
fn default_window()   -> usize { 20 }
fn default_w_price()  -> f64  { 0.50 }
fn default_w_volume() -> f64  { 0.25 }
fn default_w_cal()    -> f64  { 0.25 }
fn default_kelly_cap()-> f64  { 0.25 }
fn default_max_pos()  -> f64  { 300.0 }
fn default_poll()     -> u64  { 45 }

impl Default for FairValueConfig {
    fn default() -> Self {
        Self {
            markets:         vec![],
            edge_threshold:  default_edge(),
            vwap_window:     default_window(),
            w_price:         default_w_price(),
            w_volume:        default_w_volume(),
            w_calibration:   default_w_cal(),
            kelly_cap:       default_kelly_cap(),
            max_position_usd: default_max_pos(),
            poll_secs:       default_poll(),
        }
    }
}

// ── FV estimate result ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FvEstimate {
    pub fv:          f64,   // fair-value probability for YES
    pub price_mid:   f64,
    pub vwap_signal: f64,
    pub calibrated:  f64,
    pub edge:        f64,   // signed: positive = BUY YES, negative = BUY NO
    pub kelly:       f64,
    pub action:      FvAction,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FvAction {
    BuyYes,
    BuyNo,
    Hold,
}

// ── Trade record ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct FvTrade {
    market_slug:  String,
    side:         Side,
    size_usd:     f64,
    entry_price:  f64,
    exit_price:   f64,
    profit_usd:   f64,
    win:          bool,
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct FairValueEngine {
    config:    FairValueConfig,
    mode:      ExecutionMode,
    /// Rolling candle buffer per slug for VWAP.
    buffers:   HashMap<String, VecDeque<CandleSnap>>,
    trades:    Vec<FvTrade>,
    total_profit: f64,
    /// Open position per slug (DryRun/Backtest).
    positions: HashMap<String, (Side, f64, f64)>, // slug → (side, size_usd, entry_price)
}

impl FairValueEngine {
    pub fn new(config: FairValueConfig) -> Self {
        Self {
            config,
            mode: ExecutionMode::Backtest,
            buffers: HashMap::new(),
            trades: vec![],
            total_profit: 0.0,
            positions: HashMap::new(),
        }
    }

    // ── Core FV computation ────────────────────────────────────────────────────

    /// Compute fair-value estimate for YES probability.
    ///
    /// `yes_mid` and `no_mid` are current CLOB mid-prices (or proxies in backtest).
    /// `candles` is the recent history for VWAP.
    pub fn estimate(
        yes_mid:  f64,
        no_mid:   f64,
        candles:  &VecDeque<CandleSnap>,
        cfg:      &FairValueConfig,
    ) -> FvEstimate {
        // 1. Price signal: consensus from both sides.
        let price_mid = (yes_mid + (1.0 - no_mid)) / 2.0;

        // 2. VWAP signal: volume-weighted average close over the window.
        let vwap_signal = if candles.is_empty() {
            price_mid
        } else {
            let (sum_pv, sum_v) = candles.iter().fold((0.0, 0.0), |(pv, v), c| {
                (pv + c.close * c.volume, v + c.volume)
            });
            if sum_v > 0.0 { (sum_pv / sum_v).clamp(0.01, 0.99) } else { price_mid }
        };

        // 3. Calibrated probability: logistic squeeze around 0.5 (k=4).
        let k = 4.0;
        let calibrated = 1.0 / (1.0 + (-k * (price_mid - 0.5)).exp());

        // 4. Weighted FV (weights sum to 1 by normalisation).
        let w_sum = cfg.w_price + cfg.w_volume + cfg.w_calibration;
        let fv = (cfg.w_price * price_mid
            + cfg.w_volume * vwap_signal
            + cfg.w_calibration * calibrated) / w_sum;

        // 5. Edge and Kelly.
        let edge  = fv - yes_mid; // positive = YES cheap, negative = NO cheap
        let abs_e = edge.abs();

        let (p, b) = if edge > 0.0 {
            // BUY YES: p = fv, b = 1/yes_mid - 1
            (fv, (1.0 / yes_mid - 1.0).max(0.001))
        } else {
            // BUY NO: p = 1-fv, b = 1/no_mid - 1
            (1.0 - fv, (1.0 / no_mid - 1.0).max(0.001))
        };
        let kelly = ((p * b - (1.0 - p)) / b).max(0.0).min(cfg.kelly_cap);

        let action = if abs_e >= cfg.edge_threshold && edge > 0.0 {
            FvAction::BuyYes
        } else if abs_e >= cfg.edge_threshold && edge < 0.0 {
            FvAction::BuyNo
        } else {
            FvAction::Hold
        };

        FvEstimate { fv, price_mid, vwap_signal, calibrated, edge, kelly, action }
    }

    fn push_candle(&mut self, slug: &str, candle: &CandleSnap) {
        let buf = self.buffers.entry(slug.to_string()).or_default();
        buf.push_back(candle.clone());
        if buf.len() > self.config.vwap_window {
            buf.pop_front();
        }
    }

    fn position_size(&self, kelly: f64, balance: f64) -> f64 {
        (balance * kelly).min(self.config.max_position_usd)
    }

    /// Simulate close of an open position in Backtest/DryRun.
    fn close_position(&mut self, slug: &str, exit_price: f64, portfolio: &mut Portfolio) {
        if let Some((side, size_usd, entry_price)) = self.positions.remove(slug) {
            let pnl = size_usd * (exit_price / entry_price - 1.0);
            portfolio.balance_usdc += size_usd + pnl;
            portfolio.realized_pnl += pnl;
            self.total_profit += pnl;
            let win = pnl > 0.0;
            debug!("[fair_value] close {slug} {:?} pnl=${pnl:.4}", side);
            self.trades.push(FvTrade {
                market_slug: slug.to_string(), side, size_usd,
                entry_price, exit_price, profit_usd: pnl, win,
            });
        }
    }
}

// ── StrategyEngine impl ───────────────────────────────────────────────────────

impl StrategyEngine for FairValueEngine {
    fn name(&self) -> &str {
        strategy_core::engines::FAIR_VALUE
    }

    async fn initialize(&mut self, mode: ExecutionMode, _portfolio: &Portfolio) -> Result<(), EngineError> {
        self.mode = mode;
        self.buffers.clear();
        self.trades.clear();
        self.positions.clear();
        self.total_profit = 0.0;
        info!("[fair_value] initialised in {} mode", self.mode);
        Ok(())
    }

    /// Backtest: candle close = YES mid; close - 0.02 = NO mid proxy.
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

        let est = Self::estimate(yes_mid, no_mid, &buf, &self.config);

        debug!("[fair_value:bt] {} fv={:.3} yes={:.3} edge={:.3} action={:?}",
            snap.slug, est.fv, yes_mid, est.edge, est.action);

        // Check if existing position converged (edge crossed zero = FV met).
        if let Some((side, _, entry_price)) = self.positions.get(&snap.slug).cloned() {
            let exit_price = match side { Side::Yes => yes_mid, Side::No => no_mid };
            let converged  = match side {
                Side::Yes => yes_mid >= est.fv - 0.01,
                Side::No  => no_mid  >= (1.0 - est.fv) - 0.01,
            };
            let loss_pct = (entry_price - exit_price) / entry_price;
            if converged || loss_pct >= 0.30 {
                self.close_position(&snap.slug, exit_price, portfolio);
            }
        }

        // Enter new position if no open position on this slug.
        if !self.positions.contains_key(&snap.slug) && est.action != FvAction::Hold {
            let size = self.position_size(est.kelly, portfolio.balance_usdc);
            if size >= 1.0 && portfolio.balance_usdc >= size {
                let (side, price) = match est.action {
                    FvAction::BuyYes => (Side::Yes, yes_mid),
                    FvAction::BuyNo  => (Side::No,  no_mid),
                    FvAction::Hold   => unreachable!(),
                };
                portfolio.balance_usdc -= size;
                self.positions.insert(snap.slug.clone(), (side, size, price));
                info!("[fair_value:bt] enter {} {:?} @ {:.3} fv={:.3} size=${:.2}",
                    snap.slug, self.positions[&snap.slug].0, price, est.fv, size);
            }
        }

        Ok(vec![])
    }

    /// DryRun/Live: book snapshot drives FV.
    async fn on_book(&mut self, snap: &BookSnapshot, portfolio: &mut Portfolio) -> Result<Vec<OrderIntent>, EngineError> {
        if self.mode == ExecutionMode::Backtest {
            return Ok(vec![]);
        }

        let yes_mid = snap.yes_mid();
        let no_mid  = snap.no_mid();
        let buf = self.buffers.get(&snap.slug).cloned().unwrap_or_default();
        let est = Self::estimate(yes_mid, no_mid, &buf, &self.config);

        info!("[fair_value] {} fv={:.3} yes={:.3} no={:.3} edge={:.3} action={:?}",
            snap.slug, est.fv, yes_mid, no_mid, est.edge, est.action);

        // Convergence / stop-loss check for DryRun.
        if self.mode == ExecutionMode::DryRun {
            if let Some((side, _, entry_price)) = self.positions.get(&snap.slug).cloned() {
                let exit_price = match side { Side::Yes => snap.yes.best_ask, Side::No => snap.no.best_ask };
                let converged  = match side {
                    Side::Yes => yes_mid >= est.fv - 0.01,
                    Side::No  => no_mid  >= (1.0 - est.fv) - 0.01,
                };
                let loss = (entry_price - exit_price) / entry_price;
                if converged || loss >= 0.30 {
                    self.close_position(&snap.slug, exit_price, portfolio);
                }
            }
        }

        if est.action == FvAction::Hold {
            return Ok(vec![]);
        }

        if self.positions.contains_key(&snap.slug) {
            return Ok(vec![]);
        }

        let size = self.position_size(est.kelly, portfolio.balance_usdc);
        if size < 1.0 {
            return Ok(vec![]);
        }

        match self.mode {
            ExecutionMode::DryRun => {
                let (side, price) = match est.action {
                    FvAction::BuyYes => (Side::Yes, snap.yes.best_ask),
                    FvAction::BuyNo  => (Side::No,  snap.no.best_ask),
                    FvAction::Hold   => unreachable!(),
                };
                portfolio.balance_usdc -= size;
                self.positions.insert(snap.slug.clone(), (side, size, price));
                Ok(vec![])
            }
            ExecutionMode::Live => {
                let (token_id, price, side) = match est.action {
                    FvAction::BuyYes => (snap.market_id.clone(), snap.yes.best_ask, Side::Yes),
                    FvAction::BuyNo  => (format!("{}_no", snap.slug), snap.no.best_ask, Side::No),
                    FvAction::Hold   => unreachable!(),
                };
                Ok(vec![OrderIntent::Buy { token_id, side, size_usd: size, limit_price: Some(price) }])
            }
            ExecutionMode::Backtest => Ok(vec![]),
        }
    }

    async fn on_event(&mut self, event: EngineEvent, portfolio: &mut Portfolio) -> Result<(), EngineError> {
        if let EngineEvent::MarketResolved { market_id, winning_side, .. } = &event {
            if let Some((side, size_usd, entry_price)) = self.positions.remove(market_id.as_str()) {
                let win        = side == *winning_side;
                let exit_price = if win { 0.97 } else { 0.02 };
                let pnl        = size_usd * (exit_price / entry_price - 1.0);
                portfolio.balance_usdc += size_usd + pnl;
                portfolio.realized_pnl += pnl;
                self.total_profit      += pnl;
                self.trades.push(FvTrade {
                    market_slug: market_id.clone(), side,
                    size_usd, entry_price, exit_price, profit_usd: pnl, win,
                });
                info!("[fair_value] market {market_id} resolved: {} pnl=${pnl:.4}", if win { "WIN" } else { "LOSS" });
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

        let analysis = if n == 0 {
            "No fair-value trades executed during this run.".to_string()
        } else {
            format!(
                "fair_value: {n} trades ({wins} wins, {win_rate:.1}% win rate), \
                 total profit ${:.4}, Sharpe {sharpe:.2}. \
                 FV formula: {:.0}% price + {:.0}% VWAP + {:.0}% calibrated.",
                self.total_profit,
                self.config.w_price     * 100.0,
                self.config.w_volume    * 100.0,
                self.config.w_calibration * 100.0,
            )
        };

        let mut extra = HashMap::new();
        extra.insert("trades".to_string(), n as f64);
        extra.insert("total_profit_usd".to_string(), self.total_profit);
        extra.insert("open_positions".to_string(), self.positions.len() as f64);

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

pub async fn run_fair_value_loop(
    store: Arc<crate::strategy_runner::StrategyRunnerStore>,
    config: crate::strategy_runner::RunnerConfig,
    _workspace_dir: std::path::PathBuf,
) {
    use crate::strategy_runner::{set_runner_error, set_runner_status};

    let id = config.id.clone();
    let markets: Vec<String> = config.symbol.split(',')
        .map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();

    let fv_cfg = FairValueConfig {
        markets:          markets.clone(),
        edge_threshold:   config.threshold.unwrap_or(0.05),
        max_position_usd: config.initial_balance * config.live_sizing_value.max(0.05),
        ..Default::default()
    };

    let mode = match config.mode.as_str() { "live" => ExecutionMode::Live, _ => ExecutionMode::DryRun };
    let mut engine    = FairValueEngine::new(fv_cfg.clone());
    let mut portfolio = Portfolio::new(config.initial_balance);

    if let Err(e) = engine.initialize(mode.clone(), &portfolio).await {
        set_runner_error(&store, &id, &format!("fair_value init: {e}")); return;
    }
    set_runner_status(&store, &id, "running");

    let poll = Duration::from_secs(fv_cfg.poll_secs);

    loop {
        for slug in &markets.clone() {
            let market = match polymarket_trader::markets::get_market(slug).await {
                Ok(m)  => m,
                Err(e) => { warn!("[fair_value] resolve {slug}: {e}"); continue; }
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
                                    Ok(o)  => info!("[fair_value] order {}", o.order_id),
                                    Err(e) => warn!("[fair_value] order failed: {e}"),
                                }
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("[fair_value] on_book: {e}"),
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
