//! Hyperliquid trader crate.
//!
//! Provides a unified client for Hyperliquid HyperCore (perps + spot):
//! - Info API: mids, L2 book, funding rates, clearinghouse state.
//! - Exchange API: place/cancel orders, close positions (signed).
//! - WebSocket: L2 book, user fills, funding updates (auto-reconnect).
//!
//! Usage:
//! ```rust,no_run
//! use hyperliquid_trader::{HyperliquidClient, Signer};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let signer = Signer::from_pk("0x...")?;
//! let client = HyperliquidClient::new_mainnet_with_signer(signer);
//! let mids = client.mids().await?;
//! println!("BTC mid: {:?}", mids.get("BTC"));
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod error;
pub mod exchange;
pub mod info;
pub mod types;
pub mod ws;

pub use client::HyperliquidClient;
pub use error::{HyperliquidError, Result};
pub use exchange::Signer;
pub use info::InfoClient;
pub use types::{
    AssetPosition, ClearinghouseState, FundingRate, L2Book, L2BookEntry, MarginSummary, Order,
    OrderResponse, OrderSide, OrderType, Position, UserFill, WsFundingUpdate,
};
pub use ws::{FundingStream, L2BookStream, UserEventStream, WsClient};
