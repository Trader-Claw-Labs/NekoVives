use crate::auth::PolyCredentials;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::str::FromStr as _;

use polymarket_client_sdk_v2::POLYGON;
use polymarket_client_sdk_v2::auth::{Credentials as SdkCredentials, LocalSigner, Signer, Uuid};
use polymarket_client_sdk_v2::clob::types::request::OrdersRequest;
use polymarket_client_sdk_v2::clob::types::{Amount, AssetType, OrderType, Side as SdkSide, SignatureType};
use polymarket_client_sdk_v2::clob::{Client as SdkClient, Config as SdkConfig};
use polymarket_client_sdk_v2::clob::types::request::BalanceAllowanceRequest;
use polymarket_client_sdk_v2::derive_proxy_wallet;
use polymarket_client_sdk_v2::derive_safe_wallet;
use polymarket_client_sdk_v2::types::{B256, Decimal, U256};

// V2 protocol is served at the same host. clob-v2.polymarket.com just 301-redirects
// here, which breaks POST. The SDK queries /version to pick V1 vs V2 payload shape.
const CLOB_BASE_URL: &str = "https://clob.polymarket.com";

/// Trader-Claw Polymarket Builder Code.
///
/// Hard-coded so every order placed through this binary is attributed to the
/// Trader-Claw builder, regardless of which user/wallet runs it.  This is the
/// equivalent of the TS SDK `builderCode` argument on `createAndPostOrder`.
///
/// Do NOT make this user-configurable: end users running their own forks
/// should change this constant if they need to point trades at a different
/// builder identity.
const TRADER_CLAW_BUILDER_CODE: B256 = polymarket_client_sdk_v2::types::b256!(
    "0x81ede7d2e551e79016287430c4901461785f2823610ad3c545f6b2ebed8048a6"
);

/// Order side
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    fn to_sdk_side(self) -> SdkSide {
        match self {
            Side::Buy => SdkSide::Buy,
            Side::Sell => SdkSide::Sell,
        }
    }
}

/// Order response from CLOB API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderResponse {
    pub order_id: String,
    pub status: String,
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
}

impl ClobClient {
    pub fn new(creds: PolyCredentials) -> Self {
        Self { creds }
    }

    /// Re-authenticate via Polymarket L1 EIP-712 to obtain fresh L2 session
    /// credentials (api_key / secret / passphrase).  Returns a new ClobClient
    /// that carries the renewed credentials while preserving private_key,
    /// proxy_address, and signature_type from the original.
    /// Requires private_key to be set in the stored credentials.
    pub async fn renew(&self) -> Result<Self> {
        let pk = self.creds.private_key.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Cannot renew credentials: private_key not configured"))?;
        // Use derive-api-key (deterministic) instead of create-api-key, since the
        // L2 credentials already exist and create-api-key returns "Could not create".
        tracing::info!("CLOB credentials renewing via derive-api-key…");
        let fresh = crate::auth::derive_api_key(pk).await?;
        tracing::info!("CLOB credentials renewed: new api_key={}", &fresh.api_key[..8.min(fresh.api_key.len())]);
        Ok(Self {
            creds: PolyCredentials {
                api_key: fresh.api_key,
                secret: fresh.secret,
                passphrase: fresh.passphrase,
                wallet_address: self.creds.wallet_address.clone(), // keep original wallet address
                private_key: self.creds.private_key.clone(),
                is_builder: self.creds.is_builder,
                proxy_address: self.creds.proxy_address.clone(),
                signature_type: self.creds.signature_type.clone(),
            },
        })
    }

    /// Return the current credentials (cloned).  Used to persist renewed creds.
    pub fn credentials(&self) -> &PolyCredentials {
        &self.creds
    }

    /// Returns the hard-coded Trader-Claw [`TRADER_CLAW_BUILDER_CODE`].
    /// Every order created via this client is tagged with this code so trades
    /// are attributed to the Trader-Claw builder identity.
    fn builder_code(&self) -> B256 {
        TRADER_CLAW_BUILDER_CODE
    }

