//! HTTP info endpoints (read-only, no signing required).

use crate::error::{HyperliquidError, Result};
use crate::types::{ClearinghouseState, FundingRate, L2Book};
use std::collections::HashMap;

const MAINNET_INFO_URL: &str = "https://api.hyperliquid.xyz/info";
const TESTNET_INFO_URL: &str = "https://api.hyperliquid-testnet.xyz/info";

#[derive(Debug, Clone)]
pub struct InfoClient {
    http: reqwest::Client,
    base_url: String,
}

impl InfoClient {
    pub fn new_mainnet() -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: MAINNET_INFO_URL.to_string(),
        }
    }

    pub fn new_testnet() -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: TESTNET_INFO_URL.to_string(),
        }
    }

    pub fn with_url(url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: url.into(),
        }
    }

    /// Current mid prices for all coins.
    pub async fn mids(&self) -> Result<HashMap<String, f64>> {
        let body = serde_json::json!({"type": "allMids"});
        let resp = self
            .http
            .post(&self.base_url)
            .json(&body)
            .send()
            .await
            .map_err(HyperliquidError::Network)?;

        let text = resp.text().await.map_err(HyperliquidError::Network)?;
        let parsed: serde_json::Value = serde_json::from_str(&text)?;

        let mut map = HashMap::new();
        if let Some(obj) = parsed.as_object() {
            for (k, v) in obj {
                if let Some(px) = v.as_str().and_then(|s| s.parse::<f64>().ok()) {
                    map.insert(k.clone(), px);
                }
            }
        }
        Ok(map)
    }

    /// L2 orderbook for a coin.
    pub async fn l2_book(&self, coin: &str) -> Result<L2Book> {
        let body = serde_json::json!({
            "type": "l2Book",
            "coin": coin,
        });
        let resp = self
            .http
            .post(&self.base_url)
            .json(&body)
            .send()
            .await
            .map_err(HyperliquidError::Network)?;

        let mut book: L2Book = resp.json().await.map_err(HyperliquidError::Network)?;
        book.coin = coin.to_string();
        Ok(book)
    }

    /// Current funding rate for a coin.
    pub async fn funding_rate(&self, coin: &str) -> Result<FundingRate> {
        let body = serde_json::json!({
            "type": "fundingHistory",
            "coin": coin,
            "startTime": 0i64,
            "endTime": chrono::Utc::now().timestamp_millis(),
        });
        let resp = self
            .http
            .post(&self.base_url)
            .json(&body)
            .send()
            .await
            .map_err(HyperliquidError::Network)?;

        let arr: Vec<serde_json::Value> = resp.json().await.map_err(HyperliquidError::Network)?;
        let latest = arr
            .last()
            .ok_or_else(|| HyperliquidError::Other("empty funding history".to_string()))?;

        let rate = latest["fundingRate"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| latest["fundingRate"].as_f64())
            .unwrap_or(0.0);

        let next_time = latest["nextFundingTime"]
            .as_i64()
            .or_else(|| latest["time"].as_i64())
            .unwrap_or(0);

        Ok(FundingRate {
            coin: coin.to_string(),
            funding_rate: rate,
            next_funding_time: next_time,
        })
    }

    /// Predicted funding rates for all coins.
    pub async fn predicted_funding(&self) -> Result<HashMap<String, f64>> {
        let body = serde_json::json!({"type": "predictedFundings"});
        let resp = self
            .http
            .post(&self.base_url)
            .json(&body)
            .send()
            .await
            .map_err(HyperliquidError::Network)?;

        let parsed: serde_json::Value = resp.json().await.map_err(HyperliquidError::Network)?;
        let mut map = HashMap::new();

        if let Some(arr) = parsed.as_array() {
            for entry in arr {
                if let (Some(coin), Some(rate_val)) = (
                    entry["coin"].as_str(),
                    entry["predFundingRate"]
                        .as_str()
                        .and_then(|s| s.parse::<f64>().ok()),
                ) {
                    map.insert(coin.to_string(), rate_val);
                }
            }
        }
        Ok(map)
    }

    /// Clearinghouse state for an address.
    pub async fn clearinghouse_state(&self, address: &str) -> Result<ClearinghouseState> {
        let body = serde_json::json!({
            "type": "clearinghouseState",
            "user": address,
        });
        let resp = self
            .http
            .post(&self.base_url)
            .json(&body)
            .send()
            .await
            .map_err(HyperliquidError::Network)?;

        let state: ClearinghouseState = resp.json().await.map_err(HyperliquidError::Network)?;
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_info_client_urls() {
        let mainnet = InfoClient::new_mainnet();
        assert_eq!(mainnet.base_url, MAINNET_INFO_URL);

        let testnet = InfoClient::new_testnet();
        assert_eq!(testnet.base_url, TESTNET_INFO_URL);
    }
}
