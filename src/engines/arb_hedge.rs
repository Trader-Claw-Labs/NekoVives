//! ARB-HEDGE engine (HYB-02)
//!
//! Two complementary strategies on binary Polymarket markets:
//!
//! 1. **Synthetic arbitrage**: YES ask + NO ask < 1.0 − min_arb_edge
//!    → buy both sides; collect guaranteed profit at resolution.
//!    Profit = 1.0 − YES_ask − NO_ask per contract.
//!
//! 2. **Hedge overlay**: an open directional position falls more than
//!    `hedge_trigger_pct` from entry → open the opposite side to cap drawdown.
//!    When the original side recovers, the hedge is unwound.
//!
//! ## Modes
//! - **Backtest**: candle close as YES price; arb path disabled (needs two-sided book);
//!   hedge overlay simulated using high/low spread.
//! - **DryRun / Live**: full two-sided book; both arb and hedge paths active.

use std::collections::HashMap;
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

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArbHedgeConfig {
    /// Polymarket market slugs to monitor.
    pub markets: Vec<String>,
    /// Minimum combined book discount below 1.0 to enter synthetic arb (e.g. 0.03 = 3 cents profit).
    #[serde(default = "default_arb_edge")]
    pub min_arb_edge: f64,
    /// How far a directional position must drop from entry before adding a hedge (e.g. 0.20 = 20%).
    #[serde(default = "default_hedge_trigger")]
    pub hedge_trigger_pct: f64,
    /// Maximum USD allocated per market position.
    #[serde(default = "default_max_pos")]
    pub max_position_usd: f64,
    /// Seconds between polls in DryRun/Live mode.
    #[serde(default = "default_poll")]
    pub poll_secs: u64,
}

fn default_arb_edge()      -> f64  { 0.03 }
fn default_hedge_trigger() -> f64  { 0.20 }
fn default_max_pos()       -> f64  { 200.0 }
fn default_poll()          -> u64  { 60 }

impl Default for ArbHedgeConfig {
    fn default() -> Self {
        Self {
            markets:          vec![],
            min_arb_edge:     default_arb_edge(),
            hedge_trigger_pct: default_hedge_trigger(),
            max_position_usd: default_max_pos(),
            poll_secs:        default_poll(),
        }
    }
}

// ── Per-market state ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum MarketState {
    /// No open positions.
    Idle,
    /// Single directional position (entered before arb-hedge).
    Long {
        side:        Side,
        size_usd:    f64,
        entry_price: f64,
    },
    /// Hedged directional position (original side + hedge leg).
    Hedged {
        yes_size:  f64,
        no_size:   f64,
        yes_entry: f64,
        no_entry:  f64,
    },
    /// Synthetic arb: both sides bought because yes+no < 1.
    ArbOpen {
        yes_size:  f64,
        no_size:   f64,
        yes_entry: f64,
        no_entry:  f64,
    },
}

// ── Trade record ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct HedgeTrade {
    slug:        String,
    kind:        &'static str, // "arb" | "hedge" | "directional"
    pnl_usd:     f64,
    win:         bool,
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct ArbHedgeEngine {
    config:       ArbHedgeConfig,
    mode:         ExecutionMode,
    states:       HashMap<String, MarketState>,
    trades:       Vec<HedgeTrade>,
    total_profit: f64,
}

impl ArbHedgeEngine {
    pub fn new(config: ArbHedgeConfig) -> Self {
        Self {
            config,
            mode: ExecutionMode::Backtest,
            states: HashMap::new(),
            trades: vec![],
            total_profit: 0.0,
        }
    }

    /// Detect synthetic arb: returns `(has_arb, profit_margin)`.
    ///
    /// `yes_ask + no_ask < 1.0 - min_arb_edge` → guaranteed profit if both resolve.
    pub fn score_arb(yes_ask: f64, no_ask: f64, cfg: &ArbHedgeConfig) -> (bool, f64) {
        let sum    = yes_ask + no_ask;
        let margin = 1.0 - sum;
        let has    = sum < 1.0 - cfg.min_arb_edge;
        (has, margin)
    }

    fn state_of(&self, slug: &str) -> &MarketState {
        self.states.get(slug).unwrap_or(&MarketState::Idle)
    }

    fn position_size(&self, balance: f64) -> f64 {
        (balance * 0.10).min(self.config.max_position_usd).max(1.0)
    }

