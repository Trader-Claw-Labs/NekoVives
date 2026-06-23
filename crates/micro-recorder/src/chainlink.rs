//! Polymarket Real-Time Data Socket (RTDS) — the Chainlink resolving price.
//!
//! Polymarket's BTC Up/Down markets settle against a Chainlink Data Stream,
//! surfaced over RTDS under the `crypto_prices_chainlink` topic. This is the
//! price the market ACTUALLY resolves against — not Binance. The first tick at
//! or after a window's open boundary is the "Price to Beat".
//!
//! Endpoint: wss://ws-live-data.polymarket.com
//! Sub:      { action: "subscribe", subscriptions: [{ topic, type:"*", filters }] }
//! Heartbeat: send "PING" every 5s or the server drops the connection.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::types::RawEvent;

const RTDS_URL: &str = "wss://ws-live-data.polymarket.com";
const TOPIC: &str = "crypto_prices_chainlink";
const SYMBOL: &str = "btc/usd";

#[derive(Deserialize)]
struct Update {
    #[allow(dead_code)]
    #[serde(default)]
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

pub fn spawn(tx: Sender<RawEvent>) {
    tokio::spawn(run(tx));
}

async fn run(tx: Sender<RawEvent>) {
    let mut backoff = Duration::from_millis(500);
    loop {
        match connect_once(&tx).await {
            Ok(_) => {
                tracing::warn!("[CHAINLINK] RTDS closed; reconnecting");
                backoff = Duration::from_millis(500);
            }
            Err(e) => tracing::warn!("[CHAINLINK] error: {e}; reconnect in {backoff:?}"),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(15));
    }
}

async fn connect_once(tx: &Sender<RawEvent>) -> anyhow::Result<()> {
    let (ws, _) = connect_async(RTDS_URL).await?;
    let (mut w, mut r) = ws.split();

    let sub = serde_json::json!({
        "action": "subscribe",
        "subscriptions": [{
            "topic": TOPIC,
            "type": "*",
            "filters": serde_json::json!({ "symbol": SYMBOL }).to_string()
        }]
    });
    w.send(Message::text(sub.to_string())).await?;
    tracing::info!("[CHAINLINK] subscribed {TOPIC} {SYMBOL}");

    let mut ping = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = ping.tick() => { w.send(Message::text("PING")).await?; }
            msg = r.next() => {
                let Some(msg) = msg else { break };
                let msg = msg?;
                if let Message::Text(txt) = msg {
                    if let Ok(u) = serde_json::from_str::<Update>(&txt) {
                        if u.payload.symbol.eq_ignore_ascii_case(SYMBOL) || u.payload.symbol.is_empty() {
                            let _ = tx.send(RawEvent::Oracle {
                                ts_ms: u.payload.timestamp,
                                price: u.payload.value,
                            }).await;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