    /// Parse the stored private key into a LocalSigner.
    fn make_signer(&self) -> Result<polymarket_client_sdk_v2::auth::LocalSigner<k256::ecdsa::SigningKey>> {
        let pk_hex = self
            .creds
            .private_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Private key is required for order signing. Set polymarket.private_key in config."))?;
        let signer = LocalSigner::from_str(pk_hex)?
            .with_chain_id(Some(POLYGON));
        Ok(signer)
    }

    /// Create SDK credentials from our stored credentials.
    fn sdk_credentials(&self) -> Result<SdkCredentials> {
        let key = Uuid::parse_str(&self.creds.api_key)
            .map_err(|e| anyhow::anyhow!("Invalid API key UUID: {e}"))?;
        Ok(SdkCredentials::new(
            key,
            self.creds.secret.clone(),
            self.creds.passphrase.clone(),
        ))
    }

    /// Build an authenticated SDK CLOB client.
    /// Automatically detects whether the user has a Gnosis Safe or an EIP-1167 proxy
    /// (MetaMask users typically get a Safe; Magic/email users get a Proxy).
    async fn sdk_client(&self) -> Result<SdkClient<polymarket_client_sdk_v2::auth::state::Authenticated<polymarket_client_sdk_v2::auth::Normal>>> {
        let signer = self.make_signer()?;
        let sdk_creds = self.sdk_credentials()?;

        let client = SdkClient::new(CLOB_BASE_URL, SdkConfig::default())?;
        let auth = client.authentication_builder(&signer).credentials(sdk_creds);
        let signer_addr = signer.address();
        tracing::info!("CLOB signer address derived from private_key: {}", signer_addr);

        // If the user has explicitly set a signature_type, honour it — this
        // overrides auto-detection and fixes order_version_mismatch for wallets
        // that are plain EOA or a Proxy that wasn't derived from this signer.
        if let Some(sig_type) = self.creds.signature_type.as_deref() {
            let auth = match sig_type.to_lowercase().as_str() {
                "eoa" => {
                    tracing::info!("CLOB auth: forced EOA (no proxy/funder)");
                    auth
                }
                "proxy" => {
                    let proxy_addr = self.creds.proxy_address.as_deref()
                        .filter(|s| !s.is_empty())
                        .and_then(|s| s.parse::<polymarket_client_sdk_v2::types::Address>().ok())
                        .or_else(|| derive_proxy_wallet(signer_addr, POLYGON));
                    if let Some(addr) = proxy_addr {
                        tracing::info!("CLOB auth: forced Proxy {}", addr);
                        auth.funder(addr).signature_type(SignatureType::Proxy)
                    } else {
                        tracing::warn!("CLOB auth: forced proxy but no proxy address derivable, using EOA");
                        auth
                    }
                }
                "gnosis_safe" | "safe" => {
                    let safe_addr = self.creds.proxy_address.as_deref()
                        .filter(|s| !s.is_empty())
                        .and_then(|s| s.parse::<polymarket_client_sdk_v2::types::Address>().ok())
                        .or_else(|| derive_safe_wallet(signer_addr, POLYGON));
                    if let Some(addr) = safe_addr {
                        tracing::info!("CLOB auth: forced Gnosis Safe {}", addr);
                        auth.funder(addr).signature_type(SignatureType::GnosisSafe)
                    } else {
                        tracing::warn!("CLOB auth: forced gnosis_safe but no safe address derivable, using EOA");
                        auth
                    }
                }
                other => {
                    tracing::warn!("CLOB auth: unknown signature_type '{}', falling back to auto-detect", other);
                    // fall through to auto-detect below by re-entering default path
                    auth
                }
            };
            let authenticated = auth.authenticate().await?;
            return Ok(authenticated);
        }

        let auth = match self.creds.proxy_address.as_deref() {
            Some(proxy) if !proxy.is_empty() => {
                let proxy_addr = proxy.parse::<polymarket_client_sdk_v2::types::Address>()
                    .map_err(|e| anyhow::anyhow!("Invalid proxy address: {e}"))?;

                // Determine whether the explicit address matches a derived Safe or Proxy
                let derived_safe  = derive_safe_wallet(signer_addr, POLYGON);
                let derived_proxy = derive_proxy_wallet(signer_addr, POLYGON);

                let is_safe  = derived_safe.map_or(false, |d| d == proxy_addr);
                let is_proxy = !is_safe && derived_proxy.map_or(false, |d| d == proxy_addr);

                if is_safe {
                    tracing::info!("CLOB auth using explicit Gnosis Safe: {}", proxy_addr);
                    auth.funder(proxy_addr)
                        .signature_type(SignatureType::GnosisSafe)
                } else if is_proxy {
                    tracing::info!("CLOB auth using explicit EIP-1167 proxy: {}", proxy_addr);
                    auth.funder(proxy_addr)
                        .signature_type(SignatureType::Proxy)
                } else {
                    // Address doesn't match either derived wallet — assume a custom Safe
                    // (most modern Polymarket wallets are Gnosis Safe variants).
                    tracing::info!(
                        "CLOB auth using explicit funder {} (no derived match for signer {})",
                        proxy_addr, signer_addr
                    );
                    auth.funder(proxy_addr)
                        .signature_type(SignatureType::GnosisSafe)
                }
            }
            _ => {
                // No explicit proxy — auto-derive.  Try Safe first (MetaMask/browser wallets),
                // then fall back to Proxy (Magic/email wallets), then EOA.
                if let Some(safe) = derive_safe_wallet(signer_addr, POLYGON) {
                    tracing::info!(
                        "CLOB auth auto-derived Gnosis Safe {} from signer {}",
                        safe, signer_addr
                    );
                    auth.funder(safe)
                        .signature_type(SignatureType::GnosisSafe)
                } else if let Some(proxy) = derive_proxy_wallet(signer_addr, POLYGON) {
                    tracing::info!(
                        "CLOB auth auto-derived EIP-1167 proxy {} from signer {}",
                        proxy, signer_addr
                    );
                    auth.funder(proxy)
                        .signature_type(SignatureType::Proxy)
                } else {
                    tracing::info!(
                        "CLOB auth using EOA (no wallet derivation available for signer {})",
                        signer_addr
                    );
                    auth
                }
            }
        };

        let authenticated = auth.authenticate().await?;
        Ok(authenticated)
    }