    /// Close a directional (Long) position; returns PnL.
    fn close_long(&mut self, slug: &str, exit_price: f64, portfolio: &mut Portfolio) -> f64 {
        if let Some(MarketState::Long { size_usd, entry_price, .. }) =
            self.states.remove(slug)
        {
            let pnl = size_usd * (exit_price / entry_price - 1.0);
            portfolio.balance_usdc += size_usd + pnl;
            portfolio.realized_pnl += pnl;
            self.total_profit      += pnl;
            pnl
        } else {
            0.0
        }
    }

    /// Unwind hedge leg only (convert Hedged → Long).
    fn unwind_hedge(&mut self, slug: &str, no_exit: f64, portfolio: &mut Portfolio) {
        if let Some(MarketState::Hedged { yes_size, no_size, yes_entry, no_entry }) =
            self.states.remove(slug)
        {
            let pnl = no_size * (no_exit / no_entry - 1.0);
            portfolio.balance_usdc += no_size + pnl;
            portfolio.realized_pnl += pnl;
            self.total_profit      += pnl;
            // Restore directional Long leg.
            self.states.insert(slug.to_string(), MarketState::Long {
                side: Side::Yes, size_usd: yes_size, entry_price: yes_entry,
            });
            debug!("[arb_hedge] hedge unwound for {slug}, hedge pnl=${pnl:.4}");
        }
    }

    /// Close both legs of an arb or hedged position.
    fn close_both(&mut self, slug: &str, yes_exit: f64, no_exit: f64, portfolio: &mut Portfolio, kind: &'static str) {
        let state = self.states.remove(slug);
        let (yes_size, no_size, yes_entry, no_entry) = match state {
            Some(MarketState::ArbOpen { yes_size, no_size, yes_entry, no_entry }) |
            Some(MarketState::Hedged { yes_size, no_size, yes_entry, no_entry }) => {
                (yes_size, no_size, yes_entry, no_entry)
            }
            _ => return,
        };

        let pnl_yes = yes_size * (yes_exit / yes_entry - 1.0);
        let pnl_no  = no_size  * (no_exit  / no_entry  - 1.0);
        let pnl     = pnl_yes + pnl_no;

        portfolio.balance_usdc += (yes_size + no_size) + pnl;
        portfolio.realized_pnl += pnl;
        self.total_profit      += pnl;

        self.trades.push(HedgeTrade { slug: slug.to_string(), kind, pnl_usd: pnl, win: pnl > 0.0 });
        info!("[arb_hedge] closed {kind} on {slug}: pnl=${pnl:.4}");
    }
}

// ── StrategyEngine impl ───────────────────────────────────────────────────────

impl StrategyEngine for ArbHedgeEngine {
    fn name(&self) -> &str {
        strategy_core::engines::ARB_HEDGE
    }

    async fn initialize(&mut self, mode: ExecutionMode, _portfolio: &Portfolio) -> Result<(), EngineError> {
        self.mode = mode;
        self.states.clear();
        self.trades.clear();
        self.total_profit = 0.0;
        info!("[arb_hedge] initialised in {} mode", self.mode);
        Ok(())
    }

