use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;
use wallet_tracker::polymarket::PolymarketTracker;
use wallet_tracker::traits::{LeaderEvent, Venue};

pub mod consensus;
pub mod dispatcher;
pub mod mirror;
pub mod sizing;
pub mod watchlist;

use watchlist::Watchlist;
use consensus::ConsensusAccumulator;
use mirror::MirrorTracker;
use sizing::SizingEngine;

/// One row in the tracker-activity ring buffer surfaced through
/// `GET /api/copy/tracker/activity`.  Records EVERY fill the tracker observes
/// (graduated leaders + Discovery candidates + manually-added wallets) along
/// with the dispatcher outcome — so the user can confirm wallets are being
/// polled even when score thresholds drop the event.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrackerActivityEntry {
    pub timestamp: String,
    pub venue: String,
    pub leader: String,
    pub side: String,
    pub slug: String,
    pub price: f64,
    pub notional: f64,
    pub market_id: Option<String>,
    pub leader_fill_id: String,
    /// Dispatch outcome (`mirrored` / `consensus` / `discovery_tracked` /
    /// `dropped:<reason>` / `risk_rejected:<reason>` / `not_in_watchlist`).
    pub result: String,
    /// True when the leader was found in the in-memory watchlist.
    pub in_watchlist: bool,
    pub wallet_score: Option<f64>,
}

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
    /// Bounded ring buffer of recent fills + dispatch outcomes (most recent
    /// first).  Capped at `ACTIVITY_BUFFER_LEN` so memory stays flat.
    pub activity: Arc<Mutex<VecDeque<TrackerActivityEntry>>>,
}

const ACTIVITY_BUFFER_LEN: usize = 500;

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
            activity: Arc::new(Mutex::new(VecDeque::with_capacity(ACTIVITY_BUFFER_LEN))),
        }
    }

    /// Snapshot of the most recent tracker activity entries (newest first).
    pub async fn recent_activity(&self, limit: usize) -> Vec<TrackerActivityEntry> {
        let buf = self.activity.lock().await;
        buf.iter().take(limit.max(1)).cloned().collect()
    }

    async fn record_activity(&self, entry: TrackerActivityEntry) {
        let mut buf = self.activity.lock().await;
        if buf.len() == ACTIVITY_BUFFER_LEN {
            buf.pop_back();
        }
        buf.push_front(entry);
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
    indexer: Arc<wallet_indexer::Indexer>,
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
                    let (result, in_watchlist, wallet_score) = {
                        let watchlist = orchestrator.watchlist.lock().await;
                        let entry = watchlist.get(&event.leader).cloned();
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
                        (
                            result,
                            entry.is_some(),
                            entry.map(|e| e.wallet_score),
                        )
                    };
                    tracing::info!(
                        "[Orchestrator] dispatched fill leader={} slug={} result={:?} in_watchlist={}",
                        event.leader, event.symbol, result, in_watchlist
                    );
                    // Persist the fill to wallet_trades so Fill Audit is populated.
                    let side_str = match event.side {
                        wallet_tracker::traits::Side::Buy => "buy",
                        wallet_tracker::traits::Side::Sell => "sell",
                    };
                    if let Err(e) = indexer.record_fill(
                        &event.leader,
                        &event.venue.to_string(),
                        event.market_id.as_deref(),
                        side_str,
                        event.notional,
                        event.price,
                        &event.timestamp.to_rfc3339(),
                    ).await {
                        tracing::warn!("[Orchestrator] failed to persist fill for {}: {e}", event.leader);
                    }
                    orchestrator
                        .record_activity(activity_entry(&event, &result, in_watchlist, wallet_score))
                        .await;
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

fn activity_entry(
    event: &LeaderEvent,
    result: &dispatcher::DispatchResult,
    in_watchlist: bool,
    wallet_score: Option<f64>,
) -> TrackerActivityEntry {
    use dispatcher::{DispatchResult, DropReason};
    use wallet_tracker::traits::Side;

    let result_str = match result {
        DispatchResult::Mirrored(id) => format!("mirrored:{id}"),
        DispatchResult::ConsensusSignal(id) => format!("consensus:{id}"),
        DispatchResult::DiscoveryTracked => {
            if in_watchlist {
                "discovery_tracked".to_string()
            } else {
                "not_in_watchlist".to_string()
            }
        }
        DispatchResult::Dropped(reason) => match reason {
            DropReason::ScoreTooLow => "dropped:score_too_low".into(),
            DropReason::NotInWatchlist => "dropped:not_in_watchlist".into(),
            DropReason::Blacklisted => "dropped:blacklisted".into(),
            DropReason::CategoryMismatch => "dropped:category_mismatch".into(),
            DropReason::VenueNotSupported => "dropped:venue_not_supported".into(),
        },
        DispatchResult::RiskRejected(reason) => format!("risk_rejected:{reason:?}"),
    };

    TrackerActivityEntry {
        timestamp: event.timestamp.to_rfc3339(),
        venue: event.venue.to_string(),
        leader: event.leader.clone(),
        side: match event.side {
            Side::Buy => "buy".into(),
            Side::Sell => "sell".into(),
        },
        slug: event.symbol.clone(),
        price: event.price,
        notional: event.notional,
        market_id: event.market_id.clone(),
        leader_fill_id: event.leader_fill_id.clone(),
        result: result_str,
        in_watchlist,
        wallet_score,
    }
}
