use crate::{consensus::ConsensusAccumulator, mirror::MirrorPosition, watchlist::WatchlistEntry};
use risk_manager::{CopyTradeRequest, RiskDecision};
use wallet_tracker::traits::{LeaderEvent, Side, Venue};

/// Result of dispatching a leader fill through the copy-trading pipeline.
#[derive(Debug, Clone)]
pub enum DispatchResult {
    /// Trade was mirrored and executed.
    Mirrored(String),
    /// Consensus threshold reached, signal fired.
    ConsensusSignal(String),
    /// Added to discovery / shadow tracking.
    DiscoveryTracked,
    /// Dropped (score too low, blacklisted, etc.).
    Dropped(DropReason),
    /// Risk gate rejected the trade.
    RiskRejected(risk_manager::RiskRejectionReason),
}

#[derive(Debug, Clone)]
pub enum DropReason {
    ScoreTooLow,
    NotInWatchlist,
    Blacklisted,
    CategoryMismatch,
    VenueNotSupported,
}

/// Process a single leader fill event through the 3-mode state machine.
pub async fn dispatch_leader_fill(
    event: &LeaderEvent,
    watchlist: &crate::watchlist::Watchlist,
    consensus: &mut ConsensusAccumulator,
    risk_gate: &risk_manager::RiskGate,
    capital: f64,
    consensus_n: usize,
    consensus_m: usize,
    consensus_window_secs: i64,
    mirror_enabled_globally: bool,
) -> DispatchResult {
    // 1. Look up wallet score and watchlist status
    let entry = match watchlist.get(&event.leader) {
        Some(e) => e,
        None => return DispatchResult::DiscoveryTracked,
    };

    // 2. Check score thresholds
    if entry.wallet_score < 65.0 {
        return DispatchResult::Dropped(DropReason::ScoreTooLow);
    }

    // 3. Category matching for Polymarket
    if let Some(ref cat) = event.market_category {
        if entry.category.as_deref() != Some(cat) {
            return DispatchResult::Dropped(DropReason::CategoryMismatch);
        }
    }

    // 4. Mirror mode
    if mirror_enabled_globally && entry.mirror_enabled && entry.wallet_score >= 80.0 {
        let leader_pct = event.notional / capital.max(1.0);
        let my_size = capital * leader_pct * (entry.wallet_score / 100.0) * 0.5;
        let min_notional = crate::sizing::SizingEngine::venue_min_notional(&event.venue.to_string());

        if my_size < min_notional {
            return DispatchResult::Dropped(DropReason::VenueNotSupported);
        }

        let req = CopyTradeRequest {
            venue: event.venue.to_string(),
            leader: event.leader.clone(),
            symbol: event.symbol.clone(),
            side: match event.side {
                Side::Buy => "buy".into(),
                Side::Sell => "sell".into(),
            },
            notional: my_size,
            wallet_score: entry.wallet_score,
            leader_position_pct: leader_pct,
            follow_lag_seconds: 0.0,
            is_memecoin: event.venue == Venue::Solana,
        };

        match risk_gate.check_copy_trade(&req) {
            RiskDecision::Allow => {
                return DispatchResult::Mirrored(format!(
                    "mirror_{}_{}",
                    event.leader, event.leader_fill_id
                ));
            }
            RiskDecision::Reject(reason) => {
                return DispatchResult::RiskRejected(reason);
            }
        }
    }

    // 5. Consensus mode
    let side_str = match event.side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    };
    let reached = consensus.record(
        &event.symbol,
        side_str,
        &event.leader,
        consensus_window_secs,
        consensus_n,
    );

    if reached {
        let agg_size = capital * 0.05; // small fixed size for consensus
        let req = CopyTradeRequest {
            venue: event.venue.to_string(),
            leader: event.leader.clone(),
            symbol: event.symbol.clone(),
            side: side_str.into(),
            notional: agg_size,
            wallet_score: entry.wallet_score,
            leader_position_pct: 0.0,
            follow_lag_seconds: 0.0,
            is_memecoin: event.venue == Venue::Solana,
        };

        match risk_gate.check_consensus_trade(&req) {
            RiskDecision::Allow => {
                consensus.clear_symbol(&event.symbol, side_str);
                return DispatchResult::ConsensusSignal(format!(
                    "consensus_{}_{}",
                    event.symbol, side_str
                ));
            }
            RiskDecision::Reject(reason) => {
                return DispatchResult::RiskRejected(reason);
            }
        }
    }

    DispatchResult::DiscoveryTracked
}
