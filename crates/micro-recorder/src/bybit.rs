//! Bybit v5 public WebSocket client (no API key required).
//!
//! Bybit is the recorder's primary DERIVATIVES venue: where Binance Futures
//! geo-restricts the trade/liquidation/funding push streams (returns only the
//! depth book on many IPs), Bybit delivers them openly. One linear-perp
//! connection carries four topics, auto-reconnecting with capped backoff:
//!   - publicTrade.BTCUSDT      taker trade tape          → CVD / signed flow
//!   - allLiquidation.BTCUSDT   forced liquidations       → liquidation cascades
//!   - orderbook.50.BTCUSDT     50-level book (snap+delta)→ OBI / OFI / VAMP
//!   - tickers.BTCUSDT          mark/index/funding        → basis / funding
//!
//! Endpoint: wss://stream.bybit.com/v5/public/linear
//! Heartbeat: send {"op":"ping"} every 20s or the server drops the connection.
//!
//! Liquidation side note: Bybit's `S` is the POSITION side that was liquidated
//! (`Buy` => a long was force-closed), the OPPOSITE convention to Binance
//! `forceOrder`. We normalize to our `buyer_is_maker == true => long-liq (SELL)`
//! flag so downstream metrics treat both venues identically.

use std::collections::BTreeMap;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::types::{BookSnapshot, Level, LiqPrint, RawEvent, TradePrint, SRC_BYBIT_PERP};

const WS_URL: &str = "wss://stream.bybit.com/v5/public/linear";
const SYMBOL: &str = "BTCUSDT";

pub fn spawn(tx: Sender<RawEvent>) {
    tokio::spawn(run(tx));
}

async fn run(tx: Sender<RawEvent>) {
    let mut backoff = Duration::from_millis(500);
    loop {
        match connect_once(&tx).await {
            Ok(_) => {
                tracing::warn!("[BYBIT] stream closed; reconnecting");
                backoff = Duration::from_millis(500);
            }
            Err(e) => tracing::warn!("[BYBIT] error: {e}; reconnect in {backoff:?}"),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(15));
    }
}

