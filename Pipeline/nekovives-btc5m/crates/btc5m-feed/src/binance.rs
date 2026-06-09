//! Binance public WebSocket client (lead-venue spot view).
//!
//! Subscribes to `btcusdt@aggTrade` (signed trade flow + spot) and
//! `btcusdt@depth10@100ms` (top-of-book for OFI). No API key required.
//!
//! Binance recycles the connection ~every 24h and emits no application-level
//! shutdown on the public combined stream, so we treat any close/error as a
//! reconnect trigger with capped backoff. While reconnecting we publish nothing,
//! and the strategy loop must refuse to trade on a stale snapshot (see CLAUDE.md).
//!
//! NOTE: you can add Coinbase / OKX / Bybit clients with the same shape and feed
//! the same `MarketState` to get closer to the Chainlink aggregate — see
//! `multi_venue` notes in CLAUDE.md.

use std::time::Duration;

use anyhow::Result;
use futures_util::StreamExt;
use serde::Deserialize;
use tokio_tungstenite::connect_async;
use tracing::{debug, warn};

use crate::state::MarketState;
use crate::types::{BookTop, Level, Trade};

const COMBINED_URL: &str =
    "wss://stream.binance.com:9443/stream?streams=btcusdt@aggTrade/btcusdt@depth10@100ms";

#[derive(Deserialize)]
struct Combined {
    stream: String,
    data: serde_json::Value,
}

#[derive(Deserialize)]
struct AggTrade {
    #[serde(rename = "T")]
    trade_ms: i64,
    #[serde(rename = "p")]
    price: String,
    #[serde(rename = "q")]
    qty: String,
    #[serde(rename = "m")]
    buyer_is_maker: bool,
}

#[derive(Deserialize)]
struct Depth {
    #[serde(rename = "E", default)]
    event_ms: i64,
    bids: Vec<[String; 2]>,
    asks: Vec<[String; 2]>,
}

fn parse_levels(raw: &[[String; 2]]) -> Vec<Level> {
    raw.iter()
        .filter_map(|[p, q]| Some(Level { price: p.parse().ok()?, qty: q.parse().ok()? }))
        .collect()
}

/// Run forever, reconnecting with backoff. Spawn this as a tokio task.
pub async fn run(state: MarketState) {
    let mut backoff = Duration::from_millis(500);
    loop {
        match connect_once(&state).await {
            Ok(_) => {
                warn!("binance stream closed cleanly; reconnecting");
                backoff = Duration::from_millis(500);
            }
            Err(e) => {
                warn!(error = %e, "binance stream error; reconnecting in {:?}", backoff);
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(15));
    }
}

async fn connect_once(state: &MarketState) -> Result<()> {
    let (ws, _) = connect_async(COMBINED_URL).await?;
    let (_w, mut r) = ws.split();
    debug!("binance connected");

    while let Some(msg) = r.next().await {
        let msg = msg?;
        if !msg.is_text() {
            continue;
        }
        let txt = msg.into_text()?;
        let Combined { stream, data } = match serde_json::from_str(&txt) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if stream.ends_with("aggTrade") {
            if let Ok(a) = serde_json::from_value::<AggTrade>(data) {
                if let (Ok(price), Ok(qty)) = (a.price.parse::<f64>(), a.qty.parse::<f64>()) {
                    state.on_trade(Trade {
                        ts_ms: a.trade_ms,
                        price,
                        qty,
                        buyer_is_maker: a.buyer_is_maker,
                    });
                }
            }
        } else if stream.contains("depth") {
            if let Ok(d) = serde_json::from_value::<Depth>(data) {
                state.on_book(BookTop {
                    ts_ms: d.event_ms,
                    bids: parse_levels(&d.bids),
                    asks: parse_levels(&d.asks),
                });
            }
        }
    }
    Ok(())
}
