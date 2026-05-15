use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod general;
pub mod rules;

/// Decision outcome from the risk gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskDecision {
    Allow,
    Reject(RiskRejectionReason),
}

/// Reasons why a copy trade can be rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskRejectionReason {
    SingleLeaderExposureExceeded,
    VenueCopyExposureExceeded,
    AggregateMemecoinExposureExceeded,
    FollowLagExceeded,
    LeaderScoreTooLowForMirror,
    LeaderScoreTooLowForConsensus,
    MaxDrawdownExceeded,
    SingleTradeNotionalExceeded,
    CorrelatedExposureExceeded,
}

/// Request payload to evaluate a copy trade.
#[derive(Debug, Clone)]
pub struct CopyTradeRequest {
    pub venue: String,
    pub leader: String,
    pub symbol: String,
    pub side: String,
    pub notional: f64,
    pub wallet_score: f64,
    pub leader_position_pct: f64,
    pub follow_lag_seconds: f64,
    pub is_memecoin: bool,
}

/// Configuration for the risk gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    pub max_single_leader_exposure_pct: f64,
    pub max_per_venue_copy_exposure_pct: f64,
    pub max_aggregate_memecoin_copy_pct: f64,
    pub max_follow_lag_seconds: f64,
    pub min_leader_score_to_mirror: f64,
    pub min_leader_score_to_consensus: f64,
    pub max_single_trade_notional_pct: f64,
    pub max_correlated_exposure_pct: f64,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_single_leader_exposure_pct: 0.10,
            max_per_venue_copy_exposure_pct: 0.30,
            max_aggregate_memecoin_copy_pct: 0.15,
            max_follow_lag_seconds: 5.0,
            min_leader_score_to_mirror: 80.0,
            min_leader_score_to_consensus: 65.0,
            max_single_trade_notional_pct: 0.02,
            max_correlated_exposure_pct: 0.40,
        }
    }
}

/// Risk gate that enforces copy-trading-specific rules.
pub struct RiskGate {
    config: RiskConfig,
    /// Running exposure per leader (leader address -> notional USD)
    leader_exposure: parking_lot::Mutex<HashMap<String, f64>>,
    /// Running exposure per venue (venue name -> notional USD)
    venue_exposure: parking_lot::Mutex<HashMap<String, f64>>,
    /// Running memecoin aggregate exposure (USD)
    memecoin_exposure: parking_lot::Mutex<f64>,
    /// Total capital under management (USD)
    total_capital: parking_lot::Mutex<f64>,
}