    /// POST /order — GTC limit order
    pub async fn create_limit_order(
        &self,
        token_id: &str,
        side: Side,
        price: f64,
        size: f64,
    ) -> Result<OrderResponse> {
        let signer = self.make_signer()?;
        let client = self.sdk_client().await?;

        let token_id_u256 = U256::from_str(token_id)
            .map_err(|e| anyhow::anyhow!("Invalid token_id: {e}"))?;
        // Use string formatting to ensure exactly 2dp — from_f64_retain would
        // preserve IEEE-754 noise (e.g. 0.41 → 0.4099999…28dp) and the CLOB
        // rejects prices with more decimal places than the tick size (0.01).
        let price_dec = price.to_string().parse::<Decimal>()
            .or_else(|_| format!("{price:.2}").parse::<Decimal>())
            .map_err(|e| anyhow::anyhow!("Invalid price {price}: {e}"))?;
        // Round to 2dp explicitly so any float noise is stripped.
        let price_dec = price_dec.round_dp(2);
        let size_dec = Decimal::from_f64_retain(size)
            .ok_or_else(|| anyhow::anyhow!("Invalid size: {size}"))?;

        let code = self.builder_code();
        let order = client
            .limit_order()
            .token_id(token_id_u256)
            .price(price_dec)
            .size(size_dec)
            .side(side.to_sdk_side())
            .order_type(OrderType::GTC)
            .builder_code(code)
            .build()
            .await?;

        let signed = client.sign(&signer, order).await?;
        let resp = client.post_order(signed).await?;

        Ok(OrderResponse {
            order_id: resp.order_id,
            status: resp.status.to_string(),
        })
    }

