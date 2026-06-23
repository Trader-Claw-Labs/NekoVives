//! Binance public WebSocket clients (no API key required).
//!
//! Two venues, each a combined stream that auto-reconnects with capped backoff:
//!   - SPOT `wss://stream.binance.com:9443/stream` — aggTrade + depth20@100ms
//!   - PERP `wss://fstream.binance.com/stream` — aggTrade + depth20@100ms +
//!     forceOrder (liquidations) + markPrice@1s (funding)
//!
//! Each parsed message is forwarded to the recorder as a `RawEvent`. We publish
//! nothing while reconnecting; downstream analysis must treat gaps as gaps.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::types::{
    BookSnapshot, Level, LiqPrint, RawEvent, TradePrint, SRC_BINANCE_PERP, SRC_BINANCE_SPOT,
};

const SPOT_URL: &str =
    "wss://stream.binance.com:9443/stream?streams=btcusdt@aggTrade/btcusdt@depth20@100ms";
const PERP_URL: &str = "wss://fstream.binance.com/stream?streams=\
btcusdt@aggTrade/btcusdt@depth20@100ms/btcusdt@forceOrder/btcusdt@markPrice@1s";

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

/// Partial book depth. SPOT uses `bids`/`asks` with NO event time; FUTURES uses
/// `b`/`a` with `E` (event) / `T` (transaction) times. Aliases cover both shapes.
#[derive(Deserialize)]
struct Depth {
    #[serde(rename = "E", default)]
    event_ms: i64,
    #[serde(default, alias = "b")]
    bids: Vec<[String; 2]>,
    #[serde(default, alias = "a")]
    asks: Vec<[String; 2]>,
}

/// `@forceOrder` payload — the liquidation order is nested under `o`.
#[derive(Deserialize)]
struct ForceOrder {
    #[serde(rename = "E", default)]
    event_ms: i64,
    o: ForceOrderInner,
}
#[derive(Deserialize)]
struct ForceOrderInner {
    #[serde(rename = "S")]
    side: String, // "BUY" | "SELL"
    #[serde(rename = "p")]
    price: String,
    #[serde(rename = "q")]
    qty: String,
    #[serde(rename = "T", default)]
    trade_ms: i64,
}

#[derive(Deserialize)]
struct MarkPrice {
    #[serde(rename = "E", default)]
    event_ms: i64,
    #[serde(rename = "p")]
    mark_price: String,
    #[serde(rename = "i", default)]
    index_price: String,
    #[serde(rename = "r", default)]
    funding_rate: String,
    #[serde(rename = "T", default)]
    next_funding_ms: i64,
}

