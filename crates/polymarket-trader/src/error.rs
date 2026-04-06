use thiserror::Error;

#[derive(Error, Debug)]
pub enum PolyError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Auth error: {0}")]
    Auth(String),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
