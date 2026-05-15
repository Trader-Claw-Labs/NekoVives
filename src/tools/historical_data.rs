//! Historical OHLCV + funding rate storage in SQLite.
//!
//! Provides persistent caching for backtest data and a unified event timeline
//! generator for the event-driven backtest engine.

use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A single backtest event in chronological order.
#[derive(Debug, Clone)]
pub enum BacktestEvent {
    /// OHLCV candle (any venue/data source).
    Candle {
        ts_ms: i64,
        symbol: String,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
    },
    /// Funding rate snapshot for a venue+symbol.
    Funding {
        ts_ms: i64,
        venue: String,
        symbol: String,
        rate: f64,        // raw funding rate (e.g. 0.0001 = 0.01%)
        rate_apr: f64,    // annualised APR equivalent
    },
}

/// Persistent SQLite store for historical market data.
/// The connection is wrapped in a `Mutex` so the store is `Send + Sync`
/// and can be held across `.await` points in async code.
pub struct HistoricalDataStore {
    conn: Mutex<Connection>,
    db_path: PathBuf,
}

impl HistoricalDataStore {
    /// Open (or create) the SQLite store in `<workspace>/data/historical.db`.
    pub fn new(workspace_dir: &Path) -> anyhow::Result<Self> {
        let db_dir = workspace_dir.join("data");
        std::fs::create_dir_all(&db_dir)?;
        let db_path = db_dir.join("historical.db");
        let conn = Connection::open(&db_path)?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous  = NORMAL;",
        )?;

        Self::init_schema(&conn)?;

