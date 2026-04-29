//! Types for Polymarket historical on-chain data.
//!
//! Defines the dataset structs used to store real Polymarket token prices,
//! market resolutions, and reference BTC prices for accurate backtesting.

use serde::{Deserialize, Serialize};

/// A single market window's historical data record.
///
/// Each record corresponds to one recurring binary market window
/// (e.g. one 5-minute BTC UP/DOWN market on Polymarket).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalMarketWindow {
    /// Unix timestamp (seconds) when this market window opened
    pub window_open_ts: i64,
    /// Unix timestamp (seconds) when this market window closed / resolved
    pub window_close_ts: i64,
    /// Unix timestamp (seconds) when the strategy places its bet
    /// (typically window_open + (window_minutes - 2) * 60)
    pub decision_ts: i64,
    /// Polymarket condition ID for this market
    pub condition_id: String,
    /// CLOB token ID for the YES/Up outcome
    pub yes_token_id: String,
    /// CLOB token ID for the NO/Down outcome
    pub no_token_id: String,
    /// Real price of the YES token at decision time (from CLOB /prices-history)
    pub yes_token_price: Option<f64>,
    /// Real price of the NO token at decision time (from CLOB /prices-history)
    pub no_token_price: Option<f64>,
    /// Actual resolution outcome: "up" (YES paid) or "down" (NO paid)
    pub resolution: Option<String>,
    /// BTC/USD price at window open (from Binance, for reference)
    pub btc_open: Option<f64>,
    /// BTC/USD price at window close (from Binance, for reference)
    pub btc_close: Option<f64>,
    /// Market slug from Gamma API (e.g. "btc-updown-5m-1740000000")
    pub slug: String,
    /// Whether this record came from cached data (true) or was freshly fetched (false)
    #[serde(default)]
    pub from_cache: bool,
}

/// Metadata for a scraped dataset file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalDatasetMeta {
    /// Series ID (e.g. "btc_5m")
    pub series_id: String,
    /// Date range start (inclusive)
    pub from_date: String,
    /// Date range end (inclusive)
    pub to_date: String,
    /// How many windows were successfully fetched
    pub windows_fetched: usize,
    /// How many windows had missing token prices
    pub windows_missing_prices: usize,
    /// Timestamp when this dataset was generated
    pub scraped_at: String,
    /// Slug prefix used for market discovery (e.g. "btc-updown-5m")
    pub slug_prefix: String,
}

/// A lightweight index entry for fast date → file lookups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalDatasetIndex {
    /// List of dataset files available
    pub datasets: Vec<DatasetIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetIndexEntry {
    pub series_id: String,
    pub from_date: String,
    pub to_date: String,
    pub file_path: String,
    pub window_count: usize,
}

/// Parsed result from the Gamma API events endpoint for a single market.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GammaEventResponse {
    pub id: String,
    pub slug: String,
    #[serde(rename = "startDate", default)]
    pub start_date: Option<String>,
    #[serde(rename = "endDate", default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub markets: Vec<GammaMarketInEvent>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GammaMarketInEvent {
    pub id: String,
    #[serde(rename = "conditionId", default)]
    pub condition_id: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub question: String,
    #[serde(rename = "endDateIso", default)]
    pub end_date_iso: Option<String>,
    #[serde(default)]
    pub tokens: Vec<GammaToken>,
    #[serde(rename = "clobTokenIds", default)]
    pub clob_token_ids: Option<serde_json::Value>,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub closed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GammaToken {
    #[serde(rename = "token_id", default)]
    pub token_id: String,
    #[serde(default)]
    pub outcome: String,
}

/// Price point from CLOB /prices-history response.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ClobPricePoint {
    pub t: i64, // timestamp in seconds
    pub p: f64, // price 0.0-1.0
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ClobPriceHistory {
    pub history: Vec<ClobPricePoint>,
}
