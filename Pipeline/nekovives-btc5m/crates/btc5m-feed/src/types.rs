//! Core data types for the BTC 5-minute feed.
//!
//! Everything here is venue-agnostic so the same `MarketState` can be fed by
//! Binance, Coinbase, OKX, Bybit, etc. and consumed by the feature/probability
//! layer without caring where the data came from.

use serde::{Deserialize, Serialize};

/// Which outcome of the Polymarket market we are reasoning about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Up,
    Down,
}

/// A single aggregated trade print from a venue.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Trade {
    /// Exchange-local event time in ms since epoch.
    pub ts_ms: i64,
    pub price: f64,
    pub qty: f64,
    /// true if the aggressor was the buyer (taker buy). Drives signed flow.
    pub buyer_is_maker: bool,
}

impl Trade {
    /// Signed size: +qty for taker-buy, -qty for taker-sell.
    /// On Binance `buyer_is_maker == true` means the BUYER was the maker, i.e.
    /// the trade was a taker SELL, hence negative.
    #[inline]
    pub fn signed_qty(&self) -> f64 {
        if self.buyer_is_maker {
            -self.qty
        } else {
            self.qty
        }
    }
}

/// One price level of a book side.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Level {
    pub price: f64,
    pub qty: f64,
}

/// Top-of-book snapshot, L levels deep per side (we only need a handful for OFI).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BookTop {
    pub ts_ms: i64,
    pub bids: Vec<Level>, // descending price
    pub asks: Vec<Level>, // ascending price
}

impl BookTop {
    #[inline]
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.first().map(|l| l.price)
    }
    #[inline]
    pub fn best_ask(&self) -> Option<f64> {
        self.asks.first().map(|l| l.price)
    }
    #[inline]
    pub fn mid(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(b), Some(a)) => Some((a + b) / 2.0),
            _ => None,
        }
    }
}

/// The Polymarket BTC Up/Down 5-minute window currently being traded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketWindow {
    pub condition_id: String,
    pub up_token_id: String,
    pub down_token_id: String,
    /// Window open / close, ms since epoch (ET windows, stored as UTC ms).
    pub open_ms: i64,
    pub close_ms: i64,
    /// Tick size required by the CLOB for this market.
    pub tick_size: f64,
    pub neg_risk: bool,
    /// The Chainlink reference price the market resolves against ("Price to Beat").
    /// Captured from the RTDS `crypto_prices_chainlink` feed at the open boundary,
    /// NOT from Binance. `None` until the open snapshot arrives.
    pub price_to_beat: Option<f64>,
}

impl MarketWindow {
    #[inline]
    pub fn seconds_remaining(&self, now_ms: i64) -> f64 {
        ((self.close_ms - now_ms) as f64 / 1000.0).max(0.0)
    }
}

/// Top-of-book for a Polymarket outcome token (Up or Down), from the CLOB WS.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct PmBook {
    pub ts_ms: i64,
    pub best_bid: f64,
    pub best_ask: f64,
    /// Size available at `best_ask` — needed for fillability checks.
    pub ask_size: f64,
    pub bid_size: f64,
}

impl PmBook {
    #[inline]
    pub fn spread(&self) -> f64 {
        self.best_ask - self.best_bid
    }
}
