//! Polymarket historical on-chain data scraper.
//!
//! Fetches real token prices and market resolutions from Polymarket's
//! Gamma + CLOB APIs for accurate backtesting of recurring binary markets
//! (BTC UP/DOWN 5m, etc.).
//!
//! Usage (via CLI):
//!   trader-claw backtest-sync --series btc_5m --from 2026-01-29 --to 2026-04-29

use crate::tools::polymarket_historical_types::*;
use anyhow::{anyhow, Result};
use chrono::{NaiveDate, TimeZone, Utc};
use serde_json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;

const GAMMA_API_BASE: &str = "https://gamma-api.polymarket.com";
const CLOB_API_BASE: &str = "https://clob.polymarket.com";

/// Max concurrent requests to respect CLOB API rate limits.
const MAX_CONCURRENT: usize = 5;

/// Request timeout for CLOB price history calls.
const CLOB_TIMEOUT_SECS: u64 = 30;

/// ---------------------------------------------------------------------------
/// Public API
/// ---------------------------------------------------------------------------

/// Scrape historical Polymarket data for a recurring binary series.
///
/// * `series_id` — e.g. "btc_5m" (must exist in builtin_series)
/// * `from_date` — inclusive start date "YYYY-MM-DD"
/// * `to_date`   — inclusive end date "YYYY-MM-DD"
/// * `workspace_dir` — workspace root (data written to `<workspace>/data/polymarket_historical/`)
///
/// Returns the number of windows successfully fetched.
pub async fn scrape_series(
    series_id: &str,
    from_date: &str,
    to_date: &str,
    workspace_dir: &Path,
) -> Result<usize> {
    use crate::tools::series::builtin_series;

    let series = builtin_series()
        .into_iter()
        .find(|s| s.id == series_id)
        .ok_or_else(|| anyhow!("Unknown series_id: {}. Use a builtin series.", series_id))?;

    let slug_prefix = &series.slug_prefix;
    let window_minutes = parse_cadence_to_minutes(&series.cadence);

    tracing::info!(
        "[POLY-HIST] Starting scrape for series={} (slug_prefix={}, window={}m) from {} to {}",
        series_id, slug_prefix, window_minutes, from_date, to_date
    );

    // Generate expected window timestamps
    let windows = generate_window_timestamps(from_date, to_date, window_minutes)?;
    tracing::info!("[POLY-HIST] Generated {} windows to fetch", windows.len());

    // Check existing cache to skip already-fetched windows
    let cache_path = historical_cache_path(workspace_dir, series_id, from_date, to_date);
    let mut existing: HashMap<i64, HistoricalMarketWindow> = load_existing_cache(&cache_path)?;
    tracing::info!("[POLY-HIST] Loaded {} existing cached windows", existing.len());

    let client = reqwest::Client::new();
    let sem = Arc::new(Semaphore::new(MAX_CONCURRENT));

    let mut tasks = tokio::task::JoinSet::new();

    for (window_open_ts, window_close_ts, decision_ts) in &windows {
        if existing.contains_key(window_open_ts) {
            continue; // already cached
        }

        let slug = format!("{}-{}", slug_prefix, window_open_ts);
        let client = client.clone();
        let sem = sem.clone();
        let window_open = *window_open_ts;
        let window_close = *window_close_ts;
        let decision = *decision_ts;

        tasks.spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            let result = fetch_single_window(&client, &slug, window_open, window_close, decision).await;
            (window_open, result)
        });
    }

    let mut fetched = 0usize;
    let mut missing_prices = 0usize;

    while let Some(res) = tasks.join_next().await {
        match res {
            Ok((ts, Ok(Some(window)))) => {
                if window.yes_token_price.is_none() {
                    missing_prices += 1;
                }
                existing.insert(ts, window);
                fetched += 1;
            }
            Ok((ts, Ok(None))) => {
                tracing::warn!("[POLY-HIST] No market found for window {}", ts);
            }
            Ok((ts, Err(e))) => {
                tracing::warn!("[POLY-HIST] Error fetching window {}: {}", ts, e);
            }
            Err(e) => {
                tracing::warn!("[POLY-HIST] Task panicked: {}", e);
            }
        }

        // Periodic progress log
        if fetched > 0 && fetched % 50 == 0 {
            tracing::info!("[POLY-HIST] Progress: {} windows fetched so far", fetched);
        }
    }

    // Write updated cache
    let windows_vec: Vec<HistoricalMarketWindow> = windows
        .iter()
        .filter_map(|(ts, _, _)| existing.get(ts).cloned())
        .collect();

    write_cache(&cache_path, &windows_vec)?;

    let meta = HistoricalDatasetMeta {
        series_id: series_id.to_string(),
        from_date: from_date.to_string(),
        to_date: to_date.to_string(),
        windows_fetched: windows_vec.len(),
        windows_missing_prices: missing_prices,
        scraped_at: Utc::now().to_rfc3339(),
        slug_prefix: slug_prefix.clone(),
    };
    write_meta(&cache_path, &meta)?;

    tracing::info!(
        "[POLY-HIST] Scrape complete. Total windows: {} (newly fetched: {}, missing prices: {})",
        windows_vec.len(),
        fetched,
        missing_prices
    );

    Ok(windows_vec.len())
}

