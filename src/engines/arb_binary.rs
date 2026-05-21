//! ARB-BINARY engine (IA-04 / ARB-01 / ARB-02)
//!
//! Detects `YES + NO < 1.00` (binary arbitrage) and emits parallel BUY intents
//! for both legs.  Implements the `StrategyEngine` trait so the runner can
//! manage it identically to any other engine.
//!
//! ## Modes
//! - **Backtest**: replays price history from `polymarket_historical` as mid-price
//!   proxies.  Full order-book depth is not available, so arb edges are proxied
//!   and marked `data_confidence = "low"`.
//! - **DryRun**: connects to the live CLOB price feed and detects real edges,
//!   but only simulates order execution (no real orders placed).
//! - **Live**: detects live edges and submits parallel buy orders via
//!   `ClobClient::create_limit_order`.  Partial fills are queued in
//!   `PartialFillQueue` for the Phase-5 HYB-02 hedge layer.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use strategy_core::{
    engine::StrategyEngine,
    types::{
        BookLevel, BookSnapshot, EngineError, EngineEvent, EngineMetrics, ExecutionMode,
        MarketSnapshot, OrderIntent, Portfolio, Side,
    },
};
use tracing::{debug, info, warn};

// ── Config ────────────────────────────────────────────────────────────────────

/// Arb-binary engine configuration (subset of `RunnerConfig` fields).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArbBinaryConfig {
    /// Polymarket market slugs to monitor.
    pub markets: Vec<String>,
    /// Minimum net edge (1.0 - ask_yes - ask_no - fee) to trigger a trade.
    /// Default: 0.005 (0.5 %).
    #[serde(default = "default_min_edge")]
    pub min_edge_pct: f64,
    /// Maximum USD position per arbitrage opportunity.
    #[serde(default = "default_max_pos")]
    pub max_position_usd: f64,
    /// Minimum USD liquidity required on each side before entering.
    #[serde(default = "default_liq_floor")]
    pub liquidity_floor_usd: f64,
    /// Estimated fee per side (Polymarket charges ~0.1% on smaller side).
    /// Default: 0.002 (0.2% round-trip).
    #[serde(default = "default_fee")]
    pub fee_pct: f64,
    /// How often to poll the CLOB price API in DryRun/Live mode (seconds).
    #[serde(default = "default_poll")]
    pub poll_secs: u64,
    /// Maximum concurrent opportunities tracked simultaneously.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
}

fn default_min_edge()       -> f64  { 0.005 }
fn default_max_pos()        -> f64  { 500.0 }
fn default_liq_floor()      -> f64  { 100.0 }
fn default_fee()            -> f64  { 0.002 }
fn default_poll()           -> u64  { 30 }
fn default_max_concurrent() -> usize { 5 }

impl Default for ArbBinaryConfig {
    fn default() -> Self {
        Self {
            markets: vec![],
            min_edge_pct: default_min_edge(),
            max_position_usd: default_max_pos(),
            liquidity_floor_usd: default_liq_floor(),
            fee_pct: default_fee(),
            poll_secs: default_poll(),
            max_concurrent: default_max_concurrent(),
        }
    }
}

// ── Partial fill queue ────────────────────────────────────────────────────────

/// Tracks one-sided partial fills so that HYB-02 (Phase 5) can hedge them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PartialFillRecord {
    pub market_slug: String,
    pub yes_token_id: String,
    pub no_token_id: String,
    /// Which side got filled (the other side failed).
    pub filled_side: Side,
    pub filled_size_usd: f64,
    pub filled_price: f64,
    pub detected_at: chrono::DateTime<Utc>,
}

/// Shared queue consumed by the Phase-5 hedge layer.
pub type PartialFillQueue = Arc<Mutex<Vec<PartialFillRecord>>>;

