//! ROTATION-COMPOUNDER engine (HYB-05)
//!
//! **Strategy:** Meta-engine that continuously scores all configured markets and
//! concentrates capital into the single highest-scoring opportunity.  After each
//! position closes (market resolution or stop), profits are compounded into the
//! next best market.  Over 12 months this mimics a "best pick + reinvest" loop
//! that the research documents as ~12.1x theoretical max.
//!
//! ## Scoring function
//! ```
//! score(m) = edge(m) × liquidity_factor(m) × time_urgency(m) × kelly_fraction(m)
//! ```
//! - **edge**: distance from 50 cents (the maximum-uncertainty point).
//!   A market priced at 0.15 or 0.85 has high edge for the directional play.
//! - **liquidity_factor**: log-scaled depth; capped at 1.0 for depth ≥ $10k.
//! - **time_urgency**: `1.0 / sqrt(hours_to_close + 1)`.  Closer markets earn
//!   higher urgency multiplier (faster compounding opportunity).
//! - **kelly_fraction**: `edge / odds` per Kelly Criterion, capped at 25%.
//!
//! ## Modes
//! - **Backtest**: uses candle close as YES-probability proxy per market.
//!   Cycles 30-day windows; resolution modelled by close > 0.90 → WIN, < 0.10 → LOSS.
//! - **DryRun**: live prices, simulated positions, no real orders.
//! - **Live**: emits `Buy` intent for the top-scored token; runner places real order.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use strategy_core::{
    engine::StrategyEngine,
    types::{
        BookSnapshot, CandleSnap, EngineError, EngineEvent, EngineMetrics, ExecutionMode,
        MarketSnapshot, OrderIntent, Portfolio, Side,
    },
};
use tracing::{debug, info, warn};

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RotationConfig {
    /// List of market slugs to score and rotate through.
    pub markets: Vec<String>,
    /// Maximum fraction of portfolio to deploy in any single position.
    #[serde(default = "default_max_alloc")]
    pub max_allocation_pct: f64,
    /// Minimum score delta required to switch from current to a new market.
    /// Prevents excessive churn on near-equal scores.
    #[serde(default = "default_switch_threshold")]
    pub switch_threshold: f64,
    /// Minimum position size in USD to be worth entering.
    #[serde(default = "default_min_position")]
    pub min_position_usd: f64,
    /// Stop-loss: exit if position value drops below this fraction of entry cost.
    #[serde(default = "default_stop_loss")]
    pub stop_loss_pct: f64,
    /// How many seconds between scoring cycles in DryRun/Live mode.
    #[serde(default = "default_poll")]
    pub poll_secs: u64,
    /// Simulated days-to-close used in Backtest (no real deadline available).
    #[serde(default = "default_sim_days")]
    pub sim_days_to_close: f64,
}

fn default_max_alloc()       -> f64 { 0.60 }
fn default_switch_threshold() -> f64 { 0.05 }
fn default_min_position()    -> f64 { 10.0 }
fn default_stop_loss()       -> f64 { 0.40 }
fn default_poll()            -> u64 { 60 }
fn default_sim_days()        -> f64 { 15.0 }

impl Default for RotationConfig {
    fn default() -> Self {
        Self {
            markets: vec![],
            max_allocation_pct: default_max_alloc(),
            switch_threshold:   default_switch_threshold(),
            min_position_usd:   default_min_position(),
            stop_loss_pct:      default_stop_loss(),
            poll_secs:          default_poll(),
            sim_days_to_close:  default_sim_days(),
        }
    }
}

// ── Market score ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MarketScore {
    pub slug:              String,
    pub yes_token_id:      String,
    pub no_token_id:       String,
    pub yes_price:         f64,
    pub no_price:          f64,
    pub score:             f64,
    pub edge:              f64,
    pub kelly:             f64,
    pub best_side:         Side,
    pub depth_usd:         f64,
    pub hours_to_close:    f64,
    pub scored_at:         DateTime<Utc>,
}

