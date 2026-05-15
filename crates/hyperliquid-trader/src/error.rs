use thiserror::Error;

#[derive(Debug, Error)]
pub enum HyperliquidError {
    #[error("rate limited")]
    RateLimited,

    #[error("insufficient margin")]
    InsufficientMargin,

    #[error("invalid symbol: {0}")]
    InvalidSymbol(String),

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("sdk error: {0}")]
    Sdk(String),

    #[error("websocket error: {0}")]
    WebSocket(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("other: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, HyperliquidError>;