/// Load historical data for a series and date range.
/// Returns a map from `window_open_ts` to the historical window record.
pub fn load_historical_data(
    workspace_dir: &Path,
    series_id: &str,
    from_date: &str,
    to_date: &str,
) -> Result<HashMap<i64, HistoricalMarketWindow>> {
    let cache_path = historical_cache_path(workspace_dir, series_id, from_date, to_date);
    load_existing_cache(&cache_path)
}

/// Check if historical data exists for a given series and date range.
pub fn has_historical_data(
    workspace_dir: &Path,
    series_id: &str,
    from_date: &str,
    to_date: &str,
) -> bool {
    let cache_path = historical_cache_path(workspace_dir, series_id, from_date, to_date);
    cache_path.exists()
}

/// ---------------------------------------------------------------------------
/// Internal helpers
/// ---------------------------------------------------------------------------

/// Fetch BTC open and close prices for a window from Binance klines.
/// Returns (open_price, close_price) at the window boundaries.
async fn fetch_btc_window_prices(
    client: &reqwest::Client,
    window_open_ts: i64,
    window_close_ts: i64,
) -> Option<(f64, f64)> {
    let url = format!(
        "https://api.binance.com/api/v3/klines?symbol=BTCUSDT&interval=1m&startTime={}&endTime={}&limit=2",
        window_open_ts * 1000,
        window_close_ts * 1000,
    );
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let klines: Vec<Vec<serde_json::Value>> = resp.json().await.ok()?;
    // First kline open → window open price
    let open = klines.first()?.get(1)?.as_str()?.parse::<f64>().ok()?;
    // Last kline close → window close price
    let close = klines.last()?.get(4)?.as_str()?.parse::<f64>().ok()?;
    Some((open, close))
}

/// Fetch data for a single market window.
async fn fetch_single_window(
    client: &reqwest::Client,
    slug: &str,
    window_open_ts: i64,
    window_close_ts: i64,
    decision_ts: i64,
) -> Result<Option<HistoricalMarketWindow>> {
    // 1. Resolve market via Gamma API
    let market = match fetch_gamma_market(client, slug).await? {
        Some(m) => m,
        None => return Ok(None),
    };

    // 2. Resolve token IDs via CLOB
    let (yes_token_id, no_token_id) = match fetch_clob_token_ids(client, &market.condition_id).await? {
        Some(ids) => ids,
        None => return Ok(None),
    };

    // 3. Fetch token prices at decision time (minute 4 of a 5m window)
    let yes_price = fetch_price_at_time(client, &yes_token_id, decision_ts).await.ok();
    let no_price = fetch_price_at_time(client, &no_token_id, decision_ts).await.ok();

    // 4. Fetch BTC open/close to determine resolution (UP or DOWN)
    let (btc_open, btc_close, resolution) =
        match fetch_btc_window_prices(client, window_open_ts, window_close_ts).await {
            Some((o, c)) => {
                let res = if c > o { "up" } else { "down" };
                (Some(o), Some(c), Some(res.to_string()))
            }
            None => (None, None, market.resolution),
        };

    // 5. Build record
    let record = HistoricalMarketWindow {
        window_open_ts,
        window_close_ts,
        decision_ts,
        condition_id: market.condition_id,
        yes_token_id,
        no_token_id,
        yes_token_price: yes_price,
        no_token_price: no_price,
        resolution,
        btc_open,
        btc_close,
        slug: slug.to_string(),
        from_cache: false,
    };

    Ok(Some(record))
}

