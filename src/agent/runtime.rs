//! Clean agentic conversation loop, modeled after ultraworkers/claw-code's
//! `ConversationRuntime`. Uses proper Anthropic-native `InputContentBlock`
//! types so tool-use blocks stream correctly and feed back reliably.

use std::collections::BTreeMap;

use anyhow::Result;
use claw_api::{
    ContentBlockDelta, InputContentBlock, InputMessage, MessageRequest, OutputContentBlock,
    ProviderClient, StreamEvent, ToolDefinition, ToolResultContentBlock,
};
use serde_json::Value;

const MAX_TOOL_ITERATIONS: usize = 20;

// ── Tool executor trait ───────────────────────────────────────────────────────

/// Implemented by the adapter layer to run actual tools.
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, name: &str, input: Value) -> Result<String, String>;
}

// ── Session (in-memory conversation history) ─────────────────────────────────

/// Per-WebSocket-connection conversation history.
/// Each WS connection owns one Session; messages accumulate across turns.
#[derive(Debug, Default, Clone)]
pub struct Session {
    pub history: Vec<InputMessage>,
}

impl Session {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Trim history when it grows too large (keep last N non-system messages).
    pub fn maybe_compact(&mut self, max_messages: usize) {
        if self.history.len() > max_messages {
            let keep = max_messages / 2;
            let drain_to = self.history.len().saturating_sub(keep);
            self.history.drain(..drain_to);
        }
    }
}

// ── ConversationRuntime ───────────────────────────────────────────────────────

pub struct ConversationRuntime {
    client: ProviderClient,
    model: String,
    max_tokens: u32,
    temperature: Option<f64>,
    system_prompt: String,
}

impl ConversationRuntime {
    #[must_use]
    pub fn new(
        client: ProviderClient,
        model: impl Into<String>,
        max_tokens: u32,
        temperature: Option<f64>,
        system_prompt: impl Into<String>,
    ) -> Self {
        Self {
            client,
            model: model.into(),
            max_tokens,
            temperature,
            system_prompt: system_prompt.into(),
        }
    }

    /// Execute one agent turn: stream from the model, dispatch any tool calls,
    /// feed results back, and repeat until the model stops requesting tools.
    ///
    /// `session` is updated in-place so history persists across calls.
    /// Events are sent to `ws_tx` for real-time UI feedback.
    pub async fn run_turn(
        &self,
        user_message: &str,
        tools: &[ToolDefinition],
        tool_executor: &dyn ToolExecutor,
        session: &mut Session,
        ws_tx: Option<&tokio::sync::mpsc::UnboundedSender<Value>>,
    ) -> Result<String> {
        // Append user turn
        session.history.push(InputMessage::user_text(user_message));
        // Keep history bounded
        session.maybe_compact(200);

        let mut final_text = String::new();
        let mut thinking_rounds = 0u32;

        for iteration in 0..MAX_TOOL_ITERATIONS {
            // Notify UI: new LLM round starting
            thinking_rounds += 1;
            if let Some(tx) = ws_tx {
                let _ = tx.send(serde_json::json!({
                    "type": "thinking",
                    "iteration": thinking_rounds,
                    "rounds": thinking_rounds,
                }));
            }

            let request = MessageRequest {
                model: self.model.clone(),
                max_tokens: self.max_tokens,
                messages: session.history.clone(),
                system: Some(self.system_prompt.clone()),
                tools: if tools.is_empty() { None } else { Some(tools.to_vec()) },
                stream: true,
                temperature: self.temperature,
                ..Default::default()
            };

            // Stream and accumulate one assistant turn
            let (assistant_blocks, tool_calls) =
                self.stream_turn(&request, &mut final_text, ws_tx).await?;

            // Append assistant message to history
            if !assistant_blocks.is_empty() {
                session.history.push(InputMessage {
                    role: "assistant".to_string(),
                    content: assistant_blocks,
                });
            }

            // If no tools were requested, we're done
            if tool_calls.is_empty() {
                break;
            }

            // Execute each tool and collect results as a single user turn
            let mut result_blocks: Vec<InputContentBlock> = Vec::new();
            for (id, name, input) in &tool_calls {
                // Notify UI: tool starting
                if let Some(tx) = ws_tx {
                    let _ = tx.send(serde_json::json!({
                        "type": "tool_call",
                        "name": name,
                        "args": input,
                    }));
                }

                let (output, is_error) = match tool_executor.execute(name, input.clone()).await {
                    Ok(result) => (result, false),
                    Err(e) => (format!("Error: {e}"), true),
                };

                // Notify UI: tool done
                if let Some(tx) = ws_tx {
                    let _ = tx.send(serde_json::json!({
                        "type": "tool_result",
                        "name": name,
                        "success": !is_error,
                    }));
                }

                result_blocks.push(InputContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: vec![ToolResultContentBlock::Text { text: output }],
                    is_error,
                });
            }

            // Add tool results as a user message
            if !result_blocks.is_empty() {
                session.history.push(InputMessage {
                    role: "user".to_string(),
                    content: result_blocks,
                });
            }

            // Safety: prevent runaway loops
            if iteration + 1 >= MAX_TOOL_ITERATIONS {
                tracing::warn!(
                    iterations = MAX_TOOL_ITERATIONS,
                    "Agent hit max tool iterations — stopping"
                );
                break;
            }
        }

