//! Batch strategy validation — runs EVERY script in `<workspace>/scripts/`
//! through its matching backtest engine over the available historical data, then
//! the 3-leg `edge_validator` on each. The point is the house rule: passing a
//! backtest is NOT edge — only an EDGE verdict from the validator is.
//!
//! Engine routing by Rhai entry point:
//!   `fn on_candle(ctx)` → `archive_candles` (tick JSONL → 1m candles + real CLOB price)
//!   `fn on_tick(ctx)`   → `clob_1hz`        (1 Hz tick replay)
//!   `fn on_event(ctx)`  → `clob_events`     (ms event stream; needs data/events/<slug>)
//!
//! Slug routing: per-asset scripts (eth/sol/xrp/doge/hype/bnb in the filename)
//! map to that asset's 5m slug; everything else defaults to `btc_5m`. on_event
//! scripts use the `_ev` event-stream slugs.

use std::path::Path;
use crate::tools::backtest::{run_backtest_engine, list_tick_slugs, list_event_slugs};
use crate::tools::edge_validator::{self, ValidationResult};

#[derive(Debug, Clone)]
enum Engine {
    ArchiveCandles, // on_candle
    Clob1hz,        // on_tick
    ClobEvents,     // on_event
    Unknown,
}

fn classify(src: &str) -> Engine {
    if src.contains("fn on_event") { Engine::ClobEvents }
    else if src.contains("fn on_tick") { Engine::Clob1hz }
    else if src.contains("fn on_candle") { Engine::ArchiveCandles }
    else { Engine::Unknown }
}

/// Pick the data slug for a script by asset keyword in its filename.
fn slug_for(filename: &str, engine: &Engine, available: &[String]) -> Option<String> {
    let name = filename.to_lowercase();
    let asset = ["eth", "sol", "xrp", "doge", "hype", "bnb", "btc"]
        .iter()
        .find(|a| name.contains(*a))
        .copied()
        .unwrap_or("btc");
    // Candidate slugs in preference order.
    let candidates: Vec<String> = match engine {
        Engine::ClobEvents => vec![format!("{asset}_5m_ev"), "btc_5m_ev".to_string()],
        _ => vec![format!("{asset}_5m"), "btc_5m".to_string()],
    };
    candidates.into_iter().find(|c| available.iter().any(|a| a == c))
}

pub struct ValidateAllReport {
    pub rows: Vec<ScriptVerdict>,
}

pub struct ScriptVerdict {
    pub script: String,
    pub engine: String,
    pub slug: String,
    pub trades: u32,
    pub return_pct: f64,
    pub validation: Option<ValidationResult>,
    pub note: String,
}

