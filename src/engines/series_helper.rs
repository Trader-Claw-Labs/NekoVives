//! Helpers for resolving the *current* Polymarket market slug of a recurring
//! series (e.g. "btc-updown-5m") to a concrete `<prefix>-<window_ts>` slug
//! that the CLOB / Gamma API will accept.
//!
//! Engines (arb_binary, fair_value, fv_momentum, arb_hedge) use this so a
//! single live runner can ride consecutive 5-minute / 15-minute / 1-hour
//! windows of a recurring market without the operator having to paste the
//! slug for every window.

use chrono::Utc;

use crate::tools::series::{builtin_series, MarketSeries};

/// Convert an interval string (`"5m"`, `"1h"`, `"1d"`, ...) into seconds.
/// Mirrors the `interval_to_secs` helper used by the legacy strategy runner.
fn interval_to_secs(interval: &str) -> u64 {
    let s = interval.trim().to_lowercase();
    if let Some(rest) = s.strip_suffix('s') {
        rest.parse::<u64>().unwrap_or(60)
    } else if let Some(rest) = s.strip_suffix('m') {
        rest.parse::<u64>().unwrap_or(1) * 60
    } else if let Some(rest) = s.strip_suffix('h') {
        rest.parse::<u64>().unwrap_or(1) * 3600
    } else if let Some(rest) = s.strip_suffix('d') {
        rest.parse::<u64>().unwrap_or(1) * 86_400
    } else {
        s.parse::<u64>().unwrap_or(60) * 60
    }
}

/// Look up a built-in market series by id.
pub fn find_series(series_id: &str) -> Option<MarketSeries> {
    builtin_series().into_iter().find(|s| s.id == series_id)
}

/// Compute the candidate slug for the *current* window of `series` —
/// **without** confirming with Polymarket.  Use [`resolve_current_slug`] to
/// also verify the market exists.
pub fn current_window_slug(series: &MarketSeries) -> String {
    let cadence_secs = interval_to_secs(&series.cadence).max(60);
    let now = Utc::now().timestamp();
    let window_ts = now - (now % cadence_secs as i64);
    format!("{}-{}", series.slug_prefix, window_ts)
}

/// Resolve the active slug for the current window of `series_id`.
///
/// Polymarket's recurring markets have varied historically between encoding
/// the window timestamp in seconds (`btc-updown-5m-1716316800`) and in
/// milliseconds (`btc-updown-5m-1716316800000`).  We try seconds first and
/// fall back to milliseconds if the seconds slug is not registered yet.
///
/// Returns `Ok(slug)` on success; the caller can then pass `slug` to
/// `polymarket_trader::markets::get_market` to fetch token IDs.
pub async fn resolve_current_slug(series_id: &str) -> anyhow::Result<String> {
    let series = find_series(series_id)
        .ok_or_else(|| anyhow::anyhow!("Unknown market series '{series_id}'"))?;

    let cadence_secs = interval_to_secs(&series.cadence).max(60);
    let now = Utc::now().timestamp();
    let window_ts = now - (now % cadence_secs as i64);
    let slug_seconds = format!("{}-{}", series.slug_prefix, window_ts);
    let slug_millis = format!("{}-{}", series.slug_prefix, window_ts * 1000);

    match polymarket_trader::markets::get_market(&slug_seconds).await {
        Ok(_) => Ok(slug_seconds),
        Err(_) => match polymarket_trader::markets::get_market(&slug_millis).await {
            Ok(_) => Ok(slug_millis),
            Err(e) => Err(anyhow::anyhow!(
                "No Polymarket market for slugs {slug_seconds} or {slug_millis}: {e}"
            )),
        },
    }
}

/// Build the per-poll list of slugs an engine should evaluate.
///
/// - When `series_id` is set, returns the single slug of the current window
///   (re-resolved every poll so the engine rides successive windows).
/// - Otherwise, falls back to splitting the legacy comma-separated `symbol`
///   field that engines historically used.
///
/// Returning an empty `Vec` is valid (the engine simply has nothing to do
/// this tick — typical when a window just rolled over and the next slug is
/// not yet listed on Gamma).
pub async fn engine_market_slugs(
    series_id: Option<&str>,
    fallback_symbol: &str,
) -> Vec<String> {
    if let Some(sid) = series_id.filter(|s| !s.is_empty()) {
        match resolve_current_slug(sid).await {
            Ok(slug) => vec![slug],
            Err(e) => {
                tracing::warn!(
                    "[engine] could not resolve current slug for series '{sid}': {e}"
                );
                vec![]
            }
        }
    } else {
        fallback_symbol
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}
