//! Polymarket discovery + CLOB market-channel WebSocket.
//!
//! Discovery: the rolling BTC Up/Down 5-min market has a deterministic slug
//! `btc-updown-5m-<window_open_unix_secs>`. Gamma resolves slug → conditionId +
//! clobTokenIds (`[0]`=Up/YES, `[1]`=Down/NO, confirmed via CLOB `/markets`).
//!
//! WebSocket: `wss://ws-subscriptions-clob.polymarket.com/ws/market`. We subscribe
//! to the active window's Up+Down token ids, ingest the initial `book` snapshot and
//! `price_change` deltas (size "0" => level removed), and forward both raw books and
//! trade prints. The token ids change every 5 minutes, so we re-discover and
//! re-subscribe at each window boundary, holding current+next for seamless coverage.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::types::{Level, RawEvent, WindowInfo};

const GAMMA: &str = "https://gamma-api.polymarket.com";
const WS_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
const WINDOW_SECS: i64 = 300;
const SERIES_PREFIX: &str = "btc-updown-5m";

#[derive(Deserialize)]
struct GammaMarket {
    #[serde(rename = "conditionId")]
    condition_id: Option<String>,
    /// JSON-encoded string array: `"[\"<up>\", \"<down>\"]"`.
    #[serde(rename = "clobTokenIds")]
    clob_token_ids: Option<String>,
}

/// Resolve one window-open timestamp → WindowInfo, or None if not listed yet.
async fn discover(client: &reqwest::Client, open_secs: i64) -> Option<WindowInfo> {
    let slug = format!("{SERIES_PREFIX}-{open_secs}");
    let url = format!("{GAMMA}/markets?slug={slug}");
    let markets: Vec<GammaMarket> = client.get(&url).send().await.ok()?.json().await.ok()?;
    let m = markets.into_iter().next()?;
    let condition_id = m.condition_id?;
    let toks: Vec<String> = serde_json::from_str(&m.clob_token_ids?).ok()?;
    if toks.len() < 2 {
        return None;
    }
    Some(WindowInfo {
        slug,
        condition_id,
        up_token: toks[0].clone(),
        down_token: toks[1].clone(),
        open_ms: open_secs * 1000,
        close_ms: (open_secs + WINDOW_SECS) * 1000,
    })
}

/// Spawn the discovery + WS task. `tx` receives Window + PmBook/PmPriceChange/PmTrade.
pub fn spawn(tx: Sender<RawEvent>) {
    tokio::spawn(run(tx));
}

