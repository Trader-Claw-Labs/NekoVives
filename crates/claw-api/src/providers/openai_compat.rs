use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{jittered_backoff, ApiError};
use crate::types::{
    ContentBlockDelta, ContentBlockDeltaEvent, ContentBlockStartEvent, ContentBlockStopEvent,
    InputContentBlock, InputMessage, MessageDelta, MessageDeltaEvent, MessageRequest,
    MessageResponse, MessageStartEvent, MessageStopEvent, OutputContentBlock, StreamEvent,
    ToolChoice, ToolDefinition, ToolResultContentBlock, Usage,
};

pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct OpenAiCompatClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    provider_name: String,
    max_retries: u32,
}

impl OpenAiCompatClient {
    #[must_use]
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        provider_name: impl Into<String>,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: api_key.into(),
            base_url: base_url.into(),
            provider_name: provider_name.into(),
            max_retries: DEFAULT_MAX_RETRIES,
        }
    }

    pub async fn stream_message(
        &self,
        request: &MessageRequest,
    ) -> Result<MessageStream, ApiError> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let payload = build_chat_completion_request(request);

        for attempt in 0..=self.max_retries {
            let resp = self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("content-type", "application/json")
                .json(&payload)
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
                let provider = self.provider_name.clone();
                return Ok(MessageStream {
                    inner: resp.bytes_stream().boxed(),
                    state: StreamState::new(model.clone()),
                    queue: VecDeque::new(),
                    request_id,
                    done: false,
                    provider,
                    model,
                    buffer: Vec::new(),
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
    state: StreamState,
    queue: VecDeque<StreamEvent>,
    request_id: Option<String>,
    done: bool,
    #[allow(dead_code)]
    provider: String,
    #[allow(dead_code)]
    model: String,
    /// Byte buffer for incomplete SSE lines
    buffer: Vec<u8>,
}

impl std::fmt::Debug for MessageStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatMessageStream")
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
                // Flush finish events
                let events = self.state.finish()?;
                if !events.is_empty() {
                    self.queue.extend(events);
                    continue;
                }
                return Ok(None);
            }
            match self.inner.next().await {
                Some(Ok(chunk)) => {
                    self.buffer.extend_from_slice(&chunk);
                    // Process complete lines
                    while let Some(pos) = self.buffer.iter().position(|&b| b == b'\n') {
                        let line = self.buffer.drain(..=pos).collect::<Vec<_>>();
                        let line = String::from_utf8_lossy(&line).trim_end().to_string();
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                self.done = true;
                                break;
                            }
                            match serde_json::from_str::<ChatCompletionChunk>(data) {
                                Ok(chunk) => {
                                    let events = self.state.ingest_chunk(chunk)?;
                                    self.queue.extend(events);
                                }
                                Err(e) => {
                                    // Skip unknown chunks silently
                                    let _ = e;
                                }
                            }
                        }
                    }
                }
                Some(Err(e)) => return Err(ApiError::Http(e)),
                None => {
                    self.done = true;
                }
            }
        }
    }
}

// ── OpenAI streaming response types ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ChatCompletionChunk {
    id: String,
    #[serde(default)]
    model: Option<String>,
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<ChunkUsage>,
}

#[derive(Debug, Deserialize)]
struct ChunkChoice {
    delta: ChunkDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ChunkDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<DeltaToolCall>,
}

#[derive(Debug, Deserialize)]
struct DeltaToolCall {
    index: u32,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: DeltaFunction,
}

#[derive(Debug, Deserialize, Default)]
struct DeltaFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChunkUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

// ── Stream state machine (translates OpenAI events → StreamEvent) ────────────

#[derive(Debug)]
struct StreamState {
    model: String,
    message_started: bool,
    text_started: bool,
    text_finished: bool,
    finished: bool,
    stop_reason: Option<String>,
    usage: Option<Usage>,
    tool_calls: BTreeMap<u32, ToolCallState>,
}

impl StreamState {
    fn new(model: String) -> Self {
        Self {
            model,
            message_started: false,
            text_started: false,
            text_finished: false,
            finished: false,
            stop_reason: None,
            usage: None,
            tool_calls: BTreeMap::new(),
        }
    }

