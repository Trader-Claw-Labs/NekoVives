use crate::error::ApiError;
use crate::providers::{
    anthropic::{AnthropicClient, DEFAULT_BASE_URL as ANTHROPIC_DEFAULT_URL},
    openai_compat::{OpenAiCompatClient, DEFAULT_BASE_URL as OPENAI_DEFAULT_URL},
    ProviderKind,
};
use crate::types::{MessageRequest, StreamEvent};

/// Trader-Claw provider configuration — built from Config at call time.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub api_key: String,
    pub base_url: String,
    pub provider_kind: ProviderKind,
    pub provider_name: String,
}

/// Unified client over all supported provider backends.
#[derive(Debug, Clone)]
pub enum ProviderClient {
    Anthropic(AnthropicClient),
    OpenAiCompat(OpenAiCompatClient),
}

impl ProviderClient {
    /// Build a client directly from a [`ProviderConfig`] (Trader-Claw's config-driven path).
    #[must_use]
    pub fn from_config(cfg: ProviderConfig) -> Self {
        match cfg.provider_kind {
            ProviderKind::Anthropic => Self::Anthropic(AnthropicClient::new(
                cfg.api_key,
                if cfg.base_url.is_empty() { ANTHROPIC_DEFAULT_URL.to_string() } else { cfg.base_url },
            )),
            ProviderKind::OpenAiCompat => Self::OpenAiCompat(OpenAiCompatClient::new(
                cfg.api_key,
                if cfg.base_url.is_empty() { OPENAI_DEFAULT_URL.to_string() } else { cfg.base_url },
                cfg.provider_name,
            )),
        }
    }

    pub async fn stream_message(
        &self,
        request: &MessageRequest,
    ) -> Result<MessageStream, ApiError> {
        match self {
            Self::Anthropic(c) => c.stream_message(request).await.map(MessageStream::Anthropic),
            Self::OpenAiCompat(c) => {
                c.stream_message(request).await.map(MessageStream::OpenAiCompat)
            }
        }
    }
}

#[derive(Debug)]
pub enum MessageStream {
    Anthropic(crate::providers::anthropic::MessageStream),
    OpenAiCompat(crate::providers::openai_compat::MessageStream),
}

impl MessageStream {
    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Anthropic(s) => s.request_id(),
            Self::OpenAiCompat(s) => s.request_id(),
        }
    }

    pub async fn next_event(&mut self) -> Result<Option<StreamEvent>, ApiError> {
        match self {
            Self::Anthropic(s) => s.next_event().await,
            Self::OpenAiCompat(s) => s.next_event().await,
        }
    }
}
