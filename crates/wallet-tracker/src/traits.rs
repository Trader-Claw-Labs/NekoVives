use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use tokio_stream::Stream;

/// Supported trading venues.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Venue {
    Polymarket,
    Solana,
    Hyperliquid,
    Evm,
}

impl std::fmt::Display for Venue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Venue::Polymarket => write!(f, "polymarket"),
            Venue::Solana => write!(f, "solana"),
            Venue::Hyperliquid => write!(f, "hyperliquid"),
            Venue::Evm => write!(f, "evm"),
        }
    }
}

/// Trade side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

/// A fill event produced by a leader wallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderEvent {
    pub venue: Venue,
    pub leader: String,
    pub side: Side,
    pub symbol: String,
    pub notional: f64,
    pub price: f64,
    pub timestamp: DateTime<Utc>,
    pub market_category: Option<String>,
    pub market_id: Option<String>,
    pub leader_fill_id: String,
}

/// Abstraction over venue-specific leader activity streams.
#[async_trait]
pub trait LeaderActivityStream: Send + Sync {
    /// Event type produced by this stream.
    type Event;

    /// Subscribe to fills for the given leader addresses.
    /// Returns a stream that yields events until unsubscribed or errored.
    async fn subscribe(
        &self,
        leaders: Vec<String>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Self::Event>> + Send>>>;

    /// Unsubscribe from a specific leader.
    async fn unsubscribe(&self, leader: String) -> Result<()>;
}