    fn ingest_chunk(&mut self, chunk: ChatCompletionChunk) -> Result<Vec<StreamEvent>, ApiError> {
        let mut events = Vec::new();

        if !self.message_started {
            self.message_started = true;
            events.push(StreamEvent::MessageStart(MessageStartEvent {
                message: MessageResponse {
                    id: chunk.id.clone(),
                    kind: "message".to_string(),
                    role: "assistant".to_string(),
                    content: Vec::new(),
                    model: chunk.model.clone().unwrap_or_else(|| self.model.clone()),
                    stop_reason: None,
                    stop_sequence: None,
                    usage: Usage::default(),
                    request_id: None,
                },
            }));
        }

        if let Some(usage) = chunk.usage {
            self.usage = Some(Usage {
                input_tokens: usage.prompt_tokens,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                output_tokens: usage.completion_tokens,
            });
        }

        for choice in chunk.choices {
            if let Some(content) = choice.delta.content.filter(|v| !v.is_empty()) {
                if !self.text_started {
                    self.text_started = true;
                    events.push(StreamEvent::ContentBlockStart(ContentBlockStartEvent {
                        index: 0,
                        content_block: OutputContentBlock::Text {
                            text: String::new(),
                        },
                    }));
                }
                events.push(StreamEvent::ContentBlockDelta(ContentBlockDeltaEvent {
                    index: 0,
                    delta: ContentBlockDelta::TextDelta { text: content },
                }));
            }

            for tool_call in choice.delta.tool_calls {
                let state = self.tool_calls.entry(tool_call.index).or_default();
                state.apply(tool_call);
                let block_index = state.block_index();

                if !state.started {
                    if let Some(start_event) = state.start_event() {
                        state.started = true;
                        events.push(StreamEvent::ContentBlockStart(start_event));
                    } else {
                        continue;
                    }
                }
                if let Some(delta_event) = state.delta_event() {
                    events.push(StreamEvent::ContentBlockDelta(delta_event));
                }
                if choice.finish_reason.as_deref() == Some("tool_calls") && !state.stopped {
                    state.stopped = true;
                    events.push(StreamEvent::ContentBlockStop(ContentBlockStopEvent {
                        index: block_index,
                    }));
                }
            }

            if let Some(finish_reason) = choice.finish_reason {
                self.stop_reason = Some(normalize_finish_reason(&finish_reason));
                if finish_reason == "tool_calls" {
                    for state in self.tool_calls.values_mut() {
                        if state.started && !state.stopped {
                            state.stopped = true;
                            events.push(StreamEvent::ContentBlockStop(ContentBlockStopEvent {
                                index: state.block_index(),
                            }));
                        }
                    }
                }
            }
        }

        Ok(events)
    }

    fn finish(&mut self) -> Result<Vec<StreamEvent>, ApiError> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;

        let mut events = Vec::new();

        if self.text_started && !self.text_finished {
            self.text_finished = true;
            events.push(StreamEvent::ContentBlockStop(ContentBlockStopEvent { index: 0 }));
        }

        for state in self.tool_calls.values_mut() {
            if !state.started {
                if let Some(start_event) = state.start_event() {
                    state.started = true;
                    events.push(StreamEvent::ContentBlockStart(start_event));
                    if let Some(delta_event) = state.delta_event() {
                        events.push(StreamEvent::ContentBlockDelta(delta_event));
                    }
                }
            }
            if state.started && !state.stopped {
                state.stopped = true;
                events.push(StreamEvent::ContentBlockStop(ContentBlockStopEvent {
                    index: state.block_index(),
                }));
            }
        }

        if self.message_started {
            events.push(StreamEvent::MessageDelta(MessageDeltaEvent {
                delta: MessageDelta {
                    stop_reason: Some(
                        self.stop_reason
                            .clone()
                            .unwrap_or_else(|| "end_turn".to_string()),
                    ),
                    stop_sequence: None,
                },
                usage: self.usage.clone().unwrap_or_default(),
            }));
            events.push(StreamEvent::MessageStop(MessageStopEvent {}));
        }

        Ok(events)
    }
}

#[derive(Debug, Default)]
struct ToolCallState {
    openai_index: u32,
    id: Option<String>,
    name: Option<String>,
    arguments: String,
    emitted_len: usize,
    started: bool,
    stopped: bool,
}

impl ToolCallState {
    fn apply(&mut self, tool_call: DeltaToolCall) {
        self.openai_index = tool_call.index;
        if let Some(id) = tool_call.id {
            self.id = Some(id);
        }
        if let Some(name) = tool_call.function.name {
            self.name = Some(name);
        }
        if let Some(args) = tool_call.function.arguments {
            self.arguments.push_str(&args);
        }
    }

    const fn block_index(&self) -> u32 {
        self.openai_index + 1
    }