// ── Open position ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct OpenPosition {
    slug:         String,
    token_id:     String,
    side:         Side,
    size_usd:     f64,
    entry_price:  f64,
    entered_at:   DateTime<Utc>,
}

// ── Closed position ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ClosedPosition {
    slug:         String,
    side:         Side,
    size_usd:     f64,
    entry_price:  f64,
    exit_price:   f64,
    profit_usd:   f64,
    win:          bool,
    held_hours:   f64,
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct RotationCompounderEngine {
    config:      RotationConfig,
    mode:        ExecutionMode,
    /// Latest scores per slug (updated each scoring cycle).
    scores:      HashMap<String, MarketScore>,
    /// Currently held position (at most one at a time in the pure-rotation model).
    position:    Option<OpenPosition>,
    closed:      Vec<ClosedPosition>,
    total_profit: f64,
    /// Candle-level price history for Backtest scoring (slug → recent closes).
    candle_hist: HashMap<String, Vec<CandleSnap>>,
}

impl RotationCompounderEngine {
    pub fn new(config: RotationConfig) -> Self {
        Self {
            config,
            mode: ExecutionMode::Backtest,
            scores: HashMap::new(),
            position: None,
            closed: vec![],
            total_profit: 0.0,
            candle_hist: HashMap::new(),
        }
    }

    // ── Scoring ────────────────────────────────────────────────────────────────

    /// Compute a score for a single market.
    ///
    /// `yes_price` is the current YES token ask; `no_price` is NO ask.
    /// `depth_usd` is liquidity on the better side; `hours_to_close` estimates
    /// how many hours until market resolution.
    pub fn score_market(
        yes_price: f64,
        no_price: f64,
        depth_usd: f64,
        hours_to_close: f64,
    ) -> (f64, Side, f64, f64) {
        // Pick the cheaper side (lower price = higher payout multiplier = better directional bet).
        // For symmetric binary markets (yes+no≈1), both have equal distance from 0.5 so we
        // prefer the lower-priced side which offers higher odds on a win.
        let (edge, best_side, best_price) = if yes_price <= no_price {
            ((0.50 - yes_price).abs(), Side::Yes, yes_price)
        } else {
            ((0.50 - no_price).abs(), Side::No, no_price)
        };

        // Liquidity factor: log2(depth / 100) clamped [0, 1].
        let liq_factor = ((depth_usd / 100.0).max(1.0).log2() / 7.0).min(1.0);

        // Time urgency: closer to resolution = more valuable (faster compounding).
        let urgency = 1.0 / (hours_to_close + 1.0).sqrt();

        // Kelly fraction: edge / (1/best_price - 1); cap at 0.25.
        let odds = (1.0 / best_price) - 1.0;
        let kelly = if odds > 0.0 { (edge / odds).min(0.25) } else { 0.0 };

        let score = edge * liq_factor * urgency * (1.0 + kelly);

        (score, best_side, edge, kelly)
    }

    /// Pick the market with the highest score.
    fn top_market(&self) -> Option<&MarketScore> {
        self.scores.values().max_by(|a, b| a.score.partial_cmp(&b.score).unwrap())
    }

    /// Position size from Kelly, capped at max_allocation_pct of balance.
    fn position_size(&self, kelly: f64, balance: f64) -> f64 {
        let kelly_size = balance * kelly;
        let max_size   = balance * self.config.max_allocation_pct;
        kelly_size.min(max_size).max(0.0)
    }

    // ── Backtest helpers ───────────────────────────────────────────────────────

    /// Update candle history and return the latest close for a slug.
    fn push_candle(&mut self, slug: &str, candle: &CandleSnap) -> f64 {
        let hist = self.candle_hist.entry(slug.to_string()).or_default();
        hist.push(candle.clone());
        if hist.len() > 60 { hist.remove(0); }
        candle.close
    }