impl RiskGate {
    pub fn new(config: RiskConfig) -> Self {
        Self {
            config,
            leader_exposure: parking_lot::Mutex::new(HashMap::new()),
            venue_exposure: parking_lot::Mutex::new(HashMap::new()),
            memecoin_exposure: parking_lot::Mutex::new(0.0),
            total_capital: parking_lot::Mutex::new(0.0),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(RiskConfig::default())
    }

    pub fn set_total_capital(&self, capital: f64) {
        *self.total_capital.lock() = capital;
    }

    pub fn update_leader_exposure(&self, leader: &str, delta: f64) {
        let mut map = self.leader_exposure.lock();
        *map.entry(leader.to_string()).or_insert(0.0) += delta;
    }

    pub fn update_venue_exposure(&self, venue: &str, delta: f64) {
        let mut map = self.venue_exposure.lock();
        *map.entry(venue.to_string()).or_insert(0.0) += delta;
    }

    pub fn update_memecoin_exposure(&self, delta: f64) {
        *self.memecoin_exposure.lock() += delta;
    }

    /// Evaluate a copy trade request against all configured risk rules.
    pub fn check_copy_trade(&self, req: &CopyTradeRequest) -> RiskDecision {
        let capital = *self.total_capital.lock();
        if capital <= 0.0 {
            // If no capital is set, reject conservatively
            return RiskDecision::Reject(RiskRejectionReason::SingleLeaderExposureExceeded);
        }

        // Rule 1: min score for mirror
        if req.wallet_score < self.config.min_leader_score_to_mirror {
            return RiskDecision::Reject(RiskRejectionReason::LeaderScoreTooLowForMirror);
        }

        // Rule 2: follow lag
        if req.follow_lag_seconds > self.config.max_follow_lag_seconds {
            return RiskDecision::Reject(RiskRejectionReason::FollowLagExceeded);
        }

        // Rule 3: single trade notional cap
        let trade_pct = req.notional / capital;
        if trade_pct > self.config.max_single_trade_notional_pct {
            return RiskDecision::Reject(RiskRejectionReason::SingleTradeNotionalExceeded);
        }

        // Rule 4: single leader exposure cap
        let leader_exp = self.leader_exposure.lock().get(&req.leader).copied().unwrap_or(0.0);
        let new_leader_exp = leader_exp + req.notional;
        if new_leader_exp / capital > self.config.max_single_leader_exposure_pct {
            return RiskDecision::Reject(RiskRejectionReason::SingleLeaderExposureExceeded);
        }

        // Rule 5: venue exposure cap
        let venue_exp = self.venue_exposure.lock().get(&req.venue).copied().unwrap_or(0.0);
        let new_venue_exp = venue_exp + req.notional;
        if new_venue_exp / capital > self.config.max_per_venue_copy_exposure_pct {
            return RiskDecision::Reject(RiskRejectionReason::VenueCopyExposureExceeded);
        }

        // Rule 6: memecoin aggregate cap
        if req.is_memecoin {
            let mem_exp = *self.memecoin_exposure.lock();
            let new_mem_exp = mem_exp + req.notional;
            if new_mem_exp / capital > self.config.max_aggregate_memecoin_copy_pct {
                return RiskDecision::Reject(RiskRejectionReason::AggregateMemecoinExposureExceeded);
            }
        }

        RiskDecision::Allow
    }

    /// Evaluate consensus-mode trade with lower score threshold.
    pub fn check_consensus_trade(&self, req: &CopyTradeRequest) -> RiskDecision {
        let capital = *self.total_capital.lock();
        if capital <= 0.0 {
            return RiskDecision::Reject(RiskRejectionReason::SingleLeaderExposureExceeded);
        }

        if req.wallet_score < self.config.min_leader_score_to_consensus {
            return RiskDecision::Reject(RiskRejectionReason::LeaderScoreTooLowForConsensus);
        }

        let trade_pct = req.notional / capital;
        if trade_pct > self.config.max_single_trade_notional_pct {
            return RiskDecision::Reject(RiskRejectionReason::SingleTradeNotionalExceeded);
        }

        let venue_exp = self.venue_exposure.lock().get(&req.venue).copied().unwrap_or(0.0);
        let new_venue_exp = venue_exp + req.notional;
        if new_venue_exp / capital > self.config.max_per_venue_copy_exposure_pct {
            return RiskDecision::Reject(RiskRejectionReason::VenueCopyExposureExceeded);
        }

        RiskDecision::Allow
    }

    pub fn check_venue_exposure(&self, venue: &str, notional: f64) -> RiskDecision {
        let capital = *self.total_capital.lock();
        if capital <= 0.0 {
            return RiskDecision::Reject(RiskRejectionReason::VenueCopyExposureExceeded);
        }
        let venue_exp = self.venue_exposure.lock().get(venue).copied().unwrap_or(0.0);
        if (venue_exp + notional) / capital > self.config.max_per_venue_copy_exposure_pct {
            return RiskDecision::Reject(RiskRejectionReason::VenueCopyExposureExceeded);
        }
        RiskDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RiskConfig::default();
        assert_eq!(config.max_single_leader_exposure_pct, 0.10);
        assert_eq!(config.min_leader_score_to_mirror, 80.0);
    }

    #[test]
    fn test_mirror_score_rejection() {
        let gate = RiskGate::with_defaults();
        gate.set_total_capital(100_000.0);

        let req = CopyTradeRequest {
            venue: "polymarket".into(),
            leader: "0xabc".into(),
            symbol: "BTC-USD".into(),
            side: "buy".into(),
            notional: 1000.0,
            wallet_score: 50.0,
            leader_position_pct: 0.05,
            follow_lag_seconds: 1.0,
            is_memecoin: false,
        };

        assert_eq!(
            gate.check_copy_trade(&req),
            RiskDecision::Reject(RiskRejectionReason::LeaderScoreTooLowForMirror)
        );
    }

    #[test]
    fn test_venue_exposure_cap() {
        let gate = RiskGate::with_defaults();
        gate.set_total_capital(100_000.0);
        gate.update_venue_exposure("polymarket", 29_600.0);

        let req = CopyTradeRequest {
            venue: "polymarket".into(),
            leader: "0xabc".into(),
            symbol: "BTC-USD".into(),
            side: "buy".into(),
            notional: 500.0,
            wallet_score: 90.0,
            leader_position_pct: 0.05,
            follow_lag_seconds: 1.0,
            is_memecoin: false,
        };

        assert_eq!(
            gate.check_copy_trade(&req),
            RiskDecision::Reject(RiskRejectionReason::VenueCopyExposureExceeded)
        );
    }

    #[test]
    fn test_allow_valid_trade() {
        let gate = RiskGate::with_defaults();
        gate.set_total_capital(100_000.0);

        let req = CopyTradeRequest {
            venue: "polymarket".into(),
            leader: "0xabc".into(),
            symbol: "BTC-USD".into(),
            side: "buy".into(),
            notional: 1000.0,
            wallet_score: 85.0,
            leader_position_pct: 0.05,
            follow_lag_seconds: 1.0,
            is_memecoin: false,
        };

        assert_eq!(gate.check_copy_trade(&req), RiskDecision::Allow);
    }
}
