//! MINTING-MM engine (MINT-04 / MINT-01)
//!
//! **Strategy:** Mint 1 USDC → 1 YES + 1 NO token via the Gnosis CTF contract,
//! then place limit *sell* orders on both sides at `mid + premium_cents`.
//! When both fills arrive the cycle closes with `2 × premium_cents` profit per
//! USDC deployed.  If the cycle times out (`cycle_hours`) without full fill, the
//! remaining tokens are merged back to USDC (merge = splitPosition reversed).
//!
//! ## FSM
//! ```
//! Idle ──mint──► Minted ──orders──► OrdersPlaced ──fills──► Filled ──reset──► Cycled → Idle
//!                                         │
//!                              timeout / merge
//!                                         ▼
//!                                      Idle (with merge)
//! ```
//!
//! ## Modes
//! - **Backtest**: simulates mint+fill using historical mid-prices.  Spread
//!   proxy = `high - low` of the candle.  Profit modelled when spread > 2×premium.
//! - **DryRun**: connects to live CLOB for real prices but calls CTF in DryRun
//!   mode (`private_key = None`) — no on-chain tx, no real orders.
//! - **Live**: submits real `splitPosition` tx via `polymarket_trader::ctf`, then
//!   places real limit sells via `ClobClient`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use strategy_core::{
    engine::StrategyEngine,
    types::{
        BookSnapshot, EngineError, EngineEvent, EngineMetrics, ExecutionMode,
        MarketSnapshot, OrderIntent, Portfolio, Side,
    },
};
use tracing::{debug, info, warn};

// ── Config ────────────────────────────────────────────────────────────────────

/// Minting-MM engine configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MintingMmConfig {
    /// Polymarket market slugs to operate on.
    pub markets: Vec<String>,

    /// Target premium per token side in USD (e.g. 0.02 = 2 cents).
    #[serde(default = "default_premium")]
    pub premium_cents: f64,

    /// Maximum USDC to commit per minting cycle.
    #[serde(default = "default_cycle_usd")]
    pub max_cycle_usd: f64,

    /// Hours to wait for fills before timing out and merging back.
    #[serde(default = "default_cycle_hours")]
    pub cycle_hours: u64,

    /// Annual ROI target used for cycle-sizing heuristic (e.g. 0.40 = 40%).
    #[serde(default = "default_target_apy")]
    pub target_apy: f64,

    /// Minimum bid-ask spread (in price units) for the market to be worth entering.
    #[serde(default = "default_min_spread")]
    pub min_spread: f64,

    /// How often to poll prices in DryRun/Live mode (seconds).
    #[serde(default = "default_poll")]
    pub poll_secs: u64,

    /// Collateral token: "usdc_e" or "pusd".
    #[serde(default = "default_collateral")]
    pub collateral: String,
}

fn default_premium()      -> f64   { 0.02 }
fn default_cycle_usd()    -> f64   { 200.0 }
fn default_cycle_hours()  -> u64   { 24 }
fn default_target_apy()   -> f64   { 0.40 }
fn default_min_spread()   -> f64   { 0.04 }
fn default_poll()         -> u64   { 30 }
fn default_collateral()   -> String { "usdc_e".to_string() }

impl Default for MintingMmConfig {
    fn default() -> Self {
        Self {
            markets:      vec![],
            premium_cents: default_premium(),
            max_cycle_usd: default_cycle_usd(),
            cycle_hours:   default_cycle_hours(),
            target_apy:    default_target_apy(),
            min_spread:    default_min_spread(),
            poll_secs:     default_poll(),
            collateral:    default_collateral(),
        }
    }
}