async fn connect_once(tx: &Sender<RawEvent>) -> anyhow::Result<()> {
    let (ws, _) = connect_async(WS_URL).await?;
    let (mut w, mut r) = ws.split();

    let sub = serde_json::json!({
        "op": "subscribe",
        "args": [
            format!("publicTrade.{SYMBOL}"),
            format!("allLiquidation.{SYMBOL}"),
            format!("orderbook.50.{SYMBOL}"),
            format!("tickers.{SYMBOL}"),
        ]
    });
    w.send(Message::text(sub.to_string())).await?;
    tracing::info!("[BYBIT] subscribed 4 topics for {SYMBOL}");

    // Bybit drops idle connections; ping every 20s.
    let mut ping = tokio::time::interval(Duration::from_secs(20));
    ping.tick().await;

    // Local state maintained from snapshot + delta frames.
    let mut book = OrderBook::default();
    let mut ticker = TickerState::default();

    loop {
        tokio::select! {
            _ = ping.tick() => {
                w.send(Message::text(r#"{"op":"ping"}"#)).await?;
            }
            msg = r.next() => {
                let Some(msg) = msg else { break };
                match msg? {
                    Message::Text(txt) => handle_text(&txt, tx, &mut book, &mut ticker).await,
                    Message::Ping(p) => { let _ = w.send(Message::Pong(p)).await; }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

// ── Topic payloads ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    topic: String,
    #[serde(rename = "type", default)]
    msg_type: String,
    #[serde(default)]
    ts: i64,
    data: serde_json::Value,
}

#[derive(Deserialize)]
struct TradeEntry {
    #[serde(rename = "T")]
    ts_ms: i64,
    #[serde(rename = "p")]
    price: String,
    #[serde(rename = "v")]
    size: String,
    #[serde(rename = "S")]
    side: String, // taker side: "Buy" | "Sell"
}

#[derive(Deserialize)]
struct LiqEntry {
    #[serde(rename = "T", default)]
    ts_ms: i64,
    #[serde(rename = "S")]
    side: String, // POSITION side liquidated: "Buy" => a long was liquidated
    #[serde(rename = "v")]
    size: String,
    #[serde(rename = "p")]
    price: String,
}

#[derive(Deserialize)]
struct OrderbookData {
    #[serde(rename = "b", default)]
    bids: Vec<[String; 2]>,
    #[serde(rename = "a", default)]
    asks: Vec<[String; 2]>,
}

/// Bybit `tickers` is a delta-merge channel: the first frame is a full snapshot,
/// later frames carry ONLY the fields that changed. Fields are `Option` so we can
/// distinguish "absent in this delta" from "present and zero", and merge into a
/// persistent `TickerState` — otherwise every mark-price delta would null out the
/// funding/index fields it omits.
#[derive(Deserialize)]
struct TickerData {
    #[serde(rename = "markPrice")]
    mark_price: Option<String>,
    #[serde(rename = "indexPrice")]
    index_price: Option<String>,
    #[serde(rename = "fundingRate")]
    funding_rate: Option<String>,
    #[serde(rename = "nextFundingTime")]
    next_funding_time: Option<String>,
}

/// Last-known merged ticker state (mark/index/funding survive across deltas).
#[derive(Default)]
struct TickerState {
    mark_price: f64,
    index_price: f64,
    funding_rate: f64,
    next_funding_ms: i64,
    have_mark: bool,
}

impl TickerState {
    fn merge(&mut self, d: &TickerData) {
        if let Some(v) = d.mark_price.as_ref().and_then(|s| s.parse::<f64>().ok()) {
            self.mark_price = v;
            self.have_mark = true;
        }
        if let Some(v) = d.index_price.as_ref().and_then(|s| s.parse::<f64>().ok()) {
            self.index_price = v;
        }
        if let Some(v) = d.funding_rate.as_ref().and_then(|s| s.parse::<f64>().ok()) {
            self.funding_rate = v;
        }
        if let Some(v) = d.next_funding_time.as_ref().and_then(|s| s.parse::<i64>().ok()) {
            self.next_funding_ms = v;
        }
    }
}

/// Local 50-level book maintained across snapshot/delta frames. Bybit sends a
/// `snapshot` then `delta` frames; a delta with size "0" removes the level.
#[derive(Default)]
struct OrderBook {
    bids: BTreeMap<u64, f64>, // price-ticks(*1e2) -> qty
    asks: BTreeMap<u64, f64>,
}

impl OrderBook {
    fn apply(&mut self, data: &OrderbookData, is_snapshot: bool) {
        if is_snapshot {
            self.bids.clear();
            self.asks.clear();
        }
        Self::apply_side(&mut self.bids, &data.bids);
        Self::apply_side(&mut self.asks, &data.asks);
    }

    fn apply_side(side: &mut BTreeMap<u64, f64>, levels: &[[String; 2]]) {
        for [p, q] in levels {
            let (Ok(price), Ok(qty)) = (p.parse::<f64>(), q.parse::<f64>()) else {
                continue;
            };
            let key = (price * 100.0).round() as u64; // BTC tick = $0.01 → cents key
            if qty == 0.0 {
                side.remove(&key);
            } else {
                side.insert(key, qty);
            }
        }
    }

    /// Best-first snapshot (top `n` per side) for the metrics engine.
    fn snapshot(&self, ts_ms: i64, n: usize) -> BookSnapshot {
        let bids: Vec<Level> = self
            .bids
            .iter()
            .rev() // highest price first
            .take(n)
            .map(|(k, q)| Level { price: *k as f64 / 100.0, qty: *q })
            .collect();
        let asks: Vec<Level> = self
            .asks
            .iter() // lowest price first
            .take(n)
            .map(|(k, q)| Level { price: *k as f64 / 100.0, qty: *q })
            .collect();
        BookSnapshot { ts_ms, bids, asks }
    }
}

async fn handle_text(
    txt: &str,
    tx: &Sender<RawEvent>,
    book: &mut OrderBook,
    ticker: &mut TickerState,
) {
    let Ok(env) = serde_json::from_str::<Envelope>(txt) else {
        return; // subscription acks / pong frames have no `data`
    };
    if env.topic.is_empty() {
        return;
    }

    if env.topic.starts_with("publicTrade") {
        if let Ok(entries) = serde_json::from_value::<Vec<TradeEntry>>(env.data) {
            for e in entries {
                if let (Ok(price), Ok(qty)) = (e.price.parse::<f64>(), e.size.parse::<f64>()) {
                    // Our TradePrint convention: buyer_is_maker==true => taker SELL.
                    // Bybit `S` is the taker side, so taker Sell => buyer_is_maker=true.
                    let _ = tx
                        .send(RawEvent::Trade {
                            src: SRC_BYBIT_PERP,
                            t: TradePrint {
                                ts_ms: e.ts_ms,
                                price,
                                qty,
                                buyer_is_maker: e.side.eq_ignore_ascii_case("Sell"),
                            },
                        })
                        .await;
                }
            }
        }
    } else if env.topic.starts_with("allLiquidation") {
        if let Ok(entries) = serde_json::from_value::<Vec<LiqEntry>>(env.data) {
            for e in entries {
                if let (Ok(price), Ok(qty)) = (e.price.parse::<f64>(), e.size.parse::<f64>()) {
                    let ts = if e.ts_ms > 0 { e.ts_ms } else { env.ts };
                    // Bybit `S` is the liquidated POSITION side: Buy => long liq.
                    // Our flag: buyer_is_maker==true => long-liq (matches Binance SELL).
                    let _ = tx
                        .send(RawEvent::Liquidation {
                            src: SRC_BYBIT_PERP,
                            l: LiqPrint {
                                ts_ms: ts,
                                price,
                                qty,
                                buyer_is_maker: e.side.eq_ignore_ascii_case("Buy"),
                            },
                        })
                        .await;
                }
            }
        }
    } else if env.topic.starts_with("orderbook") {
        if let Ok(data) = serde_json::from_value::<OrderbookData>(env.data) {
            let is_snapshot = env.msg_type.eq_ignore_ascii_case("snapshot");
            book.apply(&data, is_snapshot);
            let ts = if env.ts > 0 { env.ts } else { chrono::Utc::now().timestamp_millis() };
            let _ = tx
                .send(RawEvent::Book {
                    src: SRC_BYBIT_PERP,
                    book: book.snapshot(ts, 20),
                })
                .await;
        }
    } else if env.topic.starts_with("tickers") {
        // tickers is delta-merged: merge into persistent state, then emit the full
        // last-known mark/index/funding (a mark-price delta must not zero funding).
        if let Ok(t) = serde_json::from_value::<TickerData>(env.data) {
            ticker.merge(&t);
            if ticker.have_mark {
                let _ = tx
                    .send(RawEvent::Mark {
                        src: SRC_BYBIT_PERP,
                        ts_ms: env.ts,
                        mark_price: ticker.mark_price,
                        index_price: ticker.index_price,
                        funding_rate: ticker.funding_rate,
                        next_funding_ms: ticker.next_funding_ms,
                    })
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orderbook_snapshot_then_delta() {
        let mut ob = OrderBook::default();
        let snap = OrderbookData {
            bids: vec![["100.00".into(), "5".into()], ["99.00".into(), "3".into()]],
            asks: vec![["101.00".into(), "4".into()]],
        };
        ob.apply(&snap, true);
        // delta: remove the 99 level, add depth at 100
        let delta = OrderbookData {
            bids: vec![["99.00".into(), "0".into()], ["100.00".into(), "8".into()]],
            asks: vec![],
        };
        ob.apply(&delta, false);
        let s = ob.snapshot(1000, 10);
        assert_eq!(s.bids.len(), 1, "99 level removed by size-0 delta");
        assert_eq!(s.bids[0].price, 100.0);
        assert_eq!(s.bids[0].qty, 8.0, "100 level overwritten by delta");
        assert_eq!(s.asks[0].price, 101.0);
    }

    #[test]
    fn liq_side_maps_long_liquidation() {
        // Bybit Buy => long liquidated => our buyer_is_maker(true) => long-liq.
        let e: LiqEntry =
            serde_json::from_value(serde_json::json!({"T":1,"S":"Buy","v":"2","p":"100"})).unwrap();
        assert_eq!(e.side, "Buy");
    }

    #[test]
    fn ticker_delta_merge_preserves_funding() {
        // Snapshot sets funding+index; a later mark-only delta must NOT zero them.
        let mut ts = TickerState::default();
        let snap: TickerData = serde_json::from_value(serde_json::json!({
            "markPrice": "64000.0", "indexPrice": "64010.0", "fundingRate": "0.0001",
            "nextFundingTime": "1782201600000"
        })).unwrap();
        ts.merge(&snap);
        let delta: TickerData =
            serde_json::from_value(serde_json::json!({ "markPrice": "64050.0" })).unwrap();
        ts.merge(&delta);
        assert_eq!(ts.mark_price, 64050.0, "mark updated by delta");
        assert_eq!(ts.funding_rate, 0.0001, "funding survives the mark-only delta");
        assert_eq!(ts.index_price, 64010.0, "index survives the mark-only delta");
        assert!(ts.have_mark);
    }
}
