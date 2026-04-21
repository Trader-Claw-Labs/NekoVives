//! Real-time price feed via Binance WebSocket kline streams.
//!
//! Connects to `<symbol>@kline_<interval>` and emits a [`LiveCandle`] for
//! every closed candle. Auto-reconnects on disconnect.
//!
//! Other feed sources (Chainlink Data Streams, etc.) can be added here
//! following the same `spawn_*` / mpsc pattern.

use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc;
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