// ── FSM ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum MmState {
    /// Waiting to start a new cycle.
    Idle,
    /// CTF split executed; YES + NO tokens held.
    Minted {
        condition_id: String,
        amount_usdc:  f64,
        minted_at:    DateTime<Utc>,
    },
    /// Limit sell orders are resting on the CLOB.
    OrdersPlaced {
        condition_id:  String,
        amount_usdc:   f64,
        yes_order_id:  String,
        no_order_id:   String,
        sell_price_yes: f64,
        sell_price_no:  f64,
        placed_at:     DateTime<Utc>,
    },
    /// Both fills received; ready to tally and reset.
    Filled {
        condition_id: String,
        amount_usdc:  f64,
        gross_recv:   f64,
        filled_at:    DateTime<Utc>,
    },
    /// Cycle complete — engine records metrics and returns to Idle.
    Cycled,
}

// ── Cycle record ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct MintCycle {
    market_slug:  String,
    amount_usdc:  f64,
    profit_usd:   f64,
    outcome:      CycleOutcome,
    duration_hrs: f64,
    timestamp:    DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
enum CycleOutcome {
    BothFilled,
    Timeout,    // merged back, recovered ≈ cost
    Partial,    // one side filled, one merged
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct MintingMmEngine {
    config:  MintingMmConfig,
    mode:    ExecutionMode,
    /// Per-market FSM state (one FSM per market slug).
    states:  HashMap<String, MmState>,
    cycles:  Vec<MintCycle>,
    total_profit: f64,
}

impl MintingMmEngine {
    pub fn new(config: MintingMmConfig) -> Self {
        Self {
            config,
            mode: ExecutionMode::Backtest,
            states: HashMap::new(),
            cycles: vec![],
            total_profit: 0.0,
        }
    }

    /// Returns the current FSM state for a market slug (for testing).
    pub fn state_of(&self, slug: &str) -> MmState {
        self.states.get(slug).cloned().unwrap_or(MmState::Idle)
    }

    // ── FSM helpers ────────────────────────────────────────────────────────────

    fn state_for(&mut self, slug: &str) -> &mut MmState {
        self.states.entry(slug.to_string()).or_insert(MmState::Idle)
    }

    fn is_timed_out(placed_at: &DateTime<Utc>, cycle_hours: u64) -> bool {
        let elapsed = Utc::now().signed_duration_since(*placed_at);
        elapsed.num_hours() >= cycle_hours as i64
    }

    // ── Sizing ─────────────────────────────────────────────────────────────────

    fn cycle_size(&self, balance: f64) -> f64 {
        // Use ≤ 20% of balance per cycle, capped at max_cycle_usd.
        let balance_cap = balance * 0.20;
        self.config.max_cycle_usd.min(balance_cap).max(0.0)
    }

    // ── Backtest simulation ────────────────────────────────────────────────────

    /// Simulate a minting cycle using candle OHLC data.
    /// Spread proxy = high - low.  If spread > 2×premium → both fills, else timeout.
    fn simulate_backtest_cycle(
        &mut self,
        slug: &str,
        yes_mid: f64,
        no_mid: f64,
        spread_proxy: f64,
        portfolio: &mut Portfolio,
    ) {
        let size = self.cycle_size(portfolio.balance_usdc);
        if size < 1.0 {
            return;
        }

        let premium = self.config.premium_cents;
        let spread_ok = spread_proxy >= 2.0 * premium;

        let (profit, outcome) = if spread_ok {
            // Both limit sells fill: collect (mid + premium) × 2 per $1 minted.
            let gross = size * (yes_mid + premium + no_mid + premium);
            let net   = gross - size; // cost was `size` USDC to mint
            (net, CycleOutcome::BothFilled)
        } else {
            // Timeout: merge back; small loss from fee (~0.2%).
            let recovered = size * (1.0 - 0.002);
            (recovered - size, CycleOutcome::Timeout)
        };

        portfolio.balance_usdc += profit;
        portfolio.realized_pnl += profit;
        self.total_profit += profit;

        self.cycles.push(MintCycle {
            market_slug: slug.to_string(),
            amount_usdc: size,
            profit_usd:  profit,
            outcome,
            duration_hrs: if spread_ok { 4.0 } else { self.config.cycle_hours as f64 },
            timestamp: Utc::now(),
        });

        debug!(
            "[minting_mm:backtest] {slug} yes_mid={yes_mid:.3} no_mid={no_mid:.3} \
             spread={spread_proxy:.3} profit=${profit:.4}"
        );
    }