    /// Backtest: uses candle high/low to simulate hedge overlay only (arb needs two-sided book).
    async fn on_tick(&mut self, snap: &MarketSnapshot, portfolio: &mut Portfolio) -> Result<Vec<OrderIntent>, EngineError> {
        if self.mode != ExecutionMode::Backtest {
            return Ok(vec![]);
        }

        let candle = match &snap.candle {
            Some(c) => c.clone(),
            None    => return Ok(vec![]),
        };

        let slug      = &snap.slug;
        let yes_price = candle.close.clamp(0.05, 0.95);

        // Convergence / stop-loss check for open Long position.
        if let MarketState::Long { side, size_usd, entry_price } = self.state_of(slug).clone() {
            let price_now = match side { Side::Yes => yes_price, Side::No => 1.0 - yes_price };
            let loss_pct  = (entry_price - price_now) / entry_price;

            if yes_price >= 0.92 || yes_price <= 0.08 {
                // Market resolved: close at resolution price.
                let exit = if yes_price >= 0.92 { 0.97 } else { 0.02 };
                let exit_p = match side { Side::Yes => exit, Side::No => 1.0 - exit };
                let pnl = size_usd * (exit_p / entry_price - 1.0);
                portfolio.balance_usdc += size_usd + pnl;
                portfolio.realized_pnl += pnl;
                self.total_profit      += pnl;
                self.trades.push(HedgeTrade { slug: slug.clone(), kind: "directional", pnl_usd: pnl, win: pnl > 0.0 });
                self.states.remove(slug);
                return Ok(vec![]);
            }

            // Hedge trigger: position down >= hedge_trigger_pct.
            if loss_pct >= self.config.hedge_trigger_pct {
                let no_price = (1.0 - yes_price).clamp(0.05, 0.95);
                let hedge_size = size_usd * 0.50; // half-size hedge
                if portfolio.balance_usdc >= hedge_size {
                    portfolio.balance_usdc -= hedge_size;
                    let (yes_size, yes_entry, no_size, no_entry) = match side {
                        Side::Yes => (size_usd, entry_price, hedge_size, no_price),
                        Side::No  => (hedge_size, no_price, size_usd, entry_price),
                    };
                    self.states.insert(slug.clone(), MarketState::Hedged {
                        yes_size, no_size, yes_entry, no_entry,
                    });
                    info!("[arb_hedge:bt] hedge triggered on {slug}: loss={loss_pct:.2}");
                }
                return Ok(vec![]);
            }
        }

        // For Hedged state: unwind hedge if YES recovers.
        if let MarketState::Hedged { yes_entry, .. } = self.state_of(slug).clone() {
            if yes_price >= yes_entry * 0.98 {
                let no_price = (1.0 - yes_price).clamp(0.05, 0.95);
                self.unwind_hedge(slug, no_price, portfolio);
            }
            return Ok(vec![]);
        }

        // Enter directional: enter Long(Yes) when price is well below 0.50.
        if self.state_of(slug) == &MarketState::Idle && yes_price < 0.45 {
            let size = self.position_size(portfolio.balance_usdc);
            if portfolio.balance_usdc >= size {
                portfolio.balance_usdc -= size;
                self.states.insert(slug.clone(), MarketState::Long {
                    side: Side::Yes, size_usd: size, entry_price: yes_price,
                });
                debug!("[arb_hedge:bt] enter Long(Yes) {slug} @ {yes_price:.3}");
            }
        }

        Ok(vec![])
    }

