//! HTTP exchange endpoints (signed, require private key).

use crate::error::{HyperliquidError, Result};
use crate::types::{Order, OrderResponse};
use serde_json::json;

const MAINNET_EXCHANGE_URL: &str = "https://api.hyperliquid.xyz/exchange";
const TESTNET_EXCHANGE_URL: &str = "https://api.hyperliquid-testnet.xyz/exchange";

/// A simple signer abstraction. In practice this wraps an EVM private key
/// (Hyperliquid uses secp256k1 ECDSA signing).
#[derive(Debug, Clone)]
pub struct Signer {
    pub address: String,
    // The actual signing logic delegates to the SDK or wallet-manager.
    // For now we store the raw bytes so the SDK can sign.
    #[allow(dead_code)]
    pub(crate) secret_bytes: Vec<u8>,
}

impl Signer {
    /// Create a signer from a hex private key (with or without 0x prefix).
    pub fn from_pk(pk_hex: &str) -> Result<Self> {
        let stripped = pk_hex.strip_prefix("0x").unwrap_or(pk_hex);
        let bytes = hex::decode(stripped)
            .map_err(|e| HyperliquidError::Other(format!("invalid pk hex: {e}")))?;
        Self::from_pk_bytes(bytes)
    }

    /// Create a signer from raw 32-byte private key bytes.
    pub fn from_pk_bytes(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() != 32 {
            return Err(HyperliquidError::Other(
                "private key must be 32 bytes".to_string(),
            ));
        }
        // Derive address from pk (simplified — real impl uses secp256k1 pub key recovery)
        let address = format!("0x{}", hex::encode(&bytes[..20]));
        Ok(Self {
            address,
            secret_bytes: bytes,
        })
    }

    pub fn address(&self) -> &str {
        &self.address
    }
}

#[derive(Debug, Clone)]
pub struct ExchangeClient {
    http: reqwest::Client,
    base_url: String,
    signer: Signer,
}

impl ExchangeClient {
    pub fn new_mainnet(signer: Signer) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: MAINNET_EXCHANGE_URL.to_string(),
            signer,
        }
    }

    pub fn new_testnet(signer: Signer) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: TESTNET_EXCHANGE_URL.to_string(),
            signer,
        }
    }

    pub fn with_url(url: impl Into<String>, signer: Signer) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: url.into(),
            signer,
        }
    }

    pub fn signer_address(&self) -> &str {
        self.signer.address()
    }

    /// Place a signed order.
    ///
    /// NOTE: Full signing integration with the SDK is a TODO.
    /// This skeleton sends an unsigned placeholder so the network layer
    /// compiles and tests pass. The real implementation signs the action
    /// with the secp256k1 key before sending.
    pub async fn place_order(&self, order: &Order) -> Result<OrderResponse> {
        let action = json!({
            "type": "order",
            "orders": [{
                "coin": order.coin,
                "is_buy": matches!(order.side, crate::types::OrderSide::Buy),
                "sz": order.size.to_string(),
                "limit_px": order.price.map(|p| p.to_string()).unwrap_or_default(),
                "order_type": match order.order_type {
                    crate::types::OrderType::Limit => "Limit",
                    crate::types::OrderType::Market => "Market",
                },
                "reduce_only": order.reduce_only,
            }],
            "grouping": "na",
        });

        let payload = json!({
            "action": action,
            "nonce": chrono::Utc::now().timestamp_millis(),
            "signature": self.sign_placeholder(),
        });

        let resp = self
            .http
            .post(&self.base_url)
            .json(&payload)
            .send()
            .await
            .map_err(HyperliquidError::Network)?;

        let status = resp.status();
        let text = resp.text().await.map_err(HyperliquidError::Network)?;

        if !status.is_success() {
            if text.to_lowercase().contains("rate limit") {
                return Err(HyperliquidError::RateLimited);
            }
            if text.to_lowercase().contains("margin") {
                return Err(HyperliquidError::InsufficientMargin);
            }
            return Err(HyperliquidError::Other(format!(
                "exchange error {}: {}",
                status, text
            )));
        }

        let parsed: OrderResponse = serde_json::from_str(&text)?;
        Ok(parsed)
    }

    /// Cancel an order by coin and order id.
    pub async fn cancel_order(&self, coin: &str, oid: u64) -> Result<()> {
        let action = json!({
            "type": "cancel",
            "cancels": [{"coin": coin, "oid": oid}],
        });

        let payload = json!({
            "action": action,
            "nonce": chrono::Utc::now().timestamp_millis(),
            "signature": self.sign_placeholder(),
        });

        let resp = self
            .http
            .post(&self.base_url)
            .json(&payload)
            .send()
            .await
            .map_err(HyperliquidError::Network)?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(HyperliquidError::Other(format!(
                "cancel error {}: {}",
                status, text
            )));
        }
        Ok(())
    }

    /// Cancel all orders, optionally scoped to a coin.
    pub async fn cancel_all(&self, coin: Option<&str>) -> Result<()> {
        let action = if let Some(c) = coin {
            json!({"type": "cancelAll", "coin": c})
        } else {
            json!({"type": "cancelAll"})
        };

        let payload = json!({
            "action": action,
            "nonce": chrono::Utc::now().timestamp_millis(),
            "signature": self.sign_placeholder(),
        });

        let resp = self
            .http
            .post(&self.base_url)
            .json(&payload)
            .send()
            .await
            .map_err(HyperliquidError::Network)?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(HyperliquidError::Other(format!(
                "cancelAll error: {}",
                text
            )));
        }
        Ok(())
    }

    /// Close a position for a coin (market order that reduces position to zero).
    pub async fn close_position(&self, coin: &str) -> Result<OrderResponse> {
        let _order = Order::market_sell(coin, 0.0); // size ignored by "close" action
        let action = json!({
            "type": "close",
            "coin": coin,
        });

        let payload = json!({
            "action": action,
            "nonce": chrono::Utc::now().timestamp_millis(),
            "signature": self.sign_placeholder(),
        });

        let resp = self
            .http
            .post(&self.base_url)
            .json(&payload)
            .send()
            .await
            .map_err(HyperliquidError::Network)?;

        let text = resp.text().await.map_err(HyperliquidError::Network)?;
        let parsed: OrderResponse = serde_json::from_str(&text)?;
        Ok(parsed)
    }

    // TODO: Replace with real ECDSA secp256k1 signing via the SDK.
    fn sign_placeholder(&self) -> serde_json::Value {
        json!({
            "r": "0x",
            "s": "0x",
            "v": 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signer_from_pk() {
        let pk = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let signer = Signer::from_pk(pk).unwrap();
        assert!(!signer.address().is_empty());
        assert_eq!(signer.secret_bytes.len(), 32);
    }

    #[test]
    fn test_signer_from_pk_bytes() {
        let bytes = vec![0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef,
                         0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef,
                         0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef,
                         0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef];
        let signer = Signer::from_pk_bytes(bytes).unwrap();
        assert!(!signer.address().is_empty());
        assert_eq!(signer.secret_bytes.len(), 32);
    }

    #[test]
    fn test_exchange_urls() {
        let signer =
            Signer::from_pk("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
                .unwrap();
        let mainnet = ExchangeClient::new_mainnet(signer.clone());
        assert_eq!(mainnet.base_url, MAINNET_EXCHANGE_URL);

        let testnet = ExchangeClient::new_testnet(signer);
        assert_eq!(testnet.base_url, TESTNET_EXCHANGE_URL);
    }
}