/// Fetch a single market from the Gamma API by slug.
async fn fetch_gamma_market(
    client: &reqwest::Client,
    slug: &str,
) -> Result<Option<ResolvedMarket>> {
    let url = format!("{}/events?slug={}", GAMMA_API_BASE, slug);

    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| anyhow!("Gamma events request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(anyhow!("Gamma API returned {} for slug {}", resp.status(), slug));
    }

    let events: Vec<GammaEventResponse> = resp
        .json()
        .await
        .map_err(|e| anyhow!("Gamma events JSON parse failed: {}", e))?;

    // Take the first event with active/closed markets
    for event in events {
        for m in event.markets {
            // For recurring binaries, we want markets that have tokens
            let has_tokens = !m.tokens.is_empty() || m.clob_token_ids.is_some();
            if !has_tokens {
                continue;
            }

            // Determine resolution if available
            let resolution = if m.closed {
                // Try to infer from question or tokens
                None // Will be filled later from Binance or on-chain
            } else {
                None
            };

            return Ok(Some(ResolvedMarket {
                condition_id: m.condition_id,
                slug: m.slug,
                resolution,
            }));
        }
    }

    // Fallback: try /markets endpoint directly
    let url = format!("{}/markets?slug={}", GAMMA_API_BASE, slug);
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    if let Ok(resp) = resp {
        if resp.status().is_success() {
            if let Ok(markets) = resp.json::<Vec<crate::tools::polymarket_historical_types::GammaMarketInEvent>>().await {
                for m in markets {
                    if !m.condition_id.is_empty() {
                        return Ok(Some(ResolvedMarket {
                            condition_id: m.condition_id,
                            slug: m.slug,
                            resolution: None,
                        }));
                    }
                }
            }
        }
    }

    Ok(None)
}

/// Fetch YES/NO token IDs from CLOB for a given condition ID.
async fn fetch_clob_token_ids(
    client: &reqwest::Client,
    condition_id: &str,
) -> Result<Option<(String, String)>> {
    let url = format!("{}/markets/{}", CLOB_API_BASE, condition_id);

    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| anyhow!("CLOB token fetch failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(anyhow!("CLOB returned {} for condition {}", resp.status(), condition_id));
    }

    let body = resp.text().await?;

    // Try structured response first
    #[derive(serde::Deserialize)]
    struct ClobMarketResponse {
        tokens: Vec<ClobToken>,
    }
    #[derive(serde::Deserialize)]
    struct ClobToken {
        token_id: String,
        outcome: String,
    }

    if let Ok(parsed) = serde_json::from_str::<ClobMarketResponse>(&body) {
        let yes = parsed.tokens.iter().find(|t|
            t.outcome.eq_ignore_ascii_case("Yes") || t.outcome.eq_ignore_ascii_case("Up")
        ).map(|t| t.token_id.clone());
        let no = parsed.tokens.iter().find(|t|
            t.outcome.eq_ignore_ascii_case("No") || t.outcome.eq_ignore_ascii_case("Down")
        ).map(|t| t.token_id.clone());

        if let (Some(y), Some(n)) = (yes, no) {
            return Ok(Some((y, n)));
        }
    }

    // Fallback: extract from generic JSON
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| anyhow!("CLOB response is not valid JSON: {}", e))?;

    if let Some(tokens) = value.get("tokens").and_then(|v| v.as_array()) {
        let yes = tokens.iter().filter_map(|t| {
            let token_id = t.get("token_id")
                .or_else(|| t.get("tokenId"))
                .or_else(|| t.get("asset_id"))
                .and_then(|v| v.as_str())?;
            let outcome = t.get("outcome").and_then(|v| v.as_str()).unwrap_or("");
            if outcome.eq_ignore_ascii_case("Yes") || outcome.eq_ignore_ascii_case("Up") {
                Some(token_id.to_string())
            } else {
                None
            }
        }).next();

        let no = tokens.iter().filter_map(|t| {
            let token_id = t.get("token_id")
                .or_else(|| t.get("tokenId"))
                .or_else(|| t.get("asset_id"))
                .and_then(|v| v.as_str())?;
            let outcome = t.get("outcome").and_then(|v| v.as_str()).unwrap_or("");
            if outcome.eq_ignore_ascii_case("No") || outcome.eq_ignore_ascii_case("Down") {
                Some(token_id.to_string())
            } else {
                None
            }
        }).next();

        if let (Some(y), Some(n)) = (yes, no) {
            return Ok(Some((y, n)));
        }
    }

    Err(anyhow!("Could not extract YES/NO token IDs from CLOB response for {}", condition_id))
}