    /// DryRun/Live: full two-sided book; both arb and hedge overlay active.
    async fn on_book(&mut self, snap: &BookSnapshot, portfolio: &mut Portfolio) -> Result<Vec<OrderIntent>, EngineError> {
        if self.mode == ExecutionMode::Backtest {
            return Ok(vec![]);
        }

        let slug    = &snap.slug;
        let yes_ask = snap.yes.best_ask.clamp(0.01, 0.99);
        let no_ask  = snap.no.best_ask .clamp(0.01, 0.99);

        // ── Synthetic arb check ──────────────────────────────────────────────
        let (has_arb, margin) = Self::score_arb(yes_ask, no_ask, &self.config);
        if has_arb && self.state_of(slug) == &MarketState::Idle {
            let size = self.position_size(portfolio.balance_usdc);
            if portfolio.balance_usdc >= size * 2.0 {
                portfolio.balance_usdc -= size * 2.0;
                self.states.insert(slug.clone(), MarketState::ArbOpen {
                    yes_size: size, no_size: size,
                    yes_entry: yes_ask, no_entry: no_ask,
                });
                info!("[arb_hedge] open arb on {slug}: yes={yes_ask:.3} no={no_ask:.3} margin={margin:.3}");
                if self.mode == ExecutionMode::Live {
                    return Ok(vec![
                        OrderIntent::Buy { token_id: snap.market_id.clone(),         side: Side::Yes, size_usd: size, limit_price: Some(yes_ask) },
                        OrderIntent::Buy { token_id: format!("{}_no", snap.slug),    side: Side::No,  size_usd: size, limit_price: Some(no_ask)  },
                    ]);
                }
            }
        }

        // ── Arb resolution check ─────────────────────────────────────────────
        if let MarketState::ArbOpen { .. } = self.state_of(slug).clone() {
            if yes_ask >= 0.95 {
                self.close_both(slug, 0.97, 0.02, portfolio, "arb");
            } else if no_ask >= 0.95 {
                self.close_both(slug, 0.02, 0.97, portfolio, "arb");
            }
            return Ok(vec![]);
        }

        // ── Hedge overlay ────────────────────────────────────────────────────
        if let MarketState::Long { side, size_usd, entry_price } = self.state_of(slug).clone() {
            let price_now = match side { Side::Yes => yes_ask, Side::No => no_ask };
            let loss_pct  = (entry_price - price_now) / entry_price;

            if loss_pct >= self.config.hedge_trigger_pct {
                let hedge_size = size_usd * 0.50;
                if portfolio.balance_usdc >= hedge_size {
                    portfolio.balance_usdc -= hedge_size;
                    let (yes_s, yes_e, no_s, no_e) = match side {
                        Side::Yes => (size_usd, entry_price, hedge_size, no_ask),
                        Side::No  => (hedge_size, yes_ask, size_usd, entry_price),
                    };
                    self.states.insert(slug.clone(), MarketState::Hedged {
                        yes_size: yes_s, no_size: no_s, yes_entry: yes_e, no_entry: no_e,
                    });
                    info!("[arb_hedge] hedge triggered on {slug}: loss={loss_pct:.2}");
                    if self.mode == ExecutionMode::Live {
                        let (token_id, limit_price, hedge_side) = match side {
                            Side::Yes => (format!("{}_no", slug), no_ask, Side::No),
                            Side::No  => (snap.market_id.clone(), yes_ask, Side::Yes),
                        };
                        return Ok(vec![OrderIntent::Buy {
                            token_id, side: hedge_side, size_usd: hedge_size, limit_price: Some(limit_price),
                        }]);
                    }
                }
            }
        }

        // ── Hedged: unwind if YES recovers ───────────────────────────────────
        if let MarketState::Hedged { yes_entry, .. } = self.state_of(slug).clone() {
            if yes_ask >= yes_entry * 0.98 {
                self.unwind_hedge(slug, no_ask, portfolio);
            }
            return Ok(vec![]);
        }

        // ── Enter directional if Idle and no arb (DryRun only) ──────────────
        if self.mode == ExecutionMode::DryRun && self.state_of(slug) == &MarketState::Idle {
            let (enter, side, price) = if yes_ask < 0.45 {
                (true, Side::Yes, yes_ask)
            } else if no_ask < 0.45 {
                (true, Side::No, no_ask)
            } else {
                (false, Side::Yes, yes_ask)
            };

            if enter {
                let size = self.position_size(portfolio.balance_usdc);
                if portfolio.balance_usdc >= size {
                    portfolio.balance_usdc -= size;
                    let side_str = format!("{side:?}");
                    self.states.insert(slug.clone(), MarketState::Long {
                        side, size_usd: size, entry_price: price,
                    });
                    debug!("[arb_hedge:dry] enter Long({side_str}) {slug} @ {price:.3}");
                }
            }
        }

        Ok(vec![])
    }

    async fn on_event(&mut self, event: EngineEvent, portfolio: &mut Portfolio) -> Result<(), EngineError> {
        if let EngineEvent::MarketResolved { market_id, winning_side, .. } = &event {
            let slug = market_id.as_str();
            match self.state_of(slug).clone() {
                MarketState::Long { side, size_usd, entry_price } => {
                    let win        = side == *winning_side;
                    let exit_price = if win { 0.97 } else { 0.02 };
                    let pnl        = size_usd * (exit_price / entry_price - 1.0);
                    portfolio.balance_usdc += size_usd + pnl;
                    portfolio.realized_pnl += pnl;
                    self.total_profit      += pnl;
                    self.trades.push(HedgeTrade { slug: slug.to_string(), kind: "directional", pnl_usd: pnl, win });
                    self.states.remove(slug);
                }
                MarketState::ArbOpen { yes_size, no_size, yes_entry, no_entry } => {
                    let (yes_exit, no_exit) = if *winning_side == Side::Yes {
                        (0.97_f64, 0.02_f64)
                    } else {
                        (0.02_f64, 0.97_f64)
                    };
                    let pnl = yes_size * (yes_exit / yes_entry - 1.0)
                            + no_size  * (no_exit  / no_entry  - 1.0);
                    portfolio.balance_usdc += (yes_size + no_size) + pnl;
                    portfolio.realized_pnl += pnl;
                    self.total_profit      += pnl;
                    self.trades.push(HedgeTrade { slug: slug.to_string(), kind: "arb", pnl_usd: pnl, win: pnl > 0.0 });
                    self.states.remove(slug);
                }
                MarketState::Hedged { yes_size, no_size, yes_entry, no_entry } => {
                    let (yes_exit, no_exit) = if *winning_side == Side::Yes { (0.97, 0.02) } else { (0.02, 0.97) };
                    let pnl = yes_size * (yes_exit / yes_entry - 1.0)
                            + no_size  * (no_exit  / no_entry  - 1.0);
                    portfolio.balance_usdc += (yes_size + no_size) + pnl;
                    portfolio.realized_pnl += pnl;
                    self.total_profit      += pnl;
                    self.trades.push(HedgeTrade { slug: slug.to_string(), kind: "hedge", pnl_usd: pnl, win: pnl > 0.0 });
                    self.states.remove(slug);
                }
                MarketState::Idle => {}
            }
        }
        Ok(())
    }