    // ── DryRun book-snap handler ───────────────────────────────────────────────

    /// Simulate the full FSM transition for one book snapshot (DryRun).
    fn dryrun_on_book(&mut self, snap: &BookSnapshot, portfolio: &mut Portfolio) {
        let slug = snap.slug.clone();
        let spread = snap.yes.best_ask - snap.yes.best_bid;

        if spread < self.config.min_spread {
            debug!("[minting_mm:dryrun] {slug} spread={spread:.3} < min={:.3}", self.config.min_spread);
            return;
        }

        let state = self.states.entry(slug.clone()).or_insert(MmState::Idle).clone();

        match state {
            MmState::Idle => {
                let size = self.cycle_size(portfolio.balance_usdc);
                if size < 1.0 {
                    return;
                }
                let cond_id = format!("sim-{slug}-{}", Utc::now().timestamp());
                info!("[minting_mm:dryrun] {slug} Idle→Minted size=${size:.2}");
                portfolio.balance_usdc -= size;
                *self.states.get_mut(&slug).unwrap() = MmState::Minted {
                    condition_id: cond_id,
                    amount_usdc:  size,
                    minted_at:    Utc::now(),
                };
            }
            MmState::Minted { condition_id, amount_usdc, .. } => {
                let yes_sell = snap.yes.best_ask + self.config.premium_cents;
                let no_sell  = snap.no.best_ask  + self.config.premium_cents;
                info!("[minting_mm:dryrun] {slug} Minted→OrdersPlaced YES@{yes_sell:.3} NO@{no_sell:.3}");
                *self.states.get_mut(&slug).unwrap() = MmState::OrdersPlaced {
                    condition_id,
                    amount_usdc,
                    yes_order_id:  format!("dry-yes-{}", Utc::now().timestamp()),
                    no_order_id:   format!("dry-no-{}", Utc::now().timestamp()),
                    sell_price_yes: yes_sell,
                    sell_price_no:  no_sell,
                    placed_at:     Utc::now(),
                };
            }
            MmState::OrdersPlaced { condition_id: _, amount_usdc, sell_price_yes, sell_price_no, placed_at, .. } => {
                // In DryRun, simulate fill when market ask drops to our sell price.
                let ask_yes_now = snap.yes.best_ask;
                let ask_no_now  = snap.no.best_ask;
                let yes_filled  = ask_yes_now >= sell_price_yes - 0.005;
                let no_filled   = ask_no_now  >= sell_price_no  - 0.005;

                if yes_filled && no_filled {
                    let gross = amount_usdc * (sell_price_yes + sell_price_no);
                    info!("[minting_mm:dryrun] {slug} OrdersPlaced→Filled gross=${gross:.4}");
                    *self.states.get_mut(&slug).unwrap() = MmState::Filled {
                        condition_id: String::new(),
                        amount_usdc,
                        gross_recv:   gross,
                        filled_at:    Utc::now(),
                    };
                } else if Self::is_timed_out(&placed_at, self.config.cycle_hours) {
                    // Timeout: merge back.
                    let recovered = amount_usdc * (1.0 - 0.002);
                    let profit    = recovered - amount_usdc;
                    portfolio.balance_usdc += recovered;
                    portfolio.realized_pnl += profit;
                    self.total_profit      += profit;
                    warn!("[minting_mm:dryrun] {slug} timeout — merged back, profit=${profit:.4}");
                    self.cycles.push(MintCycle {
                        market_slug: slug.clone(),
                        amount_usdc,
                        profit_usd:  profit,
                        outcome:     CycleOutcome::Timeout,
                        duration_hrs: self.config.cycle_hours as f64,
                        timestamp:   Utc::now(),
                    });
                    *self.states.get_mut(&slug).unwrap() = MmState::Idle;
                }
            }
            MmState::Filled { amount_usdc, gross_recv, filled_at, .. } => {
                let profit = gross_recv - amount_usdc;
                let hrs    = Utc::now().signed_duration_since(filled_at).num_minutes() as f64 / 60.0;
                portfolio.balance_usdc += gross_recv;
                portfolio.realized_pnl += profit;
                self.total_profit      += profit;
                info!("[minting_mm:dryrun] {slug} Filled→Cycled profit=${profit:.4} in {hrs:.1}h");
                self.cycles.push(MintCycle {
                    market_slug: slug.clone(),
                    amount_usdc,
                    profit_usd:  profit,
                    outcome:     CycleOutcome::BothFilled,
                    duration_hrs: hrs,
                    timestamp:   Utc::now(),
                });
                *self.states.get_mut(&slug).unwrap() = MmState::Idle;
            }
            MmState::Cycled => {
                *self.states.get_mut(&slug).unwrap() = MmState::Idle;
            }
        }
    }

