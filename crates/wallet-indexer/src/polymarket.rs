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
        "https://data.polymarket.com/leaderboard?limit={}",
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
#[derive(Debug, Deserialize)]
pub struct TradeEntry {
    #[serde(rename = "id")]
    pub trade_id: String,
    #[serde(rename = "takerSide")]
    pub side: String,
    #[serde(rename = "marketSlug")]
    pub market_slug: String,
    pub size: String,
    pub price: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

pub async fn fetch_wallet_trades(
    client: &reqwest::Client,
    address: &str,
    limit: usize,
) -> Result<Vec<TradeEntry>> {
    let url = format!(
        "https://data.polymarket.com/trades?user={}&limit={}",
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
