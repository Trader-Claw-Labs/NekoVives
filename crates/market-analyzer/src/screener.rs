use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketData {
    pub symbol: String,
    pub exchange: String,
    pub price: f64,
    pub volume: Option<f64>,
    pub rsi: Option<f64>,
    pub macd: Option<f64>,
    pub macd_signal: Option<f64>,
}

// --- Internal deserialization helpers ---

#[derive(Deserialize)]
struct ScannerResponse {
    data: Vec<ScannerRow>,
}

#[derive(Deserialize)]
struct ScannerRow {
    s: String,
    d: Vec<serde_json::Value>,
}

fn value_to_f64_opt(v: Option<&serde_json::Value>) -> Option<f64> {
    v.and_then(|val| val.as_f64())
}

/// Split "BINANCE:BTCUSDT" into ("BINANCE", "BTCUSDT").
fn parse_exchange_symbol(s: &str) -> (String, String) {
    if let Some((exchange, sym)) = s.split_once(':') {
        (exchange.to_string(), sym.to_string())
    } else {
        ("UNKNOWN".to_string(), s.to_string())
    }
}

fn rows_to_market_data(rows: Vec<ScannerRow>, has_volume: bool) -> Vec<MarketData> {
    rows.into_iter()
        .map(|row| {
            let (exchange, symbol) = parse_exchange_symbol(&row.s);
            if has_volume {
                // columns: close, volume, RSI, MACD.macd, MACD.signal
                let price = value_to_f64_opt(row.d.get(0)).unwrap_or(0.0);
                let volume = value_to_f64_opt(row.d.get(1));
                let rsi = value_to_f64_opt(row.d.get(2));
                let macd = value_to_f64_opt(row.d.get(3));
                let macd_signal = value_to_f64_opt(row.d.get(4));
                MarketData { symbol, exchange, price, volume, rsi, macd, macd_signal }
            } else {
                // columns: close, RSI, MACD.macd, MACD.signal
                let price = value_to_f64_opt(row.d.get(0)).unwrap_or(0.0);
                let rsi = value_to_f64_opt(row.d.get(1));
                let macd = value_to_f64_opt(row.d.get(2));
                let macd_signal = value_to_f64_opt(row.d.get(3));
                MarketData { symbol, exchange, price, volume: None, rsi, macd, macd_signal }
            }
        })
        .collect()
}

/// Fetch price and indicators for a list of explicit symbols.
///
/// `symbols` can be bare (`"BTCUSDT"`) or exchange-qualified (`"BINANCE:BTCUSDT"`).
/// Bare symbols are prefixed with `"BINANCE:"`.
pub async fn fetch_indicators(symbols: &[&str]) -> Result<Vec<MarketData>> {
    let full_symbols: Vec<String> = symbols
        .iter()
        .map(|s| {
            if s.contains(':') {
                (*s).to_string()
            } else {
                format!("BINANCE:{s}")
            }
        })
        .collect();

    let body = serde_json::json!({
        "columns": ["close", "RSI", "MACD.macd", "MACD.signal"],
        "symbols": { "tickers": full_symbols }
    });

    let client = reqwest::Client::new();
    let resp: ScannerResponse = client
        .post("https://scanner.tradingview.com/crypto/scan")
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(rows_to_market_data(resp.data, false))
}

/// Fetch the top `limit` crypto pairs by 24-hour volume from TradingView Screener.
///
/// Uses the screener's native sort instead of a hardcoded list — results reflect
/// actual current market activity.
pub async fn fetch_top_by_volume(limit: usize) -> Result<Vec<MarketData>> {
    let body = serde_json::json!({
        "columns": ["close", "volume", "RSI", "MACD.macd", "MACD.signal"],
        "sort": { "sortBy": "volume", "sortOrder": "desc" },
        "filter": [
            { "left": "close", "operation": "greater", "right": 0 },
            { "left": "volume", "operation": "greater", "right": 0 }
        ],
        "range": [0, limit]
    });

    let client = reqwest::Client::new();
    let resp: ScannerResponse = client
        .post("https://scanner.tradingview.com/crypto/scan")
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(rows_to_market_data(resp.data, true))
}

/// Static fallback list used when the network is unavailable.
pub fn top_crypto_symbols_fallback() -> Vec<&'static str> {
    vec![
        "BTCUSDT", "ETHUSDT", "BNBUSDT", "SOLUSDT", "XRPUSDT", "ADAUSDT", "DOGEUSDT",
        "AVAXUSDT", "DOTUSDT", "LINKUSDT", "MATICUSDT", "LTCUSDT", "UNIUSDT", "ATOMUSDT",
        "ETCUSDT", "XLMUSDT", "BCHUSDT", "ALGOUSDT", "VETUSDT", "FILUSDT",
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_exchange_symbol_qualified() {
        let (ex, sym) = parse_exchange_symbol("BINANCE:BTCUSDT");
        assert_eq!(ex, "BINANCE");
        assert_eq!(sym, "BTCUSDT");
    }

    #[test]
    fn parse_exchange_symbol_bare() {
        let (ex, sym) = parse_exchange_symbol("BTCUSDT");
        assert_eq!(ex, "UNKNOWN");
        assert_eq!(sym, "BTCUSDT");
    }

    #[test]
    fn test_parse_screener_response() {
        let json = r#"{
            "data": [
                {"s": "BINANCE:BTCUSDT", "d": [65000.0, 55.3, 120.5, 118.2]},
                {"s": "BINANCE:ETHUSDT", "d": [3200.0, 48.1, 10.2, 9.8]}
            ]
        }"#;

        let resp: ScannerResponse = serde_json::from_str(json).expect("parse failed");
        let data = rows_to_market_data(resp.data, false);
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].symbol, "BTCUSDT");
        assert_eq!(data[0].exchange, "BINANCE");
        assert!((data[0].price - 65000.0).abs() < 0.01);
        assert_eq!(data[0].rsi, Some(55.3));
    }

    #[test]
    fn test_parse_volume_response() {
        let json = r#"{
            "data": [
                {"s": "BINANCE:BTCUSDT", "d": [65000.0, 1500000000.0, 55.3, 120.5, 118.2]}
            ]
        }"#;
        let resp: ScannerResponse = serde_json::from_str(json).expect("parse failed");
        let data = rows_to_market_data(resp.data, true);
        assert_eq!(data[0].volume, Some(1_500_000_000.0));
        assert_eq!(data[0].rsi, Some(55.3));
    }

    #[test]
    fn fallback_list_not_empty() {
        assert_eq!(top_crypto_symbols_fallback().len(), 20);
    }

    #[tokio::test]
    #[ignore]
    async fn test_fetch_network() {
        let symbols = vec!["BTCUSDT", "ETHUSDT"];
        let data = fetch_indicators(&symbols).await.expect("network call failed");
        assert!(!data.is_empty());
        for d in &data {
            assert!(d.price > 0.0, "price should be positive for {}", d.symbol);
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_fetch_top_by_volume_network() {
        let data = fetch_top_by_volume(10).await.expect("network call failed");
        assert!(!data.is_empty());
        assert!(data.len() <= 10);
        // Results should be roughly sorted by volume desc
        let vols: Vec<f64> = data.iter().filter_map(|d| d.volume).collect();
        assert!(!vols.is_empty());
    }
}