/// Fetch the token price at a specific timestamp (or nearest within ±2 min).
async fn fetch_price_at_time(
    client: &reqwest::Client,
    token_id: &str,
    target_ts: i64,
) -> Result<f64> {
    let fidelity = 1; // 1 minute
    let start_ts = target_ts - 120; // 2 min before
    let end_ts = target_ts + 120;   // 2 min after

    let url = format!(
        "{}/prices-history?market={}&fidelity={}&startTs={}&endTs={}",
        CLOB_API_BASE, token_id, fidelity, start_ts, end_ts
    );

    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(CLOB_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| anyhow!("CLOB prices-history request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("CLOB prices-history error ({}): {}", status, body));
    }

    let body = resp.text().await?;
    let data: ClobPriceHistory = serde_json::from_str(&body)
        .map_err(|e| anyhow!("Failed to parse CLOB prices: {}", e))?;

    // Find the price point closest to target_ts
    let closest = data
        .history
        .into_iter()
        .min_by_key(|p| (p.t - target_ts).abs())
        .ok_or_else(|| anyhow!("No price points found for token {} near {}", token_id, target_ts))?;

    Ok(closest.p)
}

/// Generate (window_open, window_close, decision) timestamps for a date range.
fn generate_window_timestamps(
    from_date: &str,
    to_date: &str,
    window_minutes: i64,
) -> Result<Vec<(i64, i64, i64)>> {
    let from = NaiveDate::parse_from_str(from_date, "%Y-%m-%d")
        .map_err(|e| anyhow!("Invalid from_date: {}", e))?;
    let to = NaiveDate::parse_from_str(to_date, "%Y-%m-%d")
        .map_err(|e| anyhow!("Invalid to_date: {}", e))?;

    let from_dt = Utc.from_utc_datetime(&from.and_hms_opt(0, 0, 0).unwrap());
    let to_dt = Utc.from_utc_datetime(&to.and_hms_opt(23, 59, 59).unwrap());

    let window_secs = window_minutes * 60;
    let decision_offset = (window_minutes - 1) * 60; // decision at minute N-1 (last minute before resolution)

    // Align first window to the next boundary
    let first_ts = align_up_to(from_dt.timestamp(), window_secs);

    let mut windows = Vec::new();
    let mut ts = first_ts;
    while ts <= to_dt.timestamp() {
        windows.push((ts, ts + window_secs, ts + decision_offset));
        ts += window_secs;
    }

    Ok(windows)
}

/// Align a timestamp up to the next window boundary.
fn align_up_to(ts: i64, window_secs: i64) -> i64 {
    let remainder = ts % window_secs;
    if remainder == 0 {
        ts
    } else {
        ts + window_secs - remainder
    }
}

/// Parse cadence string like "5m", "15m", "1h" to minutes.
fn parse_cadence_to_minutes(cadence: &str) -> i64 {
    if let Some(m) = cadence.strip_suffix('m') {
        m.parse::<i64>().unwrap_or(5)
    } else if let Some(h) = cadence.strip_suffix('h') {
        h.parse::<i64>().unwrap_or(1) * 60
    } else if let Some(d) = cadence.strip_suffix('d') {
        d.parse::<i64>().unwrap_or(1) * 60 * 24
    } else {
        5
    }
}

/// ---------------------------------------------------------------------------
/// Cache I/O
/// ---------------------------------------------------------------------------

fn historical_cache_path(
    workspace_dir: &Path,
    series_id: &str,
    _from_date: &str,
    _to_date: &str,
) -> PathBuf {
    workspace_dir
        .join("data")
        .join("polymarket_historical")
        .join(format!("{}.jsonl", series_id))
}

fn load_existing_cache(path: &Path) -> Result<HashMap<i64, HistoricalMarketWindow>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = std::fs::read_to_string(path)?;
    let mut map = HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(window) = serde_json::from_str::<HistoricalMarketWindow>(line) {
            map.insert(window.window_open_ts, window);
        }
    }

    Ok(map)
}

fn write_cache(path: &Path, windows: &[HistoricalMarketWindow]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut content = String::new();
    for window in windows {
        let json = serde_json::to_string(window)?;
        content.push_str(&json);
        content.push('\n');
    }

    std::fs::write(path, content)?;
    Ok(())
}

fn write_meta(cache_path: &Path, meta: &HistoricalDatasetMeta) -> Result<()> {
    let meta_path = cache_path.with_extension("meta.json");
    let json = serde_json::to_string_pretty(meta)?;
    std::fs::write(meta_path, json)?;
    Ok(())
}

/// ---------------------------------------------------------------------------
/// Internal structs
/// ---------------------------------------------------------------------------

struct ResolvedMarket {
    condition_id: String,
    slug: String,
    resolution: Option<String>,
}
