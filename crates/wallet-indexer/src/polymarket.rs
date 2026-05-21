//! Polymarket-specific indexer: leaderboard + per-wallet trade history.

use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

/// Polymarket leaderboard entry from the public Data API.
#[derive(Debug, Deserialize)]
pub struct LeaderboardEntry {
    pub address: String,
    pub profit: f64,
    pub volume: f64,
    pub roi: f64,
}

/// Fetch the top-N leaderboard from Polymarket Data API.
pub async fn fetch_leaderboard(
    client: &reqwest::Client,
    limit: usize,
) -> Result<Vec<LeaderboardEntry>> {
    let url = format!(
        "https://data-api.polymarket.com/leaderboard?limit={}",
        limit
    );
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Leaderboard API returned {}", resp.status());
    }
    let entries: Vec<LeaderboardEntry> = resp.json().await?;
    Ok(entries)
}

/// Fetch recent trades for a specific wallet.
///
/// Mirrors the live Polymarket Data API shape (https://data-api.polymarket.com/trades).
#[derive(Debug, Deserialize)]
pub struct TradeEntry {
    #[serde(rename = "proxyWallet", default)]
    pub proxy_wallet: String,
    #[serde(default)]
    pub side: String,
    #[serde(default)]
    pub asset: String,
    #[serde(rename = "conditionId", default)]
    pub condition_id: Option<String>,
    #[serde(default)]
    pub size: f64,
    #[serde(default)]
    pub price: f64,
    #[serde(default)]
    pub timestamp: i64,
    #[serde(rename = "slug", default)]
    pub market_slug: String,
    #[serde(rename = "transactionHash", default)]
    pub transaction_hash: Option<String>,
}

pub async fn fetch_wallet_trades(
    client: &reqwest::Client,
    address: &str,
    limit: usize,
) -> Result<Vec<TradeEntry>> {
    let url = format!(
        "https://data-api.polymarket.com/trades?user={}&limit={}",
        address, limit
    );
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Trades API returned {}", resp.status());
    }
    let trades: Vec<TradeEntry> = resp.json().await?;
    Ok(trades)
}

/// Build a simple category map from market slugs.
/// In production this should query the Gamma API for tags.
pub fn infer_category(slug: &str) -> Option<String> {
    let lower = slug.to_lowercase();
    if lower.contains("election")
        || lower.contains("president")
        || lower.contains("politic")
    {
        Some("politics".into())
    } else if lower.contains("bitcoin")
        || lower.contains("ethereum")
        || lower.contains("crypto")
    {
        Some("crypto".into())
    } else if lower.contains("nba")
        || lower.contains("nfl")
        || lower.contains("soccer")
        || lower.contains("sport")
    {
        Some("sports".into())
    } else if lower.contains("gdp")
        || lower.contains("inflation")
        || lower.contains("fed")
    {
        Some("finance".into())
    } else {
        Some("entertainment".into())
    }
}
