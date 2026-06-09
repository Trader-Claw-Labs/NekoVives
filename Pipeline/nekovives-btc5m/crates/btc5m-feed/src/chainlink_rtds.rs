//! Polymarket Real-Time Data Socket (RTDS) client — the resolving price feed.
//!
//! This is the single most important connection for avoiding basis-risk blow-ups.
//! Polymarket's 5m BTC markets resolve against a Chainlink Data Stream
//! (`BTC/USD-RefPrice-DS-Premium-Global-003`), surfaced to clients through RTDS
//! under the `crypto_prices_chainlink` topic. The first tick at/after a window's
//! open boundary IS the "Price to Beat", to +0ms. We subscribe here, stream the
//! resolving price continuously, and snapshot the price-to-beat at each boundary.
//!
//! Endpoint shape (verify against current docs before live):
//!   wss RTDS, subscribe { topic: "crypto_prices_chainlink", filters: "btc/usd" }
//! Price-to-beat REST fallback:
//!   GET https://polymarket.com/api/equity/price-to-beat/{slug}
//!
//! Docs: https://docs.polymarket.com/market-data/websocket/rtds

use std::time::Duration;

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

use crate::state::MarketState;

const RTDS_URL: &str = "wss://ws-live-data.polymarket.com";
const TOPIC: &str = "crypto_prices_chainlink";
const SYMBOL: &str = "btc/usd";

#[derive(Deserialize)]
struct Update {
    #[allow(dead_code)]
    topic: String,
    payload: Payload,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(default)]
    symbol: String,
    value: f64,
    timestamp: i64,
}

/// Run forever, reconnecting with backoff. Spawn as a tokio task.
///
/// `state` is updated with every Chainlink tick. Window boundary detection and
/// price-to-beat capture are handled by the orchestrator (it knows `open_ms`);
/// this task just keeps `snap.chainlink` fresh and the history buffer filled.
pub async fn run(state: MarketState) {
    let mut backoff = Duration::from_millis(500);
    loop {
        match connect_once(&state).await {
            Ok(_) => {
                warn!("RTDS closed; reconnecting");
                backoff = Duration::from_millis(500);
            }
            Err(e) => warn!(error = %e, "RTDS error; reconnecting in {:?}", backoff),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(15));
    }
}

async fn connect_once(state: &MarketState) -> Result<()> {
    let (ws, _) = connect_async(RTDS_URL).await?;
    let (mut w, mut r) = ws.split();

    let sub = serde_json::json!({
        "action": "subscribe",
        "subscriptions": [{
            "topic": TOPIC,
            "type": "*",
            // filters is a JSON *string*, e.g. {"symbol":"btc/usd"} (verified).
            "filters": serde_json::json!({ "symbol": SYMBOL }).to_string()
        }]
    });
    w.send(Message::text(sub.to_string())).await?;
    debug!("RTDS subscribed to {TOPIC} {SYMBOL}");

    // Heartbeat task: RTDS drops idle connections; send PING every 5s (verified).
    let mut ping = tokio::time::interval(Duration::from_secs(5));

    loop {
        tokio::select! {
            _ = ping.tick() => {
                w.send(Message::text("PING")).await?;
            }
            msg = r.next() => {
                let Some(msg) = msg else { break };
                let msg = msg?;
                if !msg.is_text() { continue; }
                let txt = msg.into_text()?;
                if let Ok(u) = serde_json::from_str::<Update>(&txt) {
                    if u.payload.symbol.eq_ignore_ascii_case(SYMBOL)
                        || u.payload.symbol.is_empty()
                    {
                        state.on_chainlink(u.payload.timestamp, u.payload.value);
                    }
                }
            }
        }
    }
    Ok(())
}

/// REST fallback to fetch the price-to-beat for a given market slug, in case the
/// boundary tick was missed on (re)connect. Uses the public equity endpoint shape.
pub async fn fetch_price_to_beat(client: &reqwest::Client, slug: &str) -> Result<f64> {
    #[derive(Deserialize)]
    struct Ptb {
        #[serde(alias = "priceToBeat", alias = "value")]
        value: f64,
    }
    let url = format!("https://polymarket.com/api/equity/price-to-beat/{slug}");
    let ptb: Ptb = client.get(url).send().await?.error_for_status()?.json().await?;
    Ok(ptb.value)
}