    fn start_event(&self) -> Option<ContentBlockStartEvent> {
        let name = self.name.clone()?;
        let id = self
            .id
            .clone()
            .unwrap_or_else(|| format!("tool_call_{}", self.openai_index));
        Some(ContentBlockStartEvent {
            index: self.block_index(),
            content_block: OutputContentBlock::ToolUse {
                id,
                name,
                input: json!({}),
            },
        })
    }

    fn delta_event(&mut self) -> Option<ContentBlockDeltaEvent> {
        if self.emitted_len >= self.arguments.len() {
            return None;
        }
        let delta = self.arguments[self.emitted_len..].to_string();
        self.emitted_len = self.arguments.len();
        Some(ContentBlockDeltaEvent {
            index: self.block_index(),
            delta: ContentBlockDelta::InputJsonDelta { partial_json: delta },
        })
    }
}

// ── Request building: InputMessage → OpenAI format ───────────────────────────

fn build_chat_completion_request(request: &MessageRequest) -> Value {
    let mut messages = Vec::new();

    if let Some(system) = request.system.as_ref().filter(|s| !s.is_empty()) {
        messages.push(json!({ "role": "system", "content": system }));
    }

    for message in &request.messages {
        messages.extend(translate_message(message));
    }

    let mut payload = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "messages": messages,
        "stream": request.stream,
        "stream_options": { "include_usage": true },
    });

    if let Some(tools) = &request.tools {
        payload["tools"] = Value::Array(tools.iter().map(openai_tool_definition).collect());
    }
    if let Some(tool_choice) = &request.tool_choice {
        payload["tool_choice"] = openai_tool_choice(tool_choice);
    }
    if let Some(temperature) = request.temperature {
        payload["temperature"] = json!(temperature);
    }
    if let Some(top_p) = request.top_p {
        payload["top_p"] = json!(top_p);
    }

    payload
}

fn translate_message(message: &InputMessage) -> Vec<Value> {
    match message.role.as_str() {
        "assistant" => {
            let mut text = String::new();
            let mut tool_calls: Vec<Value> = Vec::new();

            for block in &message.content {
                match block {
                    InputContentBlock::Text { text: value } => text.push_str(value),
                    InputContentBlock::ToolUse { id, name, input } => tool_calls.push(json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": input.to_string(),
                        }
                    })),
                    InputContentBlock::ToolResult { .. } => {}
                }
            }

            if text.is_empty() && tool_calls.is_empty() {
                Vec::new()
            } else {
                vec![json!({
                    "role": "assistant",
                    "content": if text.is_empty() { Value::Null } else { json!(text) },
                    "tool_calls": if tool_calls.is_empty() { Value::Null } else { json!(tool_calls) },
                })]
            }
        }
        _ => message
            .content
            .iter()
            .filter_map(|block| match block {
                InputContentBlock::Text { text } => Some(json!({
                    "role": "user",
                    "content": text,
                })),
                InputContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error: _,
                } => Some(json!({
                    "role": "tool",
                    "tool_call_id": tool_use_id,
                    "content": flatten_tool_result(content),
                })),
                InputContentBlock::ToolUse { .. } => None,
            })
            .collect(),
    }
}

fn flatten_tool_result(content: &[ToolResultContentBlock]) -> String {
    content
        .iter()
        .map(|b| match b {
            ToolResultContentBlock::Text { text } => text.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn openai_tool_definition(tool: &ToolDefinition) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.input_schema,
        }
    })
}

fn openai_tool_choice(tool_choice: &ToolChoice) -> Value {
    match tool_choice {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::Any => json!("required"),
        ToolChoice::Tool { name } => json!({
            "type": "function",
            "function": { "name": name },
        }),
    }
}

fn normalize_finish_reason(reason: &str) -> String {
    match reason {
        "tool_calls" => "tool_use".to_string(),
        "stop" => "end_turn".to_string(),
        other => other.to_string(),
    }
}

fn parse_error_body(body: &str) -> (Option<String>, Option<String>) {
    #[derive(Deserialize)]
    struct Wrapper {
        error: Option<ErrBody>,
    }
    #[derive(Deserialize)]
    struct ErrBody {
        #[serde(rename = "type")]
        kind: Option<String>,
        message: Option<String>,
    }
    if let Ok(w) = serde_json::from_str::<Wrapper>(body) {
        if let Some(e) = w.error {
            return (e.kind, e.message);
        }
    }
    (None, None)
}