        Ok(Self { conn: Mutex::new(conn), db_path })
    }

    fn init_schema(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS ohlcv_candles (
                symbol       TEXT NOT NULL,
                interval     TEXT NOT NULL,
                open_time_ms INTEGER NOT NULL,
                open         REAL NOT NULL,
                high         REAL NOT NULL,
                low          REAL NOT NULL,
                close        REAL NOT NULL,
                volume       REAL NOT NULL,
                PRIMARY KEY (symbol, interval, open_time_ms)
            ) WITHOUT ROWID;
            CREATE INDEX IF NOT EXISTS idx_candles_time
                ON ohlcv_candles(symbol, interval, open_time_ms);

            CREATE TABLE IF NOT EXISTS funding_rates (
                venue        TEXT NOT NULL,
                symbol       TEXT NOT NULL,
                funding_time INTEGER NOT NULL,
                rate         REAL NOT NULL,
                rate_apr     REAL NOT NULL,
                PRIMARY KEY (venue, symbol, funding_time)
            ) WITHOUT ROWID;
            CREATE INDEX IF NOT EXISTS idx_funding_time
                ON funding_rates(venue, symbol, funding_time);",
        )?;
        Ok(())
    }

    // ── Candle ops ─────────────────────────────────────────────────────

    pub fn insert_candles(
        &self,
        symbol: &str,
        interval: &str,
        candles: &[crate::tools::backtest::Candle],
    ) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO ohlcv_candles
             (symbol, interval, open_time_ms, open, high, low, close, volume)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        let mut count = 0;
        for c in candles {
            stmt.execute(params![
                symbol, interval, c.open_time_ms, c.open, c.high, c.low, c.close, c.volume
            ])?;
            count += 1;
        }
        Ok(count)
    }

    pub fn get_candles(
        &self,
        symbol: &str,
        interval: &str,
        from_ms: i64,
        to_ms: i64,
    ) -> anyhow::Result<Vec<crate::tools::backtest::Candle>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT open_time_ms, open, high, low, close, volume
             FROM ohlcv_candles
             WHERE symbol = ?1 AND interval = ?2
               AND open_time_ms >= ?3 AND open_time_ms <= ?4
             ORDER BY open_time_ms ASC",
        )?;
        let rows = stmt.query_map(params![symbol, interval, from_ms, to_ms], |row| {
            Ok(crate::tools::backtest::Candle {
                open_time_ms: row.get(0)?,
                open: row.get(1)?,
                high: row.get(2)?,
                low: row.get(3)?,
                close: row.get(4)?,
                volume: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("candle query error: {e}"))
    }

    /// Check how many candles exist in the date range.
    pub fn candle_coverage(
        &self,
        symbol: &str,
        interval: &str,
        from_ms: i64,
        to_ms: i64,
    ) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM ohlcv_candles
             WHERE symbol = ?1 AND interval = ?2
               AND open_time_ms >= ?3 AND open_time_ms <= ?4",
            params![symbol, interval, from_ms, to_ms],
            |row| row.get(0),
        )?;
        #[allow(clippy::cast_sign_loss)]
        Ok(count as usize)
    }

    // ── Funding rate ops ───────────────────────────────────────────────

    pub fn insert_funding_rates(
        &self,
        venue: &str,
        symbol: &str,
        rates: &[(i64, f64, f64)],
    ) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO funding_rates
             (venue, symbol, funding_time, rate, rate_apr)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        let mut count = 0;
        for (ft, rate, apr) in rates {
            stmt.execute(params![venue, symbol, ft, rate, apr])?;
            count += 1;
        }
        Ok(count)
    }

    pub fn get_funding_rates(
        &self,
        venue: &str,
        symbol: &str,
        from_ms: i64,
        to_ms: i64,
    ) -> anyhow::Result<Vec<(i64, f64, f64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT funding_time, rate, rate_apr
             FROM funding_rates
             WHERE venue = ?1 AND symbol = ?2
               AND funding_time >= ?3 AND funding_time <= ?4
             ORDER BY funding_time ASC",
        )?;
        let rows = stmt.query_map(params![venue, symbol, from_ms, to_ms], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?, row.get::<_, f64>(2)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("funding query error: {e}"))
    }

    // ── Event timeline ─────────────────────────────────────────────────

    /// Build a merged chronological event stream from stored data.
    ///
    /// `candle_key` = (symbol, interval) for the price data.
    /// `funding_keys` = list of (venue, symbol) pairs to include.
    pub fn build_timeline(
        &self,
        candle_key: (&str, &str),
        funding_keys: &[(&str, &str)],
        from_ms: i64,
        to_ms: i64,
    ) -> anyhow::Result<Vec<BacktestEvent>> {
        let mut events: Vec<BacktestEvent> = Vec::new();

        // Candles
        let candles = self.get_candles(candle_key.0, candle_key.1, from_ms, to_ms)?;
        for c in candles {
            events.push(BacktestEvent::Candle {
                ts_ms: c.open_time_ms,
                symbol: candle_key.0.to_string(),
                open: c.open,
                high: c.high,
                low: c.low,
                close: c.close,
                volume: c.volume,
            });
        }

        // Funding rates
        for (venue, symbol) in funding_keys {
            let rates = self.get_funding_rates(venue, symbol, from_ms, to_ms)?;
            for (ft, rate, apr) in rates {
                events.push(BacktestEvent::Funding {
                    ts_ms: ft,
                    venue: venue.to_string(),
                    symbol: symbol.to_string(),
                    rate,
                    rate_apr: apr,
                });
            }
        }

        events.sort_by_key(|e| match e {
            BacktestEvent::Candle { ts_ms, .. } => *ts_ms,
            BacktestEvent::Funding { ts_ms, .. } => *ts_ms,
        });
        Ok(events)
    }

    /// Build a timeline with **synthetic** funding events every `interval_ms`.
    /// Used when real funding data hasn't been downloaded yet.
    /// The synthetic rate is computed from recent price momentum as a crude
    /// proxy: positive momentum → positive funding (longs pay), negative → negative.
    pub fn build_timeline_with_synthetic_funding(
        &self,
        candle_key: (&str, &str),
        venues: &[&str],
        from_ms: i64,
        to_ms: i64,
        funding_interval_ms: i64,
    ) -> anyhow::Result<Vec<BacktestEvent>> {
        let mut events = self.build_timeline(candle_key, &[], from_ms, to_ms)?;

        // Extract close prices for synthetic funding computation
        let mut closes: Vec<(i64, f64)> = Vec::new();
        for e in &events {
            if let BacktestEvent::Candle { ts_ms, close, .. } = e {
                closes.push((*ts_ms, *close));
            }
        }

        if closes.len() < 2 {
            return Ok(events);
        }

        // Generate synthetic funding events
        let mut t = from_ms;
        while t <= to_ms {
            // Find closest candle for momentum calc
            let idx = closes.binary_search_by_key(&t, |(ts, _)| *ts);
            let idx = match idx {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            };
            let (ts0, close0) = closes[idx];
            let lookback = closes.get(idx.saturating_sub(12)).copied().unwrap_or((ts0, close0));
            let momentum = if lookback.1 > 0.0 {
                (close0 - lookback.1) / lookback.1
            } else {
                0.0
            };
            // Synthetic rate: ±0.01% per 1% momentum, clamped to ±0.1%
            let rate = (momentum * 0.0001).clamp(-0.001, 0.001);
            // APR: hourly compound approximation
            let rate_apr = rate * 24.0 * 365.0;

            for venue in venues {
                events.push(BacktestEvent::Funding {
                    ts_ms: t,
                    venue: venue.to_string(),
                    symbol: candle_key.0.to_string(),
                    rate,
                    rate_apr,
                });
            }
            t += funding_interval_ms;
        }

        events.sort_by_key(|e| match e {
            BacktestEvent::Candle { ts_ms, .. } => *ts_ms,
            BacktestEvent::Funding { ts_ms, .. } => *ts_ms,
        });
        Ok(events)
    }
}

