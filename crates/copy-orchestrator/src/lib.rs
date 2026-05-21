use std::sync::Arc;
use tokio::sync::Mutex;
use wallet_tracker::polymarket::PolymarketTracker;
use wallet_tracker::traits::Venue;

pub mod consensus;
pub mod dispatcher;
pub mod mirror;
pub mod sizing;
pub mod watchlist;

use watchlist::Watchlist;
use consensus::ConsensusAccumulator;
use mirror::MirrorTracker;
use sizing::SizingEngine;

/// Central dispatcher for copy-trading modes (Discovery, Consensus, Mirror).
pub struct Orchestrator {
    pub watchlist: Arc<Mutex<Watchlist>>,
    pub consensus: Arc<Mutex<ConsensusAccumulator>>,
    pub mirror: Arc<Mutex<MirrorTracker>>,
    pub sizing: SizingEngine,
    pub risk_gate: Arc<risk_manager::RiskGate>,
    /// Total capital under management (USD)
    pub capital: Arc<Mutex<f64>>,
    /// Background poller for Polymarket leader fills.
    pub polymarket_tracker: Arc<PolymarketTracker>,
}

impl Orchestrator {
    pub fn new(
        risk_gate: Arc<risk_manager::RiskGate>,
        capital_usd: f64,
    ) -> Self {
        Self {
            watchlist: Arc::new(Mutex::new(Watchlist::new())),
            consensus: Arc::new(Mutex::new(ConsensusAccumulator::new())),
            mirror: Arc::new(Mutex::new(MirrorTracker::new())),
            sizing: SizingEngine::new(),
            risk_gate,
            capital: Arc::new(Mutex::new(capital_usd)),
            polymarket_tracker: Arc::new(PolymarketTracker::new(String::new())),
        }
    }

    pub async fn set_capital(&self, capital: f64) {
        *self.capital.lock().await = capital;
        self.risk_gate.set_total_capital(capital);
    }

    /// Recompute the Polymarket tracker's leader list from the current
    /// watchlist + an optional list of additional candidate addresses to
    /// shadow-track. Idempotent — call after every add/remove.
    pub async fn refresh_polymarket_tracker(&self, extra_candidates: Vec<String>) {
        let watchlist = self.watchlist.lock().await;
        let mut leaders: Vec<String> = watchlist
            .list()
            .iter()
            .filter(|e| e.venue.eq_ignore_ascii_case("polymarket"))
            .map(|e| e.address.to_lowercase())
            .collect();
        for addr in extra_candidates {
            let lower = addr.to_lowercase();
            if !leaders.contains(&lower) {
                leaders.push(lower);
            }
        }
        drop(watchlist);
        tracing::info!(
            "[Orchestrator] refreshing Polymarket tracker with {} leader(s)",
            leaders.len()
        );
        self.polymarket_tracker.set_leaders(leaders).await;
    }
}

/// Spawn the long-lived task that consumes leader fills from the Polymarket
/// tracker and feeds them through the dispatcher pipeline.  Returns once the
/// background task is running.
pub fn spawn_polymarket_dispatch_loop(
    orchestrator: Arc<Orchestrator>,
    consensus_window_secs: i64,
    consensus_n: usize,
    consensus_m: usize,
    mirror_enabled_globally: bool,
) {
    let mut rx = orchestrator.polymarket_tracker.subscribe_events();
    orchestrator.polymarket_tracker.start_polling(5);

    tokio::spawn(async move {
        tracing::info!("[Orchestrator] Polymarket dispatch loop started");
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if event.venue != Venue::Polymarket {
                        continue;
                    }
                    let watchlist = orchestrator.watchlist.lock().await;
                    let mut consensus = orchestrator.consensus.lock().await;
                    let capital = *orchestrator.capital.lock().await;
                    let result = dispatcher::dispatch_leader_fill(
                        &event,
                        &watchlist,
                        &mut consensus,
                        &orchestrator.risk_gate,
                        capital,
                        consensus_n,
                        consensus_m,
                        consensus_window_secs,
                        mirror_enabled_globally,
                    )
                    .await;
                    tracing::info!(
                        "[Orchestrator] dispatched fill leader={} slug={} result={:?}",
                        event.leader, event.symbol, result
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        "[Orchestrator] dispatch loop lagged behind by {n} events"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::warn!("[Orchestrator] tracker broadcast closed; dispatch loop exiting");
                    break;
                }
            }
        }
    });
}