// ── Opportunity ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ArbOpportunity {
    market_slug: String,
    yes_token_id: String,
    no_token_id: String,
    ask_yes: f64,
    ask_no: f64,
    edge: f64,
    liq_yes: f64,
    liq_no: f64,
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct ArbBinaryEngine {
    config: ArbBinaryConfig,
    mode: ExecutionMode,
    /// Completed arb cycles (both sides filled or simulated).
    cycles: Vec<ArbCycle>,
    /// Running total of simulated/real profit.
    total_profit: f64,
    partial_fills: PartialFillQueue,
    /// Token IDs resolved per slug (cache to avoid redundant API calls).
    token_cache: HashMap<String, (String, String)>, // slug → (yes_id, no_id)
}

#[derive(Debug, Clone)]
struct ArbCycle {
    market_slug: String,
    edge: f64,
    position_usd: f64,
    profit_usd: f64,
    success: bool,
    timestamp: chrono::DateTime<Utc>,
}

impl ArbBinaryEngine {
    pub fn new(config: ArbBinaryConfig) -> Self {
        Self {
            config,
            mode: ExecutionMode::Backtest,
            cycles: vec![],
            total_profit: 0.0,
            partial_fills: Arc::new(Mutex::new(vec![])),
            token_cache: HashMap::new(),
        }
    }

    /// Returns the shared partial-fill queue (for Phase-5 hedge layer).
    pub fn partial_fill_queue(&self) -> PartialFillQueue {
        self.partial_fills.clone()
    }

    // ── Detection ──────────────────────────────────────────────────────────────

    /// Detect arb opportunity from a live `BookSnapshot`.
    fn detect(snap: &BookSnapshot, cfg: &ArbBinaryConfig) -> Option<ArbOpportunity> {
        let ask_yes = snap.yes.best_ask;
        let ask_no  = snap.no.best_ask;
        let liq_yes = snap.yes.ask_depth_usd;
        let liq_no  = snap.no.ask_depth_usd;

        // Liquidity gate
        if liq_yes < cfg.liquidity_floor_usd || liq_no < cfg.liquidity_floor_usd {
            return None;
        }

        let edge = 1.0 - ask_yes - ask_no - cfg.fee_pct;
        if edge < cfg.min_edge_pct {
            return None;
        }

        Some(ArbOpportunity {
            market_slug: snap.slug.clone(),
            yes_token_id: snap.market_id.clone(), // overwritten by caller when real IDs available
            no_token_id: snap.market_id.clone(),
            ask_yes,
            ask_no,
            edge,
            liq_yes,
            liq_no,
        })
    }

    /// Detect arb from mid-price proxies (Backtest mode).
    ///
    /// In backtest we only have mid prices, not the full book.  We model
    /// `ask ≈ mid + 0.005` as a conservative estimate and flag data_confidence low.
    fn detect_from_mid(yes_mid: f64, no_mid: f64, slug: &str, cfg: &ArbBinaryConfig) -> Option<ArbOpportunity> {
        let ask_yes = yes_mid + 0.005;
        let ask_no  = no_mid  + 0.005;
        let edge = 1.0 - ask_yes - ask_no - cfg.fee_pct;
        if edge < cfg.min_edge_pct {
            return None;
        }
        Some(ArbOpportunity {
            market_slug: slug.to_string(),
            yes_token_id: format!("{slug}_yes"),
            no_token_id:  format!("{slug}_no"),
            ask_yes,
            ask_no,
            edge,
            liq_yes: 1e9, // unknown; set large so liquidity gate passes
            liq_no:  1e9,
        })
    }

    // ── Sizing ─────────────────────────────────────────────────────────────────

    /// Optimal position size respecting liquidity and max_position.
    fn size_for(&self, opp: &ArbOpportunity, balance: f64) -> f64 {
        let liquidity_cap = opp.liq_yes.min(opp.liq_no) * 0.20; // ≤ 20% of thinner side
        let balance_cap   = balance * 0.25;                      // ≤ 25% of available balance
        self.config.max_position_usd
            .min(liquidity_cap)
            .min(balance_cap)
    }

    // ── Order intents ──────────────────────────────────────────────────────────

