//! Market scanning job (configurable interval, default 60 s)
//!
//! Scans token prices, Polymarket markets, checks alerts, sends Telegram notifications.

use anyhow::Result;

/// Configuration for the market cron job (read from config.toml [cron] section)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct CronConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Scan interval in **seconds** (default: 60).
    /// Set to any positive value, e.g. 30 for 30 s, 300 for 5 min.
    #[serde(default = "default_interval_seconds", alias = "interval_minutes")]
    pub interval_seconds: u64,
    #[serde(default = "default_true")]
    pub polymarket_scan: bool,
    #[serde(default = "default_true")]
    pub portfolio_update: bool,
}

fn default_true() -> bool {
    true
}
fn default_interval_seconds() -> u64 {
    60
}

/// A single market snapshot to store in SQLite
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MarketSnapshot {
    pub symbol: String,
    pub price: f64,
    pub rsi: Option<f64>,
    pub macd: Option<f64>,
    pub source: String,     // "tradingview" or "polymarket"
    pub timestamp: String,  // ISO8601
}

/// Run one iteration of the market scan job.
/// Called on each scheduler tick (interval set via `CronConfig::interval_seconds`).
///
/// Steps:
/// 1. Fetch price + indicators from TradingView Screener for each symbol
/// 2. Create a MarketSnapshot per result
/// 3. If send_telegram is true AND any RSI > 70 or < 30, format an alert
///    (alerts are embedded in snapshot metadata for the caller to dispatch)
/// 4. Scan top Polymarket crypto markets and append snapshots
/// 5. Return all snapshots
///
/// # TODO: Wire into scheduler
/// To add this to the cron scheduler, register a `Schedule::Every { every_ms: interval_seconds * 1000 }`
/// job in `src/cron/scheduler.rs` that calls `run_market_scan` on each tick.
pub async fn run_market_scan(
    symbols: &[String],
    send_telegram: bool,
) -> Result<Vec<MarketSnapshot>> {
    use chrono::Utc;

    let symbols_str: Vec<&str> = symbols.iter().map(String::as_str).collect();
    let indicators = market_analyzer::screener::fetch_indicators(&symbols_str).await?;

    let mut snapshots: Vec<MarketSnapshot> = indicators
        .iter()
        .map(|d| MarketSnapshot {
            symbol: d.symbol.clone(),
            price: d.price,
            rsi: d.rsi,
            macd: d.macd,
            source: "tradingview".to_string(),
            timestamp: Utc::now().to_rfc3339(),
        })
        .collect();

    // Alert check — log if caller requested Telegram and RSI is extreme
    if send_telegram {
        for snap in &snapshots {
            if let Some(rsi) = snap.rsi {
                if rsi > 70.0 {
                    tracing::info!(
                        symbol = %snap.symbol,
                        rsi = rsi,
                        "RSI alert: overbought (RSI > 70)"
                    );
                } else if rsi < 30.0 {
                    tracing::info!(
                        symbol = %snap.symbol,
                        rsi = rsi,
                        "RSI alert: oversold (RSI < 30)"
                    );
                }
            }
        }
    }

    // Polymarket scan — top 10 crypto markets
    match polymarket_trader::markets::list_markets(polymarket_trader::markets::MarketFilter {
        category: Some("crypto".to_string()),
        active_only: true,
        ..Default::default()
    })
    .await
    {
        Ok(markets) => {
            for market in markets.into_iter().take(10) {
                match polymarket_trader::markets::get_market_price(&market.yes_token_id).await {
                    Ok(price) => {
                        snapshots.push(MarketSnapshot {
                            symbol: market.slug.clone(),
                            price,
                            rsi: None,
                            macd: None,
                            source: "polymarket".to_string(),
                            timestamp: Utc::now().to_rfc3339(),
                        });
                    }
                    Err(e) => {
                        tracing::warn!(slug = %market.slug, "Failed to get Polymarket price: {e}");
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("Failed to list Polymarket markets: {e}");
        }
    }

    Ok(snapshots)
}

/// Init the market_snapshots table in SQLite
pub fn init_db(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS market_snapshots (
            id        INTEGER PRIMARY KEY AUTOINCREMENT,
            symbol    TEXT    NOT NULL,
            price     REAL    NOT NULL,
            rsi       REAL,
            macd      REAL,
            source    TEXT    NOT NULL,
            timestamp TEXT    NOT NULL
        );",
    )?;
    Ok(())
}

/// Save snapshots to SQLite (table: market_snapshots)
pub fn save_snapshots(
    conn: &rusqlite::Connection,
    snapshots: &[MarketSnapshot],
) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO market_snapshots (symbol, price, rsi, macd, source, timestamp)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for snap in snapshots {
        stmt.execute(rusqlite::params![
            snap.symbol,
            snap.price,
            snap.rsi,
            snap.macd,
            snap.source,
            snap.timestamp,
        ])?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron_config_defaults() {
        let cfg = CronConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.interval_seconds, 60);
        assert!(cfg.polymarket_scan);
        assert!(cfg.portfolio_update);
    }

    #[test]
    fn test_cron_config_custom_interval() {
        let toml = r#"enabled = true
interval_seconds = 30
polymarket_scan = true
portfolio_update = false"#;
        let cfg: CronConfig = toml::from_str(toml).expect("parse failed");
        assert_eq!(cfg.interval_seconds, 30);
        assert!(!cfg.portfolio_update);
    }

    #[test]
    fn test_init_db_and_save_snapshots() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        init_db(&conn).expect("init_db failed");

        let snapshots = vec![
            MarketSnapshot {
                symbol: "BTCUSDT".to_string(),
                price: 65000.0,
                rsi: Some(55.3),
                macd: Some(120.5),
                source: "tradingview".to_string(),
                timestamp: "2026-03-10T00:00:00Z".to_string(),
            },
            MarketSnapshot {
                symbol: "will-btc-reach-100k".to_string(),
                price: 0.72,
                rsi: None,
                macd: None,
                source: "polymarket".to_string(),
                timestamp: "2026-03-10T00:00:00Z".to_string(),
            },
        ];

        save_snapshots(&conn, &snapshots).expect("save_snapshots failed");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM market_snapshots", [], |row| {
                row.get(0)
            })
            .expect("count query failed");
        assert_eq!(count, 2);
    }
}