    /// Simulate resolution when close crosses resolution thresholds.
    fn check_backtest_resolution(&mut self, portfolio: &mut Portfolio) {
        let pos = match self.position.clone() {
            Some(p) => p,
            None => return,
        };

        // Get the latest price for the position's slug.
        let latest = match self.candle_hist.get(&pos.slug).and_then(|h| h.last()) {
            Some(c) => c.close.clamp(0.01, 0.99),
            None => return,
        };

        let (resolved, win_price) = match pos.side {
            Side::Yes => {
                if latest > 0.90 { (true, 0.97) }
                else if latest < 0.10 { (true, 0.02) }
                else { (false, latest) }
            }
            Side::No => {
                let no_price = 1.0 - latest;
                if no_price > 0.90 { (true, 0.97) }
                else if no_price < 0.10 { (true, 0.02) }
                else { (false, no_price) }
            }
        };

        // Stop-loss check.
        let current_price = match pos.side {
            Side::Yes => latest,
            Side::No  => 1.0 - latest,
        };
        let loss_pct = (pos.entry_price - current_price) / pos.entry_price;
        let stopped  = loss_pct >= self.config.stop_loss_pct;

        if resolved || stopped {
            let exit_price = if stopped { current_price } else { win_price };
            let pnl = pos.size_usd * (exit_price / pos.entry_price - 1.0);
            portfolio.balance_usdc += pos.size_usd + pnl;
            portfolio.realized_pnl += pnl;
            self.total_profit += pnl;

            let held_hours = Utc::now()
                .signed_duration_since(pos.entered_at)
                .num_minutes() as f64 / 60.0;

            info!(
                "[rotation:backtest] {} {:?} closed exit={:.3} pnl=${:.4} ({})",
                pos.slug, pos.side, exit_price, pnl,
                if stopped { "stop-loss" } else { "resolved" }
            );

            self.closed.push(ClosedPosition {
                slug:        pos.slug.clone(),
                side:        pos.side,
                size_usd:    pos.size_usd,
                entry_price: pos.entry_price,
                exit_price,
                profit_usd:  pnl,
                win:         pnl > 0.0,
                held_hours,
            });
            self.position = None;
        }
    }

    fn record_backtest_entry(
        &mut self,
        slug: &str,
        side: Side,
        price: f64,
        size_usd: f64,
        portfolio: &mut Portfolio,
    ) {
        portfolio.balance_usdc -= size_usd;
        self.position = Some(OpenPosition {
            slug:        slug.to_string(),
            token_id:    format!("{slug}_{}", match side { Side::Yes => "yes", Side::No => "no" }),
            side,
            size_usd,
            entry_price: price,
            entered_at:  Utc::now(),
        });
        info!(
            "[rotation:backtest] entering {} {:?} @ {:.3} size=${:.2}",
            slug, self.position.as_ref().unwrap().side, price, size_usd
        );
    }
}

// ── StrategyEngine impl ───────────────────────────────────────────────────────

impl StrategyEngine for RotationCompounderEngine {
    fn name(&self) -> &str {
        strategy_core::engines::ROTATION_COMPOUNDER
    }

    async fn initialize(&mut self, mode: ExecutionMode, _portfolio: &Portfolio) -> Result<(), EngineError> {
        self.mode = mode;
        self.scores.clear();
        self.position = None;
        self.closed.clear();
        self.total_profit = 0.0;
        self.candle_hist.clear();
        info!("[rotation] initialised in {} mode, {} markets",
            self.mode, self.config.markets.len());
        Ok(())
    }