    fn make_intents(&self, opp: &ArbOpportunity, size_usd: f64) -> Vec<OrderIntent> {
        vec![
            OrderIntent::Buy {
                token_id:    opp.yes_token_id.clone(),
                side:        Side::Yes,
                size_usd:    size_usd * opp.ask_yes,
                limit_price: Some(opp.ask_yes),
            },
            OrderIntent::Buy {
                token_id:    opp.no_token_id.clone(),
                side:        Side::No,
                size_usd:    size_usd * opp.ask_no,
                limit_price: Some(opp.ask_no),
            },
        ]
    }

    // ── Simulate a cycle (Backtest / DryRun) ───────────────────────────────────

    fn record_simulated_cycle(&mut self, opp: &ArbOpportunity, size_usd: f64, portfolio: &mut Portfolio) {
        let cost   = size_usd * (opp.ask_yes + opp.ask_no);
        let payout = size_usd; // 1 contract pays $1.00 regardless of outcome
        let profit = payout - cost;

        portfolio.balance_usdc -= cost;
        portfolio.balance_usdc += payout;
        portfolio.realized_pnl += profit;
        self.total_profit += profit;

        self.cycles.push(ArbCycle {
            market_slug: opp.market_slug.clone(),
            edge:         opp.edge,
            position_usd: size_usd,
            profit_usd:   profit,
            success:      true,
            timestamp:    Utc::now(),
        });

        info!(
            "[arb_binary] simulated cycle on {}: edge={:.3}% profit=${:.4}",
            opp.market_slug, opp.edge * 100.0, profit
        );
    }
}

// ── StrategyEngine impl ───────────────────────────────────────────────────────

impl StrategyEngine for ArbBinaryEngine {
    fn name(&self) -> &str {
        strategy_core::engines::ARB_BINARY
    }

    async fn initialize(&mut self, mode: ExecutionMode, _portfolio: &Portfolio) -> Result<(), EngineError> {
        self.mode = mode;
        self.cycles.clear();
        self.total_profit = 0.0;
        self.token_cache.clear();
        info!("[arb_binary] initialised in {} mode, monitoring {} markets",
            self.mode, self.config.markets.len());
        Ok(())
    }

    /// `on_tick` is called by the candle runner but arb is book-driven.
    /// We use it in Backtest mode: interpret candle close as YES mid-price.
    async fn on_tick(&mut self, snap: &MarketSnapshot, portfolio: &mut Portfolio) -> Result<Vec<OrderIntent>, EngineError> {
        if self.mode != ExecutionMode::Backtest {
            return Ok(vec![]);
        }

        // In backtest we receive candle data as BTC price — not a Polymarket yes/no price.
        // For binary arb backtest, use candle.close as a proxy for YES mid and
        // derive NO = 1 - YES (binary market invariant).
        let candle = match &snap.candle {
            Some(c) => c,
            None => return Ok(vec![]),
        };

        // Interpret close as YES probability (clamp to realistic range).
        let yes_mid = candle.close.clamp(0.01, 0.99);
        let no_mid  = (1.0 - yes_mid).clamp(0.01, 0.99);

        if let Some(opp) = Self::detect_from_mid(yes_mid, no_mid, &snap.slug, &self.config) {
            let size = self.size_for(&opp, portfolio.balance_usdc);
            if size > 0.0 && portfolio.balance_usdc >= size {
                debug!("[arb_binary:backtest] opportunity on {} edge={:.3}", snap.slug, opp.edge);
                self.record_simulated_cycle(&opp, size, portfolio);
            }
        }

        Ok(vec![])
    }

