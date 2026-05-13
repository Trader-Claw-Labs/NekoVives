//! WebSocket subscriptions with auto-reconnect.

use crate::error::{HyperliquidError, Result};
use crate::types::{L2Book, UserFill, WsFundingUpdate};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::{interval, sleep};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const MAINNET_WS_URL: &str = "wss://api.hyperliquid.xyz/ws";
const TESTNET_WS_URL: &str = "wss://api.hyperliquid-testnet.xyz/ws";

/// A managed WebSocket connection that auto-reconnects.
pub struct WsClient {
    url: String,
    subscriptions: Vec<serde_json::Value>,
    _shutdown: broadcast::Sender<()>,
}

impl WsClient {
    pub fn new_mainnet() -> Self {
        Self::with_url(MAINNET_WS_URL)
    }

    pub fn new_testnet() -> Self {
        Self::with_url(TESTNET_WS_URL)
    }

    pub fn with_url(url: impl Into<String>) -> Self {
        let (tx, _rx) = broadcast::channel(1);
        Self {
            url: url.into(),
            subscriptions: Vec::new(),
            _shutdown: tx,
        }
    }

    pub fn subscribe_l2_book(&mut self, coin: &str) {
        self.subscriptions.push(json!({
            "method": "subscribe",
            "subscription": { "type": "l2Book", "coin": coin }
        }));
    }

    pub fn subscribe_user_events(&mut self, address: &str) {
        self.subscriptions.push(json!({
            "method": "subscribe",
            "subscription": { "type": "userEvents", "user": address }
        }));
    }

    pub fn subscribe_funding_updates(&mut self) {
        self.subscriptions.push(json!({
            "method": "subscribe",
            "subscription": { "type": "fundingUpdates" }
        }));
    }

    /// Start the connection and return channels for each event type.
    ///
    /// This spawns a background task that handles reconnects.
    pub fn start(
        self,
    ) -> Result<(
        broadcast::Receiver<L2Book>,
        broadcast::Receiver<UserFill>,
        broadcast::Receiver<WsFundingUpdate>,
    )> {
        let (l2_tx, l2_rx) = broadcast::channel::<L2Book>(256);
        let (fill_tx, fill_rx) = broadcast::channel::<UserFill>(256);
        let (fund_tx, fund_rx) = broadcast::channel::<WsFundingUpdate>(256);

        let url = self.url;
        let subs = self.subscriptions;

        tokio::spawn(async move {
            let mut backoff = Duration::from_millis(250);
            let max_backoff = Duration::from_secs(8);

            loop {
                match connect_and_run(&url, &subs, &l2_tx, &fill_tx, &fund_tx).await {
                    Ok(()) => {
                        tracing::info!("[hyperliquid-ws] connection closed cleanly");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[hyperliquid-ws] connection error: {}, reconnecting in {:?}",
                            e,
                            backoff
                        );
                        sleep(backoff).await;
                        backoff = std::cmp::min(backoff * 2, max_backoff);
                    }
                }
            }
        });

        Ok((l2_rx, fill_rx, fund_rx))
    }
}

async fn connect_and_run(
    url: &str,
    subs: &[serde_json::Value],
    l2_tx: &broadcast::Sender<L2Book>,
    fill_tx: &broadcast::Sender<UserFill>,
    fund_tx: &broadcast::Sender<WsFundingUpdate>,
) -> Result<()> {
    let (ws_stream, _) = connect_async(url)
        .await
        .map_err(|e| HyperliquidError::WebSocket(e.to_string()))?;

    let (mut write, mut read) = ws_stream.split();

    // Send subscriptions
    for sub in subs {
        let msg = Message::Text(sub.to_string().into());
        write
            .send(msg)
            .await
            .map_err(|e| HyperliquidError::WebSocket(e.to_string()))?;
    }

    // Heartbeat every 30s
    let mut heartbeat = interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            maybe_msg = read.next() => {
                let msg = match maybe_msg {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => return Err(HyperliquidError::WebSocket(e.to_string())),
                    None => return Ok(()), // connection closed
                };

                if let Message::Text(text) = msg {
                    if let Err(e) = handle_message(&text, l2_tx, fill_tx, fund_tx) {
                        tracing::debug!("[hyperliquid-ws] parse error: {}", e);
                    }
                }
            }
            _ = heartbeat.tick() => {
                let ping = Message::Text(json!({"method": "ping"}).to_string().into());
                if let Err(e) = write.send(ping).await {
                    return Err(HyperliquidError::WebSocket(e.to_string()));
                }
            }
        }
    }
}

fn handle_message(
    text: &str,
    l2_tx: &broadcast::Sender<L2Book>,
    fill_tx: &broadcast::Sender<UserFill>,
    fund_tx: &broadcast::Sender<WsFundingUpdate>,
) -> Result<()> {
    let parsed: serde_json::Value = serde_json::from_str(text)?;

    // Determine message type from subscription field or top-level structure
    if let Some(channel) = parsed.get("channel").and_then(|v| v.as_str()) {
        match channel {
            "l2Book" => {
                if let Ok(book) = serde_json::from_value::<L2Book>(parsed["data"].clone()) {
                    let _ = l2_tx.send(book);
                }
            }
            "userEvents" => {
                if let Some(fills) = parsed["data"]["fills"].as_array() {
                    for f in fills {
                        if let Ok(fill) = serde_json::from_value::<UserFill>(f.clone()) {
                            let _ = fill_tx.send(fill);
                        }
                    }
                }
            }
            "fundingUpdates" => {
                if let Ok(upd) = serde_json::from_value::<WsFundingUpdate>(parsed["data"].clone()) {
                    let _ = fund_tx.send(upd);
                }
            }
            _ => {}
        }
    }

    Ok(())
}

/// Typed stream wrappers for ergonomic access.
pub struct L2BookStream {
    pub rx: broadcast::Receiver<L2Book>,
}

pub struct UserEventStream {
    pub rx: broadcast::Receiver<UserFill>,
}

pub struct FundingStream {
    pub rx: broadcast::Receiver<WsFundingUpdate>,
}
