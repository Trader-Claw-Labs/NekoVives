use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketData {
    pub symbol: String,
    pub price: f64,
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

fn symbol_from_exchange_pair(s: &str) -> String {
    // "BINANCE:BTCUSDT" -> "BTCUSDT"
    s.split(':').nth(1).unwrap_or(s).to_string()
}

/// Fetch price and indicators for a list of symbols.
/// symbols: e.g. ["BTCUSDT", "ETHUSDT"] — will be prefixed with "BINANCE:"
pub async fn fetch_indicators(symbols: &[&str]) -> Result<Vec<MarketData>> {
    let full_symbols: Vec<String> = symbols
        .iter()
        .map(|s| format!("BINANCE:{s}"))
        .collect();

    let body = serde_json::json!({
        "columns": ["close", "RSI", "MACD.macd", "MACD.signal"],
        "filter": [{
            "left": "name",
            "operation": "in_range",
            "right": full_symbols
        }]
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

    let results = resp
        .data
        .into_iter()
        .map(|row| {
            let symbol = symbol_from_exchange_pair(&row.s);
            let price = value_to_f64_opt(row.d.get(0)).unwrap_or(0.0);
            let rsi = value_to_f64_opt(row.d.get(1));
            let macd = value_to_f64_opt(row.d.get(2));
            let macd_signal = value_to_f64_opt(row.d.get(3));
            MarketData {
                symbol,
                price,
                rsi,
                macd,
                macd_signal,
            }
        })
        .collect();

    Ok(results)
}

/// Fetch top crypto symbols by volume (hardcoded list of 20 common symbols).
pub fn top_crypto_symbols() -> Vec<&'static str> {
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
    fn test_parse_screener_response() {
        let json = r#"{
            "data": [
                {"s": "BINANCE:BTCUSDT", "d": [65000.0, 55.3, 120.5, 118.2]},
                {"s": "BINANCE:ETHUSDT", "d": [3200.0, 48.1, 10.2, 9.8]}
            ]
        }"#;

        let resp: ScannerResponse = serde_json::from_str(json).expect("parse failed");
        assert_eq!(resp.data.len(), 2);

        let btc = &resp.data[0];
        assert_eq!(btc.s, "BINANCE:BTCUSDT");
        assert_eq!(btc.d[0].as_f64().unwrap(), 65000.0);
        assert_eq!(btc.d[1].as_f64().unwrap(), 55.3);
        assert_eq!(btc.d[2].as_f64().unwrap(), 120.5);
        assert_eq!(btc.d[3].as_f64().unwrap(), 118.2);

        // Parse into MarketData
        let symbol = symbol_from_exchange_pair(&btc.s);
        let price = value_to_f64_opt(btc.d.get(0)).unwrap_or(0.0);
        let rsi = value_to_f64_opt(btc.d.get(1));
        let macd = value_to_f64_opt(btc.d.get(2));
        let macd_signal = value_to_f64_opt(btc.d.get(3));

        assert_eq!(symbol, "BTCUSDT");
        assert!((price - 65000.0).abs() < 0.01);
        assert_eq!(rsi, Some(55.3));
        assert_eq!(macd, Some(120.5));
        assert_eq!(macd_signal, Some(118.2));
    }

    #[test]
    fn test_top_crypto_symbols() {
        let symbols = top_crypto_symbols();
        assert!(!symbols.is_empty());
        assert!(symbols.contains(&"BTCUSDT"));
        assert!(symbols.contains(&"ETHUSDT"));
        assert_eq!(symbols.len(), 20);
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
}