    /// `on_book` is the primary entry point for DryRun and Live modes.
    async fn on_book(&mut self, snap: &BookSnapshot, portfolio: &mut Portfolio) -> Result<Vec<OrderIntent>, EngineError> {
        let opp = match Self::detect(snap, &self.config) {
            Some(o) => o,
            None => return Ok(vec![]),
        };

        let size = self.size_for(&opp, portfolio.balance_usdc);
        if size <= 0.0 {
            warn!("[arb_binary] opportunity found but size=0 (balance={:.2})", portfolio.balance_usdc);
            return Ok(vec![]);
        }

        info!(
            "[arb_binary] opportunity: {} ask_yes={:.4} ask_no={:.4} edge={:.3}% size=${:.2}",
            opp.market_slug, opp.ask_yes, opp.ask_no, opp.edge * 100.0, size
        );

        match self.mode {
            ExecutionMode::Backtest => {
                // Should not happen (on_book not called in backtest), but handle gracefully.
                self.record_simulated_cycle(&opp, size, portfolio);
                Ok(vec![])
            }
            ExecutionMode::DryRun => {
                // Simulate: debit cost, credit payout immediately.
                self.record_simulated_cycle(&opp, size, portfolio);
                Ok(vec![])
            }
            ExecutionMode::Live => {
                // Emit both intents — runner places real orders.
                Ok(self.make_intents(&opp, size))
            }
        }
    }

    /// Receives execution feedback for the Live path.
    async fn on_event(&mut self, event: EngineEvent, _portfolio: &mut Portfolio) -> Result<(), EngineError> {
        match &event {
            EngineEvent::PartialFill { order_id, token_id, side, filled_size, .. } => {
                warn!(
                    "[arb_binary] PARTIAL FILL order={} token={} side={:?} filled={}",
                    order_id, token_id, side, filled_size
                );
                // Find the affected market slug from the token cache.
                let slug = self.token_cache.iter()
                    .find(|(_, (yes, no))| yes == token_id || no == token_id)
                    .map(|(s, _)| s.clone())
                    .unwrap_or_else(|| "unknown".to_string());

                let (yes_id, no_id) = self.token_cache.get(&slug)
                    .cloned()
                    .unwrap_or_else(|| (token_id.clone(), token_id.clone()));

                let record = PartialFillRecord {
                    market_slug:     slug,
                    yes_token_id:    yes_id,
                    no_token_id:     no_id,
                    filled_side:     side.clone(),
                    filled_size_usd: *filled_size,
                    filled_price:    0.0, // populated by runner from order state
                    detected_at:     Utc::now(),
                };

                if let Ok(mut q) = self.partial_fills.lock() {
                    q.push(record);
                }
            }
            EngineEvent::OrderFilled { token_id, size, fee, .. } => {
                debug!("[arb_binary] fill confirmed token={} size={} fee={}", token_id, size, fee);
            }
            _ => {}
        }
        Ok(())
    }

    async fn finalize(&mut self, _portfolio: &Portfolio) -> EngineMetrics {
        let n = self.cycles.len() as u32;
        let wins = self.cycles.iter().filter(|c| c.profit_usd > 0.0).count() as u32;
        let win_rate = if n > 0 { wins as f64 / n as f64 * 100.0 } else { 0.0 };

        // Simplified Sharpe: assume daily cycle, std of profits.
        let profits: Vec<f64> = self.cycles.iter().map(|c| c.profit_usd).collect();
        let mean = if profits.is_empty() { 0.0 } else {
            profits.iter().sum::<f64>() / profits.len() as f64
        };
        let variance = if profits.len() < 2 { 0.0 } else {
            profits.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / (profits.len() - 1) as f64
        };
        let std_dev = variance.sqrt();
        let sharpe = if std_dev > 0.0 { mean / std_dev * (252_f64.sqrt()) } else { 0.0 };

        let analysis = if n == 0 {
            "No arbitrage opportunities detected during this run.".to_string()
        } else {
            format!(
                "arb_binary: {} cycles, {wins} wins ({win_rate:.1}% win rate), \
                 total profit ${:.4}, avg edge {:.3}%, Sharpe {sharpe:.2}. \
                 Note: backtest edges use mid-price proxy (confidence: low).",
                n, self.total_profit,
                self.cycles.iter().map(|c| c.edge).sum::<f64>() / n as f64
            )
        };

        let mut extra = HashMap::new();
        extra.insert("cycles".to_string(), n as f64);
        extra.insert("total_profit_usd".to_string(), self.total_profit);
        extra.insert("partial_fills".to_string(),
            self.partial_fills.lock().map(|q| q.len() as f64).unwrap_or(0.0));

        EngineMetrics {
            total_return_pct: if _portfolio.initial_balance > 0.0 {
                self.total_profit / _portfolio.initial_balance * 100.0
            } else { 0.0 },
            sharpe_ratio: sharpe,
            max_drawdown_pct: 0.0, // arb has no directional exposure
            win_rate_pct: win_rate,
            total_trades: n * 2, // 2 orders per cycle
            analysis,
            extra,
            data_confidence: if self.mode == ExecutionMode::Backtest {
                "low".to_string()
            } else {
                "high".to_string()
            },
        }
    }
}