        Ok(final_text)
    }

    /// Stream a single model response, returning:
    /// - `assistant_blocks`: the accumulated `InputContentBlock`s for history
    /// - `tool_calls`: `(id, name, input)` tuples for each tool the model requested
    async fn stream_turn(
        &self,
        request: &MessageRequest,
        final_text: &mut String,
        ws_tx: Option<&tokio::sync::mpsc::UnboundedSender<Value>>,
    ) -> Result<(Vec<InputContentBlock>, Vec<(String, String, Value)>)> {
        let mut stream = self.client.stream_message(request).await?;

        // Accumulators keyed by content-block index
        let mut text_blocks: BTreeMap<u32, String> = BTreeMap::new();
        let mut tool_use_ids: BTreeMap<u32, (String, String)> = BTreeMap::new(); // index → (id, name)
        let mut input_json_bufs: BTreeMap<u32, String> = BTreeMap::new();

        while let Some(event) = stream.next_event().await? {
            match event {
                StreamEvent::ContentBlockStart(e) => match &e.content_block {
                    OutputContentBlock::Text { .. } => {
                        text_blocks.insert(e.index, String::new());
                    }
                    OutputContentBlock::ToolUse { id, name, .. } => {
                        tool_use_ids.insert(e.index, (id.clone(), name.clone()));
                        input_json_bufs.insert(e.index, String::new());
                    }
                    OutputContentBlock::Thinking { .. } => {}
                    OutputContentBlock::RedactedThinking { .. } => {}
                },

                StreamEvent::ContentBlockDelta(e) => match e.delta {
                    ContentBlockDelta::TextDelta { text } => {
                        text_blocks.entry(e.index).or_default().push_str(&text);
                        // Stream text chunk to UI
                        if let Some(tx) = ws_tx {
                            let _ = tx.send(serde_json::json!({
                                "type": "chunk",
                                "content": text,
                            }));
                        }
                    }
                    ContentBlockDelta::InputJsonDelta { partial_json } => {
                        input_json_bufs
                            .entry(e.index)
                            .or_default()
                            .push_str(&partial_json);
                    }
                    ContentBlockDelta::ThinkingDelta { thinking } => {
                        if let Some(tx) = ws_tx {
                            let _ = tx.send(serde_json::json!({
                                "type": "thinking",
                                "content": thinking,
                            }));
                        }
                    }
                    ContentBlockDelta::SignatureDelta { .. } => {}
                },

                StreamEvent::ContentBlockStop(e) => {
                    // Finalize tool input JSON
                    if let Some((id, name)) = tool_use_ids.get(&e.index) {
                        let json_str = input_json_bufs.remove(&e.index).unwrap_or_default();
                        // Store finalized JSON; we'll process after MessageStop
                        let _ = (id, name, json_str); // kept in tool_use_ids
                    }
                }

                StreamEvent::MessageStop(_) => break,
                _ => {}
            }
        }

        // Build assistant content blocks (text first, then tool uses)
        let mut assistant_blocks: Vec<InputContentBlock> = Vec::new();

        for (_, text) in &text_blocks {
            if !text.is_empty() {
                assistant_blocks.push(InputContentBlock::Text { text: text.clone() });
                *final_text = text.clone();
            }
        }

        // Collect tool calls
        let mut tool_calls: Vec<(String, String, Value)> = Vec::new();
        for (index, (id, name)) in &tool_use_ids {
            let json_str = input_json_bufs.get(index).cloned().unwrap_or_default();
            let input: Value = serde_json::from_str(&json_str)
                .unwrap_or(Value::Object(serde_json::Map::new()));
            assistant_blocks.push(InputContentBlock::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            });
            tool_calls.push((id.clone(), name.clone(), input));
        }

        Ok((assistant_blocks, tool_calls))
    }
}
