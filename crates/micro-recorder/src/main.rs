//! micro-recorder — high-frequency BTC market-microstructure recorder.
//!
//! Captures, in real time and to disk, everything needed to study how BTC price
//! moves at the millisecond/minute scale around Polymarket's BTC Up/Down 5-min
//! markets:
//!   • Binance SPOT + PERP: aggTrade, depth20@100ms, forceOrder (liquidations),
//!     markPrice@1s (mark/index/funding)            → OBI / OFI / CVD / VAMP / liq
//!   • Chainlink RTDS: the exact price Polymarket resolves against (basis)
//!   • Polymarket CLOB market-channel: live two-sided book + trades per window
//!
//! Two output planes per slug under `<out>/<slug>/<YYYY-MM-DD>/`:
//!   events.jsonl.gz   — every raw event, full fidelity (recompute any metric)
//!   metrics.jsonl.gz  — OBI/OFI/CVD/liq/VAMP/basis snapshots on a fixed cadence
//!
//! All feeds are independent tokio tasks funneling into a single recorder task
//! over an mpsc channel; that task owns the writers and the rolling state, so
//! there are no shared locks on the hot path.
//!
//! Usage:
//!   micro-recorder --slug btc_5m --out ~/.traderclaw/workspace/data/micro \
//!     --metrics-hz 5 [--no-spot] [--no-perp] [--no-poly] [--no-chainlink]

mod binance;
mod bybit;
mod chainlink;
mod metrics;
mod polymarket;
mod types;
mod writer;

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use tokio::sync::mpsc;

use metrics::RecorderState;
use types::RawEvent;
use writer::RotatingGzWriter;

#[derive(Parser)]
#[command(name = "micro-recorder", about = "HFT BTC microstructure recorder (OBI/OFI/CVD/liq + Chainlink + Polymarket book)")]
struct Args {
    /// Slug used in the output path (e.g. "btc_5m").
    #[arg(long, default_value = "btc_5m")]
    slug: String,
    /// Output base dir. Default: $NV_WORKSPACE/data/micro or ~/.traderclaw/workspace/data/micro
    #[arg(long)]
    out: Option<PathBuf>,
    /// Metrics snapshot frequency in Hz (snapshots per second). 0 = event-driven off.
    #[arg(long, default_value_t = 5)]
    metrics_hz: u32,
    #[arg(long)]
    no_spot: bool,
    #[arg(long)]
    no_perp: bool,
    /// Disable the Bybit perp venue (trades + liquidations + funding).
    #[arg(long)]
    no_bybit: bool,
    #[arg(long)]
    no_poly: bool,
    #[arg(long)]
    no_chainlink: bool,
}

fn default_out() -> PathBuf {
    if let Ok(ws) = std::env::var("NV_WORKSPACE") {
        return PathBuf::from(ws).join("data").join("micro");
    }
    directories_home()
        .map(|h| h.join(".traderclaw").join("workspace").join("data").join("micro"))
        .unwrap_or_else(|| PathBuf::from("./micro-data"))
}

fn directories_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let out = args.out.clone().unwrap_or_else(default_out);
    std::fs::create_dir_all(&out)?;
    tracing::info!(
        "micro-recorder starting — slug={} out={} metrics_hz={}",
        args.slug,
        out.display(),
        args.metrics_hz
    );

    // Single bus: all feeds → recorder task.
    let (tx, mut rx) = mpsc::channel::<RawEvent>(8192);

    if !args.no_spot {
        binance::spawn_spot(tx.clone());
    }
    if !args.no_perp {
        binance::spawn_perp(tx.clone());
    }
    if !args.no_bybit {
        bybit::spawn(tx.clone());
    }
    if !args.no_chainlink {
        chainlink::spawn(tx.clone());
    }
    if !args.no_poly {
        polymarket::spawn(tx.clone());
    }
    drop(tx); // recorder exits when all feeds stop (never, in practice)

    let mut events = RotatingGzWriter::new(out.clone(), &args.slug, "events");
    let mut metrics_w = RotatingGzWriter::new(out.clone(), &args.slug, "metrics");
    let mut state = RecorderState::default();

    // Metrics cadence + periodic flush.
    let tick_ms = if args.metrics_hz == 0 { 1000 } else { 1000 / args.metrics_hz.max(1) as u64 };
    let mut metric_tick = tokio::time::interval(Duration::from_millis(tick_ms.max(50)));
    let mut flush_tick = tokio::time::interval(Duration::from_secs(2));
    let mut sigterm = unix_sigterm();

    let mut raw_count: u64 = 0;
    let mut metric_count: u64 = 0;

    loop {
        tokio::select! {
            biased;

            _ = tokio::signal::ctrl_c() => { tracing::info!("SIGINT — flushing & exiting"); break; }
            _ = recv_sigterm(&mut sigterm) => { tracing::info!("SIGTERM — flushing & exiting"); break; }

            maybe = rx.recv() => {
                let Some(ev) = maybe else { break };
                if let Some(line) = fold_and_serialize(&mut state, &ev) {
                    if events.write_line(&line).is_ok() { raw_count += 1; }
                }
            }

            _ = metric_tick.tick() => {
                if args.metrics_hz > 0 {
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    let snap = state.snapshot(now_ms);
                    if let Ok(line) = serde_json::to_string(&snap) {
                        if metrics_w.write_line(&line).is_ok() { metric_count += 1; }
                    }
                }
            }

            _ = flush_tick.tick() => {
                events.flush();
                metrics_w.flush();
                tracing::info!("[REC] raw={raw_count} metrics={metric_count}");
            }
        }
    }

    events.finish();
    metrics_w.finish();
    tracing::info!("micro-recorder stopped — raw={raw_count} metrics={metric_count}");
    Ok(())
}