// ── Binance downloaders ──────────────────────────────────────────────

/// Download 1m candles from Binance REST and store them.
pub async fn download_binance_klines(
    store: &HistoricalDataStore,
    symbol: &str,
    interval: &str,
    from_date: &str,
    to_date: &str,
) -> anyhow::Result<usize> {
    use chrono::NaiveDate;

    let from_ms = NaiveDate::parse_from_str(from_date, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("Invalid from_date: {e}"))?
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    let to_ms = NaiveDate::parse_from_str(to_date, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("Invalid to_date: {e}"))?
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp_millis();

    let client = reqwest::Client::new();
    let mut all_candles: Vec<crate::tools::backtest::Candle> = Vec::new();
    let mut current_start = from_ms;
    let max_requests = 500;
    let mut request_count = 0;

    tracing::info!("[HISTORICAL] Downloading {symbol} {interval} candles from Binance...");

    while current_start < to_ms && request_count < max_requests {
        let url = format!(
            "https://api.binance.com/api/v3/klines?symbol={}&interval={}&startTime={}&endTime={}&limit=1000",
            symbol.to_uppercase(),
            interval,
            current_start,
            to_ms
        );

        let body = client
            .get(&url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Binance request failed: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("Binance error: {e}"))?
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("Binance response error: {e}"))?;

        let batch = parse_binance_klines(&body)?;
        if batch.is_empty() {
            break;
        }
        if let Some(last) = batch.last() {
            current_start = last.open_time_ms + 1;
        }
        all_candles.extend(batch);
        request_count += 1;
        if all_candles.len() < 1000 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    tracing::info!(
        "[HISTORICAL] Downloaded {} candles in {} requests",
        all_candles.len(),
        request_count
    );

    let count = store.insert_candles(symbol, interval, &all_candles)?;
    Ok(count)
}