async fn run(tx: Sender<RawEvent>) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap_or_default();

    loop {
        let now_s = chrono::Utc::now().timestamp();
        let cur_open = now_s - now_s.rem_euclid(WINDOW_SECS);
        let next_open = cur_open + WINDOW_SECS;

        // Hold current + next window tokens for seamless boundary coverage.
        let cur = discover(&client, cur_open).await;
        let next = discover(&client, next_open).await;

        let mut asset_ids: Vec<String> = Vec::new();
        if let Some(w) = &cur {
            tracing::info!("[POLY] window {} cid={} up={}…", w.slug, &w.condition_id[..10.min(w.condition_id.len())], &w.up_token[..8]);
            let _ = tx.send(RawEvent::Window(w.clone())).await;
            asset_ids.push(w.up_token.clone());
            asset_ids.push(w.down_token.clone());
        }
        if let Some(w) = &next {
            asset_ids.push(w.up_token.clone());
            asset_ids.push(w.down_token.clone());
        }

        if asset_ids.is_empty() {
            tracing::warn!("[POLY] no active window found; retry in 5s");
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }

        // Run the WS until the current window closes, then re-discover (next→current).
        let run_until_ms = (cur_open + WINDOW_SECS) * 1000;
        if let Err(e) = ws_session(&asset_ids, run_until_ms, &tx).await {
            tracing::warn!("[POLY] ws session error: {e}");
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

#[derive(Deserialize)]
struct PmLevel {
    price: String,
    size: String,
}

#[derive(Deserialize)]
struct BookMsg {
    asset_id: String,
    #[serde(default)]
    bids: Vec<PmLevel>,
    #[serde(default)]
    asks: Vec<PmLevel>,
    #[serde(default)]
    hash: String,
    #[serde(default)]
    timestamp: String,
}

#[derive(Deserialize)]
struct PriceChangeMsg {
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    price_changes: Vec<PriceChangeEntry>,
}

#[derive(Deserialize)]
struct PriceChangeEntry {
    asset_id: String,
    price: String,
    size: String,
    #[serde(default)]
    side: String,
    #[serde(default)]
    best_bid: String,
    #[serde(default)]
    best_ask: String,
}

#[derive(Deserialize)]
struct LastTradeMsg {
    asset_id: String,
    price: String,
    size: String,
    #[serde(default)]
    side: String,
    #[serde(default)]
    timestamp: String,
}

fn parse_pm_levels(raw: &[PmLevel]) -> Vec<Level> {
    raw.iter()
        .filter_map(|l| {
            Some(Level {
                price: l.price.parse().ok()?,
                qty: l.size.parse().ok()?,
            })
        })
        .collect()
}

fn ts_or_now(s: &str) -> i64 {
    s.parse::<i64>().ok().filter(|v| *v > 0).unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
}

async fn ws_session(
    asset_ids: &[String],
    run_until_ms: i64,
    tx: &Sender<RawEvent>,
) -> anyhow::Result<()> {
    let (ws, _) = connect_async(WS_URL).await?;
    let (mut w, mut r) = ws.split();

    let sub = serde_json::json!({
        "assets_ids": asset_ids,
        "type": "market",
        "custom_feature_enabled": true,
    });
    w.send(Message::text(sub.to_string())).await?;
    tracing::info!("[POLY] subscribed {} tokens", asset_ids.len());

    // RTDS-style servers drop idle sockets; PING every 10s.
    let mut ping = tokio::time::interval(Duration::from_secs(10));
    ping.tick().await;

    loop {
        // Re-discover once the current window has closed.
        if chrono::Utc::now().timestamp_millis() >= run_until_ms {
            return Ok(());
        }
        tokio::select! {
            _ = ping.tick() => { let _ = w.send(Message::text("PING")).await; }
            _ = tokio::time::sleep(Duration::from_secs(2)) => { /* wake to re-check run_until */ }
            msg = r.next() => {
                let Some(msg) = msg else { break };
                match msg? {
                    Message::Text(txt) => dispatch(&txt, tx).await,
                    Message::Ping(p) => { let _ = w.send(Message::Pong(p)).await; }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

/// A market-channel frame may be a single object or an array of them.
async fn dispatch(txt: &str, tx: &Sender<RawEvent>) {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(txt) else {
        return;
    };
    match val {
        serde_json::Value::Array(items) => {
            for it in items {
                handle_one(it, tx).await;
            }
        }
        other => handle_one(other, tx).await,
    }
}

async fn handle_one(val: serde_json::Value, tx: &Sender<RawEvent>) {
    let event_type = val.get("event_type").and_then(|v| v.as_str()).unwrap_or("");
    match event_type {
        "book" => {
            if let Ok(b) = serde_json::from_value::<BookMsg>(val) {
                let _ = tx
                    .send(RawEvent::PmBook {
                        ts_ms: ts_or_now(&b.timestamp),
                        asset_id: b.asset_id,
                        bids: parse_pm_levels(&b.bids),
                        asks: parse_pm_levels(&b.asks),
                        hash: b.hash,
                    })
                    .await;
            }
        }
        "price_change" => {
            if let Ok(pc) = serde_json::from_value::<PriceChangeMsg>(val) {
                let ts = ts_or_now(&pc.timestamp);
                for e in pc.price_changes {
                    let _ = tx
                        .send(RawEvent::PmPriceChange {
                            ts_ms: ts,
                            asset_id: e.asset_id,
                            price: e.price.parse().unwrap_or(0.0),
                            size: e.size.parse().unwrap_or(0.0),
                            side: e.side,
                            best_bid: e.best_bid.parse().unwrap_or(0.0),
                            best_ask: e.best_ask.parse().unwrap_or(0.0),
                        })
                        .await;
                }
            }
        }
        "last_trade_price" => {
            if let Ok(t) = serde_json::from_value::<LastTradeMsg>(val) {
                let _ = tx
                    .send(RawEvent::PmTrade {
                        ts_ms: ts_or_now(&t.timestamp),
                        asset_id: t.asset_id,
                        price: t.price.parse().unwrap_or(0.0),
                        size: t.size.parse().unwrap_or(0.0),
                        side: t.side,
                    })
                    .await;
            }
        }
        _ => {}
    }
}