fn parse_levels(raw: &[[String; 2]]) -> Vec<Level> {
    raw.iter()
        .filter_map(|[p, q]| {
            Some(Level {
                price: p.parse().ok()?,
                qty: q.parse().ok()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::Depth;

    // Spot depth uses `bids`/`asks`; futures uses `b`/`a`. The serde aliases must
    // accept both, or the perp book parses empty (regression guard).
    #[test]
    fn parses_spot_depth_shape() {
        let v = serde_json::json!({ "bids": [["100.0", "1.5"]], "asks": [["101.0", "2.0"]] });
        let d: Depth = serde_json::from_value(v).unwrap();
        assert_eq!(d.bids.len(), 1);
        assert_eq!(d.asks.len(), 1);
    }

    #[test]
    fn parses_futures_depth_shape() {
        let v = serde_json::json!({ "E": 1700, "b": [["100.0", "1.5"]], "a": [["101.0", "2.0"]] });
        let d: Depth = serde_json::from_value(v).unwrap();
        assert_eq!(d.event_ms, 1700);
        assert_eq!(d.bids.len(), 1, "futures `b` field must populate bids");
        assert_eq!(d.asks.len(), 1, "futures `a` field must populate asks");
    }
}

/// Spawn the SPOT feed task.
pub fn spawn_spot(tx: Sender<RawEvent>) {
    tokio::spawn(run(SPOT_URL, SRC_BINANCE_SPOT, tx));
}

/// Spawn the PERP (futures) feed task.
pub fn spawn_perp(tx: Sender<RawEvent>) {
    tokio::spawn(run(PERP_URL, SRC_BINANCE_PERP, tx));
}

async fn run(url: &'static str, src: &'static str, tx: Sender<RawEvent>) {
    let mut backoff = Duration::from_millis(500);
    loop {
        match connect_once(url, src, &tx).await {
            Ok(_) => {
                tracing::warn!("[BINANCE:{src}] stream closed; reconnecting");
                backoff = Duration::from_millis(500);
            }
            Err(e) => tracing::warn!("[BINANCE:{src}] error: {e}; reconnect in {backoff:?}"),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(15));
    }
}

async fn connect_once(
    url: &str,
    src: &'static str,
    tx: &Sender<RawEvent>,
) -> anyhow::Result<()> {
    let (ws, _) = connect_async(url).await?;
    let (mut w, mut r) = ws.split();
    tracing::info!("[BINANCE:{src}] connected");
    // Binance sends server pings; reply with pong. A 3-min app ping keeps NAT alive.
    let mut ping = tokio::time::interval(Duration::from_secs(150));
    ping.tick().await;

    loop {
        tokio::select! {
            _ = ping.tick() => {
                w.send(Message::Ping(Default::default())).await?;
            }
            msg = r.next() => {
                let Some(msg) = msg else { break };
                let msg = msg?;
                match msg {
                    Message::Ping(p) => { let _ = w.send(Message::Pong(p)).await; continue; }
                    Message::Text(txt) => handle_text(&txt, src, tx).await,
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

async fn handle_text(txt: &str, src: &'static str, tx: &Sender<RawEvent>) {
    let Ok(Combined { stream, data }) = serde_json::from_str::<Combined>(txt) else {
        return;
    };

    if stream.ends_with("aggTrade") {
        if let Ok(a) = serde_json::from_value::<AggTrade>(data) {
            if let (Ok(price), Ok(qty)) = (a.price.parse::<f64>(), a.qty.parse::<f64>()) {
                let _ = tx
                    .send(RawEvent::Trade {
                        src,
                        t: TradePrint {
                            ts_ms: a.trade_ms,
                            price,
                            qty,
                            buyer_is_maker: a.buyer_is_maker,
                        },
                    })
                    .await;
            }
        }
    } else if stream.contains("depth") {
        if let Ok(d) = serde_json::from_value::<Depth>(data) {
            // Spot's partial-book stream carries no event time; stamp wall-clock.
            let ts_ms = if d.event_ms > 0 {
                d.event_ms
            } else {
                chrono::Utc::now().timestamp_millis()
            };
            let mut book = BookSnapshot {
                ts_ms,
                bids: parse_levels(&d.bids),
                asks: parse_levels(&d.asks),
            };
            book.normalize();
            let _ = tx.send(RawEvent::Book { src, book }).await;
        }
    } else if stream.ends_with("forceOrder") {
        if let Ok(f) = serde_json::from_value::<ForceOrder>(data) {
            if let (Ok(price), Ok(qty)) = (f.o.price.parse::<f64>(), f.o.qty.parse::<f64>()) {
                let ts = if f.o.trade_ms > 0 { f.o.trade_ms } else { f.event_ms };
                let _ = tx
                    .send(RawEvent::Liquidation {
                        src,
                        l: LiqPrint {
                            ts_ms: ts,
                            price,
                            qty,
                            // SELL liquidation == a long got force-sold.
                            buyer_is_maker: f.o.side.eq_ignore_ascii_case("SELL"),
                        },
                    })
                    .await;
            }
        }
    } else if stream.contains("markPrice") {
        if let Ok(m) = serde_json::from_value::<MarkPrice>(data) {
            let _ = tx
                .send(RawEvent::Mark {
                    src,
                    ts_ms: m.event_ms,
                    mark_price: m.mark_price.parse().unwrap_or(0.0),
                    index_price: m.index_price.parse().unwrap_or(0.0),
                    funding_rate: m.funding_rate.parse().unwrap_or(0.0),
                    next_funding_ms: m.next_funding_ms,
                })
                .await;
        }
    }
}
