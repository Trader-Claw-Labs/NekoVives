use std::time::Duration;

use futures_util::StreamExt;
use serde::Deserialize;

use crate::error::{jittered_backoff, ApiError};
use crate::sse::SseParser;
use crate::types::{MessageRequest, MessageResponse, StreamEvent};

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct AnthropicClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    max_retries: u32,
}

impl AnthropicClient {
    #[must_use]
    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: api_key.into(),
            base_url: base_url.into(),
            max_retries: DEFAULT_MAX_RETRIES,
        }
    }

    pub async fn send_message(
        &self,
        request: &MessageRequest,
    ) -> Result<MessageResponse, ApiError> {
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let mut req = request.clone();
        req.stream = false;

        for attempt in 0..=self.max_retries {
            let resp = self
                .http
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&req)
                .send()
                .await
                .map_err(ApiError::Http)?;

            let status = resp.status();
            if status.is_success() {
                let body = resp.text().await.map_err(ApiError::Http)?;
                return serde_json::from_str::<MessageResponse>(&body).map_err(|e| {
                    ApiError::json_deserialize("anthropic", &request.model, &body, e)
                });
            }

            let body = resp.text().await.unwrap_or_default();
            let (error_type, message) = parse_error_body(&body);
            let retryable = status.as_u16() == 429 || status.as_u16() >= 500;

            if retryable && attempt < self.max_retries {
                let delay = jittered_backoff(attempt, INITIAL_BACKOFF, MAX_BACKOFF);
                tokio::time::sleep(delay).await;
                continue;
            }

            return Err(ApiError::Api {
                status,
                error_type,
                message,
                request_id: None,
                body,
                retryable,
            });
        }

        unreachable!()
    }

    pub async fn stream_message(
        &self,
        request: &MessageRequest,
    ) -> Result<MessageStream, ApiError> {
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let mut req = request.clone();
        req.stream = true;

        for attempt in 0..=self.max_retries {
            let resp = self
                .http
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&req)
                .send()
                .await
                .map_err(ApiError::Http)?;

            let status = resp.status();
            if status.is_success() {
                let request_id = resp
                    .headers()
                    .get("request-id")
                    .or_else(|| resp.headers().get("x-request-id"))
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                let model = request.model.clone();
                return Ok(MessageStream {
                    inner: resp.bytes_stream().boxed(),
                    parser: SseParser::new().with_context("anthropic", &model),
                    queue: std::collections::VecDeque::new(),
                    request_id,
                    done: false,
                });
            }

            let body = resp.text().await.unwrap_or_default();
            let (error_type, message) = parse_error_body(&body);
            let retryable = status.as_u16() == 429 || status.as_u16() >= 500;

            if retryable && attempt < self.max_retries {
                let delay = jittered_backoff(attempt, INITIAL_BACKOFF, MAX_BACKOFF);
                tokio::time::sleep(delay).await;
                continue;
            }

            return Err(ApiError::Api {
                status,
                error_type,
                message,
                request_id: None,
                body,
                retryable,
            });
        }

        unreachable!()
    }
}

pub struct MessageStream {
    inner: futures_util::stream::BoxStream<'static, Result<bytes::Bytes, reqwest::Error>>,
    parser: SseParser,
    queue: std::collections::VecDeque<StreamEvent>,
    request_id: Option<String>,
    done: bool,
}

impl std::fmt::Debug for MessageStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicMessageStream")
            .field("done", &self.done)
            .finish()
    }
}

impl MessageStream {
    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    pub async fn next_event(&mut self) -> Result<Option<StreamEvent>, ApiError> {
        loop {
            if let Some(event) = self.queue.pop_front() {
                return Ok(Some(event));
            }
            if self.done {
                return Ok(None);
            }
            match self.inner.next().await {
                Some(Ok(chunk)) => {
                    let events = self.parser.push(&chunk)?;
                    self.queue.extend(events);
                }
                Some(Err(e)) => return Err(ApiError::Http(e)),
                None => {
                    self.done = true;
                    let events = self.parser.finish()?;
                    self.queue.extend(events);
                }
            }
        }
    }
}

fn parse_error_body(body: &str) -> (Option<String>, Option<String>) {
    #[derive(Deserialize)]
    struct Wrapper {
        #[serde(rename = "type")]
        kind: Option<String>,
        error: Option<ErrorBody>,
        message: Option<String>,
    }
    #[derive(Deserialize)]
    struct ErrorBody {
        #[serde(rename = "type")]
        kind: Option<String>,
        message: Option<String>,
    }
    if let Ok(w) = serde_json::from_str::<Wrapper>(body) {
        let msg = w
            .error
            .as_ref()
            .and_then(|e| e.message.clone())
            .or(w.message);
        let kind = w.error.and_then(|e| e.kind).or(w.kind);
        return (kind, msg);
    }
    (None, None)
}