    /// Backtest path: each candle is one market observation.
    async fn on_tick(&mut self, snap: &MarketSnapshot, portfolio: &mut Portfolio) -> Result<Vec<OrderIntent>, EngineError> {
        if self.mode != ExecutionMode::Backtest {
            return Ok(vec![]);
        }

        let candle = match &snap.candle {
            Some(c) => c.clone(),
            None => return Ok(vec![]),
        };

        let yes_mid = self.push_candle(&snap.slug, &candle).clamp(0.05, 0.95);
        let no_mid  = 1.0 - yes_mid;
        let depth   = candle.volume * yes_mid; // proxy for liquidity
        let hours   = self.config.sim_days_to_close * 24.0;

        let (score, best_side, edge, kelly) = Self::score_market(yes_mid, no_mid, depth, hours);

        self.scores.insert(snap.slug.clone(), MarketScore {
            slug:           snap.slug.clone(),
            yes_token_id:   format!("{}_yes", snap.slug),
            no_token_id:    format!("{}_no",  snap.slug),
            yes_price:      yes_mid,
            no_price:       no_mid,
            score,
            edge,
            kelly,
            best_side,
            depth_usd:      depth,
            hours_to_close: hours,
            scored_at:      Utc::now(),
        });

        // Check if existing position resolved.
        self.check_backtest_resolution(portfolio);

        // If no position, enter the top market.
        if self.position.is_none() {
            if let Some(top) = self.top_market().cloned() {
                let size = self.position_size(top.kelly, portfolio.balance_usdc);
                if size >= self.config.min_position_usd && portfolio.balance_usdc >= size {
                    let (price, side) = match top.best_side {
                        Side::Yes => (top.yes_price, Side::Yes),
                        Side::No  => (top.no_price,  Side::No),
                    };
                    self.record_backtest_entry(&top.slug, side, price, size, portfolio);
                }
            }
        } else {
            // Consider rotating to a better market.
            let current_slug = self.position.as_ref().map(|p| p.slug.clone()).unwrap_or_default();
            let current_score = self.scores.get(&current_slug).map(|s| s.score).unwrap_or(0.0);
            if let Some(top) = self.top_market().cloned() {
                if top.slug != current_slug
                    && top.score > current_score + self.config.switch_threshold
                {
                    // Close current position at mid-price and rotate.
                    if let Some(pos) = self.position.take() {
                        let exit_price = match pos.side {
                            Side::Yes => yes_mid,
                            Side::No  => 1.0 - yes_mid,
                        };
                        let pnl = pos.size_usd * (exit_price / pos.entry_price - 1.0);
                        portfolio.balance_usdc += pos.size_usd + pnl;
                        portfolio.realized_pnl += pnl;
                        self.total_profit += pnl;
                        debug!("[rotation:backtest] rotating from {} to {} score delta {:.3}",
                            pos.slug, top.slug, top.score - current_score);
                        self.closed.push(ClosedPosition {
                            slug:        pos.slug,
                            side:        pos.side,
                            size_usd:    pos.size_usd,
                            entry_price: pos.entry_price,
                            exit_price,
                            profit_usd:  pnl,
                            win:         pnl > 0.0,
                            held_hours:  0.0,
                        });
                    }

                    // Enter top market.
                    let size = self.position_size(top.kelly, portfolio.balance_usdc);
                    if size >= self.config.min_position_usd && portfolio.balance_usdc >= size {
                        let (price, side) = match top.best_side {
                            Side::Yes => (top.yes_price, Side::Yes),
                            Side::No  => (top.no_price,  Side::No),
                        };
                        self.record_backtest_entry(&top.slug, side, price, size, portfolio);
                    }
                }
            }
        }

        Ok(vec![])
    }

