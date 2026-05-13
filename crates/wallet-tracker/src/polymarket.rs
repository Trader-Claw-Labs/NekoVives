use crate::traits::{LeaderActivityStream, LeaderEvent, Side, Venue};
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tokio::time::{interval, Duration};
use tokio_stream::{wrappers::IntervalStream, Stream, StreamExt};

/// Polymarket tracker using Data API polling + optional Polygon RPC subscription.
///
/// Primary mode (Phase 0): poll `data.polymarket.com/trades` every 5-10s per wallet.
/// Future mode: Polygon RPC WebSocket subscription to CTF Exchange `OrderFilled` events.
pub struct PolymarketTracker {
    rpc_ws_url: String,
    http_client: reqwest::Client,
    data_api_base: String,
    /// Seen fill IDs to deduplicate
    seen_fills: Arc<Mutex<HashSet<String>>>,
    /// Broadcast channel for events
    tx: broadcast::Sender<LeaderEvent>,
}

impl PolymarketTracker {
    pub fn new(rpc_ws_url: String) -> Self {
        let (tx, _rx) = broadcast::channel::<LeaderEvent>(1024);
        Self {
            rpc_ws_url,
            http_client: reqwest::Client::new(),
            data_api_base: "https://data.polymarket.com".into(),
            seen_fills: Arc::new(Mutex::new(HashSet::new())),
            tx,
        }
    }

    /// Returns a receiver for the event broadcast.
    pub fn subscribe_events(&self) -> broadcast::Receiver<LeaderEvent> {
        self.tx.subscribe()
    }

    /// Start polling the Data API for the given leader wallets.
    /// This spawns a background task and returns immediately.
    pub fn start_polling(&self, leaders: Vec<String>, interval_secs: u64) {
        let client = self.http_client.clone();
        let base = self.data_api_base.clone();
        let seen = self.seen_fills.clone();
        let tx = self.tx.clone();

        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(interval_secs));
            loop {
                ticker.tick().await;
                for leader in &leaders {
                    match fetch_trades(&client, &base, leader).await {
                        Ok(trades) => {
                            let mut seen_guard = seen.lock().await;
                            for trade in trades {
                                if seen_guard.insert(trade.leader_fill_id.clone()) {
                                    let _ = tx.send(trade);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("PolymarketTracker poll error for {}: {}", leader, e);
                        }
                    }
                }
            }
        });
    }
}

#[async_trait]
impl LeaderActivityStream for PolymarketTracker {
    type Event = LeaderEvent;

    async fn subscribe(
        &self,
        leaders: Vec<String>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Self::Event>> + Send>>> {
        self.start_polling(leaders, 5);
        let rx = self.subscribe_events();
        let stream = tokio_stream::wrappers::BroadcastStream::new(rx)
            .filter_map(|r| r.ok())
            .map(Ok);
        Ok(Box::pin(stream))
    }

    async fn unsubscribe(&self, _leader: String) -> Result<()> {
        // Polling stops when the task is dropped; granular unsubscribe requires ref-counting.
        Ok(())
    }
}

/// Trade record from Polymarket Data API.
#[derive(Debug, serde::Deserialize)]
struct ApiTrade {
    #[serde(rename = "id")]
    trade_id: String,
    #[serde(rename = "takerSide")]
    taker_side: String,
    #[serde(rename = "marketSlug")]
    market_slug: String,
    #[serde(rename = "marketId")]
    market_id: String,
    size: String,
    price: String,
    #[serde(rename = "createdAt")]
    created_at: String,
}

async fn fetch_trades(
    client: &reqwest::Client,
    base: &str,
    user: &str,
) -> Result<Vec<LeaderEvent>> {
    let url = format!("{}/trades?user={}&limit=100", base, user);
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Data API returned {}", resp.status());
    }
    let trades: Vec<ApiTrade> = resp.json().await?;
    let events: Vec<LeaderEvent> = trades
        .into_iter()
        .map(|t| LeaderEvent {
            venue: Venue::Polymarket,
            leader: user.to_string(),
            side: if t.taker_side.eq_ignore_ascii_case("buy") {
                Side::Buy
            } else {
                Side::Sell
            },
            symbol: t.market_slug,
            notional: t.size.parse().unwrap_or(0.0),
            price: t.price.parse().unwrap_or(0.0),
            timestamp: Utc::now(),
            market_category: None, // populated later by market metadata
            market_id: Some(t.market_id),
            leader_fill_id: t.trade_id,
        })
        .collect();
    Ok(events)
}
