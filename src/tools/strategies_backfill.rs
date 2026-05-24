//! `trader-claw backfill-strategies`
//!
//! Reconciles every order in `<workspace>/live_strategies.json` against the
//! real Polymarket data:
//!
//! 1. **Fill price**: GET `/data/trades` (CLOB authenticated endpoint) returns
//!    the on-chain fill VWAP for each `taker_order_id`. Replaces the
//!    decision-time midpoint that the runner originally stored.
//! 2. **Outcome**: GET `gamma-api.polymarket.com/markets?slug=…` returns the
//!    `outcomePrices` field, which is the chain-truth resolution (Chainlink
//!    oracle for UP/DOWN markets). Replaces the legacy Binance-candle
//!    inference that produced phantom wins on flatlines.
//! 3. **P&L**: Recomputed from the corrected fill price + outcome.
//!
//! Marks each reconciled order with `backfilled: true` so re-runs are idempotent.
//! Writes a `.bak.<unix-ts>` snapshot before overwriting the file.
//!
//! Usage:
//! ```text
//! trader-claw backfill-strategies --dry-run
//! trader-claw backfill-strategies                  # full run
//! trader-claw backfill-strategies --runner-id ABC  # one runner
//! trader-claw backfill-strategies --force          # re-do already-backfilled
//! ```

use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::time::Duration;

use polymarket_trader::auth::PolyCredentials;
use polymarket_trader::orders::{ClobClient, TradeFill};
use polymarket_trader::markets::{get_market_resolution, MarketResolution};

use crate::config::schema::Config;
use crate::strategy_runner::StoredRunner;
use crate::tools::series::builtin_series;

