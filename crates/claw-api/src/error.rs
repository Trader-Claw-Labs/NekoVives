use std::fmt::{Display, Formatter};
use std::time::Duration;

#[derive(Debug)]
pub enum ApiError {
    MissingCredentials {
        provider: &'static str,
        env_vars: &'static [&'static str],
    },
    Http(reqwest::Error),
    Json {
        provider: String,
        model: String,
        body_snippet: String,
        source: serde_json::Error,
    },
    Api {
        status: reqwest::StatusCode,
        error_type: Option<String>,
        message: Option<String>,
        request_id: Option<String>,
        body: String,
        retryable: bool,
    },
    RetriesExhausted {
        attempts: u32,
        last_error: Box<ApiError>,
    },
    Other(String),
}

impl ApiError {
    #[must_use]
    pub fn json_deserialize(
        provider: impl Into<String>,
        model: impl Into<String>,
        body: &str,
        source: serde_json::Error,
    ) -> Self {
        let snippet = if body.len() > 200 { &body[..200] } else { body };
        Self::Json {
            provider: provider.into(),
            model: model.into(),
            body_snippet: snippet.to_string(),
            source,
        }
    }

    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Http(e) => e.is_connect() || e.is_timeout(),
            Self::Api { retryable, .. } => *retryable,
            Self::RetriesExhausted { last_error, .. } => last_error.is_retryable(),
            _ => false,
        }
    }

    #[must_use]
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::Api { status, .. } if status.as_u16() == 429)
    }
}

impl Display for ApiError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingCredentials { provider, env_vars } => write!(
                f,
                "Missing {provider} credentials. Set one of: {}",
                env_vars.join(", ")
            ),
            Self::Http(e) => write!(f, "HTTP error: {e}"),
            Self::Json { provider, model, body_snippet, source } => write!(
                f,
                "JSON parse error from {provider}/{model}: {source}. Body: {body_snippet}"
            ),
            Self::Api { status, message, body, .. } => {
                if let Some(msg) = message {
                    write!(f, "API error {status}: {msg}")
                } else {
                    write!(f, "API error {status}: {body}")
                }
            }
            Self::RetriesExhausted { attempts, last_error } => {
                write!(f, "Retries exhausted after {attempts} attempts: {last_error}")
            }
            Self::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ApiError {}

impl From<reqwest::Error> for ApiError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

/// Compute exponential backoff with jitter.
pub(crate) fn jittered_backoff(attempt: u32, initial: Duration, max: Duration) -> Duration {
    let base_ms = initial.as_millis() as f64 * (2_f64.powi(attempt as i32));
    let capped_ms = base_ms.min(max.as_millis() as f64);
    // Add ±25% jitter
    let jitter = capped_ms * 0.25 * (rand_f64() - 0.5) * 2.0;
    let total = (capped_ms + jitter).max(0.0) as u64;
    Duration::from_millis(total)
}

fn rand_f64() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos % 1000) as f64 / 1000.0
}
