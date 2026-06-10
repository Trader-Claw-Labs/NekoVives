use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A watched leader wallet with its configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WatchlistEntry {
    pub address: String,
    pub venue: String,
    pub category: Option<String>,
    pub mirror_enabled: bool,
    pub consensus_weight: f64,
    pub size_factor: f64,
    pub wallet_score: f64,
    pub added_at: String,
    /// Execution mode for this leader's mirror. false = Dry Run (simulated fills,
    /// no real orders — the default and current behaviour); true = Live (real orders,
    /// requires guardrails). Defaults to Dry Run so nothing executes by accident.
    #[serde(default)]
    pub live_mode: bool,
    /// Per-leader guardrails, only enforced in Live mode.
    #[serde(default)]
    pub max_notional_per_trade: Option<f64>,
    #[serde(default)]
    pub max_daily_loss: Option<f64>,
    #[serde(default)]
    pub max_open_positions: Option<u32>,
}

/// Active watchlist of graduated leader wallets.
#[derive(Debug, Default)]
pub struct Watchlist {
    entries: HashMap<String, WatchlistEntry>,
}

impl Watchlist {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn add(&mut self, entry: WatchlistEntry) {
        self.entries.insert(entry.address.clone(), entry);
    }

    pub fn remove(&mut self, address: &str) -> Option<WatchlistEntry> {
        self.entries.remove(address)
    }

    pub fn get(&self, address: &str) -> Option<&WatchlistEntry> {
        self.entries.get(address)
    }

    pub fn get_mut(&mut self, address: &str) -> Option<&mut WatchlistEntry> {
        self.entries.get_mut(address)
    }

    pub fn list(&self) -> Vec<&WatchlistEntry> {
        self.entries.values().collect()
    }

    pub fn list_by_venue(&self, venue: &str) -> Vec<&WatchlistEntry> {
        self.entries
            .values()
            .filter(|e| e.venue == venue)
            .collect()
    }

    pub fn list_by_category(&self, category: &str) -> Vec<&WatchlistEntry> {
        self.entries
            .values()
            .filter(|e| e.category.as_deref() == Some(category))
            .collect()
    }

    pub fn toggle_mirror(&mut self, address: &str) -> bool {
        if let Some(e) = self.entries.get_mut(address) {
            e.mirror_enabled = !e.mirror_enabled;
            e.mirror_enabled
        } else {
            false
        }
    }

    /// Update mutable knobs on a leader. Returns true if the leader existed.
    pub fn update(
        &mut self,
        address: &str,
        size_factor: Option<f64>,
        consensus_weight: Option<f64>,
        category: Option<Option<String>>,
        mirror_enabled: Option<bool>,
    ) -> bool {
        let Some(e) = self.entries.get_mut(address) else {
            return false;
        };
        if let Some(v) = size_factor {
            e.size_factor = v.clamp(0.0, 10.0);
        }
        if let Some(v) = consensus_weight {
            e.consensus_weight = v.clamp(0.0, 10.0);
        }
        if let Some(c) = category {
            e.category = c;
        }
        if let Some(v) = mirror_enabled {
            e.mirror_enabled = v;
        }
        true
    }

    /// Update the wallet_score for a leader from the edge-validator result.
    /// score = 0-100 scale: 0 = no_edge/hft, 100 = confirmed edge.
    pub fn update_score(&mut self, address: &str, score: f64) -> bool {
        let Some(e) = self.entries.get_mut(address) else { return false; };
        e.wallet_score = score.clamp(0.0, 100.0);
        true
    }

    /// Update execution mode (Dry Run / Live) and per-leader guardrails.
    pub fn set_mode(
        &mut self,
        address: &str,
        live_mode: Option<bool>,
        max_notional_per_trade: Option<f64>,
        max_daily_loss: Option<f64>,
        max_open_positions: Option<u32>,
    ) -> bool {
        let Some(e) = self.entries.get_mut(address) else { return false; };
        if let Some(v) = live_mode { e.live_mode = v; }
        if max_notional_per_trade.is_some() { e.max_notional_per_trade = max_notional_per_trade; }
        if max_daily_loss.is_some() { e.max_daily_loss = max_daily_loss; }
        if max_open_positions.is_some() { e.max_open_positions = max_open_positions; }
        true
    }
}