/// Download Binance USD-M futures funding rate history and store it.
///
/// Endpoint: `GET /fapi/v1/fundingRate?symbol={}&startTime={}&endTime={}&limit=1000`
pub async fn download_binance_funding_rates(
    store: &HistoricalDataStore,
    symbol: &str,
    from_date: &str,
    to_date: &str,
) -> anyhow::Result<usize> {
    use chrono::NaiveDate;

    let from_ms = NaiveDate::parse_from_str(from_date, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("Invalid from_date: {e}"))?
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    let to_ms = NaiveDate::parse_from_str(to_date, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("Invalid to_date: {e}"))?
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp_millis();

    let client = reqwest::Client::new();
    let mut all_rates: Vec<(i64, f64, f64)> = Vec::new();
    let mut current_start = from_ms;

    tracing::info!(
        "[HISTORICAL] Downloading Binance funding rates for {}...",
        symbol.to_uppercase()
    );

    loop {
        let url = format!(
            "https://fapi.binance.com/fapi/v1/fundingRate?symbol={}&startTime={}&endTime={}&limit=1000",
            symbol.to_uppercase(),
            current_start,
            to_ms
        );

        let resp = client
            .get(&url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Binance funding request failed: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("Binance funding error: {e}"))?
            .json::<Vec<serde_json::Value>>()
            .await
            .map_err(|e| anyhow::anyhow!("Binance funding parse error: {e}"))?;

        if resp.is_empty() {
            break;
        }

        let mut batch_max_time = current_start;
        for item in &resp {
            let funding_time = item["fundingTime"].as_i64().unwrap_or(0);
            let rate_str = item["fundingRate"].as_str().unwrap_or("0");
            let rate: f64 = rate_str.parse().unwrap_or(0.0);
            // Binance funding is every 8h; APR = rate * 3 * 365
            let rate_apr = rate * 3.0 * 365.0;
            if funding_time > 0 {
                all_rates.push((funding_time, rate, rate_apr));
                batch_max_time = batch_max_time.max(funding_time);
            }
        }

        if resp.len() < 1000 {
            break;
        }
        current_start = batch_max_time + 1;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    tracing::info!(
        "[HISTORICAL] Downloaded {} funding rate records",
        all_rates.len()
    );

    let count = store.insert_funding_rates("binance", symbol, &all_rates)?;
    Ok(count)
}

fn parse_binance_klines(body: &str) -> anyhow::Result<Vec<crate::tools::backtest::Candle>> {
    let raw: Vec<Vec<serde_json::Value>> = serde_json::from_str(body)
        .map_err(|e| anyhow::anyhow!("Failed to parse Binance klines: {e}"))?;
    let candles = raw
        .into_iter()
        .filter_map(|row| {
            if row.len() < 6 {
                return None;
            }
            let open_time = row[0].as_i64()?;
            let open = row[1].as_str()?.parse::<f64>().ok()?;
            let high = row[2].as_str()?.parse::<f64>().ok()?;
            let low = row[3].as_str()?.parse::<f64>().ok()?;
            let close = row[4].as_str()?.parse::<f64>().ok()?;
            let vol = row[5].as_str()?.parse::<f64>().ok()?;
            Some(crate::tools::backtest::Candle {
                open_time_ms: open_time,
                open,
                high,
                low,
                close,
                volume: vol,
            })
        })
        .collect();
    Ok(candles)
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_store() -> (TempDir, HistoricalDataStore) {
        let tmp = TempDir::new().unwrap();
        let store = HistoricalDataStore::new(tmp.path()).unwrap();
        (tmp, store)
    }

    #[test]
    fn store_creates_schema() {
        let (_tmp, store) = temp_store();
        let conn = store.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='ohlcv_candles'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn insert_and_get_candles() {
        let (_tmp, store) = temp_store();
        let candles = vec![
            crate::tools::backtest::Candle {
                open_time_ms: 1_000_000,
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.5,
                volume: 10.0,
            },
            crate::tools::backtest::Candle {
                open_time_ms: 1_060_000,
                open: 100.5,
                high: 102.0,
                low: 100.0,
                close: 101.5,
                volume: 15.0,
            },
        ];
        store.insert_candles("BTCUSDT", "1m", &candles).unwrap();
        let fetched = store.get_candles("BTCUSDT", "1m", 0, 2_000_000).unwrap();
        assert_eq!(fetched.len(), 2);
        assert!((fetched[0].close - 100.5).abs() < 1e-9);
    }

    #[test]
    fn insert_and_get_funding() {
        let (_tmp, store) = temp_store();
        let rates = vec![(1_000_000, 0.0001, 0.1095), (1_500_000, 0.0002, 0.219)];
        store.insert_funding_rates("binance", "BTCUSDT", &rates).unwrap();
        let fetched = store.get_funding_rates("binance", "BTCUSDT", 0, 2_000_000).unwrap();
        assert_eq!(fetched.len(), 2);
        assert!((fetched[0].1 - 0.0001).abs() < 1e-9);
    }

    #[test]
    fn build_timeline_merges_and_sorts() {
        let (_tmp, store) = temp_store();
        let candles = vec![
            crate::tools::backtest::Candle {
                open_time_ms: 1_000_000,
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.5,
                volume: 10.0,
            },
            crate::tools::backtest::Candle {
                open_time_ms: 1_060_000,
                open: 100.5,
                high: 102.0,
                low: 100.0,
                close: 101.5,
                volume: 15.0,
            },
        ];
        store.insert_candles("BTCUSDT", "1m", &candles).unwrap();
        let rates = vec![(1_030_000, 0.0001, 0.1095)];
        store.insert_funding_rates("binance", "BTCUSDT", &rates).unwrap();

        let timeline = store
            .build_timeline(("BTCUSDT", "1m"), &[("binance", "BTCUSDT")], 0, 2_000_000)
            .unwrap();
        assert_eq!(timeline.len(), 3);
        // Should be sorted by timestamp
        assert_eq!(
            match &timeline[0] {
                BacktestEvent::Candle { ts_ms, .. } => *ts_ms,
                BacktestEvent::Funding { ts_ms, .. } => *ts_ms,
            },
            1_000_000
        );
        assert_eq!(
            match &timeline[1] {
                BacktestEvent::Candle { ts_ms, .. } => *ts_ms,
                BacktestEvent::Funding { ts_ms, .. } => *ts_ms,
            },
            1_030_000
        );
        assert_eq!(
            match &timeline[2] {
                BacktestEvent::Candle { ts_ms, .. } => *ts_ms,
                BacktestEvent::Funding { ts_ms, .. } => *ts_ms,
            },
            1_060_000
        );
    }
}
