//! `trader-claw backfill-historical`
//!
//! Re-resolves every cached Polymarket window in
//! `<workspace>/data/polymarket_historical/<series>.jsonl` against the live
//! Gamma `outcomePrices` (chain-truth, Chainlink-backed for UP/DOWN markets).
//!
//! The original scraper inferred resolution from Binance close>open which is
//! NOT what Polymarket pays out. This tool walks each cached record, queries
//! Gamma per slug, and overwrites only the `resolution` field. Token prices
//! and timestamps are preserved.
//!
//! Usage:
//! ```text
//! trader-claw backfill-historical --dry-run
//! trader-claw backfill-historical --series btc_5m
//! trader-claw backfill-historical                    # all series
//! ```

use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::time::Duration;

use crate::config::schema::Config;
use crate::tools::polymarket_historical_types::HistoricalMarketWindow;

pub async fn run_backfill(
    config: &Config,
    dry_run: bool,
    only_series: Option<&str>,
) -> Result<()> {
    let dir: PathBuf = config.workspace_dir.join("data").join("polymarket_historical");
    if !dir.exists() {
        return Err(anyhow!("Historical cache dir not found: {}", dir.display()));
    }

    // Each series id has TWO cached files: `<series>.jsonl` (decision-time
    // prices) and `min3_<series>.jsonl` (one minute earlier). Both store the
    // same resolution column, so both need correcting.
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let p = entry?.path();
        if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let stem = match p.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if let Some(s) = only_series {
            // match either `btc_5m` or `min3_btc_5m`
            if stem != s && stem != format!("min3_{}", s) {
                continue;
            }
        }
        files.push(p);
    }
    files.sort();
    if files.is_empty() {
        return Err(anyhow!(
            "No matching cache files in {} (series filter: {:?})",
            dir.display(), only_series
        ));
    }
    println!("Found {} cache file(s) to process", files.len());

    let mut total_examined = 0usize;
    let mut total_changed  = 0usize;
    let mut total_unchanged = 0usize;
    let mut total_unknown  = 0usize;

    for path in &files {
        println!("\nProcessing {}", path.display());
        let raw = std::fs::read_to_string(path)?;
        let mut records: Vec<HistoricalMarketWindow> = raw.lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<HistoricalMarketWindow>(l).ok())
            .collect();
        println!("  Loaded {} records", records.len());

        let mut examined = 0usize;
        let mut changed  = 0usize;
        let mut unchanged = 0usize;
        let mut unknown  = 0usize;

        for rec in records.iter_mut() {
            examined += 1;
            let before = rec.resolution.clone();
            let res = match polymarket_trader::markets::get_market_resolution(&rec.slug).await {
                Ok(r) => r,
                Err(_) => {
                    unknown += 1;
                    continue;
                }
            };
            let new = match res.yes_won {
                Some(true)  => Some("up".to_string()),
                Some(false) => Some("down".to_string()),
                None => { unknown += 1; continue; }
            };
            if new != before {
                rec.resolution = new;
                changed += 1;
            } else {
                unchanged += 1;
            }
            // Soft rate-limit: Gamma is cheap but we're hitting it many times.
            tokio::time::sleep(Duration::from_millis(40)).await;
        }

        println!(
            "  examined={examined}  changed={changed}  unchanged={unchanged}  unknown/closed={unknown}"
        );

        if !dry_run && changed > 0 {
            let backup = path.with_extension(format!(
                "jsonl.bak.{}", chrono::Utc::now().timestamp()
            ));
            std::fs::copy(path, &backup)?;
            let mut buf = String::with_capacity(raw.len());
            for r in &records {
                buf.push_str(&serde_json::to_string(r)?);
                buf.push('\n');
            }
            std::fs::write(path, buf)?;
            println!("  wrote (backup: {})", backup.display());
        }

        total_examined += examined;
        total_changed += changed;
        total_unchanged += unchanged;
        total_unknown += unknown;
    }

    println!("\n────────────────────────────────────────────────────────");
    println!("Files processed         : {}", files.len());
    println!("Records examined        : {total_examined}");
    println!("Records resolution-fixed: {total_changed}");
    println!("Records unchanged       : {total_unchanged}");
    println!("Records unknown/missing : {total_unknown}");
    println!("────────────────────────────────────────────────────────");

    if dry_run {
        println!("\n[dry-run] No changes written.");
    }

    Ok(())
}
