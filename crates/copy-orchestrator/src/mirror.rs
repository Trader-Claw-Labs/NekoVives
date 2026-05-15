use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A tracked mirror position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorPosition {
    pub leader_address: String,
    pub leader_fill_id: String,
    pub my_order_id: Option<String>,
    pub venue: String,
    pub symbol: String,
    pub side: String,
    pub notional: f64,
    pub entry_price: f64,
    pub status: PositionStatus,
    pub opened_at: String,
    pub closed_at: Option<String>,
    pub pnl: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PositionStatus {
    Open,
    Closed,
    Cancelled,
}

/// Tracks active mirror positions.
#[derive(Debug, Default)]
pub struct MirrorTracker {
    positions: HashMap<String, MirrorPosition>,
}

impl MirrorTracker {
    pub fn new() -> Self {
        Self {
            positions: HashMap::new(),
        }
    }

    pub fn open(&mut self, pos: MirrorPosition) {
        self.positions
            .insert(pos.leader_fill_id.clone(), pos);
    }

    pub fn close(&mut self, leader_fill_id: &str, pnl: f64) -> Option<MirrorPosition> {
        self.positions.get_mut(leader_fill_id).map(|pos| {
            pos.status = PositionStatus::Closed;
            pos.closed_at = Some(chrono::Utc::now().to_rfc3339());
            pos.pnl = Some(pnl);
            pos.clone()
        })
    }

    pub fn get(&self, leader_fill_id: &str) -> Option<&MirrorPosition> {
        self.positions.get(leader_fill_id)
    }

    pub fn list_open(&self) -> Vec<&MirrorPosition> {
        self.positions
            .values()
            .filter(|p| matches!(p.status, PositionStatus::Open))
            .collect()
    }

    pub fn list_all(&self) -> Vec<&MirrorPosition> {
        self.positions.values().collect()
    }
}
