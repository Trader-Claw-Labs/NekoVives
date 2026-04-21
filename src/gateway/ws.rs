//! WebSocket agent chat handler.
//!
//! Protocol:
//! ```text
//! Client -> Server: {"type":"message","content":"Hello"}
//! Client -> Server: {"type":"cancel"}  — abort the current agent turn
//! Server -> Client: {"type":"chunk","content":"Hi! "}
//! Server -> Client: {"type":"tool_call","name":"shell","args":{...}}
//! Server -> Client: {"type":"tool_result","name":"shell","output":"..."}
//! Server -> Client: {"type":"done","full_response":"..."}
//! Server -> Client: {"type":"cancelled"}  — turn was cancelled by user
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

/// Outcome of an agent turn, distinguishing normal completion from external interrupts.
enum TurnOutcome {
    Done(Result<String, anyhow::Error>),
    Timeout,
    Cancelled,
    WsClosed,
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    // Per-connection conversation history — persists across multiple user messages.
    let mut session = crate::agent::Session::new();

    loop {
        // ── Wait for a user message ──────────────────────────────────────
        let raw = loop {
            match receiver.next().await {
                None | Some(Err(_)) => return,
                Some(Ok(Message::Close(_))) => return,
                Some(Ok(Message::Text(t))) => break t,
                _ => continue,
            }
        };

        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => {
                let err = serde_json::json!({"type": "error", "message": "Invalid JSON"});
                let _ = sender.send(Message::Text(err.to_string().into())).await;
                continue;
            }
        };

        let msg_type = parsed["type"].as_str().unwrap_or("");
        // Cancel while idle — nothing to cancel, ignore.
        if msg_type == "cancel" { continue; }
        if msg_type != "message" { continue; }

        let content = parsed["content"].as_str().unwrap_or("").to_string();
        if content.is_empty() { continue; }

        // ── Set up event streaming ───────────────────────────────────────
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

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<serde_json::Value>();

        // Forwarder: pipes agent events to this WS client.
        let mut sender_clone = sender;
        let forward_handle = tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                let _ = sender_clone.send(Message::Text(event.to_string().into())).await;
            }
            sender_clone
        });

        // ── Run agent, racing against cancel and timeout ─────────────────
        let turn_timeout_secs = config.channels_config.agent_turn_timeout_secs;
        let turn_start = std::time::Instant::now();

        let agent_future = crate::agent::process_message_with_events(
            config, &content, event_tx, &mut session,
        );
        tokio::pin!(agent_future);

        let outcome: TurnOutcome = loop {
            tokio::select! {
                biased;

                // Agent completed normally.
                res = &mut agent_future => break TurnOutcome::Done(res),

                // Incoming WS frame while agent is running — look for cancel.
                ws_msg = receiver.next() => {
                    match ws_msg {
                        None | Some(Err(_)) => break TurnOutcome::WsClosed,
                        Some(Ok(Message::Close(_))) => break TurnOutcome::WsClosed,
                        Some(Ok(Message::Text(t))) => {
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                                if v["type"].as_str() == Some("cancel") {
                                    break TurnOutcome::Cancelled;
                                }
                            }
                            // Any other message during a running turn — ignored.
                        }
                        _ => {}
                    }
                    // Not a cancel — keep polling.
                    continue;
                }

                // Hard timeout (0 = disabled).
                _ = async {
                    if turn_timeout_secs > 0 {
                        tokio::time::sleep(std::time::Duration::from_secs(turn_timeout_secs)).await
                    } else {
                        std::future::pending::<()>().await
                    }
                } => break TurnOutcome::Timeout,
            }
        };

        let turn_elapsed = turn_start.elapsed();

        // Drop agent_future here (already pinned on stack, goes out of scope).
        // Dropping event_tx closes the channel → forwarder exits → sender recoverable.
        drop(agent_future);

        let Ok(recovered) = forward_handle.await else { return };
        sender = recovered;

        // ── Handle outcome ───────────────────────────────────────────────
        match outcome {
            TurnOutcome::Cancelled => {
                tracing::info!(
                    provider = %provider_label,
                    model = %model_snap,
                    elapsed_ms = turn_elapsed.as_millis() as u64,
                    "Agent turn cancelled by user"
                );
                let msg = serde_json::json!({"type": "cancelled"});
                let _ = sender.send(Message::Text(msg.to_string().into())).await;
            }

            TurnOutcome::WsClosed => return,

            TurnOutcome::Done(Ok(response)) => {
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

            TurnOutcome::Done(Err(ref e)) => {
                tracing::error!(
                    provider = %provider_label,
                    model = %model_snap,
                    elapsed_ms = turn_elapsed.as_millis() as u64,
                    error = %e,
                    error_debug = ?e,
                    "Agent turn failed"
                );
                let raw = e.to_string();
                let detail = if raw.trim().is_empty() || raw == "408: " || raw.ends_with(": ") {
                    format!(
                        "Provider {} returned HTTP 408 (Request Timeout). \
                        The model or upstream server did not respond in time. \
                        Check that your API key is valid and the model '{}' is available.",
                        provider_label, model_snap
                    )
                } else {
                    crate::providers::sanitize_api_error(&raw)
                };
                tracing::error!(provider = %provider_label, model = %model_snap, "Agent error detail: {detail}");
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

            TurnOutcome::Timeout => {
                tracing::error!(
                    provider = %provider_label,
                    model = %model_snap,
                    timeout_secs = turn_timeout_secs,
                    "Agent turn exceeded gateway timeout"
                );
                let mins = turn_timeout_secs / 60;
                let msg = format!(
                    "Agent timed out after {mins} min. For long tasks (research, backtesting analysis) \
                     increase `agent_turn_timeout_secs` in [channels_config] of your config.toml, \
                     or set it to 0 to disable the limit."
                );
                let err = serde_json::json!({
                    "type": "error",
                    "message": msg,
                    "timeout": true,
                    "timeout_secs": turn_timeout_secs,
                    "provider": provider_label,
                    "model": model_snap,
                });
                let _ = sender.send(Message::Text(err.to_string().into())).await;
            }
        }
    }
}