// ── Live book-polling loop (called by runner_loop) ────────────────────────────

/// Runs the arb-binary engine in a continuous poll loop.
/// Called from `runner_loop` when `kind = "arb_binary"`.
///
/// Uses the Polymarket CLOB price API to approximate best-ask prices.
/// For production sub-second detection, replace with WebSocket CLOB subscription.
pub async fn run_arb_binary_loop(
    store: Arc<crate::strategy_runner::StrategyRunnerStore>,
    config: crate::strategy_runner::RunnerConfig,
    _workspace_dir: std::path::PathBuf,
) {
    use crate::strategy_runner::*;

    let id = config.id.clone();

    // Parse engine config from RunnerConfig.
    //
    // When `series_id` is set, slugs are re-resolved each poll inside the
    // loop below — this lets a single live runner ride consecutive 5m / 15m
    // / 1h windows of a recurring Polymarket market.  When `series_id` is
    // empty, we fall back to the historical comma-separated `symbol` field.
    let initial_markets: Vec<String> = if let Some(sid) = config.series_id.as_deref() {
        if !sid.is_empty() {
            vec![] // resolved per-tick
        } else {
            config
                .symbol
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
    } else {
        config
            .symbol
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let arb_cfg = ArbBinaryConfig {
        markets:           initial_markets,
        min_edge_pct:      config.threshold.unwrap_or(0.005),
        max_position_usd:  config.initial_balance * config.live_sizing_value.max(0.01),
        liquidity_floor_usd: 100.0,
        fee_pct:           config.fee_pct / 100.0,
        poll_secs:         60,
        max_concurrent:    5,
    };

    let mode = match config.mode.as_str() {
        "live" => ExecutionMode::Live,
        _      => ExecutionMode::DryRun,
    };

    let mut engine = ArbBinaryEngine::new(arb_cfg.clone());
    let mut portfolio = Portfolio::new(config.initial_balance);

    if let Err(e) = engine.initialize(mode.clone(), &portfolio).await {
        set_runner_error(&store, &id, &format!("arb_binary init failed: {e}"));
        return;
    }

    set_runner_status(&store, &id, "running");

    let poll = Duration::from_secs(arb_cfg.poll_secs);
    let series_id = config.series_id.clone();

    loop {
        // Re-resolve slugs each tick when the runner is bound to a recurring
        // series (e.g. BTC 5m UP/DOWN); otherwise reuse the static list.
        let active_markets: Vec<String> = if series_id.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
            crate::engines::series_helper::engine_market_slugs(
                series_id.as_deref(),
                &config.symbol,
            )
            .await
        } else {
            arb_cfg.markets.clone()
        };

        // For each configured market, fetch best-ask prices via REST API.
        for slug in &active_markets {
            // Resolve token IDs (with cache).
            let (yes_id, no_id) = match engine.token_cache.get(slug) {
                Some(ids) => ids.clone(),
                None => {
                    match polymarket_trader::markets::get_market(slug).await {
                        Ok(m) => {
                            let ids = (m.yes_token_id.clone(), m.no_token_id.clone());
                            engine.token_cache.insert(slug.clone(), ids.clone());
                            ids
                        }
                        Err(e) => {
                            warn!("[arb_binary] could not resolve market {slug}: {e}");
                            continue;
                        }
                    }
                }
            };

            // Fetch best-ask prices.
            let (ask_yes, ask_no) = tokio::join!(
                polymarket_trader::markets::get_market_price(&yes_id),
                polymarket_trader::markets::get_market_price(&no_id),
            );

            let ask_yes = match ask_yes { Ok(p) => p, Err(e) => { warn!("yes price err: {e}"); continue; } };
            let ask_no  = match ask_no  { Ok(p) => p, Err(e) => { warn!("no price err: {e}");  continue; } };

            // Build a BookSnapshot with approximate depth (REST API doesn't give depth).
            let snap = BookSnapshot {
                market_id: yes_id.clone(),
                slug:      slug.clone(),
                yes: BookLevel { best_ask: ask_yes, best_bid: ask_yes - 0.01, ask_depth_usd: 1000.0, bid_depth_usd: 800.0 },
                no:  BookLevel { best_ask: ask_no,  best_bid: ask_no  - 0.01, ask_depth_usd: 1000.0, bid_depth_usd: 800.0 },
                timestamp: Utc::now(),
                meta: Default::default(),
            };

            match engine.on_book(&snap, &mut portfolio).await {
                Ok(intents) if !intents.is_empty() && mode.places_real_orders() => {
                    // Live mode: place real orders.
                    if let Some(creds) = &config.poly_creds {
                        let client = polymarket_trader::orders::ClobClient::new(creds.clone());
                        let mut yes_ok = false;
                        let mut no_ok  = false;

                        // Place both orders in parallel.
                        let (r_yes, r_no) = tokio::join!(
                            client.create_limit_order(&yes_id, polymarket_trader::orders::Side::Buy, ask_yes, portfolio.balance_usdc * 0.1),
                            client.create_limit_order(&no_id,  polymarket_trader::orders::Side::Buy, ask_no,  portfolio.balance_usdc * 0.1),
                        );

                        if let Ok(o) = r_yes { info!("[arb_binary] YES order {}", o.order_id); yes_ok = true; }
                        if let Ok(o) = r_no  { info!("[arb_binary] NO  order {}", o.order_id); no_ok  = true; }

                        // Partial fill detection.
                        if yes_ok != no_ok {
                            let filled_side = if yes_ok { Side::Yes } else { Side::No };
                            let record = PartialFillRecord {
                                market_slug:     slug.clone(),
                                yes_token_id:    yes_id.clone(),
                                no_token_id:     no_id.clone(),
                                filled_side,
                                filled_size_usd: portfolio.balance_usdc * 0.1,
                                filled_price:    if yes_ok { ask_yes } else { ask_no },
                                detected_at:     Utc::now(),
                            };
                            if let Ok(mut q) = engine.partial_fills.lock() {
                                q.push(record);
                            }
                            warn!("[arb_binary] PARTIAL FILL on {slug} — one side failed");
                        }
                    }
                }
                Ok(_) => {} // DryRun or no opportunity
                Err(e) => warn!("[arb_binary] on_book error: {e}"),
            }
        }

        // Update store metrics.
        let metrics = engine.finalize(&portfolio).await;
        update_store_result(&store, &id, &portfolio, &metrics);

        tokio::time::sleep(poll).await;
    }
}

// Helper — update the runner result in the store.
fn update_store_result(
    store: &Arc<crate::strategy_runner::StrategyRunnerStore>,
    id: &str,
    portfolio: &Portfolio,
    metrics: &EngineMetrics,
) {
    store.update_result(id, |r| {
        r.total_return_pct = metrics.total_return_pct;
        r.balance          = portfolio.balance_usdc;
        r.win_rate_pct     = metrics.win_rate_pct;
        r.total_trades     = metrics.total_trades;
        r.sharpe_ratio     = metrics.sharpe_ratio;
        r.analysis         = metrics.analysis.clone();
    });
}