pub async fn run_backfill(
    config: &Config,
    dry_run: bool,
    only_runner: Option<&str>,
    force: bool,
) -> Result<()> {
    let path: PathBuf = config.workspace_dir.join("live_strategies.json");
    if !path.exists() {
        return Err(anyhow!("live_strategies.json not found at {}", path.display()));
    }

    println!("Reading {}", path.display());
    let raw = std::fs::read_to_string(&path)?;
    let mut runners: Vec<StoredRunner> = serde_json::from_str(&raw)
        .map_err(|e| anyhow!("Failed to parse live_strategies.json: {e}"))?;
    println!("Loaded {} runners", runners.len());

    // Build CLOB client from config.
    let creds = poly_creds_from_config(&config.polymarket)?;
    let clob = ClobClient::new(creds);

    // Pull the FULL trade history once. Each runner's orders are looked up by
    // taker_order_id in this map — much cheaper than a per-order API round-trip.
    println!("Fetching full CLOB trade history (this may take a minute)…");
    let all_trades = clob.get_trade_history(None, None, None).await
        .map_err(|e| anyhow!("Failed to fetch CLOB trade history: {e}"))?;
    println!("Got {} historical trades from CLOB", all_trades.len());

    // Index by taker_order_id (lowercase), since LiveOrder.order_id is the
    // taker order ID returned by POST /order.
    let mut by_order: std::collections::HashMap<String, Vec<&TradeFill>> =
        std::collections::HashMap::new();
    for t in &all_trades {
        by_order.entry(t.taker_order_id.to_lowercase()).or_default().push(t);
    }

    // Cache slug → resolution to avoid redundant Gamma calls (recurring 5m
    // markets are unique per window, but multiple orders per window are common).
    let mut res_cache: std::collections::HashMap<String, Option<MarketResolution>> =
        std::collections::HashMap::new();

    // Stats
    let mut runners_touched = 0usize;
    let mut orders_examined = 0usize;
    let mut paper_orders   = 0usize;
    let mut onchain_orders = 0usize;
    let mut orders_filled  = 0usize;
    let mut orders_resolved = 0usize;
    let mut orders_recomputed = 0usize;
    let mut onchain_unfilled = 0usize;
    let mut runner_pnl_before = 0.0_f64;
    let mut runner_pnl_after  = 0.0_f64;

    for runner in runners.iter_mut() {
        if let Some(rid) = only_runner {
            if runner.config.id != rid {
                continue;
            }
        }
        let result = match runner.result.as_mut() {
            Some(r) => r,
            None => continue,
        };
        if result.live_orders.is_empty() {
            continue;
        }
        runners_touched += 1;

        let series_id = runner.config.series_id.clone();
        let runner_id = runner.config.id.clone();
        println!(
            "\nRunner {} ({}) — {} orders",
            runner_id, runner.config.name, result.live_orders.len()
        );

        let pnl_before: f64 = result.live_orders.iter().filter_map(|o| o.pnl).sum();
        runner_pnl_before += pnl_before;

        for order in result.live_orders.iter_mut() {
            orders_examined += 1;
            if order.backfilled && !force {
                continue;
            }

            // Paper-trade orders never went on-chain (id prefix "paper-").
            // CLOB has no fill record for them, so we only correct their
            // *resolution* via Polymarket Gamma and recompute P&L from the
            // synthetic decision-time entry_price the runner stored.
            let is_paper = order.order_id.starts_with("paper-");
            if is_paper { paper_orders += 1; } else { onchain_orders += 1; }

            // ── 1. Real fill price (on-chain only) ─────────────────────
            if !is_paper {
                if let Some(matches) = by_order.get(&order.order_id.to_lowercase()) {
                    let total_size: f64 = matches.iter().map(|t| t.size).sum();
                    if total_size > 0.0 {
                        let weighted: f64 = matches.iter().map(|t| t.size * t.price).sum();
                        let vwap = weighted / total_size;
                        order.fill_price = Some(vwap);
                        order.fill_size  = Some(total_size);
                        order.tx_hash    = matches.first().map(|t| t.transaction_hash.clone());
                        if order.status == "LIVE" {
                            order.status = "MATCHED".to_string();
                        }
                        orders_filled += 1;
                    }
                }
            }

            // ── 2. Polymarket-resolved outcome ──────────────────────────
            let resolution: Option<MarketResolution> = if let Some(sid) = series_id.as_deref() {
                let series = builtin_series().into_iter().find(|s| s.id == sid);
                if let Some(series) = series {
                    let slug_secs = format!("{}-{}", series.slug_prefix, order.window_ts);
                    let slug_ms   = format!("{}-{}", series.slug_prefix, order.window_ts * 1000);
                    let mut found: Option<MarketResolution> = None;
                    for slug in [&slug_secs, &slug_ms] {
                        if let Some(cached) = res_cache.get(slug) {
                            found = cached.clone();
                            if found.as_ref().map(|r| r.closed).unwrap_or(false) {
                                break;
                            }
                            continue;
                        }
                        let r = get_market_resolution(slug).await.ok();
                        res_cache.insert(slug.clone(), r.clone());
                        if let Some(ref rr) = r { if rr.closed { found = Some(rr.clone()); break; } }
                        // Soft rate-limit: Gamma is cheap but free public tier.
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    found
                } else { None }
            } else { None };

            if let Some(ref res) = resolution {
                if let Some(yw) = res.yes_won {
                    order.resolution_yes_won = Some(yw);
                    order.resolution_source  = Some("polymarket".to_string());
                    orders_resolved += 1;
                }
            }

            // ── 3. Recompute P&L ────────────────────────────────────────
            // Paper orders: simulated bets — entry_price IS the fill, payout is
            // computed against it. On-chain orders: only MATCHED ones produce
            // a payout; truly unfilled rejected orders zero out.
            let resolution_known = order.resolution_yes_won.is_some();
            if is_paper {
                // For paper trades the synthetic entry_price stored at decision
                // time is the only price they ever had. Treat fill_price as
                // entry_price when missing so downstream code can still reason
                // about it.
                let entry = order.fill_price.or(order.entry_price);
                if let (Some(fp), Some(yes_won)) = (entry, order.resolution_yes_won) {
                    let won = (order.side.starts_with("yes") && yes_won)
                           || (order.side.starts_with("no")  && !yes_won);
                    let ep = fp.clamp(0.01, 0.99);
                    let new_pnl = if won {
                        order.amount_usdc * (1.0 / ep - 1.0)
                    } else {
                        -order.amount_usdc
                    };
                    order.pnl = Some(new_pnl);
                    order.result = Some(if won { "WIN".to_string() } else { "LOSS".to_string() });
                    orders_recomputed += 1;
                } else if !resolution_known {
                    // Market hasn't resolved yet — leave the runner's prior
                    // estimate intact rather than wiping it.
                }
            } else if order.status == "MATCHED" {
                if let (Some(fp), Some(yes_won)) = (order.fill_price, order.resolution_yes_won) {
                    let won = (order.side.starts_with("yes") && yes_won)
                           || (order.side.starts_with("no")  && !yes_won);
                    let ep = fp.clamp(0.01, 0.99);
                    let new_pnl = if won {
                        order.amount_usdc * (1.0 / ep - 1.0)
                    } else {
                        -order.amount_usdc
                    };
                    order.pnl = Some(new_pnl);
                    order.result = Some(if won { "WIN".to_string() } else { "LOSS".to_string() });
                    orders_recomputed += 1;
                }
            } else {
                // On-chain LIVE / cancelled: never received tokens.
                if order.fill_price.is_none() {
                    order.pnl = Some(0.0);
                    order.result = Some("UNFILLED".to_string());
                    onchain_unfilled += 1;
                }
            }

            order.backfilled = true;
        }

        let pnl_after: f64 = result.live_orders.iter().filter_map(|o| o.pnl).sum();
        runner_pnl_after += pnl_after;

        // Recompute live_total_trades / live_wins from the updated orders.
        let total = result.live_orders.iter()
            .filter(|o| matches!(o.result.as_deref(), Some("WIN") | Some("LOSS")))
            .count() as u32;
        let wins = result.live_orders.iter()
            .filter(|o| o.result.as_deref() == Some("WIN"))
            .count() as u32;
        result.live_total_trades = total;
        result.live_wins = wins;
        let wr = if total > 0 { (wins as f64 / total as f64) * 100.0 } else { 0.0 };
        result.win_rate_pct = wr;
        result.balance = runner.config.initial_balance + pnl_after;
        result.total_return_pct = if runner.config.initial_balance > 0.0 {
            (pnl_after / runner.config.initial_balance) * 100.0
        } else {
            0.0
        };

        println!(
            "  P&L  {:>+10.2}  →  {:>+10.2}   (wins {}/{},  WR {:.1}%)",
            pnl_before, pnl_after, wins, total, wr
        );
    }

    println!("\n────────────────────────────────────────────────────────");
    println!("Runners touched         : {runners_touched}");
    println!("Orders examined         : {orders_examined}");
    println!("  paper                 : {paper_orders}");
    println!("  on-chain              : {onchain_orders}");
    println!("Orders fill-reconciled  : {orders_filled}");
    println!("Orders outcome-resolved : {orders_resolved}");
    println!("Orders P&L recomputed   : {orders_recomputed}");
    println!("On-chain unfilled→0     : {onchain_unfilled}");
    println!("Σ P&L before            : {:>+10.2}", runner_pnl_before);
    println!("Σ P&L after             : {:>+10.2}", runner_pnl_after);
    println!("Δ                       : {:>+10.2}", runner_pnl_after - runner_pnl_before);
    println!("────────────────────────────────────────────────────────");

    if dry_run {
        println!("\n[dry-run] No changes written. Re-run without --dry-run to persist.");
        return Ok(());
    }

    // Snapshot the existing file before overwriting.
    let backup = path.with_extension(format!(
        "json.bak.{}",
        chrono::Utc::now().timestamp()
    ));
    std::fs::copy(&path, &backup)?;
    println!("Backup saved to {}", backup.display());

    let serialized = serde_json::to_string_pretty(&runners)?;
    std::fs::write(&path, serialized)?;
    println!("Wrote updated {}", path.display());

    Ok(())
}

fn poly_creds_from_config(p: &crate::config::schema::PolymarketConfig) -> Result<PolyCredentials> {
    let api_key = p.api_key.clone()
        .ok_or_else(|| anyhow!("polymarket.api_key not set in config — cannot reach CLOB"))?;
    let secret = p.secret.clone()
        .ok_or_else(|| anyhow!("polymarket.secret not set in config"))?;
    let passphrase = p.passphrase.clone()
        .ok_or_else(|| anyhow!("polymarket.passphrase not set in config"))?;
    let wallet_address = p.wallet_address.clone()
        .ok_or_else(|| anyhow!("polymarket.wallet_address not set in config"))?;
    Ok(PolyCredentials {
        api_key,
        secret,
        passphrase,
        wallet_address,
        private_key: p.private_key.clone(),
        is_builder: p.is_builder.unwrap_or(false),
        proxy_address: p.proxy_address.clone(),
        signature_type: p.signature_type.clone(),
    })
}
