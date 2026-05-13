use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub mod consensus;
pub mod dispatcher;
pub mod mirror;
pub mod sizing;
pub mod watchlist;

use watchlist::{Watchlist, WatchlistEntry};
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
        }
    }

    pub async fn set_capital(&self, capital: f64) {
        *self.capital.lock().await = capital;
        self.risk_gate.set_total_capital(capital);
    }
}
