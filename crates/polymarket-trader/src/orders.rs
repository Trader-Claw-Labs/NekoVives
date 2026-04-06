use crate::auth::{create_l2_headers, PolyCredentials};
use anyhow::Result;
use serde::{Deserialize, Serialize};

const CLOB_BASE_URL: &str = "https://clob.polymarket.com";

/// Order side
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    fn as_str(self) -> &'static str {
        match self {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        }
    }
}

/// Order response from CLOB API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderResponse {
    pub order_id: String,
    pub status: String,
}

/// Raw API response shape: field is `orderID`
#[derive(Deserialize)]
struct RawOrderResponse {
    #[serde(rename = "orderID")]
    order_id: String,
    status: String,
}

/// Open order from GET /orders
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub token_id: String,
    pub side: String,
    pub price: String,
    pub size: String,
    pub status: String,
}

/// A CLOB client that holds credentials
pub struct ClobClient {
    creds: PolyCredentials,
    http: reqwest::Client,
}

impl ClobClient {
    pub fn new(creds: PolyCredentials) -> Self {
        Self {
            creds,
            http: reqwest::Client::new(),
        }
    }

    /// POST /order — GTC limit order
    pub async fn create_limit_order(
        &self,
        token_id: &str,
        side: Side,
        price: f64,
        size: f64,
    ) -> Result<OrderResponse> {
        let body = serde_json::json!({
            "order": {
                "tokenID": token_id,
                "price": format!("{:.2}", price),
                "size": format!("{:.2}", size),
                "side": side.as_str(),
                "type": "GTC"
            },
            "owner": self.creds.api_key
        });
        let body_str = serde_json::to_string(&body)?;
        let headers = create_l2_headers(&self.creds, "POST", "/order", Some(&body_str));

        let mut req = self
            .http
            .post(format!("{CLOB_BASE_URL}/order"))
            .header("Content-Type", "application/json")
            .body(body_str);

        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("create_limit_order failed ({}): {}", status, text);
        }

        let raw: RawOrderResponse = resp.json().await?;
        Ok(OrderResponse {
            order_id: raw.order_id,
            status: raw.status,
        })
    }

    /// POST /order — FOK market order
    pub async fn create_market_order(
        &self,
        token_id: &str,
        side: Side,
        amount_usdc: f64,
        worst_price: f64,
    ) -> Result<OrderResponse> {
        let body = serde_json::json!({
            "order": {
                "tokenID": token_id,
                "amount": format!("{:.2}", amount_usdc),
                "side": side.as_str(),
                "type": "FOK",
                "worstPrice": format!("{:.2}", worst_price)
            },
            "owner": self.creds.api_key
        });
        let body_str = serde_json::to_string(&body)?;
        let headers = create_l2_headers(&self.creds, "POST", "/order", Some(&body_str));

        let mut req = self
            .http
            .post(format!("{CLOB_BASE_URL}/order"))
            .header("Content-Type", "application/json")
            .body(body_str);

        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("create_market_order failed ({}): {}", status, text);
        }

        let raw: RawOrderResponse = resp.json().await?;
        Ok(OrderResponse {
            order_id: raw.order_id,
            status: raw.status,
        })
    }

    /// DELETE /orders/<order_id>
    pub async fn cancel_order(&self, order_id: &str) -> Result<()> {
        let path = format!("/orders/{}", order_id);
        let headers = create_l2_headers(&self.creds, "DELETE", &path, None);

        let mut req = self.http.delete(format!("{CLOB_BASE_URL}{}", path));

        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("cancel_order failed ({}): {}", status, text);
        }

        Ok(())
    }

    /// GET /orders?owner=<api_key>&status=LIVE
    pub async fn get_open_orders(&self) -> Result<Vec<Order>> {
        let path = "/orders";
        let headers = create_l2_headers(&self.creds, "GET", path, None);

        let mut req = self
            .http
            .get(format!("{CLOB_BASE_URL}{}", path))
            .query(&[("owner", &self.creds.api_key), ("status", &"LIVE".to_string())]);

        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("get_open_orders failed ({}): {}", status, text);
        }

        let orders: Vec<Order> = resp.json().await?;
        Ok(orders)
    }

    /// POST /heartbeat — keep session alive (call every 5s while orders open)
    pub async fn heartbeat(&self) -> Result<()> {
        let path = "/heartbeat";
        let headers = create_l2_headers(&self.creds, "POST", path, None);

        let mut req = self.http.post(format!("{CLOB_BASE_URL}{}", path));

        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("heartbeat failed ({}): {}", status, text);
        }

        Ok(())
    }

    /// Spawn a tokio task that calls heartbeat() every 5 seconds.
    /// Returns a CancellationToken to stop it.
    pub fn start_heartbeat(
        self: std::sync::Arc<Self>,
    ) -> tokio_util::sync::CancellationToken {
        let token = tokio_util::sync::CancellationToken::new();
        let child = token.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = child.cancelled() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                        if let Err(_e) = self.heartbeat().await {
                            // Do not log credentials; just note the failure
                        }
                    }
                }
            }
        });

        token
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::PolyCredentials;

    fn mock_creds() -> PolyCredentials {
        PolyCredentials {
            api_key: "test-key".to_string(),
            secret: "test-secret".to_string(),
            passphrase: "test-pass".to_string(),
        }
    }

    #[test]
    fn test_clob_client_new() {
        let creds = mock_creds();
        let client = ClobClient::new(creds.clone());
        assert_eq!(client.creds.api_key, creds.api_key);
        assert_eq!(client.creds.passphrase, creds.passphrase);
    }

    #[test]
    fn test_side_serialization() {
        let buy = serde_json::to_string(&Side::Buy).unwrap();
        let sell = serde_json::to_string(&Side::Sell).unwrap();
        assert_eq!(buy, "\"Buy\"");
        assert_eq!(sell, "\"Sell\"");
    }

    #[tokio::test]
    #[ignore]
    async fn test_network_create_limit_order() {
        let creds = mock_creds();
        let client = ClobClient::new(creds);
        let resp = client
            .create_limit_order("some-token-id", Side::Buy, 0.65, 10.0)
            .await
            .expect("create_limit_order failed");
        assert!(!resp.order_id.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_network_cancel_order() {
        let creds = mock_creds();
        let client = ClobClient::new(creds);
        client
            .cancel_order("some-order-id")
            .await
            .expect("cancel_order failed");
    }

    #[tokio::test]
    #[ignore]
    async fn test_network_get_open_orders() {
        let creds = mock_creds();
        let client = ClobClient::new(creds);
        let orders = client.get_open_orders().await.expect("get_open_orders failed");
        // Just verify it returns a vec (may be empty)
        let _ = orders;
    }

    #[tokio::test]
    async fn test_heartbeat_starts_and_stops() {
        let creds = mock_creds();
        let client = std::sync::Arc::new(ClobClient::new(creds));
        let token = client.start_heartbeat();
        // Cancel immediately; the spawned task should stop without panic
        token.cancel();
        // Give the task a moment to receive cancellation
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // If we reach here without panic, the test passes
    }
}