    // ── Live FSM step ──────────────────────────────────────────────────────────

    /// Returns the order intents needed for the current FSM step (Live mode).
    fn live_intents_from_book(&mut self, snap: &BookSnapshot, portfolio: &mut Portfolio) -> Vec<OrderIntent> {
        let slug   = snap.slug.clone();
        let spread = snap.yes.best_ask - snap.yes.best_bid;

        if spread < self.config.min_spread {
            return vec![];
        }

        let state = self.states.entry(slug.clone()).or_insert(MmState::Idle).clone();

        match state {
            MmState::Idle => {
                let size = self.cycle_size(portfolio.balance_usdc);
                if size < 1.0 {
                    return vec![];
                }
                // Signal the runner to call CTF mint and transition to Minted.
                info!("[minting_mm:live] {slug} Idle → requesting Mint ${size:.2}");
                // Return a Mint intent; runner_loop calls ctf::mint then transitions FSM.
                vec![OrderIntent::Mint {
                    market_slug: slug,
                    amount_usdc: size,
                    collateral:  self.config.collateral.clone(),
                }]
            }
            MmState::Minted { condition_id: _, amount_usdc, .. } => {
                let yes_sell = snap.yes.best_ask + self.config.premium_cents;
                let no_sell  = snap.no.best_ask  + self.config.premium_cents;
                // Emit sell intents for both sides.
                info!("[minting_mm:live] {slug} Minted → placing sell orders YES@{yes_sell:.3} NO@{no_sell:.3}");
                let yes_id = snap.market_id.clone();
                let no_id  = format!("{}_no", slug);
                vec![
                    OrderIntent::Sell {
                        token_id:    yes_id,
                        side:        Side::Yes,
                        size_usd:    amount_usdc,
                        limit_price: Some(yes_sell),
                    },
                    OrderIntent::Sell {
                        token_id:    no_id,
                        side:        Side::No,
                        size_usd:    amount_usdc,
                        limit_price: Some(no_sell),
                    },
                ]
            }
            MmState::OrdersPlaced { amount_usdc, placed_at, .. } => {
                // Check for timeout; if so emit Merge intent.
                if Self::is_timed_out(&placed_at, self.config.cycle_hours) {
                    warn!("[minting_mm:live] {slug} timeout — emitting Merge intent");
                    return vec![OrderIntent::Merge {
                        market_slug: slug,
                        amount_usdc,
                        collateral:  self.config.collateral.clone(),
                    }];
                }
                // Still waiting for fills — hold.
                vec![OrderIntent::Hold]
            }
            MmState::Filled { amount_usdc, gross_recv, .. } => {
                // Record the cycle and reset.
                let profit = gross_recv - amount_usdc;
                portfolio.balance_usdc += gross_recv;
                portfolio.realized_pnl += profit;
                self.total_profit      += profit;
                self.cycles.push(MintCycle {
                    market_slug: slug.clone(),
                    amount_usdc,
                    profit_usd:  profit,
                    outcome:     CycleOutcome::BothFilled,
                    duration_hrs: 0.0,
                    timestamp:   Utc::now(),
                });
                *self.states.get_mut(&slug).unwrap() = MmState::Idle;
                vec![]
            }
            MmState::Cycled => {
                *self.states.get_mut(&slug).unwrap() = MmState::Idle;
                vec![]
            }
        }
    }
}

// ── StrategyEngine impl ───────────────────────────────────────────────────────

impl StrategyEngine for MintingMmEngine {
    fn name(&self) -> &str {
        strategy_core::engines::MINTING_MM
    }

