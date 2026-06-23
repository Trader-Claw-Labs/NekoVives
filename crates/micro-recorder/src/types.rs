//! Shared data types for the microstructure recorder.
//!
//! Two output planes share these types:
//!   - RAW events (`RawEvent`) — every message from every feed, full fidelity,
//!     so any metric can be recomputed offline with a different definition.
//!   - METRICS snapshots (`MetricSnapshot`) — OBI/OFI/CVD/liquidations/VAMP/basis,
//!     computed live on a fixed cadence for ready-to-use time series.

use serde::{Deserialize, Serialize};

/// Which Polymarket outcome a token represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)] // reserved for downstream consumers of the raw stream
pub enum Side {
    Up,
    Down,
}

/// One price level `[price, qty]`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Level {
    pub price: f64,
    pub qty: f64,
}

/// A single taker trade print (signed via `buyer_is_maker`).
#[derive(Debug, Clone, Copy)]
pub struct TradePrint {
    pub ts_ms: i64,
    pub price: f64,
    pub qty: f64,
    /// On Binance, `buyer_is_maker == true` => the aggressor SOLD (taker sell).
    pub buyer_is_maker: bool,
}

impl TradePrint {
    /// +qty for taker-buy, -qty for taker-sell.
    #[inline]
    pub fn signed_qty(&self) -> f64 {
        if self.buyer_is_maker {
            -self.qty
        } else {
            self.qty
        }
    }
}

/// A forced-liquidation print from the futures `@forceOrder` stream.
#[derive(Debug, Clone, Copy)]
pub struct LiqPrint {
    pub ts_ms: i64,
    pub price: f64,
    pub qty: f64,
    /// Order side reported by the exchange. `SELL` => a LONG was liquidated
    /// (sell into bids); `BUY` => a SHORT was liquidated (buy into asks).
    pub buyer_is_maker: bool, // we reuse the sell-flag convention: true => SELL liq
}

/// Top-of-book snapshot (a handful of levels per side) from a CEX venue.
#[derive(Debug, Clone, Default)]
pub struct BookSnapshot {
    pub ts_ms: i64,
    pub bids: Vec<Level>, // sorted best-first (descending price) after normalize
    pub asks: Vec<Level>, // sorted best-first (ascending price) after normalize
}

impl BookSnapshot {
    /// Normalize so that `bids[0]` is the highest bid and `asks[0]` the lowest ask,
    /// regardless of the order the venue sent them in.
    pub fn normalize(&mut self) {
        self.bids
            .sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
        self.asks
            .sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));
    }
    #[inline]
    pub fn best_bid(&self) -> Option<Level> {
        self.bids.first().copied()
    }
    #[inline]
    pub fn best_ask(&self) -> Option<Level> {
        self.asks.first().copied()
    }
    #[inline]
    pub fn mid(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(b), Some(a)) => Some((b.price + a.price) / 2.0),
            _ => None,
        }
    }
}

/// Which CEX feed an event came from.
pub const SRC_BINANCE_SPOT: &str = "binance_spot";
pub const SRC_BINANCE_PERP: &str = "binance_perp";
/// Bybit linear perp — the recorder's primary derivatives venue (trades,
/// liquidations, funding) when Binance Futures push streams are geo-restricted.
pub const SRC_BYBIT_PERP: &str = "bybit_perp";

/// The active Polymarket window the recorder is tracking.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub slug: String,
    pub condition_id: String,
    pub up_token: String,
    pub down_token: String,
    /// Window open / close in unix-ms (5-min grid).
    pub open_ms: i64,
    pub close_ms: i64,
}

/// Internal message bus: every feed task emits these to the single recorder task,
/// which both writes the raw line and folds the update into `RecorderState`.
#[derive(Debug, Clone)]
pub enum RawEvent {
    /// CEX taker trade. `src` is SRC_BINANCE_SPOT / SRC_BINANCE_PERP.
    Trade { src: &'static str, t: TradePrint },
    /// CEX top-of-book.
    Book { src: &'static str, book: BookSnapshot },
    /// Futures forced liquidation.
    Liquidation { src: &'static str, l: LiqPrint },
    /// Futures mark price + funding.
    Mark {
        src: &'static str,
        ts_ms: i64,
        mark_price: f64,
        index_price: f64,
        funding_rate: f64,
        next_funding_ms: i64,
    },
    /// Chainlink resolving price (the value Polymarket settles against).
    Oracle { ts_ms: i64, price: f64 },
    /// Polymarket CLOB full book snapshot for one outcome token.
    PmBook {
        ts_ms: i64,
        asset_id: String,
        bids: Vec<Level>,
        asks: Vec<Level>,
        hash: String,
    },
    /// Polymarket CLOB incremental price-level change.
    PmPriceChange {
        ts_ms: i64,
        asset_id: String,
        price: f64,
        size: f64,
        side: String, // "BUY" | "SELL"
        best_bid: f64,
        best_ask: f64,
    },
    /// Polymarket CLOB trade print.
    PmTrade {
        ts_ms: i64,
        asset_id: String,
        price: f64,
        size: f64,
        side: String,
    },
    /// Window rolled / (re)discovered. Logged as a `window` raw row.
    Window(WindowInfo),
}