    /// POST /order — FOK market order
    pub async fn create_market_order(
        &self,
        token_id: &str,
        side: Side,
        amount: f64,
        worst_price: f64,
    ) -> Result<OrderResponse> {
        let signer = self.make_signer()?;
        let client = self.sdk_client().await?;

        let token_id_u256 = U256::from_str(token_id)
            .map_err(|e| anyhow::anyhow!("Invalid token_id: {e}"))?;
        let amount_dec = Decimal::from_f64_retain(amount)
            .ok_or_else(|| anyhow::anyhow!("Invalid amount: {amount}"))?;
        let price_dec = Decimal::from_f64_retain(worst_price)
            .ok_or_else(|| anyhow::anyhow!("Invalid worst_price: {worst_price}"))?;

        let sdk_amount = match side {
            Side::Buy => Amount::usdc(amount_dec)?,
            Side::Sell => Amount::shares(amount_dec)?,
        };

        let mut builder = client
            .market_order()
            .token_id(token_id_u256)
            .amount(sdk_amount)
            .side(side.to_sdk_side());

        // Only set an explicit worst-price when it is inside the valid Polymarket
        // share-price range [tick_size, 1 - tick_size].  Callers that pass 0.0 or
        // 1.0 (or a raw crypto price like 78031) are asking for a true market
        // order, so we let the SDK calculate the price from the orderbook.
        if worst_price >= 0.01 && worst_price <= 0.99 {
            builder = builder.price(price_dec);
        }

        let code = self.builder_code();
        builder = builder.builder_code(code);

        let order = builder.build().await?;

        let signed = client.sign(&signer, order).await?;
        let resp = client.post_order(signed).await?;

        Ok(OrderResponse {
            order_id: resp.order_id,
            status: resp.status.to_string(),
        })
    }

    /// DELETE /orders/<order_id>
    pub async fn cancel_order(&self, order_id: &str) -> Result<()> {
        let client = self.sdk_client().await?;
        client.cancel_order(order_id).await?;
        Ok(())
    }

    /// GET /orders?owner=<wallet_address>&status=LIVE
    pub async fn get_open_orders(&self) -> Result<Vec<Order>> {
        let client = self.sdk_client().await?;
        let request = OrdersRequest::default();
        let page = client.orders(&request, None).await?;

        let orders: Vec<Order> = page.data
            .into_iter()
            .filter(|o| matches!(o.status, polymarket_client_sdk_v2::clob::types::OrderStatusType::Live))
            .map(|o| Order {
                id: o.id,
                token_id: o.asset_id.to_string(),
                side: o.side.to_string(),
                price: o.price.to_string(),
                size: o.original_size.to_string(),
                status: o.status.to_string(),
            })
            .collect();

        Ok(orders)
    }

    /// GET /balance — reads USDC.e balance directly from Polygon RPC.
    /// When a proxy address is configured, reads the proxy balance (the funder).
    pub async fn get_balance(&self) -> Result<f64> {
        let target = self.creds.proxy_address.as_deref()
            .unwrap_or(&self.creds.wallet_address);
        let rpc_bal = fetch_polygon_usdc_balance(target).await;
        match &rpc_bal {
            Ok(b) => tracing::info!(
                "Polygon RPC USDC.e balance for {}: ${:.2}",
                target, b
            ),
            Err(e) => tracing::warn!("Polygon RPC balance fetch failed: {}", e),
        }
        rpc_bal
    }

    /// GET /balance-allowance — queries the CLOB API for the balance it sees
    /// for the authenticated user (takes signature_type into account).
    pub async fn get_api_balance(&self) -> Result<f64> {
        let client = self.sdk_client().await?;
        let request = BalanceAllowanceRequest::builder()
            .asset_type(AssetType::Collateral)
            .build();
        let resp = client.balance_allowance(request).await?;
        tracing::info!("CLOB API balance: {} | allowances: {:?}", resp.balance, resp.allowances);
        let bal: f64 = resp.balance.try_into().unwrap_or(0.0) / 1_000_000.0;
        Ok(bal)
    }

    /// POST /heartbeat — keep session alive (call every 5s while orders open)
    pub async fn heartbeat(&self) -> Result<()> {
        let client = self.sdk_client().await?;
        client.post_heartbeat(None).await?;
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

// ── Polygon RPC balance fetcher ─────────────────────────────────────────────

/// USDC.e contract on Polygon (Bridged USDC)
const USDC_E_CONTRACT: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";
/// pUSD contract on Polygon (Polymarket wrapped USDC)
const PUSD_CONTRACT: &str = "0xc011a7e12a19f7b1f670d46f03b03f3342e82dfb";

/// Public Polygon RPC endpoints used as a fallback chain.
/// `polygon-rpc.com` requires an API key for anonymous traffic and rejects calls,
/// so it is intentionally not the first choice.
const POLYGON_RPCS: &[&str] = &[
    "https://polygon.drpc.org",
    "https://1rpc.io/matic",
    "https://polygon-bor-rpc.publicnode.com",
    "https://polygon.llamarpc.com",
];

async fn fetch_polygon_token_balance(client: &reqwest::Client, token: &str, wallet_address: &str) -> Result<f64> {
    let addr_clean = wallet_address.strip_prefix("0x").unwrap_or(wallet_address);
    let data = format!("0x70a08231{:0>64}", addr_clean.to_lowercase());

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_call",
        "params": [{ "to": token, "data": data }, "latest"],
        "id": 1
    });