    async fn initialize(&mut self, mode: ExecutionMode, _portfolio: &Portfolio) -> Result<(), EngineError> {
        self.mode = mode;
        self.states.clear();
        self.cycles.clear();
        self.total_profit = 0.0;
        info!("[minting_mm] initialised in {} mode, {} markets",
            self.mode, self.config.markets.len());
        Ok(())
    }

    /// Backtest path: interpret candle OHLC as YES-price proxy.
    async fn on_tick(&mut self, snap: &MarketSnapshot, portfolio: &mut Portfolio) -> Result<Vec<OrderIntent>, EngineError> {
        if self.mode != ExecutionMode::Backtest {
            return Ok(vec![]);
        }

        let candle = match &snap.candle {
            Some(c) => c,
            None => return Ok(vec![]),
        };

        // YES mid proxy = candle close clamped to [0.05, 0.95].
        let yes_mid    = candle.close.clamp(0.05, 0.95);
        let no_mid     = 1.0 - yes_mid;
        let spread_proxy = candle.high - candle.low; // proxy for intraday spread

        self.simulate_backtest_cycle(&snap.slug, yes_mid, no_mid, spread_proxy, portfolio);

        Ok(vec![])
    }

    /// DryRun / Live path: driven by live book snapshots.
    async fn on_book(&mut self, snap: &BookSnapshot, portfolio: &mut Portfolio) -> Result<Vec<OrderIntent>, EngineError> {
        match self.mode {
            ExecutionMode::Backtest => Ok(vec![]), // backtest uses on_tick
            ExecutionMode::DryRun => {
                self.dryrun_on_book(snap, portfolio);
                Ok(vec![])
            }
            ExecutionMode::Live => Ok(self.live_intents_from_book(snap, portfolio)),
        }
    }

