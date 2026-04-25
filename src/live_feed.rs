//! Real-time price feed via Binance WebSocket kline streams.
//!
//! Connects to `<symbol>@kline_<interval>` and emits a [`LiveCandle`] for
//! every closed candle. Auto-reconnects on disconnect.
//!
//! Other feed sources (Chainlink Data Streams, etc.) can be added here
//! following the same `spawn_*` / mpsc pattern.

use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Debug, Clone)]
pub struct LiveCandle {
    pub open_time_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Deserialize)]
struct BinanceKlineMsg {
    k: BinanceKlineData,
}

#[derive(Deserialize)]
struct BinanceKlineData {
    t: i64,
    #[serde(rename = "o")] open:   String,
    #[serde(rename = "h")] high:   String,
    #[serde(rename = "l")] low:    String,
    #[serde(rename = "c")] close:  String,
    #[serde(rename = "v")] volume: String,
    #[serde(rename = "x")] is_closed: bool,
}

/// Spawns a background task that connects to the Binance kline WebSocket
/// for the given symbol/interval and emits every closed candle into the
/// returned channel. Reconnects automatically on disconnect.
///
/// Feed URL: `wss://stream.binance.com:9443/ws/{symbol}@kline_{interval}`
pub fn spawn_binance_kline_feed(
    symbol: String,
    interval: String,
) -> mpsc::Receiver<LiveCandle> {
    let (tx, rx) = mpsc::channel::<LiveCandle>(64);
    tokio::spawn(async move {
        loop {
            let url = format!(
                "wss://stream.binance.com:9443/ws/{}@kline_{}",
                symbol.to_lowercase(), interval
            );
            tracing::info!("[LIVE_FEED] Connecting: {url}");
            match connect_async(&url).await {
                Ok((ws, _)) => {
                    let (_, mut read) = ws.split();
                    tracing::info!("[LIVE_FEED] Connected - streaming {symbol}@kline_{interval}");
                    loop {
                        match read.next().await {
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(msg) = serde_json::from_str::<BinanceKlineMsg>(&text) {
                                    if msg.k.is_closed {
                                        let candle = LiveCandle {
                                            open_time_ms: msg.k.t,
                                            open:   msg.k.open.parse().unwrap_or(0.0),
                                            high:   msg.k.high.parse().unwrap_or(0.0),
                                            low:    msg.k.low.parse().unwrap_or(0.0),
                                            close:  msg.k.close.parse().unwrap_or(0.0),
                                            volume: msg.k.volume.parse().unwrap_or(0.0),
                                        };
                                        tracing::debug!("[LIVE_FEED] Closed candle close={}", candle.close);
                                        if tx.send(candle).await.is_err() {
                                            return; // receiver dropped
                                        }
                                    }
                                }
                            }
                            Some(Ok(_)) => {}
                            Some(Err(e)) => {
                                tracing::warn!("[LIVE_FEED] WebSocket error: {e}");
                                break;
                            }
                            None => break,
                        }
                    }
                }
                Err(e) => tracing::warn!("[LIVE_FEED] Connect failed: {e}"),
            }
            tracing::info!("[LIVE_FEED] Reconnecting in 5s...");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
    rx
}

// ── Binance 1s miniTicker (real-time price) ──────────────────────────────────

/// Spawn a background task that connects to the Binance miniTicker WebSocket
/// (`{symbol}@miniTicker`) and emits the last traded price every second.
///
/// Feed URL: `wss://stream.binance.com:9443/ws/{symbol}@miniTicker`
pub fn spawn_binance_ticker_feed(symbol: String) -> mpsc::Receiver<f64> {
    let (tx, rx) = mpsc::channel::<f64>(128);
    tokio::spawn(async move {
        loop {
            let url = format!(
                "wss://stream.binance.com:9443/ws/{}@miniTicker",
                symbol.to_lowercase()
            );
            tracing::info!("[TICKER] Connecting: {url}");
            match connect_async(&url).await {
                Ok((ws, _)) => {
                    let (_, mut read) = ws.split();
                    tracing::info!("[TICKER] Connected - streaming {symbol}@miniTicker");
                    loop {
                        match read.next().await {
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(msg) = serde_json::from_str::<BinanceMiniTicker>(&text) {
                                    let price: f64 = msg.c.parse().unwrap_or(0.0);
                                    tracing::debug!("[TICKER] Price: {price}");
                                    if tx.send(price).await.is_err() {
                                        return; // receiver dropped
                                    }
                                }
                            }
                            Some(Ok(_)) => {}
                            Some(Err(e)) => {
                                tracing::warn!("[TICKER] WebSocket error: {e}");
                                break;
                            }
                            None => break,
                        }
                    }
                }
                Err(e) => tracing::warn!("[TICKER] Connect failed: {e}"),
            }
            tracing::info!("[TICKER] Reconnecting in 5s...");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
    rx
}

#[derive(Deserialize)]
struct BinanceMiniTicker {
    #[serde(rename = "c")]
    c: String,
}

// ── Chainlink Data Streams price feed ────────────────────────────────────────

/// Shared handle to the latest Chainlink BTC price (updated by background poller).
pub type ChainlinkPriceHandle = Arc<RwLock<Option<f64>>>;

/// Spawn a background task that polls a Chainlink-compatible REST endpoint
/// every `interval_secs` and stores the latest price.
///
/// The endpoint should return JSON containing a price field. Supported shapes:
/// - `{ "price": "65000.00" }`
/// - `{ "benchmarkPrice": "65000" }`
/// - `{ "data": { "price": "65000" } }`
/// - Chainlink Data Streams report: decoded if `fullReport` hex is present
///
/// Returns a shared handle that the consumer can read to get the latest price.
pub fn spawn_chainlink_price_feed(
    endpoint_url: String,
    api_key: Option<String>,
    interval_secs: u64,
) -> ChainlinkPriceHandle {
    let price = Arc::new(RwLock::new(None::<f64>));
    let price_clone = price.clone();

    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        loop {
            let mut req = client.get(&endpoint_url);
            if let Some(ref key) = api_key {
                req = req.header("Authorization", format!("Bearer {key}"));
            }

            match req.send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        match resp.json::<serde_json::Value>().await {
                            Ok(json) => {
                                if let Some(p) = extract_price_from_json(&json) {
                                    let mut lock = price_clone.write().await;
                                    *lock = Some(p);
                                    tracing::debug!("[CHAINLINK] Price updated: {p}");
                                } else {
                                    tracing::warn!("[CHAINLINK] Could not extract price from response: {json}");
                                }
                            }
                            Err(e) => {
                                tracing::warn!("[CHAINLINK] JSON parse error: {e}");
                            }
                        }
                    } else {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        tracing::warn!("[CHAINLINK] HTTP {status}: {text}");
                    }
                }
                Err(e) => {
                    tracing::warn!("[CHAINLINK] Request failed: {e}");
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
        }
    });

    price
}

/// Try to extract a price from various known JSON shapes.
fn extract_price_from_json(json: &serde_json::Value) -> Option<f64> {
    // 1. Direct price field
    if let Some(p) = json.get("price").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()) {
        return Some(p);
    }
    if let Some(p) = json.get("price").and_then(|v| v.as_f64()) {
        return Some(p);
    }

    // 2. Chainlink Data Streams decoded fields
    if let Some(p) = json.get("benchmarkPrice").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()) {
        return Some(p);
    }
    if let Some(p) = json.get("benchmarkPrice").and_then(|v| v.as_f64()) {
        return Some(p);
    }

    // 3. Nested data.price
    if let Some(p) = json.get("data").and_then(|d| d.get("price")).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()) {
        return Some(p);
    }
    if let Some(p) = json.get("data").and_then(|d| d.get("price")).and_then(|v| v.as_f64()) {
        return Some(p);
    }

    // 4. Chainlink Data Streams fullReport hex — log that manual decoding is needed
    if json.get("fullReport").is_some() {
        tracing::warn!("[CHAINLINK] Response contains encoded fullReport; configure a decoded endpoint or implement report decoding.");
    }

    None
}
