//! WebSocket agent chat handler.
//!
//! Protocol:
//! ```text
//! Client -> Server: {"type":"message","content":"Hello"}
//! Server -> Client: {"type":"chunk","content":"Hi! "}
//! Server -> Client: {"type":"tool_call","name":"shell","args":{...}}
//! Server -> Client: {"type":"tool_result","name":"shell","output":"..."}
//! Server -> Client: {"type":"done","full_response":"..."}
//! ```

use super::AppState;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct WsQuery {
    pub token: Option<String>,
}

/// GET /ws/chat — WebSocket upgrade for agent chat
pub async fn handle_ws_chat(
    State(state): State<AppState>,
    Query(params): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Auth via query param (browser WebSocket limitation)
    if state.pairing.require_pairing() {
        let token = params.token.as_deref().unwrap_or("");
        if !state.pairing.is_authenticated(token) {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                "Unauthorized — provide ?token=<bearer_token>",
            )
                .into_response();
        }
    }

    ws.on_upgrade(move |socket| handle_socket(socket, state))
        .into_response()
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    // Per-connection conversation history — persists across multiple user messages.
    let mut session = crate::agent::Session::new();

    while let Some(msg) = receiver.next().await {
        let msg = match msg {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) => break,
            Err(_) => break,
            _ => continue,
        };

        // Parse incoming message
        let parsed: serde_json::Value = match serde_json::from_str(&msg) {
            Ok(v) => v,
            Err(_) => {
                let err = serde_json::json!({"type": "error", "message": "Invalid JSON"});
                let _ = sender.send(Message::Text(err.to_string().into())).await;
                continue;
            }
        };

        let msg_type = parsed["type"].as_str().unwrap_or("");
        if msg_type != "message" {
            continue;
        }

        let content = parsed["content"].as_str().unwrap_or("").to_string();
        if content.is_empty() {
            continue;
        }

        // Run the full agentic loop (tools, skills, MCP, shell, file I/O, memory)
        let config = state.config.lock().clone();
        let provider_label = config
            .default_provider
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let model_snap = state.model.lock().clone();
        let _ = state.event_tx.send(serde_json::json!({
            "type": "agent_start",
            "provider": provider_label,
            "model": model_snap,
        }));

        // Create an unbounded channel for streaming agent events to this WS client.
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<serde_json::Value>();

        // Spawn a forwarder: reads structured events and sends them as WS text frames.
        let mut sender_clone = sender;
        let forward_handle = tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                let _ = sender_clone.send(Message::Text(event.to_string().into())).await;
            }
            sender_clone
        });

        // Hard timeout: 5 minutes max per agent turn
        let turn_start = std::time::Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            crate::agent::process_message_with_events(config, &content, event_tx, &mut session),
        ).await;
        let turn_elapsed = turn_start.elapsed();

        // Recover the sender from the forwarder task (task cannot panic — safe to unwrap).
        let Ok(recovered) = forward_handle.await else { break };
        sender = recovered;

        match result {
            Ok(Ok(response)) => {
                tracing::info!(
                    provider = %provider_label,
                    model = %model_snap,
                    elapsed_ms = turn_elapsed.as_millis() as u64,
                    response_len = response.len(),
                    "Agent turn completed"
                );
                let done = serde_json::json!({
                    "type": "done",
                    "full_response": response,
                });
                let _ = sender.send(Message::Text(done.to_string().into())).await;

                let _ = state.event_tx.send(serde_json::json!({
                    "type": "agent_end",
                    "provider": provider_label,
                    "model": model_snap,
                }));
            }
            Ok(Err(ref e)) => {
                // Log the full error server-side (unsanitized, for debugging)
                tracing::error!(
                    provider = %provider_label,
                    model = %model_snap,
                    elapsed_ms = turn_elapsed.as_millis() as u64,
                    error = %e,
                    error_debug = ?e,
                    "Agent turn failed"
                );

                // Build a human-readable message with context
                let raw = e.to_string();
                let detail = if raw.trim().is_empty() || raw == "408: " || raw.ends_with(": ") {
                    // Empty or truncated error — describe by type
                    format!(
                        "Provider {} returned HTTP 408 (Request Timeout). \
                        The model or upstream server did not respond in time. \
                        Check that your API key is valid and the model '{}' is available.",
                        provider_label, model_snap
                    )
                } else {
                    crate::providers::sanitize_api_error(&raw)
                };

                tracing::error!(
                    provider = %provider_label,
                    model = %model_snap,
                    "Agent error detail: {detail}"
                );

                let err = serde_json::json!({
                    "type": "error",
                    "message": detail,
                    "provider": provider_label,
                    "model": model_snap,
                });
                let _ = sender.send(Message::Text(err.to_string().into())).await;

                let _ = state.event_tx.send(serde_json::json!({
                    "type": "error",
                    "component": "ws_chat",
                    "message": detail,
                }));
            }
            Err(_elapsed) => {
                tracing::error!(
                    provider = %provider_label,
                    model = %model_snap,
                    "Agent turn hit the 5-minute gateway timeout"
                );
                let err = serde_json::json!({
                    "type": "error",
                    "message": "Agent timed out after 5 minutes. The request may have stalled — please try again.",
                    "timeout": true,
                    "provider": provider_label,
                    "model": model_snap,
                });
                let _ = sender.send(Message::Text(err.to_string().into())).await;
            }
        }
    }
}