    /// Handle fill events from the CLOB (Live mode).
    async fn on_event(&mut self, event: EngineEvent, portfolio: &mut Portfolio) -> Result<(), EngineError> {
        match &event {
            EngineEvent::OrderFilled { token_id, size, fee, .. } => {
                info!("[minting_mm] fill: token={token_id} size={size} fee={fee}");
                // Advance FSM for the market that owns this token.
                let slug = self.states.iter()
                    .find(|(_, s)| match s {
                        MmState::OrdersPlaced { yes_order_id, no_order_id, .. } =>
                            yes_order_id.contains(token_id) || no_order_id.contains(token_id),
                        _ => false,
                    })
                    .map(|(s, _)| s.clone());

                if let Some(slug) = slug {
                    let state = self.states.get(&slug).cloned();
                    if let Some(MmState::OrdersPlaced {
                        condition_id,
                        amount_usdc,
                        sell_price_yes,
                        sell_price_no,
                        ..
                    }) = state {
                        let gross = amount_usdc * (sell_price_yes + sell_price_no);
                        *self.states.get_mut(&slug).unwrap() = MmState::Filled {
                            condition_id,
                            amount_usdc,
                            gross_recv: gross,
                            filled_at: Utc::now(),
                        };
                        // Credit portfolio immediately.
                        let profit = gross - amount_usdc - fee;
                        portfolio.balance_usdc += amount_usdc + profit;
                        portfolio.realized_pnl += profit;
                        self.total_profit      += profit;
                        self.cycles.push(MintCycle {
                            market_slug: slug,
                            amount_usdc,
                            profit_usd:  profit,
                            outcome:     CycleOutcome::BothFilled,
                            duration_hrs: 0.0,
                            timestamp:   Utc::now(),
                        });
                    }
                }
            }
            EngineEvent::MintConfirmed { condition_id, amount_usdc, market_slug } => {
                info!("[minting_mm] mint confirmed {market_slug} cond={condition_id} usdc={amount_usdc}");
                *self.states.entry(market_slug.clone()).or_insert(MmState::Idle) = MmState::Minted {
                    condition_id: condition_id.clone(),
                    amount_usdc:  *amount_usdc,
                    minted_at:    Utc::now(),
                };
            }
            EngineEvent::MergeConfirmed { condition_id: _, amount_usdc, market_slug } => {
                info!("[minting_mm] merge confirmed {market_slug}");
                let recovered = amount_usdc * (1.0 - 0.002);
                let profit    = recovered - amount_usdc;
                portfolio.balance_usdc += recovered;
                portfolio.realized_pnl += profit;
                self.total_profit      += profit;
                self.cycles.push(MintCycle {
                    market_slug: market_slug.clone(),
                    amount_usdc: *amount_usdc,
                    profit_usd:  profit,
                    outcome:     CycleOutcome::Timeout,
                    duration_hrs: self.config.cycle_hours as f64,
                    timestamp:   Utc::now(),
                });
                *self.states.entry(market_slug.clone()).or_insert(MmState::Idle) = MmState::Idle;
            }
            _ => {}
        }
        Ok(())
    }