/// Fold an event into rolling state AND produce its raw JSONL line.
/// Returns None for control events we don't persist as raw.
fn fold_and_serialize(state: &mut RecorderState, ev: &RawEvent) -> Option<String> {
    use serde_json::json;
    match ev {
        RawEvent::Trade { src, t } => {
            state.on_trade(src, *t);
            Some(json!({
                "kind": "cex_trade", "src": src, "ts_ms": t.ts_ms,
                "price": t.price, "qty": t.qty, "buyer_is_maker": t.buyer_is_maker,
                "signed_qty": t.signed_qty()
            }).to_string())
        }
        RawEvent::Book { src, book } => {
            state.on_book(src, book.clone());
            // Persist top 10 levels per side — enough to recompute L1–L5 OBI/OFI.
            let bids: Vec<[f64; 2]> = book.bids.iter().take(10).map(|l| [l.price, l.qty]).collect();
            let asks: Vec<[f64; 2]> = book.asks.iter().take(10).map(|l| [l.price, l.qty]).collect();
            Some(json!({
                "kind": "cex_book", "src": src, "ts_ms": book.ts_ms,
                "bids": bids, "asks": asks
            }).to_string())
        }
        RawEvent::Liquidation { src, l } => {
            state.on_liq(*l);
            Some(json!({
                "kind": "liquidation", "src": src, "ts_ms": l.ts_ms,
                "price": l.price, "qty": l.qty,
                "side": if l.buyer_is_maker { "SELL" } else { "BUY" },
                "notional": l.price * l.qty
            }).to_string())
        }
        RawEvent::Mark { src, ts_ms, mark_price, index_price, funding_rate, next_funding_ms } => {
            state.on_mark(*mark_price, *index_price, *funding_rate);
            Some(json!({
                "kind": "mark", "src": src, "ts_ms": ts_ms,
                "mark_price": mark_price, "index_price": index_price,
                "funding_rate": funding_rate, "next_funding_ms": next_funding_ms
            }).to_string())
        }
        RawEvent::Oracle { ts_ms, price } => {
            state.on_oracle(*ts_ms, *price);
            Some(json!({ "kind": "oracle", "src": "chainlink", "ts_ms": ts_ms, "price": price }).to_string())
        }
        RawEvent::PmBook { ts_ms, asset_id, bids, asks, hash } => {
            let best_bid = bids.iter().map(|l| l.price).fold(0.0_f64, f64::max);
            let best_ask = asks.iter().map(|l| l.price).filter(|p| *p > 0.0).fold(f64::INFINITY, f64::min);
            let best_ask = if best_ask.is_finite() { best_ask } else { 0.0 };
            state.on_pm_book(asset_id, best_bid, best_ask);
            let b: Vec<[f64; 2]> = bids.iter().take(15).map(|l| [l.price, l.qty]).collect();
            let a: Vec<[f64; 2]> = asks.iter().take(15).map(|l| [l.price, l.qty]).collect();
            Some(json!({
                "kind": "pm_book", "src": "polymarket", "ts_ms": ts_ms,
                "asset_id": asset_id, "bids": b, "asks": a, "hash": hash
            }).to_string())
        }
        RawEvent::PmPriceChange { ts_ms, asset_id, price, size, side, best_bid, best_ask } => {
            state.on_pm_change(asset_id, *best_bid, *best_ask);
            Some(json!({
                "kind": "pm_price_change", "src": "polymarket", "ts_ms": ts_ms,
                "asset_id": asset_id, "price": price, "size": size, "side": side,
                "best_bid": best_bid, "best_ask": best_ask
            }).to_string())
        }
        RawEvent::PmTrade { ts_ms, asset_id, price, size, side } => {
            Some(json!({
                "kind": "pm_trade", "src": "polymarket", "ts_ms": ts_ms,
                "asset_id": asset_id, "price": price, "size": size, "side": side
            }).to_string())
        }
        RawEvent::Window(w) => {
            state.set_window(w.up_token.clone(), w.down_token.clone(), w.close_ms);
            Some(json!({
                "kind": "window", "src": "polymarket", "ts_ms": chrono::Utc::now().timestamp_millis(),
                "slug": w.slug, "condition_id": w.condition_id,
                "up_token": w.up_token, "down_token": w.down_token,
                "open_ms": w.open_ms, "close_ms": w.close_ms
            }).to_string())
        }
    }
}

// ── Signal helpers (Unix; on other OSes SIGTERM future is a no-op) ──────────────
#[cfg(unix)]
type SigTerm = tokio::signal::unix::Signal;
#[cfg(unix)]
fn unix_sigterm() -> Option<SigTerm> {
    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok()
}
#[cfg(unix)]
async fn recv_sigterm(s: &mut Option<SigTerm>) {
    match s {
        Some(sig) => {
            sig.recv().await;
        }
        None => std::future::pending::<()>().await,
    }
}
#[cfg(not(unix))]
fn unix_sigterm() -> Option<()> {
    None
}
#[cfg(not(unix))]
async fn recv_sigterm(_s: &mut Option<()>) {
    std::future::pending::<()>().await
}