    let mut last_err: Option<anyhow::Error> = None;
    for rpc in POLYGON_RPCS {
        let resp = match client
            .post(*rpc)
            .timeout(std::time::Duration::from_secs(8))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => { last_err = Some(anyhow::anyhow!("{} → {}", rpc, e)); continue; }
        };
        if !resp.status().is_success() {
            last_err = Some(anyhow::anyhow!("{} → HTTP {}", rpc, resp.status()));
            continue;
        }
        let resp_json: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => { last_err = Some(anyhow::anyhow!("{} → {}", rpc, e)); continue; }
        };
        let Some(hex_result) = resp_json.get("result").and_then(|v| v.as_str()) else {
            last_err = Some(anyhow::anyhow!("{} → no result ({})", rpc, resp_json));
            continue;
        };
        let raw = u128::from_str_radix(hex_result.strip_prefix("0x").unwrap_or(hex_result), 16)
            .map_err(|e| anyhow::anyhow!("Failed to parse Polygon balance hex for {}: {}", token, e))?;
        return Ok(raw as f64 / 1_000_000.0);
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("all Polygon RPCs failed for {}", token)))
}

/// Sum USDC.e + pUSD balances for the wallet. Polymarket migrated from USDC.e
/// to pUSD; users may hold either or both.
async fn fetch_polygon_usdc_balance(wallet_address: &str) -> Result<f64> {
    let client = reqwest::Client::new();
    let mut total = 0.0;

    match fetch_polygon_token_balance(&client, USDC_E_CONTRACT, wallet_address).await {
        Ok(b) => {
            tracing::info!("Polygon USDC.e balance: ${:.2}", b);
            total += b;
        }
        Err(e) => tracing::warn!("Failed to fetch USDC.e balance: {}", e),
    }

    match fetch_polygon_token_balance(&client, PUSD_CONTRACT, wallet_address).await {
        Ok(b) => {
            tracing::info!("Polygon pUSD balance: ${:.2}", b);
            total += b;
        }
        Err(e) => tracing::warn!("Failed to fetch pUSD balance: {}", e),
    }

    Ok(total)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::PolyCredentials;

    fn mock_creds() -> PolyCredentials {
        PolyCredentials {
            api_key: "d5f666f5-ac31-81cc-95ee-c1b0bb7945ca".to_string(),
            secret: "test-secret".to_string(),
            passphrase: "test-pass".to_string(),
            wallet_address: "0x1234567890123456789012345678901234567890".to_string(),
            private_key: Some(crate::auth::ANVIL_TEST_KEY.to_string()),
            is_builder: false,
            proxy_address: None,
            signature_type: None,
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

    #[test]
    fn test_sdk_credentials_parse() {
        let creds = mock_creds();
        let client = ClobClient::new(creds);
        let sdk_creds = client.sdk_credentials().expect("parse creds");
        assert_eq!(sdk_creds.key().to_string(), "d5f666f5-ac31-81cc-95ee-c1b0bb7945ca");
    }

    #[tokio::test]
    #[ignore]
    async fn test_network_create_limit_order() {
        let creds = mock_creds();
        let client = ClobClient::new(creds);
        let resp = client
            .create_limit_order("15871154585880608648532107628464183779895785213830018178010423617714102767076", Side::Buy, 0.5, 10.0)
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
        let _ = orders;
    }

    #[tokio::test]
    async fn test_heartbeat_starts_and_stops() {
        let creds = mock_creds();
        let client = std::sync::Arc::new(ClobClient::new(creds));
        let token = client.start_heartbeat();
        token.cancel();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