    async fn finalize(&mut self, portfolio: &Portfolio) -> EngineMetrics {
        let n     = self.cycles.len() as u32;
        let wins  = self.cycles.iter().filter(|c| c.outcome == CycleOutcome::BothFilled).count() as u32;
        let win_rate = if n > 0 { wins as f64 / n as f64 * 100.0 } else { 0.0 };

        let profits: Vec<f64> = self.cycles.iter().map(|c| c.profit_usd).collect();
        let mean = if profits.is_empty() { 0.0 } else {
            profits.iter().sum::<f64>() / profits.len() as f64
        };
        let variance = if profits.len() < 2 { 0.0 } else {
            profits.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / (profits.len() - 1) as f64
        };
        let std_dev = variance.sqrt();
        let sharpe  = if std_dev > 0.0 { mean / std_dev * (252_f64.sqrt()) } else { 0.0 };

        let avg_duration = if n > 0 {
            self.cycles.iter().map(|c| c.duration_hrs).sum::<f64>() / n as f64
        } else { 0.0 };

        let analysis = if n == 0 {
            "No minting cycles completed during this run.".to_string()
        } else {
            format!(
                "minting_mm: {n} cycles ({wins} full-fill, {} timeout), \
                 win rate {win_rate:.1}%, total profit ${:.4}, \
                 avg cycle {avg_duration:.1}h, Sharpe {sharpe:.2}. \
                 Premium target: ${:.3}/token/side.",
                n - wins, self.total_profit, self.config.premium_cents
            )
        };

        let mut extra = HashMap::new();
        extra.insert("cycles_total".to_string(),   n as f64);
        extra.insert("cycles_filled".to_string(),  wins as f64);
        extra.insert("cycles_timeout".to_string(), (n - wins) as f64);
        extra.insert("total_profit_usd".to_string(), self.total_profit);
        extra.insert("avg_duration_hrs".to_string(), avg_duration);

        EngineMetrics {
            total_return_pct: if portfolio.initial_balance > 0.0 {
                self.total_profit / portfolio.initial_balance * 100.0
            } else { 0.0 },
            sharpe_ratio:     sharpe,
            max_drawdown_pct: 0.0,
            win_rate_pct:     win_rate,
            total_trades:     n * 2,
            analysis,
            extra,
            data_confidence: if self.mode == ExecutionMode::Backtest {
                "medium".to_string() // spread proxy is better than pure mid-price arb
            } else {
                "high".to_string()
            },
        }
    }
}

// ── Live poll loop (called by runner_loop) ────────────────────────────────────

/// Runs the minting-MM engine in a continuous poll loop.
/// Called from `runner_loop` when `kind = "minting_mm"`.
pub async fn run_minting_mm_loop(
    store: Arc<crate::strategy_runner::StrategyRunnerStore>,
    config: crate::strategy_runner::RunnerConfig,
    _workspace_dir: std::path::PathBuf,
) {
    use crate::strategy_runner::{set_runner_error, set_runner_status};

    let id = config.id.clone();

    let mm_cfg = MintingMmConfig {
        markets:       if config.symbol.is_empty() { vec![] } else { vec![config.symbol.clone()] },
        premium_cents: config.threshold.unwrap_or(0.02),
        max_cycle_usd: config.initial_balance * 0.20,
        cycle_hours:   24,
        target_apy:    0.40,
        min_spread:    0.04,
        poll_secs:     30,
        collateral:    "usdc_e".to_string(),
    };

    let mode = match config.mode.as_str() {
        "live" => ExecutionMode::Live,
        _      => ExecutionMode::DryRun,
    };

    let mut engine    = MintingMmEngine::new(mm_cfg.clone());
    let mut portfolio = Portfolio::new(config.initial_balance);

    if let Err(e) = engine.initialize(mode.clone(), &portfolio).await {
        set_runner_error(&store, &id, &format!("minting_mm init failed: {e}"));
        return;
    }

    set_runner_status(&store, &id, "running");

    let poll = Duration::from_secs(mm_cfg.poll_secs);

    loop {
        for slug in &mm_cfg.markets.clone() {
            // Resolve token IDs.
            let market = match polymarket_trader::markets::get_market(slug).await {
                Ok(m)  => m,
                Err(e) => { warn!("[minting_mm] market resolve {slug}: {e}"); continue; }
            };

            let (ask_yes, ask_no) = tokio::join!(
                polymarket_trader::markets::get_market_price(&market.yes_token_id),
                polymarket_trader::markets::get_market_price(&market.no_token_id),
            );
            let ask_yes = match ask_yes { Ok(p) => p, Err(e) => { warn!("yes: {e}"); continue; } };
            let ask_no  = match ask_no  { Ok(p) => p, Err(e) => { warn!("no:  {e}"); continue; } };

            let snap = BookSnapshot {
                market_id: market.yes_token_id.clone(),
                slug:      slug.clone(),
                yes: strategy_core::types::BookLevel {
                    best_ask: ask_yes, best_bid: ask_yes - 0.01,
                    ask_depth_usd: 1000.0, bid_depth_usd: 800.0,
                },
                no: strategy_core::types::BookLevel {
                    best_ask: ask_no,  best_bid: ask_no  - 0.01,
                    ask_depth_usd: 1000.0, bid_depth_usd: 800.0,
                },
                timestamp: Utc::now(),
                meta: Default::default(),
            };

            match engine.on_book(&snap, &mut portfolio).await {
                Ok(intents) if !intents.is_empty() && mode.places_real_orders() => {
                    for intent in intents {
                        handle_live_intent(intent, &snap, &config, &mut engine, &mut portfolio, slug).await;
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("[minting_mm] on_book error: {e}"),
            }
        }

        let metrics = engine.finalize(&portfolio).await;
        update_store_result(&store, &id, &portfolio, &metrics);

        tokio::time::sleep(poll).await;
    }
}

/// Dispatch a single `OrderIntent` in Live mode.
async fn handle_live_intent(
    intent: OrderIntent,
    snap: &BookSnapshot,
    config: &crate::strategy_runner::RunnerConfig,
    engine: &mut MintingMmEngine,
    portfolio: &mut Portfolio,
    slug: &str,
) {
    match intent {
        OrderIntent::Mint { amount_usdc, collateral, .. } => {
            let creds = match &config.poly_creds {
                Some(c) => c,
                None    => { warn!("[minting_mm] Mint requested but no creds"); return; }
            };
            let condition_id = match polymarket_trader::markets::get_market(slug).await {
                Ok(m) => m.condition_id,
                Err(e) => { warn!("[minting_mm] condition_id lookup: {e}"); return; }
            };
            let wallet = &creds.wallet_address;
            let pk     = creds.private_key.as_deref();

            let collateral_addr = if collateral == "pusd" {
                polymarket_trader::ctf::PUSD_CONTRACT
            } else {
                polymarket_trader::ctf::USDC_E_CONTRACT
            };

            match polymarket_trader::ctf::mint(&condition_id, amount_usdc, collateral_addr, wallet, pk).await {
                Ok(result) => {
                    info!("[minting_mm] mint tx={}", result.tx_hash);
                    portfolio.balance_usdc -= amount_usdc;
                    let event = EngineEvent::MintConfirmed {
                        condition_id: condition_id.clone(),
                        amount_usdc,
                        market_slug:  slug.to_string(),
                    };
                    let _ = engine.on_event(event, portfolio).await;
                }
                Err(e) => warn!("[minting_mm] mint failed: {e}"),
            }
        }
        OrderIntent::Sell { token_id, side, size_usd, limit_price } => {
            if let Some(creds) = &config.poly_creds {
                let client = polymarket_trader::orders::ClobClient::new(creds.clone());
                let price  = limit_price.unwrap_or(0.0);
                let pm_side = match side {
                    Side::Yes => polymarket_trader::orders::Side::Sell,
                    Side::No  => polymarket_trader::orders::Side::Sell,
                };
                match client.create_limit_order(&token_id, pm_side, price, size_usd).await {
                    Ok(o)  => {
                        info!("[minting_mm] sell order {} side={:?}", o.order_id, side);
                        // Transition FSM to OrdersPlaced via state mutation.
                        let state = engine.states.entry(slug.to_string()).or_insert(MmState::Idle);
                        if let MmState::Minted { condition_id, amount_usdc, .. } = state.clone() {
                            let yes_sell = snap.yes.best_ask + engine.config.premium_cents;
                            let no_sell  = snap.no.best_ask  + engine.config.premium_cents;
                            *state = MmState::OrdersPlaced {
                                condition_id,
                                amount_usdc,
                                yes_order_id:  o.order_id.clone(),
                                no_order_id:   o.order_id,
                                sell_price_yes: yes_sell,
                                sell_price_no:  no_sell,
                                placed_at:     Utc::now(),
                            };
                        }
                    }
                    Err(e) => warn!("[minting_mm] sell order failed: {e}"),
                }
            }
        }
        OrderIntent::Merge { amount_usdc, collateral, .. } => {
            if let Some(creds) = &config.poly_creds {
                let condition_id = match polymarket_trader::markets::get_market(slug).await {
                    Ok(m) => m.condition_id,
                    Err(e) => { warn!("[minting_mm] condition_id lookup: {e}"); return; }
                };
                let collateral_addr = if collateral == "pusd" {
                    polymarket_trader::ctf::PUSD_CONTRACT
                } else {
                    polymarket_trader::ctf::USDC_E_CONTRACT
                };
                match polymarket_trader::ctf::merge(&condition_id, amount_usdc, collateral_addr, &creds.wallet_address, creds.private_key.as_deref()).await {
                    Ok(r) => {
                        info!("[minting_mm] merge tx={}", r.tx_hash);
                        let event = EngineEvent::MergeConfirmed {
                            condition_id,
                            amount_usdc,
                            market_slug: slug.to_string(),
                        };
                        let _ = engine.on_event(event, portfolio).await;
                    }
                    Err(e) => warn!("[minting_mm] merge failed: {e}"),
                }
            }
        }
        _ => {}
    }
}

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