    async fn finalize(&mut self, portfolio: &Portfolio) -> EngineMetrics {
        let n    = self.trades.len() as u32;
        let wins = self.trades.iter().filter(|t| t.win).count() as u32;
        let win_rate = if n > 0 { wins as f64 / n as f64 * 100.0 } else { 0.0 };

        let profits: Vec<f64> = self.trades.iter().map(|t| t.pnl_usd).collect();
        let mean = if profits.is_empty() { 0.0 } else { profits.iter().sum::<f64>() / profits.len() as f64 };
        let var  = if profits.len() < 2 { 0.0 } else {
            profits.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / (profits.len() - 1) as f64
        };
        let sharpe = if var.sqrt() > 0.0 { mean / var.sqrt() * 252_f64.sqrt() } else { 0.0 };

        let n_arb     = self.trades.iter().filter(|t| t.kind == "arb").count();
        let n_hedge   = self.trades.iter().filter(|t| t.kind == "hedge").count();
        let n_direct  = self.trades.iter().filter(|t| t.kind == "directional").count();
        let open      = self.states.len();

        let analysis = if n == 0 {
            format!("No arb-hedge trades closed. Open positions: {open}.")
        } else {
            format!(
                "arb_hedge: {n} closed ({n_arb} arb, {n_hedge} hedge, {n_direct} directional), \
                 {wins} wins ({win_rate:.1}%), profit ${:.4}, Sharpe {sharpe:.2}. \
                 Open: {open}.",
                self.total_profit
            )
        };

        let mut extra = HashMap::new();
        extra.insert("arb_trades".to_string(),        n_arb as f64);
        extra.insert("hedge_trades".to_string(),      n_hedge as f64);
        extra.insert("open_positions".to_string(),    open as f64);
        extra.insert("total_profit_usd".to_string(),  self.total_profit);

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

pub async fn run_arb_hedge_loop(
    store: Arc<crate::strategy_runner::StrategyRunnerStore>,
    config: crate::strategy_runner::RunnerConfig,
    _workspace_dir: std::path::PathBuf,
) {
    use crate::strategy_runner::{set_runner_error, set_runner_status};

    let id = config.id.clone();
    let markets: Vec<String> = config.symbol.split(',')
        .map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();

    let arb_cfg_base = ArbHedgeConfig {
        markets:          markets.clone(),
        min_arb_edge:     config.threshold.unwrap_or(0.03),
        max_position_usd: config.initial_balance * config.live_sizing_value.max(0.05),
        ..Default::default()
    };
    let arb_cfg = crate::tools::engine_backtest::merge_params(
        arb_cfg_base,
        config.engine_params.as_ref(),
    );

    let mode = match config.mode.as_str() { "live" => ExecutionMode::Live, _ => ExecutionMode::DryRun };
    let mut engine    = ArbHedgeEngine::new(arb_cfg.clone());
    let mut portfolio = Portfolio::new(config.initial_balance);

    if let Err(e) = engine.initialize(mode.clone(), &portfolio).await {
        set_runner_error(&store, &id, &format!("arb_hedge init: {e}")); return;
    }
    set_runner_status(&store, &id, "running");

    let poll = Duration::from_secs(arb_cfg.poll_secs);
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
                Err(e) => { warn!("[arb_hedge] resolve {slug}: {e}"); continue; }
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
                                    Ok(o)  => info!("[arb_hedge] order {}", o.order_id),
                                    Err(e) => warn!("[arb_hedge] order failed: {e}"),
                                }
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("[arb_hedge] on_book: {e}"),
            }
        }

        let metrics = engine.finalize(&portfolio).await;
        store.update_result(&id, |r| {
            r.total_return_pct = metrics.total_return_pct; r.balance = portfolio.balance_usdc;
            r.win_rate_pct     = metrics.win_rate_pct;     r.total_trades = metrics.total_trades;
            r.sharpe_ratio     = metrics.sharpe_ratio;     r.analysis = metrics.analysis.clone();
        });

        tokio::time::sleep(poll).await;
    }
}
