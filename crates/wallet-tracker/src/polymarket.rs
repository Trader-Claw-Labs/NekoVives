use crate::traits::{LeaderActivityStream, LeaderEvent, Side, Venue};
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashSet;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tokio::time::{interval, Duration};
use tokio_stream::{Stream, StreamExt};

/// Polymarket tracker using Data API polling + optional Polygon RPC subscription.
///
/// Primary mode (Phase 0): poll `data.polymarket.com/trades` every 5-10s per wallet.
/// Future mode: Polygon RPC WebSocket subscription to CTF Exchange `OrderFilled` events.
pub struct PolymarketTracker {
    #[allow(dead_code)]
    rpc_ws_url: String,
    http_client: reqwest::Client,
    data_api_base: String,
    /// Seen fill IDs to deduplicate
    seen_fills: Arc<Mutex<HashSet<String>>>,
    /// Broadcast channel for events
    tx: broadcast::Sender<LeaderEvent>,
    /// Live, mutable list of leader addresses being polled.  Updates via
    /// `set_leaders` are picked up on the next tick (no task restart needed).
    leaders: Arc<Mutex<HashSet<String>>>,
    /// Set to true once `start_polling` has spawned its background task, to
    /// prevent duplicate pollers.
    started: Arc<AtomicBool>,
}

impl PolymarketTracker {
    pub fn new(rpc_ws_url: String) -> Self {
        let (tx, _rx) = broadcast::channel::<LeaderEvent>(1024);
        Self {
            rpc_ws_url,
            http_client: reqwest::Client::new(),
            data_api_base: "https://data-api.polymarket.com".into(),
            seen_fills: Arc::new(Mutex::new(HashSet::new())),
            tx,
            leaders: Arc::new(Mutex::new(HashSet::new())),
            started: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns a receiver for the event broadcast.
    pub fn subscribe_events(&self) -> broadcast::Receiver<LeaderEvent> {
        self.tx.subscribe()
    }

    /// Replace the active leader list. Picked up on the next poll tick.
    pub async fn set_leaders(&self, leaders: Vec<String>) {
        let mut guard = self.leaders.lock().await;
        guard.clear();
        for l in leaders {
            guard.insert(l.to_lowercase());
        }
    }

    /// Start the background polling task. Idempotent — calling it more than
    /// once is a no-op.  The task reads the current leader list each tick from
    /// `self.leaders`, so callers can update the list via `set_leaders` at any
    /// time without restarting.
    pub fn start_polling(&self, interval_secs: u64) {
        if self.started.swap(true, Ordering::SeqCst) {
            return; // already running
        }
        let client = self.http_client.clone();
        let base = self.data_api_base.clone();
        let seen = self.seen_fills.clone();
        let tx = self.tx.clone();
        let leaders = self.leaders.clone();

        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(interval_secs.max(1)));
            tracing::info!(
                "[PolymarketTracker] polling loop started (interval={}s)",
                interval_secs
            );
            loop {
                ticker.tick().await;
                let snapshot: Vec<String> = {
                    leaders.lock().await.iter().cloned().collect()
                };
                if snapshot.is_empty() {
                    continue;
                }
                for leader in &snapshot {
                    match fetch_trades(&client, &base, leader).await {
                        Ok(trades) => {
                            let mut seen_guard = seen.lock().await;
                            for trade in trades {
                                if seen_guard.insert(trade.leader_fill_id.clone()) {
                                    tracing::info!(
                                        "[PolymarketTracker] new fill leader={} side={:?} slug={} size={} price={}",
                                        trade.leader, trade.side, trade.symbol, trade.notional, trade.price
                                    );
                                    let _ = tx.send(trade);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[PolymarketTracker] poll error for {}: {}",
                                leader, e
                            );
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
        self.set_leaders(leaders).await;
        self.start_polling(5);
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

/// Trade record from Polymarket Data API (https://data-api.polymarket.com/trades).
///
/// The public Data API does NOT expose a stable `id` field; we synthesise a
/// dedupe key from `transactionHash + asset + timestamp` so the same fill is
/// not re-emitted on every poll.
#[derive(Debug, serde::Deserialize)]
struct ApiTrade {
    #[serde(rename = "proxyWallet")]
    proxy_wallet: String,
    side: String,
    /// Token id (string of decimal digits)
    asset: String,
    #[serde(rename = "conditionId", default)]
    condition_id: Option<String>,
    /// Numeric size & price (Data API returns JSON numbers, not strings).
    #[serde(default)]
    size: f64,
    #[serde(default)]
    price: f64,
    #[serde(default)]
    timestamp: i64,
    #[serde(default)]
    slug: Option<String>,
    #[serde(rename = "transactionHash", default)]
    transaction_hash: Option<String>,
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
        .map(|t| {
            let dedupe_key = match (&t.transaction_hash, &t.asset) {
                (Some(tx), asset) => format!("{}:{}:{}", tx, asset, t.timestamp),
                (None, asset) => format!("{}:{}:{}", t.proxy_wallet, asset, t.timestamp),
            };
            let timestamp = chrono::DateTime::<Utc>::from_timestamp(t.timestamp, 0)
                .unwrap_or_else(Utc::now);
            LeaderEvent {
                venue: Venue::Polymarket,
                leader: t.proxy_wallet.clone(),
                side: if t.side.eq_ignore_ascii_case("buy") {
                    Side::Buy
                } else {
                    Side::Sell
                },
                symbol: t.slug.unwrap_or_else(|| t.asset.clone()),
                notional: t.size * t.price,
                price: t.price,
                timestamp,
                market_category: None,
                market_id: t.condition_id,
                leader_fill_id: dedupe_key,
            }
        })
        .collect();
    Ok(events)
}
