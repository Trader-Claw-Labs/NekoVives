//! Minimal Binance USD-M Futures REST adapter.
//!
//! Provides market-order placement, position query, and position close
//! for Binance perpetual futures.  Uses HMAC-SHA256 request signing.

use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

const BASE_URL: &str = "https://fapi.binance.com";

type HmacSha256 = Hmac<Sha256>;

/// Binance API credentials.
#[derive(Debug, Clone)]
pub struct BinanceCredentials {
    pub api_key: String,
    pub api_secret: String,
}

/// An open position returned by Binance.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BinancePosition {
    pub symbol: String,
    #[serde(rename = "positionAmt", deserialize_with = "deserialize_f64_string")]
    pub position_amt: f64,
    #[serde(rename = "entryPrice", deserialize_with = "deserialize_f64_string")]
    pub entry_price: f64,
    #[serde(rename = "markPrice", deserialize_with = "deserialize_f64_string")]
    pub mark_price: f64,
    #[serde(rename = "unRealizedProfit", deserialize_with = "deserialize_f64_string")]
    pub unrealized_pnl: f64,
    #[serde(deserialize_with = "deserialize_f64_string")]
    pub leverage: f64,
}

/// Raw position-risk response from Binance.
#[derive(Debug, Clone, serde::Deserialize)]
struct PositionRisk {
    symbol: String,
    #[serde(rename = "positionAmt", deserialize_with = "deserialize_f64_string")]
    position_amt: f64,
    #[serde(rename = "entryPrice", deserialize_with = "deserialize_f64_string")]
    entry_price: f64,
    #[serde(rename = "markPrice", deserialize_with = "deserialize_f64_string")]
    mark_price: f64,
    #[serde(rename = "unRealizedProfit", deserialize_with = "deserialize_f64_string")]
    un_realized_profit: f64,
    #[serde(deserialize_with = "deserialize_f64_string")]
    leverage: f64,
}

fn deserialize_f64_string<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse().map_err(serde::de::Error::custom)
}

fn sign_query(query: &str, secret: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(query.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Place a market order on Binance Futures.
pub async fn place_market_order(
    creds: &BinanceCredentials,
    symbol: &str,
    side: &str,
    quantity: f64,
    reduce_only: bool,
) -> anyhow::Result<serde_json::Value> {
    let client = reqwest::Client::new();
    let ts = timestamp_ms();
    let mut params = vec![
        format!("symbol={}", symbol.to_uppercase()),
        format!("side={}", side),
        "type=MARKET".to_string(),
        format!("quantity={:.6}", quantity),
        format!("timestamp={}", ts),
        "recvWindow=5000".to_string(),
    ];
    if reduce_only {
        params.push("reduceOnly=true".to_string());
    }
    let query = params.join("&");
    let signature = sign_query(&query, &creds.api_secret);
    let url = format!("{}/fapi/v1/order?{}&signature={}", BASE_URL, query, signature);

    let resp = client
        .post(&url)
        .header("X-MBX-APIKEY", &creds.api_key)
        .send()
        .await?;

    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        return Err(anyhow::anyhow!("Binance order error ({}): {}", status, text));
    }
    Ok(serde_json::from_str(&text)?)
}

/// Get the open position for a symbol.
pub async fn get_position(
    creds: &BinanceCredentials,
    symbol: &str,
) -> anyhow::Result<Option<BinancePosition>> {
    let client = reqwest::Client::new();
    let ts = timestamp_ms();
    let query = format!(
        "symbol={}&timestamp={}&recvWindow=5000",
        symbol.to_uppercase(),
        ts
    );
    let signature = sign_query(&query, &creds.api_secret);
    let url = format!("{}/fapi/v2/positionRisk?{}&signature={}", BASE_URL, query, signature);

    let resp = client
        .get(&url)
        .header("X-MBX-APIKEY", &creds.api_key)
        .send()
        .await?;

    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        return Err(anyhow::anyhow!("Binance position error ({}): {}", status, text));
    }

    let risks: Vec<PositionRisk> = serde_json::from_str(&text)?;
    Ok(risks.into_iter().find(|p| {
        p.symbol == symbol.to_uppercase() && p.position_amt != 0.0
    }).map(|p| BinancePosition {
        symbol: p.symbol,
        position_amt: p.position_amt,
        entry_price: p.entry_price,
        mark_price: p.mark_price,
        unrealized_pnl: p.un_realized_profit,
        leverage: p.leverage,
    }))
}

/// Close an open position via reduce-only market order.
pub async fn close_position(
    creds: &BinanceCredentials,
    symbol: &str,
) -> anyhow::Result<serde_json::Value> {
    let pos = get_position(creds, symbol).await?;
    let Some(p) = pos else {
        return Ok(serde_json::json!({"status": "no_position"}));
    };

    let side = if p.position_amt > 0.0 { "SELL" } else { "BUY" };
    let qty = p.position_amt.abs();
    place_market_order(creds, symbol, side, qty, true).await
}

/// Test connectivity (ping).
pub async fn test_connection(creds: &BinanceCredentials) -> anyhow::Result<bool> {
    let client = reqwest::Client::new();
    let ts = timestamp_ms();
    let query = format!("timestamp={}&recvWindow=5000", ts);
    let signature = sign_query(&query, &creds.api_secret);
    let url = format!("{}/fapi/v2/account?{}&signature={}", BASE_URL, query, signature);

    let resp = client
        .get(&url)
        .header("X-MBX-APIKEY", &creds.api_key)
        .send()
        .await?;

    Ok(resp.status().is_success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_query() {
        let sig = sign_query("symbol=BTCUSDT&side=BUY&type=MARKET&quantity=0.001&timestamp=123456789", "secret");
        assert!(!sig.is_empty());
        assert_eq!(sig.len(), 64); // SHA-256 hex = 64 chars
    }
}
