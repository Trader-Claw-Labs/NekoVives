use serde::{Deserialize, Serialize};

/// Side of an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

/// Type of an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    Limit,
    Market,
}

/// An order request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub coin: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub size: f64,
    pub price: Option<f64>,
    pub reduce_only: bool,
}

impl Order {
    pub fn limit_buy(coin: impl Into<String>, size: f64, price: f64) -> Self {
        Self {
            coin: coin.into(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            size,
            price: Some(price),
            reduce_only: false,
        }
    }

    pub fn limit_sell(coin: impl Into<String>, size: f64, price: f64) -> Self {
        Self {
            coin: coin.into(),
            side: OrderSide::Sell,
            order_type: OrderType::Limit,
            size,
            price: Some(price),
            reduce_only: false,
        }
    }

    pub fn market_buy(coin: impl Into<String>, size: f64) -> Self {
        Self {
            coin: coin.into(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            size,
            price: None,
            reduce_only: false,
        }
    }

    pub fn market_sell(coin: impl Into<String>, size: f64) -> Self {
        Self {
            coin: coin.into(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            size,
            price: None,
            reduce_only: false,
        }
    }
}

/// Response from placing an order.
#[derive(Debug, Clone, Deserialize)]
pub struct OrderResponse {
    pub status: String,
    #[serde(rename = "oid")]
    pub order_id: Option<u64>,
    #[serde(rename = "restingOid")]
    pub resting_oid: Option<u64>,
}

impl OrderResponse {
    pub fn order_id(&self) -> Option<u64> {
        self.order_id.or(self.resting_oid)
    }
}

/// A position on Hyperliquid.
#[derive(Debug, Clone, Deserialize)]
pub struct Position {
    pub coin: String,
    pub entry_px: Option<f64>,
    pub szi: f64,
    pub leverage: f64,
    pub position_value: f64,
    pub unrealized_pnl: f64,
    pub margin_used: f64,
}

/// Funding rate for a coin.
#[derive(Debug, Clone, Deserialize)]
pub struct FundingRate {
    pub coin: String,
    pub funding_rate: f64,
    pub next_funding_time: i64,
}

/// L2 book entry.
#[derive(Debug, Clone, Deserialize)]
pub struct L2BookEntry {
    pub px: String,
    pub sz: String,
    pub n: u64,
}

/// L2 orderbook.
#[derive(Debug, Clone, Deserialize)]
pub struct L2Book {
    pub coin: String,
    pub levels: Vec<Vec<L2BookEntry>>,
}

impl L2Book {
    pub fn bids(&self) -> &[L2BookEntry] {
        self.levels.first().map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn asks(&self) -> &[L2BookEntry] {
        self.levels.get(1).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn mid(&self) -> Option<f64> {
        let best_bid = self.bids().first()?.px.parse::<f64>().ok()?;
        let best_ask = self.asks().first()?.px.parse::<f64>().ok()?;
        Some((best_bid + best_ask) / 2.0)
    }
}

/// Clearinghouse state for an address.
#[derive(Debug, Clone, Deserialize)]
pub struct ClearinghouseState {
    pub margin_summary: MarginSummary,
    pub cross_margin_summary: MarginSummary,
    pub cross_maintenance_margin_used: String,
    pub withdrawable: String,
    pub asset_positions: Vec<AssetPosition>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarginSummary {
    pub account_value: String,
    pub total_ntl_pos: String,
    pub total_raw_usd: String,
    pub total_margin_used: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetPosition {
    pub position: Position,
    pub type_field: String,
}

/// A user fill event from WebSocket.
#[derive(Debug, Clone, Deserialize)]
pub struct UserFill {
    pub coin: String,
    pub side: String,
    pub px: String,
    pub sz: String,
    pub oid: u64,
    pub tid: u64,
    pub time: i64,
}

/// WebSocket funding update.
#[derive(Debug, Clone, Deserialize)]
pub struct WsFundingUpdate {
    pub coin: String,
    pub funding_rate: f64,
    pub predicted_funding_rate: Option<f64>,
}