/// Run the full batch. `only_live` restricts to the live-strategy set from CLAUDE.md.
pub async fn run_validate_all(
    workspace_dir: &Path,
    initial_balance: f64,
    include_events: bool,
) -> ValidateAllReport {
    crate::tools::backtest::ensure_default_scripts(workspace_dir);
    let scripts_dir = workspace_dir.join("scripts");

    // Available data slugs per engine family.
    let tick_slugs: Vec<String> = list_tick_slugs(workspace_dir).into_iter().map(|(s, _, _)| s).collect();
    let event_slugs: Vec<String> = list_event_slugs(workspace_dir).into_iter().map(|(s, _, _)| s).collect();

    // Date coverage per slug (use each slug's own min/max so we don't run empty ranges).
    let tick_dates: std::collections::HashMap<String, (String, String)> =
        list_tick_slugs(workspace_dir).into_iter()
            .filter_map(|(s, d, _)| Some((s, (d.first()?.clone(), d.last()?.clone()))))
            .collect();
    let event_dates: std::collections::HashMap<String, (String, String)> =
        list_event_slugs(workspace_dir).into_iter()
            .filter_map(|(s, d, _)| Some((s, (d.first()?.clone(), d.last()?.clone()))))
            .collect();

    let mut files: Vec<_> = std::fs::read_dir(&scripts_dir)
        .map(|rd| rd.flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("rhai"))
            .map(|e| e.path())
            .collect())
        .unwrap_or_default();
    files.sort();

    let total = files.len();
    let mut rows = Vec::new();
    for (idx, path) in files.into_iter().enumerate() {
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        let src = std::fs::read_to_string(&path).unwrap_or_default();
        let engine = classify(&src);
        eprint!("[{}/{}] {} … ", idx + 1, total, filename);
        use std::io::Write as _;
        let _ = std::io::stderr().flush();
        let (engine_label, market_type, slug_pool, dates_pool) = match &engine {
            Engine::ArchiveCandles => ("on_candle", "archive_candles", &tick_slugs, &tick_dates),
            Engine::Clob1hz => ("on_tick", "clob_1hz", &tick_slugs, &tick_dates),
            Engine::ClobEvents => ("on_event", "clob_events", &event_slugs, &event_dates),
            Engine::Unknown => {
                eprintln!("skip (no on_candle/on_tick/on_event)");
                rows.push(ScriptVerdict {
                    script: filename, engine: "unknown".into(), slug: "—".into(),
                    trades: 0, return_pct: 0.0, validation: None,
                    note: "no on_candle/on_tick/on_event — skipped".into(),
                });
                continue;
            }
        };

        if matches!(engine, Engine::ClobEvents) && !include_events {
            eprintln!("skip (on_event; pass --include-events)");
            rows.push(ScriptVerdict {
                script: filename, engine: engine_label.into(), slug: "—".into(),
                trades: 0, return_pct: 0.0, validation: None,
                note: "on_event skipped (use --include-events once data/events is ready)".into(),
            });
            continue;
        }

        let Some(slug) = slug_for(&filename, &engine, slug_pool) else {
            eprintln!("skip (no data slug)");
            rows.push(ScriptVerdict {
                script: filename, engine: engine_label.into(), slug: "—".into(),
                trades: 0, return_pct: 0.0, validation: None,
                note: "no matching data slug available — skipped".into(),
            });
            continue;
        };
        let (from, to) = dates_pool.get(&slug).cloned().unwrap_or_default();

        // Run the backtest with the live-parity fee model + official resolution.
        let m = run_backtest_engine(
            &path, market_type, &slug, "5m", &from, &to,
            initial_balance, 0.0, "price_up", None, None, None,
            "percent", 1.0, "historical", workspace_dir, &[],
            None, None, None, None, None, None,
            None, Some("crypto_taker"), None,
        ).await;

        // Extract (entry_price, won) via the shared extractor (handles every engine's
        // side-label convention). The 3-leg validator is for BINARY markets only.
        let (entries, wons) = edge_validator::extract_binary_trades(&m.all_trades);

        // Optional per-script trade export (NV_VALIDATE_DUMP=<dir>): writes
        // <dir>/<script>.csv with `entry_price,won` for offline analysis (price
        // structure, custom edge tests). Off by default.
        if let Ok(dump) = std::env::var("NV_VALIDATE_DUMP") {
            if !entries.is_empty() {
                let _ = std::fs::create_dir_all(&dump);
                let stem = filename.trim_end_matches(".rhai");
                let path = std::path::Path::new(&dump).join(format!("{stem}.csv"));
                let mut out = String::from("entry_price,won\n");
                for (e, w) in entries.iter().zip(&wons) {
                    out.push_str(&format!("{:.4},{}\n", e, if *w { 1 } else { 0 }));
                }
                let _ = std::fs::write(path, out);
            }
        }

        if entries.is_empty() && m.total_trades > 0 {
            // Traded, but not binary bets → the binary validator doesn't apply.
            eprintln!("N/A non-binary ({} trades)", m.total_trades);
            rows.push(ScriptVerdict {
                script: filename, engine: engine_label.into(), slug,
                trades: m.total_trades, return_pct: m.total_return_pct,
                validation: None,
                note: "non-binary trades — 3-leg validator N/A (crypto/spot script)".into(),
            });
            continue;
        }

        let validation = edge_validator::validate(&entries, &wons, 5000);
        eprintln!("{} (n={}, {} trades)", validation.verdict, validation.n, m.total_trades);

        rows.push(ScriptVerdict {
            script: filename,
            engine: engine_label.into(),
            slug,
            trades: m.total_trades,
            return_pct: m.total_return_pct,
            validation: Some(validation),
            note: String::new(),
        });
    }

    // Order: EDGE first, then NO_EDGE, then INSUFFICIENT, then skipped.
    rows.sort_by_key(|r| match r.validation.as_ref().map(|v| v.verdict.as_str()) {
        Some("EDGE") => 0,
        Some("NO_EDGE") => 1,
        Some("INSUFFICIENT") => 2,
        _ => 3,
    });

    ValidateAllReport { rows }
}

/// Pretty-print the report as a table to stdout.
pub fn print_report(report: &ValidateAllReport) {
    println!("\n{:<48} {:<9} {:<11} {:>6} {:>9}  {:>5} {}",
        "script", "engine", "slug", "trades", "return%", "n", "verdict");
    println!("{}", "─".repeat(120));
    let (mut edge, mut no_edge, mut insuf, mut skip) = (0, 0, 0, 0);
    for r in &report.rows {
        match &r.validation {
            Some(v) => {
                let verdict = match v.verdict.as_str() {
                    "EDGE" => { edge += 1; "✅ EDGE" }
                    "NO_EDGE" => { no_edge += 1; "❌ NO_EDGE" }
                    _ => { insuf += 1; "· INSUFFICIENT" }
                };
                let legs = format!("L1{} L2{} L3{}",
                    if v.leg1_pass {"✓"} else {"✗"},
                    if v.leg2_pass {"✓"} else {"✗"},
                    if v.leg3_pass {"✓"} else {"✗"});
                println!("{:<48} {:<9} {:<11} {:>6} {:>+8.1}% {:>5}  {} [{}]",
                    trunc(&r.script, 48), r.engine, r.slug, r.trades, r.return_pct, v.n, verdict, legs);
            }
            None => {
                skip += 1;
                println!("{:<48} {:<9} {:<11} {:>6} {:>9}  {:>5}  ⊘ {}",
                    trunc(&r.script, 48), r.engine, r.slug, "—", "—", "—", r.note);
            }
        }
    }
    println!("{}", "─".repeat(120));
    println!("EDGE: {edge}   NO_EDGE: {no_edge}   INSUFFICIENT: {insuf}   skipped: {skip}");
    println!("\nReminder: EDGE here = survived 3 independent statistical tests on official-resolution");
    println!("trades. It is a license for a SMALL real pilot, not a guarantee. NO_EDGE = do not commit capital.");
}

fn trunc(s: &str, n: usize) -> String {
    if s.len() <= n { s.to_string() } else { format!("{}…", &s[..n - 1]) }
}