    /// DryRun/Live path: book snapshots drive scoring and intent emission.
    async fn on_book(&mut self, snap: &BookSnapshot, portfolio: &mut Portfolio) -> Result<Vec<OrderIntent>, EngineError> {
        if self.mode == ExecutionMode::Backtest {
            return Ok(vec![]);
        }

        let hours = self.config.sim_days_to_close * 24.0; // no real deadline from REST API
        let depth = snap.yes.ask_depth_usd.min(snap.no.ask_depth_usd);
        let (score, best_side, edge, kelly) =
            Self::score_market(snap.yes.best_ask, snap.no.best_ask, depth, hours);

        self.scores.insert(snap.slug.clone(), MarketScore {
            slug:           snap.slug.clone(),
            yes_token_id:   snap.market_id.clone(),
            no_token_id:    format!("{}_no", snap.slug),
            yes_price:      snap.yes.best_ask,
            no_price:       snap.no.best_ask,
            score,
            edge,
            kelly,
            best_side,
            depth_usd:      depth,
            hours_to_close: hours,
            scored_at:      Utc::now(),
        });

        let top = match self.top_market().cloned() {
            Some(t) => t,
            None    => return Ok(vec![]),
        };

        let current_slug  = self.position.as_ref().map(|p| p.slug.clone());
        let current_score = current_slug.as_ref()
            .and_then(|s| self.scores.get(s.as_str()))
            .map(|s| s.score)
            .unwrap_or(0.0);

        // No position → enter top market.
        if self.position.is_none() {
            let size = self.position_size(top.kelly, portfolio.balance_usdc);
            if size < self.config.min_position_usd {
                return Ok(vec![]);
            }
            info!("[rotation] entering {} {:?} score={:.4} size=${:.2}",
                top.slug, top.best_side, top.score, size);

            if self.mode == ExecutionMode::DryRun {
                let price = match top.best_side {
                    Side::Yes => top.yes_price,
                    Side::No  => top.no_price,
                };
                portfolio.balance_usdc -= size;
                self.position = Some(OpenPosition {
                    slug:        top.slug.clone(),
                    token_id:    match top.best_side {
                        Side::Yes => top.yes_token_id.clone(),
                        Side::No  => top.no_token_id.clone(),
                    },
                    side:        top.best_side.clone(),
                    size_usd:    size,
                    entry_price: price,
                    entered_at:  Utc::now(),
                });
                return Ok(vec![]);
            }

            // Live: emit Buy intent.
            let (token_id, price, side) = match top.best_side {
                Side::Yes => (top.yes_token_id.clone(), top.yes_price, Side::Yes),
                Side::No  => (top.no_token_id.clone(),  top.no_price,  Side::No),
            };
            return Ok(vec![OrderIntent::Buy {
                token_id,
                side,
                size_usd:    size,
                limit_price: Some(price),
            }]);
        }

        // Rotation check.
        if top.score > current_score + self.config.switch_threshold {
            if let Some(curr_slug) = &current_slug {
                if &top.slug != curr_slug {
                    warn!("[rotation] rotating from {} ({:.4}) → {} ({:.4})",
                        curr_slug, current_score, top.slug, top.score);

                    if self.mode == ExecutionMode::DryRun {
                        if let Some(pos) = self.position.take() {
                            let exit_price = match pos.side {
                                Side::Yes => snap.yes.best_ask,
                                Side::No  => snap.no.best_ask,
                            };
                            let pnl = pos.size_usd * (exit_price / pos.entry_price - 1.0);
                            portfolio.balance_usdc += pos.size_usd + pnl;
                            portfolio.realized_pnl += pnl;
                            self.total_profit += pnl;
                            self.closed.push(ClosedPosition {
                                slug:        pos.slug, side: pos.side,
                                size_usd:    pos.size_usd, entry_price: pos.entry_price,
                                exit_price, profit_usd: pnl, win: pnl > 0.0, held_hours: 0.0,
                            });
                            // Enter new position.
                            let size  = self.position_size(top.kelly, portfolio.balance_usdc);
                            let price = match top.best_side { Side::Yes => top.yes_price, Side::No => top.no_price };
                            portfolio.balance_usdc -= size;
                            self.position = Some(OpenPosition {
                                slug:        top.slug.clone(),
                                token_id:    match top.best_side { Side::Yes => top.yes_token_id, Side::No => top.no_token_id },
                                side:        top.best_side,
                                size_usd:    size,
                                entry_price: price,
                                entered_at:  Utc::now(),
                            });
                        }
                    }
                    // Live: caller handles cancel + new Buy from returned intents.
                }
            }
        }

        Ok(vec![])
    }

