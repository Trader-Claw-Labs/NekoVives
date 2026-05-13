//! Main client that combines info, exchange, and WebSocket APIs.

use crate::error::Result;
use crate::exchange::{ExchangeClient, Signer};
use crate::info::InfoClient;
use crate::types::{ClearinghouseState, FundingRate, L2Book, Order, OrderResponse};
use crate::ws::WsClient;
use crate::ws::{FundingStream, L2BookStream, UserEventStream};
use std::collections::HashMap;

/// Unified Hyperliquid client.
#[derive(Debug, Clone)]
pub struct HyperliquidClient {
    info: InfoClient,
    exchange: Option<ExchangeClient>,
    ws_url: String,
}

impl HyperliquidClient {
    /// Create a read-only client (no signing) for mainnet.
    pub fn new_mainnet() -> Self {
        Self {
            info: InfoClient::new_mainnet(),
            exchange: None,
            ws_url: "wss://api.hyperliquid.xyz/ws".to_string(),
        }
    }

    /// Create a read-only client (no signing) for testnet.
    pub fn new_testnet() -> Self {
        Self {
            info: InfoClient::new_testnet(),
            exchange: None,
            ws_url: "wss://api.hyperliquid-testnet.xyz/ws".to_string(),
        }
    }

    /// Create a signed client for mainnet.
    pub fn new_mainnet_with_signer(signer: Signer) -> Self {
        Self {
            info: InfoClient::new_mainnet(),
            exchange: Some(ExchangeClient::new_mainnet(signer)),
            ws_url: "wss://api.hyperliquid.xyz/ws".to_string(),
        }
    }

    /// Create a signed client for testnet.
    pub fn new_testnet_with_signer(signer: Signer) -> Self {
        Self {
            info: InfoClient::new_testnet(),
            exchange: Some(ExchangeClient::new_testnet(signer)),
            ws_url: "wss://api.hyperliquid-testnet.xyz/ws".to_string(),
        }
    }

    /// Build with custom URLs (e.g. private endpoint).
    pub fn with_urls(
        info_url: impl Into<String>,
        exchange_url: impl Into<String>,
        ws_url: impl Into<String>,
        signer: Option<Signer>,
    ) -> Self {
        Self {
            info: InfoClient::with_url(info_url),
            exchange: signer.map(|s| ExchangeClient::with_url(exchange_url, s)),
            ws_url: ws_url.into(),
        }
    }

    // ── Info (read-only) ───────────────────────────────────────────────

    pub async fn mids(&self) -> Result<HashMap<String, f64>> {
        self.info.mids().await
    }

    pub async fn l2_book(&self, coin: &str) -> Result<L2Book> {
        self.info.l2_book(coin).await
    }

    pub async fn funding_rate(&self, coin: &str) -> Result<FundingRate> {
        self.info.funding_rate(coin).await
    }

    pub async fn predicted_funding(&self) -> Result<HashMap<String, f64>> {
        self.info.predicted_funding().await
    }

    pub async fn clearinghouse_state(&self, address: &str) -> Result<ClearinghouseState> {
        self.info.clearinghouse_state(address).await
    }

    // ── Exchange (signed) ──────────────────────────────────────────────

    fn exchange(&self) -> Result<&ExchangeClient> {
        self.exchange.as_ref().ok_or_else(|| {
            crate::error::HyperliquidError::Other(
                "client created without signer — use new_mainnet_with_signer()".to_string(),
            )
        })
    }

    pub async fn place_order(&self, order: &Order) -> Result<OrderResponse> {
        self.exchange()?.place_order(order).await
    }

    pub async fn cancel_order(&self, coin: &str, oid: u64) -> Result<()> {
        self.exchange()?.cancel_order(coin, oid).await
    }

    pub async fn cancel_all(&self, coin: Option<&str>) -> Result<()> {
        self.exchange()?.cancel_all(coin).await
    }

    pub async fn close_position(&self, coin: &str) -> Result<OrderResponse> {
        self.exchange()?.close_position(coin).await
    }

    pub fn signer_address(&self) -> Option<&str> {
        self.exchange.as_ref().map(|e| e.signer_address())
    }

    // ── WebSocket ──────────────────────────────────────────────────────

    pub fn subscribe_l2_book(&self, coin: &str) -> Result<L2BookStream> {
        let mut ws = WsClient::with_url(&self.ws_url);
        ws.subscribe_l2_book(coin);
        let (l2_rx, _, _) = ws.start()?;
        Ok(L2BookStream { rx: l2_rx })
    }

    pub fn subscribe_user_events(&self, address: &str) -> Result<UserEventStream> {
        let mut ws = WsClient::with_url(&self.ws_url);
        ws.subscribe_user_events(address);
        let (_, fill_rx, _) = ws.start()?;
        Ok(UserEventStream { rx: fill_rx })
    }

    pub fn subscribe_funding_updates(&self) -> Result<FundingStream> {
        let mut ws = WsClient::with_url(&self.ws_url);
        ws.subscribe_funding_updates();
        let (_, _, fund_rx) = ws.start()?;
        Ok(FundingStream { rx: fund_rx })
    }
}