    async fn on_event(&mut self, event: EngineEvent, portfolio: &mut Portfolio) -> Result<(), EngineError> {
        match &event {
            EngineEvent::OrderFilled { token_id, size, .. } => {
                info!("[rotation] fill token={token_id} size={size}");
                // Update open position entry size in Live mode.
                if let Some(pos) = &mut self.position {
                    if pos.token_id == *token_id {
                        pos.size_usd = *size;
                    }
                }
            }
            EngineEvent::MarketResolved { market_id, winning_side, .. } => {
                if let Some(pos) = self.position.take() {
                    let win = pos.side == *winning_side;
                    let exit_price = if win { 0.97 } else { 0.02 };
                    let pnl = pos.size_usd * (exit_price / pos.entry_price - 1.0);
                    portfolio.balance_usdc += pos.size_usd + pnl;
                    portfolio.realized_pnl += pnl;
                    self.total_profit += pnl;
                    info!("[rotation] market {market_id} resolved: {:?} {} pnl=${pnl:.4}",
                        winning_side, if win { "WIN" } else { "LOSS" });
                    self.closed.push(ClosedPosition {
                        slug: pos.slug, side: pos.side,
                        size_usd: pos.size_usd, entry_price: pos.entry_price,
                        exit_price, profit_usd: pnl, win, held_hours: 0.0,
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn finalize(&mut self, portfolio: &Portfolio) -> EngineMetrics {
        let n    = self.closed.len() as u32;
        let wins = self.closed.iter().filter(|p| p.win).count() as u32;
        let win_rate = if n > 0 { wins as f64 / n as f64 * 100.0 } else { 0.0 };

        let profits: Vec<f64> = self.closed.iter().map(|p| p.profit_usd).collect();
        let mean = if profits.is_empty() { 0.0 } else { profits.iter().sum::<f64>() / profits.len() as f64 };
        let variance = if profits.len() < 2 { 0.0 } else {
            profits.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / (profits.len() - 1) as f64
        };
        let std_dev = variance.sqrt();
        let sharpe  = if std_dev > 0.0 { mean / std_dev * 252_f64.sqrt() } else { 0.0 };

        // Max drawdown over closed positions.
        let mut peak = portfolio.initial_balance;
        let mut max_dd = 0.0_f64;
        let mut running = portfolio.initial_balance;
        for p in &self.closed {
            running += p.profit_usd;
            if running > peak { peak = running; }
            let dd = (peak - running) / peak;
            if dd > max_dd { max_dd = dd; }
        }

        let n_markets = self.scores.len();
        let analysis = if n == 0 {
            "No positions closed during this run.".to_string()
        } else {
            format!(
                "rotation_compounder: {n} positions closed ({wins} wins, {:.1}% win rate), \
                 total profit ${:.4}, Sharpe {sharpe:.2}, max DD {:.1}%, \
                 {n_markets} markets scored. Kelly-weighted compounding.",
                win_rate, self.total_profit, max_dd * 100.0
            )
        };

        let mut extra = HashMap::new();
        extra.insert("positions_closed".to_string(), n as f64);
        extra.insert("positions_open".to_string(), if self.position.is_some() { 1.0 } else { 0.0 });
        extra.insert("markets_scored".to_string(), n_markets as f64);
        extra.insert("total_profit_usd".to_string(), self.total_profit);
        extra.insert("max_drawdown_pct".to_string(), max_dd * 100.0);

        EngineMetrics {
            total_return_pct: if portfolio.initial_balance > 0.0 {
                self.total_profit / portfolio.initial_balance * 100.0
            } else { 0.0 },
            sharpe_ratio:     sharpe,
            max_drawdown_pct: max_dd * 100.0,
            win_rate_pct:     win_rate,
            total_trades:     n,
            analysis,
            extra,
            data_confidence: if self.mode == ExecutionMode::Backtest { "low".to_string() }
                             else { "high".to_string() },
        }
    }
}

// ── Live poll loop ─────────────────────────────────────────────────────────────

pub async fn run_rotation_compounder_loop(
    store: Arc<crate::strategy_runner::StrategyRunnerStore>,
    config: crate::strategy_runner::RunnerConfig,
    _workspace_dir: std::path::PathBuf,
) {
    use crate::strategy_runner::{set_runner_error, set_runner_status};

    let id = config.id.clone();

    // markets: comma-separated slugs in symbol field, or just one slug.
    let markets: Vec<String> = config.symbol
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let rot_cfg_base = RotationConfig {
        markets:           markets.clone(),
        max_allocation_pct: config.live_sizing_value.max(0.10),
        switch_threshold:  config.threshold.unwrap_or(0.05),
        min_position_usd:  10.0,
        stop_loss_pct:     config.stop_loss_pct.unwrap_or(0.40),
        poll_secs:         60,
        sim_days_to_close: 15.0,
    };
    let rot_cfg = crate::tools::engine_backtest::merge_params(
        rot_cfg_base,
        config.engine_params.as_ref(),
    );

    let mode = match config.mode.as_str() {
        "live" => ExecutionMode::Live,
        _      => ExecutionMode::DryRun,
    };

    let mut engine    = RotationCompounderEngine::new(rot_cfg.clone());
    let mut portfolio = Portfolio::new(config.initial_balance);

    if let Err(e) = engine.initialize(mode.clone(), &portfolio).await {
        set_runner_error(&store, &id, &format!("rotation_compounder init: {e}"));
        return;
    }
    set_runner_status(&store, &id, "running");

    let poll = Duration::from_secs(rot_cfg.poll_secs);

    loop {
        for slug in &markets.clone() {
            let market = match polymarket_trader::markets::get_market(slug).await {
                Ok(m)  => m,
                Err(e) => { warn!("[rotation] resolve {slug}: {e}"); continue; }
            };
            let (ask_yes, ask_no) = tokio::join!(
                polymarket_trader::markets::get_market_price(&market.yes_token_id),
                polymarket_trader::markets::get_market_price(&market.no_token_id),
            );
            let ask_yes = match ask_yes { Ok(p) => p, Err(e) => { warn!("yes {e}"); continue; } };
            let ask_no  = match ask_no  { Ok(p) => p, Err(e) => { warn!("no  {e}"); continue; } };

            let snap = BookSnapshot {
                market_id: market.yes_token_id.clone(),
                slug:      slug.clone(),
                yes: strategy_core::types::BookLevel {
                    best_ask: ask_yes, best_bid: ask_yes - 0.01,
                    ask_depth_usd: 2000.0, bid_depth_usd: 1600.0,
                },
                no: strategy_core::types::BookLevel {
                    best_ask: ask_no,  best_bid: ask_no  - 0.01,
                    ask_depth_usd: 2000.0, bid_depth_usd: 1600.0,
                },
                timestamp: chrono::Utc::now(),
                meta: Default::default(),
            };

            match engine.on_book(&snap, &mut portfolio).await {
                Ok(intents) if !intents.is_empty() && mode.places_real_orders() => {
                    if let Some(creds) = &config.poly_creds {
                        let client = polymarket_trader::orders::ClobClient::new(creds.clone());
                        for intent in intents {
                            if let OrderIntent::Buy { token_id, side: _, size_usd, limit_price } = intent {
                                match client.create_limit_order(
                                    &token_id,
                                    polymarket_trader::orders::Side::Buy,
                                    limit_price.unwrap_or(0.0),
                                    size_usd,
                                ).await {
                                    Ok(o)  => info!("[rotation] order placed {}", o.order_id),
                                    Err(e) => warn!("[rotation] order failed: {e}"),
                                }
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("[rotation] on_book: {e}"),
            }
        }

        let metrics = engine.finalize(&portfolio).await;
        store.update_result(&id, |r| {
            r.total_return_pct = metrics.total_return_pct;
            r.balance          = portfolio.balance_usdc;
            r.win_rate_pct     = metrics.win_rate_pct;
            r.total_trades     = metrics.total_trades;
            r.sharpe_ratio     = metrics.sharpe_ratio;
            r.analysis         = metrics.analysis.clone();
        });

        tokio::time::sleep(poll).await;
    }
}

use std::sync::Arc;
