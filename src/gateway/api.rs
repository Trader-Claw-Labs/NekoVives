//! REST API handlers for the web dashboard.
//!
//! All `/api/*` routes require bearer token authentication (PairingGuard).

use super::AppState;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde::Deserialize;
use std::sync::Arc;

const MASKED_SECRET: &str = "***MASKED***";

// ── Nullable patch helper ────────────────────────────────────────────────
// Distinguishes three JSON states for optional-but-clearable fields:
//   - field absent    → Option<Option<T>> = None       (no update)
//   - field = null    → Option<Option<T>> = Some(None) (clear/disable)
//   - field = value   → Option<Option<T>> = Some(Some(v)) (set)
//
// Use with #[serde(default, deserialize_with = "nullable::deserialize")]
mod nullable {
    use serde::{Deserialize, Deserializer};
    pub fn deserialize<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Ok(Some(Option::<T>::deserialize(d)?))
    }
}

// ── Bearer token auth extractor ─────────────────────────────────

/// Extract and validate bearer token from Authorization header.
fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|auth| auth.strip_prefix("Bearer "))
}

/// Verify bearer token against PairingGuard. Returns error response if unauthorized.
fn require_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if !state.pairing.require_pairing() {
        return Ok(());
    }

    let token = extract_bearer_token(headers).unwrap_or("");
    if state.pairing.is_authenticated(token) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "Unauthorized — pair first via POST /pair, then send Authorization: Bearer <token>"
            })),
        ))
    }
}

// ── Query parameters ─────────────────────────────────────────────

#[derive(Deserialize)]
pub struct MemoryQuery {
    pub query: Option<String>,
    pub category: Option<String>,
}

#[derive(Deserialize)]
pub struct MemoryStoreBody {
    pub key: String,
    pub content: String,
    pub category: Option<String>,
}

#[derive(Deserialize)]
pub struct CronAddBody {
    pub name: Option<String>,
    pub schedule: String,
    pub command: String,
}

// ── Handlers ────────────────────────────────────────────────────

/// GET /api/status — system status overview
pub async fn handle_api_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();
    let health = crate::health::snapshot();

    let mut channels = serde_json::Map::new();

    for (channel, present) in config.channels_config.channels() {
        channels.insert(channel.name().to_string(), serde_json::Value::Bool(present));
    }

    let body = serde_json::json!({
        "provider": config.default_provider,
        "model": state.model.lock().clone(),
        "temperature": *state.temperature.lock(),
        "uptime_seconds": health.uptime_seconds,
        "gateway_port": config.gateway.port,
        "locale": "en",
        "memory_backend": state.mem.name(),
        "paired": state.pairing.is_paired(),
        "channels": channels,
        "health": health,
    });

    Json(body).into_response()
}

/// GET /api/config — current config (api_key masked)
pub async fn handle_api_config_get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();

    // Serialize to TOML after masking sensitive fields.
    let masked_config = mask_sensitive_fields(&config);
    let toml_str = match toml::to_string_pretty(&masked_config) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to serialize config: {e}")})),
            )
                .into_response();
        }
    };

    Json(serde_json::json!({
        "format": "toml",
        "content": toml_str,
    }))
    .into_response()
}

/// PUT /api/config — update config from TOML body
pub async fn handle_api_config_put(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    // Parse the incoming TOML
    let incoming: crate::config::Config = match toml::from_str(&body) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid TOML: {e}")})),
            )
                .into_response();
        }
    };

    let current_config = state.config.lock().clone();
    let new_config = hydrate_config_for_save(incoming, &current_config);

    if let Err(e) = new_config.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("Invalid config: {e}")})),
        )
            .into_response();
    }

    // Save to disk
    if let Err(e) = new_config.save().await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to save config: {e}")})),
        )
            .into_response();
    }

    // Update in-memory config and hot-reload model/temperature
    if let Some(ref m) = new_config.default_model {
        *state.model.lock() = m.clone();
    }
    *state.temperature.lock() = new_config.default_temperature;
    *state.config.lock() = new_config;

    Json(serde_json::json!({"status": "ok"})).into_response()
}

/// GET /api/onboarding — returns onboarding status (requires auth)
pub async fn handle_api_onboarding_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let config = state.config.lock().clone();
    let onboarded = config.gateway.dashboard_onboarded;
    let api_key_set = config.api_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false);
    let provider = config.default_provider.clone().unwrap_or_else(|| "openrouter".to_string());
    let model = config.default_model.clone().unwrap_or_default();

    Json(serde_json::json!({
        "onboarded": onboarded,
        "api_key_set": api_key_set,
        "provider": provider,
        "model": model,
    }))
    .into_response()
}

/// POST /api/onboarding/complete — mark onboarding done and optionally save api_key/provider/model
pub async fn handle_api_onboarding_complete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let mut config = state.config.lock().clone();
    config.gateway.dashboard_onboarded = true;

    if let Some(key) = body.get("api_key").and_then(|v| v.as_str()) {
        if !key.is_empty() {
            config.api_key = Some(key.to_string());
        }
    }
    if let Some(provider) = body.get("provider").and_then(|v| v.as_str()) {
        if !provider.is_empty() {
            config.default_provider = Some(provider.to_string());
        }
    }
    if let Some(model) = body.get("model").and_then(|v| v.as_str()) {
        if !model.is_empty() {
            config.default_model = Some(model.to_string());
        }
    }
    if let Some(url) = body.get("api_url").and_then(|v| v.as_str()) {
        if !url.is_empty() {
            config.api_url = Some(url.to_string());
        }
    }

    if let Err(e) = config.save().await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to save config: {e}")})),
        )
            .into_response();
    }

    *state.config.lock() = config;
    Json(serde_json::json!({"status": "ok"})).into_response()
}

/// GET /api/tools — list registered tool specs
pub async fn handle_api_tools(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let tools: Vec<serde_json::Value> = state
        .tools_registry
        .iter()
        .map(|spec| {
            serde_json::json!({
                "name": spec.name,
                "description": spec.description,
                "parameters": spec.parameters,
            })
        })
        .collect();

    Json(serde_json::json!({"tools": tools})).into_response()
}

/// GET /api/cron — list cron jobs
pub async fn handle_api_cron_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();
    match crate::cron::list_jobs(&config) {
        Ok(jobs) => {
            let jobs_json: Vec<serde_json::Value> = jobs
                .iter()
                .map(|job| {
                    serde_json::json!({
                        "id": job.id,
                        "name": job.name,
                        "command": job.command,
                        "prompt": job.prompt,
                        "schedule": job.expression,
                        "next_run": job.next_run.to_rfc3339(),
                        "last_run": job.last_run.map(|t| t.to_rfc3339()),
                        "last_status": job.last_status,
                        "last_output": job.last_output,
                        "enabled": job.enabled,
                    })
                })
                .collect();
            Json(serde_json::json!({"jobs": jobs_json})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to list cron jobs: {e}")})),
        )
            .into_response(),
    }
}

/// POST /api/cron — add a new cron job
pub async fn handle_api_cron_add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CronAddBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();
    let schedule = crate::cron::Schedule::Cron {
        expr: body.schedule,
        tz: None,
    };

    match crate::cron::add_shell_job(&config, body.name, schedule, &body.command) {
        Ok(job) => Json(serde_json::json!({
            "status": "ok",
            "job": {
                "id": job.id,
                "name": job.name,
                "command": job.command,
                "enabled": job.enabled,
            }
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to add cron job: {e}")})),
        )
            .into_response(),
    }
}

/// DELETE /api/cron/:id — remove a cron job
pub async fn handle_api_cron_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();
    match crate::cron::remove_job(&config, &id) {
        Ok(()) => Json(serde_json::json!({"status": "ok"})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to remove cron job: {e}")})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct CronAgentBody {
    pub name: Option<String>,
    pub schedule: String,
    pub prompt: String,
}

/// POST /api/cron/agent — add a new agent job (with prompt)
pub async fn handle_api_cron_agent_add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CronAgentBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();
    let schedule = crate::cron::Schedule::Cron {
        expr: body.schedule,
        tz: None,
    };

    match crate::cron::add_agent_job(
        &config,
        body.name,
        schedule,
        &body.prompt,
        crate::cron::SessionTarget::Isolated,
        None,
        None,
        false,
    ) {
        Ok(job) => Json(serde_json::json!({
            "status": "ok",
            "job": {
                "id": job.id,
                "name": job.name,
                "prompt": job.prompt,
                "enabled": job.enabled,
            }
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to add agent job: {e}")})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct CronUpdateBody {
    pub enabled: Option<bool>,
    pub name: Option<String>,
    pub schedule: Option<String>,
    pub prompt: Option<String>,
    pub command: Option<String>,
}

/// PUT /api/cron/:id — update a cron job (enable/disable, rename, etc.)
pub async fn handle_api_cron_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<CronUpdateBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();

    let schedule = body.schedule.map(|expr| crate::cron::Schedule::Cron {
        expr,
        tz: None,
    });

    let patch = crate::cron::CronJobPatch {
        enabled: body.enabled,
        name: body.name,
        schedule,
        command: body.command,
        prompt: body.prompt,
        ..crate::cron::CronJobPatch::default()
    };

    match crate::cron::update_job(&config, &id, patch) {
        Ok(job) => Json(serde_json::json!({
            "status": "ok",
            "job": {
                "id": job.id,
                "name": job.name,
                "enabled": job.enabled,
            }
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to update cron job: {e}")})),
        )
            .into_response(),
    }
}

/// GET /api/integrations — list all integrations with status
pub async fn handle_api_integrations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();
    let entries = crate::integrations::registry::all_integrations();

    let integrations: Vec<serde_json::Value> = entries
        .iter()
        .map(|entry| {
            let status = (entry.status_fn)(&config);
            serde_json::json!({
                "name": entry.name,
                "description": entry.description,
                "category": entry.category,
                "status": status,
            })
        })
        .collect();

    Json(serde_json::json!({"integrations": integrations})).into_response()
}

/// POST /api/doctor — run diagnostics
pub async fn handle_api_doctor(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();
    let results = crate::doctor::diagnose(&config);

    let ok_count = results
        .iter()
        .filter(|r| r.severity == crate::doctor::Severity::Ok)
        .count();
    let warn_count = results
        .iter()
        .filter(|r| r.severity == crate::doctor::Severity::Warn)
        .count();
    let error_count = results
        .iter()
        .filter(|r| r.severity == crate::doctor::Severity::Error)
        .count();

    Json(serde_json::json!({
        "results": results,
        "summary": {
            "ok": ok_count,
            "warnings": warn_count,
            "errors": error_count,
        }
    }))
    .into_response()
}

/// GET /api/memory — list or search memory entries
pub async fn handle_api_memory_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<MemoryQuery>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    if let Some(ref query) = params.query {
        // Search mode
        match state.mem.recall(query, 50, None).await {
            Ok(entries) => Json(serde_json::json!({"entries": entries})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Memory recall failed: {e}")})),
            )
                .into_response(),
        }
    } else {
        // List mode
        let category = params.category.as_deref().map(|cat| match cat {
            "core" => crate::memory::MemoryCategory::Core,
            "daily" => crate::memory::MemoryCategory::Daily,
            "conversation" => crate::memory::MemoryCategory::Conversation,
            other => crate::memory::MemoryCategory::Custom(other.to_string()),
        });

        match state.mem.list(category.as_ref(), None).await {
            Ok(entries) => Json(serde_json::json!({"entries": entries})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Memory list failed: {e}")})),
            )
                .into_response(),
        }
    }
}

/// POST /api/memory — store a memory entry
pub async fn handle_api_memory_store(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MemoryStoreBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let category = body
        .category
        .as_deref()
        .map(|cat| match cat {
            "core" => crate::memory::MemoryCategory::Core,
            "daily" => crate::memory::MemoryCategory::Daily,
            "conversation" => crate::memory::MemoryCategory::Conversation,
            other => crate::memory::MemoryCategory::Custom(other.to_string()),
        })
        .unwrap_or(crate::memory::MemoryCategory::Core);

    match state
        .mem
        .store(&body.key, &body.content, category, None)
        .await
    {
        Ok(()) => Json(serde_json::json!({"status": "ok"})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Memory store failed: {e}")})),
        )
            .into_response(),
    }
}

/// DELETE /api/memory/:key — delete a memory entry
pub async fn handle_api_memory_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    match state.mem.forget(&key).await {
        Ok(deleted) => {
            Json(serde_json::json!({"status": "ok", "deleted": deleted})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Memory forget failed: {e}")})),
        )
            .into_response(),
    }
}

/// GET /api/cost — cost summary
pub async fn handle_api_cost(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    if let Some(ref tracker) = state.cost_tracker {
        match tracker.get_summary() {
            Ok(summary) => Json(serde_json::json!({"cost": summary})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Cost summary failed: {e}")})),
            )
                .into_response(),
        }
    } else {
        Json(serde_json::json!({
            "cost": {
                "session_cost_usd": 0.0,
                "daily_cost_usd": 0.0,
                "monthly_cost_usd": 0.0,
                "total_tokens": 0,
                "request_count": 0,
                "by_model": {},
            }
        }))
        .into_response()
    }
}

/// GET /api/cli-tools — discovered CLI tools
pub async fn handle_api_cli_tools(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let tools = crate::tools::cli_discovery::discover_cli_tools(&[], &[]);

    Json(serde_json::json!({"cli_tools": tools})).into_response()
}

/// GET /api/health — component health snapshot
pub async fn handle_api_health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let snapshot = crate::health::snapshot();
    Json(serde_json::json!({"health": snapshot})).into_response()
}

// ── Degen-Agent Web Dashboard Stubs ─────────────────────────────

#[derive(Deserialize)]
pub struct CreateWalletBody {
    pub chain: String,
    pub password: String,
    pub label: Option<String>,
}

#[derive(Deserialize)]
pub struct ExportWalletBody {
    pub address: String,
    pub password: String,
    pub export_type: String, // "mnemonic" | "private_key"
}

#[derive(Deserialize)]
pub struct PolymarketConfigBody {
    pub wallet_address: Option<String>,
    pub api_key: Option<String>,
    pub secret: Option<String>,
    pub passphrase: Option<String>,
    pub private_key: Option<String>,
    #[serde(default)]
    pub signature_type: Option<String>,
}

#[derive(Deserialize)]
pub struct TelegramConfigBody {
    pub bot_token: Option<String>,
    pub allowed_users: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct ChatBody {
    pub session_id: Option<String>,
    pub message: String,
}

/// GET /api/wallets — list wallets
pub async fn handle_api_wallets_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let store = state.wallets.lock();
    let wallets: Vec<serde_json::Value> = store
        .iter()
        .map(|w| serde_json::json!({ "chain": w.chain, "address": w.address, "label": w.label }))
        .collect();
    Json(serde_json::json!({"wallets": wallets})).into_response()
}

/// POST /api/wallets/create — create a new wallet with real key generation
pub async fn handle_api_wallets_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateWalletBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    if body.password.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Password is required"})),
        )
            .into_response();
    }

    let chain = body.chain.to_lowercase();
    let label = body.label.unwrap_or_default();

    let (address, mnemonic, encrypted_key) = match chain.as_str() {
        "evm" => {
            match wallet_manager::evm::create_wallet(0, &body.password) {
                Ok(info) => {
                    let m = info.mnemonic.clone().unwrap_or_default();
                    (info.address, m, info.encrypted_private_key)
                }
                Err(e) => return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("EVM wallet error: {e}")})),
                ).into_response(),
            }
        }
        "solana" => {
            match wallet_manager::solana::create_wallet(0, &body.password) {
                Ok(info) => {
                    let m = info.mnemonic.clone().unwrap_or_default();
                    (info.address, m, info.encrypted_private_key)
                }
                Err(e) => return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Solana wallet error: {e}")})),
                ).into_response(),
            }
        }
        "ton" => {
            match wallet_manager::ton::create_wallet(&body.password) {
                Ok(info) => {
                    let m = info.mnemonic.clone().unwrap_or_default();
                    (info.address, m, info.encrypted_private_key)
                }
                Err(e) => return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("TON wallet error: {e}")})),
                ).into_response(),
            }
        }
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Unsupported chain: {other}")})),
            )
                .into_response();
        }
    };

    {
        let mut store = state.wallets.lock();
        store.push(super::StoredWallet {
            chain: chain.clone(),
            address: address.clone(),
            label: label.clone(),
            encrypted_key,
        });
        super::save_wallets_to_disk(&store, &state.wallets_path);
    }

    Json(serde_json::json!({
        "status": "ok",
        "wallet": {
            "address": address,
            "chain": chain,
            "label": label,
            "mnemonic": mnemonic,
        }
    }))
    .into_response()
}

/// POST /api/wallets/export — decrypt and return mnemonic or private key
pub async fn handle_api_wallets_export(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ExportWalletBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let store = state.wallets.lock();
    let wallet = match store.iter().find(|w| w.address == body.address) {
        Some(w) => w.clone(),
        None => return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Wallet not found"})),
        ).into_response(),
    };
    drop(store);

    let result: Result<String, String> = match body.export_type.as_str() {
        "mnemonic" => match wallet.chain.as_str() {
            "evm" => wallet_manager::evm::export_mnemonic(&wallet.encrypted_key, &body.password)
                .map_err(|e| e.to_string()),
            "solana" => wallet_manager::solana::export_mnemonic(&wallet.encrypted_key, &body.password)
                .map_err(|e| e.to_string()),
            "ton" => wallet_manager::ton::export_mnemonic(&wallet.encrypted_key, &body.password)
                .map_err(|e| e.to_string()),
            c => Err(format!("Unsupported chain: {c}")),
        },
        "private_key" => match wallet.chain.as_str() {
            "evm" => wallet_manager::evm::export_private_key(&wallet.encrypted_key, &body.password)
                .map(|b| hex::encode(b))
                .map_err(|e| e.to_string()),
            "solana" => wallet_manager::solana::export_private_key(&wallet.encrypted_key, &body.password)
                .map(|b| hex::encode(b))
                .map_err(|e| e.to_string()),
            "ton" => wallet_manager::ton::export_private_key(&wallet.encrypted_key, &body.password)
                .map(|b| hex::encode(b))
                .map_err(|e| e.to_string()),
            c => Err(format!("Unsupported chain: {c}")),
        },
        t => Err(format!("Unknown export_type: {t}")),
    };

    match result {
        Ok(value) => Json(serde_json::json!({ "value": value })).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        ).into_response(),
    }
}

/// GET /api/wallets/:address/balance — live on-chain balance
pub async fn handle_api_wallet_balance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(address): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    // Find the wallet to determine its chain
    let chain = {
        let wallets = state.wallets.lock();
        wallets
            .iter()
            .find(|w| w.address.eq_ignore_ascii_case(&address))
            .map(|w| w.chain.to_lowercase())
    };

    let chain = match chain {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "wallet not found"})),
            )
                .into_response();
        }
    };

    let trader = solana_trader::SolanaTrader::new(None);

    match chain.as_str() {
        "solana" => {
            let sol = trader.get_sol_balance(&address).await;
            let tokens = trader.get_token_balances(&address).await.unwrap_or_default();
            let token_list: Vec<_> = tokens
                .iter()
                .map(|t| serde_json::json!({"mint": t.mint, "symbol": t.symbol, "amount": t.amount}))
                .collect();
            match sol {
                Ok(balance) => Json(serde_json::json!({
                    "address": address,
                    "chain": "solana",
                    "balance": balance,
                    "currency": "SOL",
                    "tokens": token_list,
                }))
                .into_response(),
                Err(e) => (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
                    .into_response(),
            }
        }
        "evm" => {
            let chains_rpc = state.config.lock().chains_rpc.clone();
            let chain_balances = crate::tools::wallet_balance::evm_multichain_balances(
                &address, &chains_rpc,
            ).await;
            // Primary balance = Ethereum mainnet (first result) or first chain found
            let primary_balance = chain_balances.first().map(|(_, b, _, _)| *b).unwrap_or(0.0);
            let chains: Vec<serde_json::Value> = chain_balances
                .iter()
                .map(|(name, bal, sym, explorer)| serde_json::json!({
                    "chain": name,
                    "balance": bal,
                    "symbol": sym,
                    "explorer": explorer,
                }))
                .collect();
            Json(serde_json::json!({
                "address": address,
                "chain": "evm",
                "balance": primary_balance,
                "currency": "ETH",
                "chains": chains,
                "tokens": [],
            }))
            .into_response()
        }
        _ => Json(serde_json::json!({
            "address": address,
            "chain": chain,
            "balance": 0.0,
            "currency": chain.to_uppercase(),
            "tokens": [],
            "note": "Balance query not yet implemented for this chain",
        }))
        .into_response(),
    }
}

// ── Swap body types ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SwapQuoteBody {
    pub chain: String,
    pub from_token: String,
    pub to_token: String,
    pub amount: f64,
}

#[derive(Deserialize)]
pub struct SwapExecuteBody {
    pub chain: String,
    pub from_token: String,
    pub to_token: String,
    pub amount: f64,
    pub wallet_address: String,
    pub password: Option<String>,
    pub slippage_bps: Option<u64>,
}

/// POST /api/wallets/quote — get a swap quote (EVM via Uniswap QuoterV2 or Solana via Jupiter)
pub async fn handle_api_wallets_quote(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SwapQuoteBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    match body.chain.as_str() {
        "evm" => {
            // EVM: use Uniswap QuoterV2 (chain_id 1 = Ethereum mainnet)
            let amount_in = (body.amount * 1e18) as u128; // assume 18-decimal input token
            match evm_trader::uniswap::get_quote(&body.from_token, &body.to_token, amount_in, 1).await {
                Ok(q) => Json(serde_json::json!({
                    "quote": {
                        "in_amount": body.amount,
                        "out_amount": q.amount_out_readable,
                        "out_amount_min": q.amount_out_readable * 0.995,
                        "price_impact_pct": q.price_impact_bps.map(|b| b as f64 / 100.0).unwrap_or(0.0),
                        "route": format!("{} → UniV3(0.3%) → {}", body.from_token, body.to_token),
                        "gas_estimate": q.gas_estimate,
                    }
                })).into_response(),
                Err(e) => (StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("QuoterV2 error: {e}")}))).into_response(),
            }
        }
        "solana" => {
            // Solana: use Jupiter /v6/quote
            let amount_lamports = (body.amount * 1e9) as u64; // assume SOL-like 9 decimals
            let url = format!(
                "https://quote-api.jup.ag/v6/quote?inputMint={}&outputMint={}&amount={}",
                body.from_token, body.to_token, amount_lamports
            );
            let client = reqwest::Client::new();
            match client.get(&url).send().await {
                Ok(r) => {
                    match r.json::<serde_json::Value>().await {
                        Ok(jup) => {
                            let out_amount = jup["outAmount"].as_str()
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.0) / 1e6; // assume 6-decimal output
                            let price_impact = jup["priceImpactPct"].as_str()
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.0);
                            let route = jup["routePlan"].as_array()
                                .and_then(|r| r.first())
                                .and_then(|s| s["swapInfo"]["label"].as_str())
                                .unwrap_or("Jupiter")
                                .to_string();
                            Json(serde_json::json!({
                                "quote": {
                                    "in_amount": body.amount,
                                    "out_amount": out_amount,
                                    "out_amount_min": out_amount * 0.995,
                                    "price_impact_pct": price_impact,
                                    "route": route,
                                    "_jupiter_quote": jup,
                                }
                            })).into_response()
                        }
                        Err(e) => (StatusCode::BAD_GATEWAY,
                            Json(serde_json::json!({"error": format!("Jupiter parse error: {e}")}))).into_response(),
                    }
                }
                Err(e) => (StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("Jupiter request error: {e}")}))).into_response(),
            }
        }
        chain => (StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("Unsupported chain: {chain}")}))).into_response(),
    }
}

/// POST /api/wallets/swap — execute a swap
pub async fn handle_api_wallets_swap(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SwapExecuteBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    match body.chain.as_str() {
        "evm" => {
            // EVM execute_swap requires signer integration not yet wired
            (StatusCode::NOT_IMPLEMENTED, Json(serde_json::json!({
                "error": "EVM swap execution requires signer integration. Use a hardware wallet or external signer. Quote via /api/wallets/quote and broadcast via your EVM wallet."
            }))).into_response()
        }
        "solana" => {
            // Find wallet and decrypt private key
            let (encrypted_key, _) = {
                let wallets = state.wallets.lock();
                match wallets.iter().find(|w| w.address.eq_ignore_ascii_case(&body.wallet_address) && w.chain == "solana") {
                    Some(w) => (w.encrypted_key.clone(), w.address.clone()),
                    None => return (StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": "Solana wallet not found"}))).into_response(),
                }
            };

            let password = match body.password.as_deref() {
                Some(p) if !p.is_empty() => p.to_string(),
                _ => return (StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Wallet password required for Solana swap"}))).into_response(),
            };

            let privkey_bytes = match wallet_manager::solana::export_private_key(&encrypted_key, &password) {
                Ok(b) => b,
                Err(e) => return (StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": format!("Decrypt failed: {e}")}))).into_response(),
            };

            // First get a Jupiter quote, then execute
            let amount_lamports = (body.amount * 1e9) as u64;
            let quote_url = format!(
                "https://quote-api.jup.ag/v6/quote?inputMint={}&outputMint={}&amount={}&slippageBps={}",
                body.from_token, body.to_token, amount_lamports, body.slippage_bps.unwrap_or(50)
            );
            let client = reqwest::Client::new();
            let quote = match client.get(&quote_url).send().await {
                Ok(r) => match r.json::<serde_json::Value>().await {
                    Ok(j) => j,
                    Err(e) => return (StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({"error": format!("Jupiter quote parse: {e}")}))).into_response(),
                },
                Err(e) => return (StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("Jupiter quote request: {e}")}))).into_response(),
            };

            let trader = solana_trader::SolanaTrader::new(None);
            let mut key_arr = [0u8; 32];
            if privkey_bytes.len() >= 32 {
                key_arr.copy_from_slice(&privkey_bytes[..32]);
            }
            match trader.swap(&quote, &body.wallet_address, &key_arr).await {
                Ok(sig) => Json(serde_json::json!({"status": "ok", "tx_hash": sig})).into_response(),
                Err(e) => (StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("Swap failed: {e}")}))).into_response(),
            }
        }
        chain => (StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("Unsupported chain: {chain}")}))).into_response(),
    }
}

/// POST /api/wallets/transfer — send native token to another address
#[derive(serde::Deserialize)]
pub struct TransferBody {
    pub from_address: String,
    pub to_address: String,
    pub amount: f64,
    pub chain: String,
    pub password: String,
}

pub async fn handle_api_wallets_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TransferBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    if body.to_address.is_empty() || body.amount <= 0.0 {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "Invalid to_address or amount"
        }))).into_response();
    }

    match body.chain.as_str() {
        "evm" => {
            (StatusCode::NOT_IMPLEMENTED, Json(serde_json::json!({
                "error": "EVM native transfers require signer integration. Use your EVM wallet (MetaMask, hardware wallet) to send ETH/MATIC directly."
            }))).into_response()
        }
        "solana" => {
            // Verify password is correct before showing "not implemented"
            let encrypted_key = {
                let wallets = state.wallets.lock();
                match wallets.iter().find(|w| w.address.eq_ignore_ascii_case(&body.from_address) && w.chain == "solana") {
                    Some(w) => w.encrypted_key.clone(),
                    None => return (StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": "Solana wallet not found"}))).into_response(),
                }
            };
            if let Err(e) = wallet_manager::solana::export_private_key(&encrypted_key, &body.password) {
                return (StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": format!("Decrypt failed: {e}")}))).into_response();
            }
            (StatusCode::NOT_IMPLEMENTED, Json(serde_json::json!({
                "error": "Solana native SOL transfers are coming soon. For now, use a Solana wallet (Phantom, Backpack) to send SOL. Your private key can be exported from the wallet page."
            }))).into_response()
        }
        "ton" => {
            (StatusCode::NOT_IMPLEMENTED, Json(serde_json::json!({
                "error": "TON transfers are not yet implemented. Use the TON wallet app or tonkeeper.com."
            }))).into_response()
        }
        chain => (StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("Unsupported chain: {chain}")}))).into_response(),
    }
}

/// GET /api/polymarket/prices-history — proxy to Polymarket CLOB /prices-history
/// Query params: token_id (required), interval (optional: 1h/6h/1d/1w/all, default 1d)
pub async fn handle_api_polymarket_prices_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let token_id = match params.get("token_id") {
        Some(t) => t.clone(),
        None => return (StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "token_id query param required"}))).into_response(),
    };
    let interval = params.get("interval").map(String::as_str).unwrap_or("1d");

    // Map interval to Polymarket CLOB fidelity + startTs
    let (fidelity, start_offset_secs): (u64, u64) = match interval {
        "1h"  => (1,    3_600),
        "6h"  => (5,   21_600),
        "1d"  => (10,  86_400),
        "1w"  => (60, 604_800),
        "all" => (1440, 0),
        _     => (10,  86_400),
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let start_ts = if start_offset_secs > 0 { now - start_offset_secs } else { 0 };

    let url = if start_offset_secs > 0 {
        format!(
            "https://clob.polymarket.com/prices-history?market={}&interval={}&fidelity={}&startTs={}",
            token_id, interval, fidelity, start_ts
        )
    } else {
        format!(
            "https://clob.polymarket.com/prices-history?market={}&interval={}&fidelity={}",
            token_id, interval, fidelity
        )
    };

    let client = reqwest::Client::new();
    match client.get(&url).send().await {
        Ok(r) => {
            match r.json::<serde_json::Value>().await {
                Ok(data) => {
                    // Extract the history array (Polymarket returns {"history": [{t, p}, ...]})
                    let history = data.get("history").cloned().unwrap_or(data);
                    Json(serde_json::json!({
                        "token_id": token_id,
                        "interval": interval,
                        "history": history,
                    })).into_response()
                }
                Err(e) => (StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("Parse error: {e}")}))).into_response(),
            }
        }
        Err(e) => (StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": format!("CLOB request failed: {e}")}))).into_response(),
    }
}

#[derive(serde::Deserialize, Default)]
pub struct PolymarketsQuery {
    pub q: Option<String>,
    pub limit: Option<usize>,
    /// Only include markets closing >= min_days from now
    pub min_days: Option<u32>,
    /// Only include markets closing <= max_days from now
    pub max_days: Option<u32>,
    /// Gamma API tag_slug filter (e.g. "crypto")
    pub tag: Option<String>,
    /// "volume" (default) ranks by 24h notional traded; "liquidity" ranks by
    /// current order-book depth and surfaces the deepest markets even when
    /// they closed yesterday — handy for engines that need fillable books.
    pub sort: Option<String>,
}

/// GET /api/polymarket/markets — fetch markets from Gamma API, optional ?q=search
pub async fn handle_api_polymarket_markets(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<PolymarketsQuery>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let limit = params.limit.unwrap_or(50).min(200);
    let filter = polymarket_trader::markets::MarketFilter {
        active_only: true,
        query: params.q.clone(),
        limit: Some(limit),
        min_days: params.min_days,
        max_days: params.max_days,
        tag: params.tag.clone(),
        ..Default::default()
    };
    match polymarket_trader::markets::list_markets(filter).await {
        Ok(markets) => {
            let mut sorted = markets;
            let by_liq = matches!(params.sort.as_deref(), Some("liquidity"));
            if by_liq {
                sorted.sort_by(|a, b| {
                    b.liquidity.partial_cmp(&a.liquidity).unwrap_or(std::cmp::Ordering::Equal)
                });
            } else {
                sorted.sort_by(|a, b| {
                    b.volume.partial_cmp(&a.volume).unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            let top: Vec<_> = sorted.into_iter().take(limit).collect();

            // Fetch YES prices in parallel (best-effort — ignore failures)
            let price_futs: Vec<_> = top
                .iter()
                .map(|m| polymarket_trader::markets::get_market_price(&m.yes_token_id))
                .collect();
            let prices = futures_util::future::join_all(price_futs).await;

            let result: Vec<serde_json::Value> = top
                .into_iter()
                .zip(prices)
                .map(|(m, price_res)| {
                    serde_json::json!({
                        "id": m.condition_id,
                        "slug": m.slug,
                        "question": m.question,
                        "yes_price": price_res.ok(),
                        "volume": m.volume,
                        "liquidity": m.liquidity,
                        "end_date": m.end_date_iso,
                        "yes_token_id": m.yes_token_id,
                        "category": m.category,
                    })
                })
                .collect();

            Json(serde_json::json!({"markets": result})).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": format!("Polymarket Gamma API error: {e}")})),
        )
            .into_response(),
    }
}


#[derive(Deserialize)]
pub struct PolymarketWalletProfileBody {
    pub id: Option<String>,
    pub label: Option<String>,
    pub wallet_address: Option<String>,
    pub api_key: Option<String>,
    pub secret: Option<String>,
    pub passphrase: Option<String>,
    pub private_key: Option<String>,
    #[serde(default)]
    pub is_builder: Option<bool>,
    #[serde(default)]
    pub proxy_address: Option<String>,
    #[serde(default)]
    pub signature_type: Option<String>,
}

fn mask_short(s: &Option<String>) -> Option<String> {
    s.as_deref().filter(|v| !v.trim().is_empty()).map(|v| {
        if v.len() <= 10 { "••••••••".to_string() } else { format!("{}…{}", &v[..6], &v[v.len()-4..]) }
    })
}

fn profile_summary(p: &crate::config::schema::PolymarketWalletProfile) -> serde_json::Value {
    let configured = clean_optional(&p.api_key).is_some()
        && clean_optional(&p.secret).is_some()
        && clean_optional(&p.passphrase).is_some()
        && clean_optional(&p.wallet_address).is_some()
        && clean_optional(&p.private_key).is_some();
    serde_json::json!({
        "id": p.id,
        "label": if p.label.trim().is_empty() { p.id.clone() } else { p.label.clone() },
        "configured": configured,
        "wallet_address": clean_optional(&p.wallet_address),
        "wallet_address_masked": mask_short(&p.wallet_address),
        "api_key_masked": mask_short(&p.api_key),
        "proxy_address": clean_optional(&p.proxy_address),
        "proxy_address_masked": mask_short(&p.proxy_address),
        "has_secret": clean_optional(&p.secret).is_some(),
        "has_passphrase": clean_optional(&p.passphrase).is_some(),
        "has_private_key": clean_optional(&p.private_key).is_some(),
        "is_builder": p.is_builder.unwrap_or(false),
        "signature_type": p.signature_type,
    })
}

/// GET /api/polymarket/wallets — list named wallet profiles (masked; secrets omitted).
pub async fn handle_api_polymarket_wallets_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let cfg = state.config.lock();
    let profiles = list_poly_wallet_profiles(&cfg.polymarket);
    let wallets: Vec<_> = profiles.iter().map(profile_summary).collect();
    Json(serde_json::json!({ "wallets": wallets })).into_response()
}

/// POST /api/polymarket/wallets — create/update a named wallet profile.
pub async fn handle_api_polymarket_wallets_upsert(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PolymarketWalletProfileBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    fn clean(s: Option<String>) -> Option<String> { s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) }
    fn is_placeholder(s: &str) -> bool {
        !s.is_empty() && (s.chars().all(|c| matches!(c, '•' | '*' | '·' | '●')) || s.contains('…'))
    }
    fn merge_secret(input: Option<String>, existing: Option<String>) -> Option<String> {
        match clean(input) {
            Some(v) if is_placeholder(&v) => existing,
            other => other.or(existing),
        }
    }

    let id = clean(body.id.clone()).unwrap_or_else(|| format!("wallet-{}", uuid::Uuid::new_v4().simple()));
    if id == "default" {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"Use /api/polymarket/configure for the legacy default profile; named profiles cannot use id=default."}))).into_response();
    }

    let mut config = state.config.lock().clone();
    let existing_idx = config.polymarket.wallets.iter().position(|p| p.id == id);
    let existing = existing_idx.and_then(|i| config.polymarket.wallets.get(i).cloned()).unwrap_or_default();
    let profile = crate::config::schema::PolymarketWalletProfile {
        id: id.clone(),
        label: clean(body.label).or_else(|| if existing.label.is_empty() { None } else { Some(existing.label.clone()) }).unwrap_or_else(|| id.clone()),
        wallet_address: merge_secret(body.wallet_address, existing.wallet_address),
        api_key: merge_secret(body.api_key, existing.api_key),
        secret: merge_secret(body.secret, existing.secret),
        passphrase: merge_secret(body.passphrase, existing.passphrase),
        private_key: merge_secret(body.private_key, existing.private_key),
        is_builder: body.is_builder.or(existing.is_builder),
        proxy_address: merge_secret(body.proxy_address, existing.proxy_address),
        signature_type: clean(body.signature_type).or(existing.signature_type),
    };
    if let Some(i) = existing_idx {
        config.polymarket.wallets[i] = profile.clone();
    } else {
        config.polymarket.wallets.push(profile.clone());
    }
    if let Err(e) = config.save().await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("Failed to save config: {e}")}))).into_response();
    }
    *state.config.lock() = config;
    Json(serde_json::json!({"status":"ok", "wallet": profile_summary(&profile)})).into_response()
}

/// DELETE /api/polymarket/wallets/{id} — remove a named wallet profile.
pub async fn handle_api_polymarket_wallets_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    if id == "default" {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"The legacy default profile cannot be deleted here."}))).into_response();
    }
    let mut config = state.config.lock().clone();
    let before = config.polymarket.wallets.len();
    config.polymarket.wallets.retain(|p| p.id != id);
    if config.polymarket.wallets.len() == before {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"wallet profile not found"}))).into_response();
    }
    if let Err(e) = config.save().await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("Failed to save config: {e}")}))).into_response();
    }
    *state.config.lock() = config;
    Json(serde_json::json!({"status":"deleted", "id": id})).into_response()
}

/// GET /api/polymarket/configure — return saved credentials (masked)
pub async fn handle_api_polymarket_configure_get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let cfg = state.config.lock();
    let pm = &cfg.polymarket;
    fn mask(s: &Option<String>) -> Option<String> {
        s.as_deref().filter(|v| !v.is_empty()).map(|v| {
            if v.len() <= 8 { "••••••••".to_string() }
            else { format!("{}…{}", &v[..4], &v[v.len()-4..]) }
        })
    }
    Json(serde_json::json!({
        "configured": pm.api_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false),
        "api_key_masked": mask(&pm.api_key),
        "wallet_address": pm.wallet_address,
        "has_secret": pm.secret.as_deref().map(|s| !s.is_empty()).unwrap_or(false),
        "has_passphrase": pm.passphrase.as_deref().map(|p| !p.is_empty()).unwrap_or(false),
        "has_private_key": pm.private_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false),
        "signature_type": pm.signature_type,
    }))
    .into_response()
}

/// POST /api/polymarket/configure — validate against CLOB API and save credentials
pub async fn handle_api_polymarket_configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PolymarketConfigBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    // Trim whitespace — copy/paste from browser often carries trailing spaces or newlines,
    // which break the HMAC signature with a silent 401 at request time.
    fn clean(s: Option<String>) -> Option<String> {
        s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    }
    /// A masked placeholder (e.g. "••••••••") was shown in the UI for a field
    /// that already had a value stored. If the user didn't re-type it, we must
    /// NOT overwrite the real secret with the literal bullet characters.
    /// Also detects the api_key "abcd…wxyz" mask returned by GET /configure.
    fn is_placeholder(s: &str) -> bool {
        if s.is_empty() { return false; }
        if s.chars().all(|c| matches!(c, '•' | '*' | '·' | '●')) { return true; }
        if s.contains('…') { return true; }
        false
    }

    let api_key_input = clean(body.api_key.clone());
    let secret_input = clean(body.secret.clone());
    let passphrase_input = clean(body.passphrase.clone());
    let private_key_input = clean(body.private_key.clone());
    let wallet_address = clean(body.wallet_address.clone());

    let mut config = state.config.lock().clone();
    let existing = config.polymarket.clone();

    // Skip overwriting with masked placeholders — keep the previously stored value.
    let api_key = match api_key_input {
        Some(v) if is_placeholder(&v) => existing.api_key.clone(),
        other => other.or(existing.api_key.clone()),
    };
    let secret = match secret_input {
        Some(v) if is_placeholder(&v) => existing.secret.clone(),
        other => other.or(existing.secret.clone()),
    };
    let passphrase = match passphrase_input {
        Some(v) if is_placeholder(&v) => existing.passphrase.clone(),
        other => other.or(existing.passphrase.clone()),
    };
    let private_key = match private_key_input {
        Some(v) if is_placeholder(&v) => existing.private_key.clone(),
        other => other.or(existing.private_key.clone()),
    };

    config.polymarket = crate::config::schema::PolymarketConfig {
        api_key,
        secret,
        passphrase,
        wallet_address: wallet_address.or(existing.wallet_address),
        private_key,
        is_builder: existing.is_builder,
        proxy_address: existing.proxy_address,
        signature_type: body.signature_type.filter(|s| !s.is_empty()).or(existing.signature_type),
        wallets: existing.wallets,
    };
    if let Err(e) = config.save().await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to save config: {e}")})),
        )
            .into_response();
    }
    *state.config.lock() = config;

    Json(serde_json::json!({"status": "ok", "message": "Polymarket credentials saved"}))
        .into_response()
}

/// POST /api/polymarket/test — validate API key against Polymarket CLOB API
///
/// Real validation flow:
///   1. Ping the CLOB public endpoint for connectivity.
///   2. Make an L2-authenticated request to `GET /auth/api-keys` trying each
///      secret-decoding strategy (Base64 default, then Raw, then Hex). The one
///      that returns 2xx is the real encoding Polymarket expects for this key.
///   3. On failure, report which part is likely wrong (api_key length/preview,
///      secret length, passphrase length, and hint on encoding).
///
/// Empty or masked (bullet / "abcd…wxyz") fields in the request body fall back
/// to the stored config values — so the user can hit "Test Connection" without
/// re-typing the secret every time.
pub async fn handle_api_polymarket_test(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PolymarketConfigBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    fn clean(s: Option<String>) -> Option<String> {
        s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    }
    fn is_placeholder(s: &str) -> bool {
        if s.is_empty() { return false; }
        if s.chars().all(|c| matches!(c, '•' | '*' | '·' | '●')) { return true; }
        if s.contains('…') { return true; }
        false
    }
    fn resolve(input: Option<String>, stored: Option<String>) -> Option<String> {
        match clean(input) {
            Some(v) if is_placeholder(&v) => stored,
            other => other.or(stored),
        }
    }

    // Resolve credentials: prefer the form input, fall back to stored config
    // when the user left the field empty or the UI rendered a placeholder mask.
    let stored = { state.config.lock().polymarket.clone() };
    let api_key = resolve(body.api_key.clone(), stored.api_key.clone()).unwrap_or_default();
    let secret = resolve(body.secret.clone(), stored.secret.clone()).unwrap_or_default();
    let passphrase = resolve(body.passphrase.clone(), stored.passphrase.clone()).unwrap_or_default();
    let wallet_address = resolve(body.wallet_address.clone(), stored.wallet_address.clone())
        .unwrap_or_default();

    if api_key.is_empty() || secret.is_empty() || passphrase.is_empty() {
        let mut missing = Vec::new();
        if api_key.is_empty() { missing.push("api_key"); }
        if secret.is_empty() { missing.push("secret"); }
        if passphrase.is_empty() { missing.push("passphrase"); }
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "error": format!("Missing credentials: {}", missing.join(", ")),
            })),
        )
            .into_response();
    }

    // Build a client with a real User-Agent — some CDNs block the default reqwest UA.
    let client = match reqwest::Client::builder()
        .user_agent("trader-claw/0.1 (+https://github.com/Trader-Claw-Labs/trader-claw)")
        .timeout(std::time::Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "status": "error",
                    "error": format!("Failed to build HTTP client: {e}"),
                })),
            ).into_response();
        }
    };

    // Step 1 — connectivity (with one retry on transient network errors).
    async fn send_with_retry(
        req_builder: impl Fn() -> reqwest::RequestBuilder,
        attempts: u32,
    ) -> std::result::Result<reqwest::Response, String> {
        let mut last_err = String::from("unknown network error");
        for i in 0..attempts {
            match req_builder().send().await {
                Ok(r) => return Ok(r),
                Err(e) => {
                    last_err = format!("{e}");
                    if i + 1 < attempts {
                        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    }
                }
            }
        }
        Err(last_err)
    }

    let ping = send_with_retry(
        || {
            client
                .get("https://clob.polymarket.com/markets?limit=1")
                .timeout(std::time::Duration::from_secs(10))
        },
        2,
    )
    .await;
    match ping {
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "status": "error",
                    "error": format!(
                        "Cannot reach Polymarket CLOB public endpoint: {e}. \
                         Check your internet connection or firewall."
                    ),
                })),
            ).into_response();
        }
        Ok(r) if !r.status().is_success() => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "status": "error",
                    "error": format!("Polymarket CLOB returned {} on public ping", r.status()),
                })),
            ).into_response();
        }
        _ => {}
    }

    // Step 2 — L2 auth probe on /auth/api-keys with all three secret strategies.
    //         If /auth/api-keys is unreachable, fall back to POST /order with a dummy
    //         body (401 on bad auth, 400/422 on valid auth with bad order — both tell us
    //         whether auth succeeded).
    use polymarket_trader::auth::{create_l2_headers_with_strategy, PolyCredentials, SecretDecodeStrategy};
    let creds = PolyCredentials {
        api_key: api_key.clone(),
        secret: secret.clone(),
        passphrase: passphrase.clone(),
        wallet_address: wallet_address.clone().to_lowercase(),
        private_key: None,
        is_builder: stored.is_builder.unwrap_or(false),
        proxy_address: stored.proxy_address.clone().filter(|k| !k.is_empty()).map(|s| s.to_lowercase()),
        signature_type: stored.signature_type.clone().filter(|k| !k.is_empty()),
    };

    /// Probe result: http status (0 = network error), response body, error detail.
    async fn probe_get(
        client: &reqwest::Client,
        creds: &PolyCredentials,
        path: &str,
        strategy: SecretDecodeStrategy,
    ) -> (u16, String) {
        let headers = create_l2_headers_with_strategy(creds, "GET", path, None, strategy);
        for attempt in 0..2u32 {
            let mut req = client
                .get(format!("https://clob.polymarket.com{}", path))
                .timeout(std::time::Duration::from_secs(12));
            for (k, v) in &headers {
                req = req.header(k.as_str(), v.as_str());
            }
            match req.send().await {
                Ok(r) => {
                    let s = r.status().as_u16();
                    let b = r.text().await.unwrap_or_default();
                    return (s, b);
                }
                Err(e) => {
                    if attempt == 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                        continue;
                    }
                    return (0, format!("network error: {e}"));
                }
            }
        }
        (0, String::from("network error (unreachable)"))
    }

    /// Fallback probe: POST /order with a dummy token id.
    /// - 401: auth failed.
    /// - Any other status: auth succeeded (the server looked past the headers
    ///   and started validating the order payload), so we treat it as "OK".
    async fn probe_post_order(
        client: &reqwest::Client,
        creds: &PolyCredentials,
        strategy: SecretDecodeStrategy,
    ) -> (u16, String) {
        let body = r#"{"order":{"tokenID":"0","price":"0.5","size":"1","side":"BUY","type":"GTC"},"owner":""}"#;
        let headers = create_l2_headers_with_strategy(creds, "POST", "/order", Some(body), strategy);
        for attempt in 0..2u32 {
            let mut req = client
                .post("https://clob.polymarket.com/order")
                .header("Content-Type", "application/json")
                .body(body)
                .timeout(std::time::Duration::from_secs(12));
            for (k, v) in &headers {
                req = req.header(k.as_str(), v.as_str());
            }
            match req.send().await {
                Ok(r) => {
                    let s = r.status().as_u16();
                    let b = r.text().await.unwrap_or_default();
                    return (s, b);
                }
                Err(e) => {
                    if attempt == 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                        continue;
                    }
                    return (0, format!("network error: {e}"));
                }
            }
        }
        (0, String::from("network error (unreachable)"))
    }

    let strategies = [
        ("Base64", SecretDecodeStrategy::Base64),
        ("Raw", SecretDecodeStrategy::Raw),
        ("Hex", SecretDecodeStrategy::Hex),
    ];

    let mut last_status: u16 = 0;
    let mut last_body = String::new();
    let mut last_strategy_name: &str = "Base64";
    let mut last_endpoint: &str = "/auth/api-keys";
    let mut all_network_errors = true;
    for (name, strat) in strategies {
        // First try the dedicated L2 list endpoint.
        let (mut status, mut body_text) = probe_get(&client, &creds, "/auth/api-keys", strat).await;
        last_endpoint = "/auth/api-keys";
        // If /auth/api-keys is unreachable (0) OR returns 401/403, fall back to
        // POST /order. The GET endpoint may reject valid Builder Key credentials
        // while POST /order accurately reflects whether L2 auth headers are correct.
        if status == 0 || status == 401 || status == 403 {
            let (s2, b2) = probe_post_order(&client, &creds, strat).await;
            if s2 != 0 {
                status = s2;
                body_text = b2;
                last_endpoint = "POST /order";
                // POST /order returns non-401 on auth-ok + order-invalid.
                if status != 401 && status != 403 {
                    let preview: String = body_text.chars().take(240).collect();
                    let api_key_head: String = api_key.chars().take(4).collect();
                    let api_key_tail: String = api_key.chars().rev().take(4).collect::<Vec<_>>()
                        .into_iter().rev().collect();
                    return Json(serde_json::json!({
                        "status": "ok",
                        "message": format!(
                            "Polymarket CLOB authenticated OK (fallback POST /order → HTTP {status}, \
                             auth passed; secret decoded as {name})."
                        ),
                        "strategy": name,
                        "http_status": status,
                        "endpoint": "POST /order",
                        "api_key_preview": format!("{api_key_head}…{api_key_tail}"),
                        "api_key_length": api_key.len(),
                        "secret_length": secret.len(),
                        "passphrase_length": passphrase.len(),
                        "wallet_address": wallet_address,
                        "response_preview": preview,
                    })).into_response();
                }
            }
        }
        last_status = status;
        last_body = body_text;
        last_strategy_name = name;
        if status != 0 {
            all_network_errors = false;
        }
        if (200..300).contains(&status) {
            let preview: String = last_body.chars().take(240).collect();
            let api_key_head: String = api_key.chars().take(4).collect();
            let api_key_tail: String = api_key.chars().rev().take(4).collect::<Vec<_>>()
                .into_iter().rev().collect();
            return Json(serde_json::json!({
                "status": "ok",
                "message": format!(
                    "Polymarket CLOB authenticated OK (HTTP {status}, secret decoded as {name})."
                ),
                "strategy": name,
                "http_status": status,
                "endpoint": last_endpoint,
                "api_key_preview": format!("{api_key_head}…{api_key_tail}"),
                "api_key_length": api_key.len(),
                "secret_length": secret.len(),
                "passphrase_length": passphrase.len(),
                "wallet_address": wallet_address,
                "response_preview": preview,
            })).into_response();
        }
        // Keep probing on 401/403 to see if another encoding works.
        if status != 401 && status != 403 && status != 0 {
            break;
        }
    }

    // If every strategy only produced network errors, report that explicitly
    // instead of leaking an empty "credentials rejected (HTTP 0)" message.
    if all_network_errors {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "status": "error",
                "error": format!(
                    "Cannot reach Polymarket CLOB authenticated endpoints. Last error: {last_body}. \
                     The public /markets endpoint responded but /auth/api-keys and POST /order \
                     both failed — this is almost always a transient network issue, try again."
                ),
                "http_status": 0,
                "endpoint": last_endpoint,
                "response_preview": last_body.chars().take(180).collect::<String>(),
            })),
        ).into_response();
    }

    // All strategies failed — build actionable diagnostics.
    let detail: String = last_body.chars().take(180).collect();
    let api_key_head: String = api_key.chars().take(4).collect();
    let api_key_tail: String = api_key.chars().rev().take(4).collect::<Vec<_>>()
        .into_iter().rev().collect();
    let hint = if last_status == 401 || last_status == 403 {
        "Credentials were rejected with every secret encoding (Base64/Raw/Hex). \
         Most likely the api_key/secret/passphrase trío doesn't belong to the configured wallet. \
         Open the Polymarket page and click 'Regenerate API Credentials' to derive a fresh trío from your private_key."
    } else {
        "Polymarket CLOB returned an unexpected status."
    };
    // IMPORTANT: never return 401/403 to the frontend for Polymarket-side auth
    // failures — the SPA treats any 401 as "gateway Bearer expired", wipes the
    // token and shows the pairing modal. Use 422 instead so the caller can
    // distinguish this from a gateway-auth error.
    let http_code = if last_status == 401 || last_status == 403 {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::BAD_GATEWAY
    };
    (
        http_code,
        Json(serde_json::json!({
            "status": "error",
            "error": format!(
                "Polymarket credentials rejected (HTTP {last_status}). {hint}"
            ),
            "http_status": last_status,
            "endpoint": last_endpoint,
            "last_strategy": last_strategy_name,
            "response_preview": detail,
            "api_key_preview": format!("{api_key_head}…{api_key_tail}"),
            "api_key_length": api_key.len(),
            "secret_length": secret.len(),
            "passphrase_length": passphrase.len(),
            "wallet_address": wallet_address,
        })),
    ).into_response()
}

/// POST /api/polymarket/diagnose-auth — test which secret decoding strategy works.
///
/// Tries Raw, Base64, and Hex secret decoding against real CLOB endpoints.
/// The key test is POST /order with a dummy body: if auth passes we get 400/404,
/// if auth fails we get 401.
#[derive(serde::Deserialize)]
pub struct DiagnoseAuthBody {
    pub api_key: String,
    pub secret: String,
    pub passphrase: String,
    pub wallet_address: String,
    #[serde(default)]
    pub is_builder: Option<bool>,
    #[serde(default)]
    pub proxy_address: Option<String>,
}

pub async fn handle_api_polymarket_diagnose_auth(
    State(_state): State<AppState>,
    Json(body): Json<DiagnoseAuthBody>,
) -> impl IntoResponse {
    use polymarket_trader::auth::{PolyCredentials, SecretDecodeStrategy, create_l2_headers_with_strategy};

    let client = reqwest::Client::new();
    let mut results = Vec::new();

    for strategy in [
        SecretDecodeStrategy::Raw,
        SecretDecodeStrategy::Base64,
        SecretDecodeStrategy::Hex,
    ] {
        let creds = PolyCredentials {
            api_key: body.api_key.clone(),
            secret: body.secret.clone(),
            passphrase: body.passphrase.clone(),
            wallet_address: body.wallet_address.clone(),
            private_key: None,
            is_builder: body.is_builder.unwrap_or(false),
            proxy_address: body.proxy_address.clone().filter(|k| !k.is_empty()),
            signature_type: None,
        };

        // ── Test 1: POST /order with dummy body ──
        // If auth is correct but body is malformed → 400 Bad Request
        // If auth is wrong → 401 Unauthorized
        let order_body = r#"{"order":{"tokenID":"dummy","price":"0.50","size":"1","side":"BUY","type":"GTC"},"owner":""}"#;
        let order_headers = create_l2_headers_with_strategy(
            &creds, "POST", "/order", Some(order_body), strategy);

        let mut order_req = client
            .post("https://clob.polymarket.com/order")
            .header("Content-Type", "application/json")
            .body(order_body)
            .timeout(std::time::Duration::from_secs(10));
        for (k, v) in &order_headers {
            order_req = order_req.header(k.as_str(), v.as_str());
        }

        let order_result = match order_req.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body_text = resp.text().await.unwrap_or_default();
                serde_json::json!({
                    "endpoint": "POST /order",
                    "status": status,
                    "auth_ok": status != 401,
                    "response_preview": body_text.chars().take(200).collect::<String>()
                })
            }
            Err(e) => serde_json::json!({
                "endpoint": "POST /order",
                "status": 0,
                "auth_ok": false,
                "error": format!("Network error: {}", e)
            }),
        };

        // ── Test 2: GET /sampling/simplifiedmarkets ──
        // Mentioned in original code as a lightweight authenticated endpoint
        let samp_headers = create_l2_headers_with_strategy(
            &creds, "GET", "/sampling/simplifiedmarkets", None, strategy);

        let mut samp_req = client
            .get("https://clob.polymarket.com/sampling/simplifiedmarkets")
            .timeout(std::time::Duration::from_secs(10));
        for (k, v) in &samp_headers {
            samp_req = samp_req.header(k.as_str(), v.as_str());
        }

        let samp_result = match samp_req.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body_text = resp.text().await.unwrap_or_default();
                serde_json::json!({
                    "endpoint": "GET /sampling/simplifiedmarkets",
                    "status": status,
                    "auth_ok": status != 401,
                    "response_preview": body_text.chars().take(200).collect::<String>()
                })
            }
            Err(e) => serde_json::json!({
                "endpoint": "GET /sampling/simplifiedmarkets",
                "status": 0,
                "auth_ok": false,
                "error": format!("Network error: {}", e)
            }),
        };

        results.push(serde_json::json!({
            "strategy": format!("{:?}", strategy),
            "tests": [order_result, samp_result],
        }));
    }

    Json(serde_json::json!({
        "wallet_address": body.wallet_address,
        "api_key_prefix": body.api_key.chars().take(8).collect::<String>(),
        "secret_length": body.secret.len(),
        "secret_preview": format!("{}...", &body.secret[..body.secret.len().min(4)]),
        "results": results,
    })).into_response()
}

// ── Setup wizard endpoints ─────────────────────────────────────────────────
//
// The wizard guides a new user through 4 steps:
//   1. POST /api/polymarket/setup/verify-wallet      → verify the PK derives a real EOA on Polygon,
//                                                       check whether it's a smart-account (EIP-7702).
//   2. POST /api/polymarket/setup/detect-proxy       → query Polymarket Data API for the user's
//                                                       Builder/Polymarket proxy address and detect
//                                                       its on-chain type (Safe, EIP-1167, custom).
//   3. POST /api/polymarket/setup/generate-creds     → derive (or create) L2 credentials, save to config.
//   4. (existing) POST /api/polymarket/test          → test order signing flow.

#[derive(serde::Deserialize)]
pub struct SetupVerifyWalletBody {
    pub private_key: String,
}

/// POST /api/polymarket/setup/verify-wallet
///
/// Validates the private key format, derives the EOA address, and inspects on-chain
/// bytecode to detect EIP-7702 smart accounts (which require `signature_type=poly1271`).
pub async fn handle_api_polymarket_setup_verify_wallet(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SetupVerifyWalletBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    let pk_hex = body.private_key.trim().trim_start_matches("0x");
    if pk_hex.len() != 64 || !pk_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "Invalid private key format. Expected 64-character hex (with or without 0x prefix)."
        }))).into_response();
    }

    // Derive the EOA via the polymarket-trader auth helper (which owns the k256 dep).
    // We fail through derive_api_key which uses the same address derivation path.
    let eoa_address = match polymarket_trader::auth::eoa_address_from_pk_hex(pk_hex) {
        Ok(a) => a,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": format!("Invalid private key: {e}")
        }))).into_response(),
    };
    let bytecode = fetch_polygon_bytecode(&eoa_address).await.unwrap_or_default();
    let (account_type, suggested_sig_type) = classify_bytecode(&bytecode);

    Json(serde_json::json!({
        "eoa_address": eoa_address,
        "account_type": account_type,
        "is_smart_account": bytecode.starts_with("0xef0100") || (!bytecode.is_empty() && bytecode != "0x"),
        "suggested_signature_type": suggested_sig_type,
    })).into_response()
}

#[derive(serde::Deserialize)]
pub struct SetupDetectProxyBody {
    pub eoa_address: String,
    /// Optional: if user already has a known proxy address, verify it instead of auto-detecting.
    #[serde(default)]
    pub proxy_address: Option<String>,
}

/// POST /api/polymarket/setup/detect-proxy
///
/// Inspects on-chain bytecode of the candidate proxy address to determine its type
/// (Gnosis Safe, EIP-1167, Polymarket custom proxy, etc.) and the appropriate
/// signature_type for the SDK.
pub async fn handle_api_polymarket_setup_detect_proxy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SetupDetectProxyBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    let proxy_address = match body.proxy_address.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => p.to_string(),
        None => {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "error": "Auto-detection of proxy address requires the user to paste it from the Polymarket dashboard. Provide `proxy_address` in the request body."
            }))).into_response();
        }
    };

    if !proxy_address.starts_with("0x") || proxy_address.len() != 42 {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "Invalid proxy address format. Expected 0x-prefixed 40-hex-char address."
        }))).into_response();
    }

    let bytecode = fetch_polygon_bytecode(&proxy_address).await.unwrap_or_default();
    let (proxy_type, suggested_sig_type) = classify_bytecode(&bytecode);
    let owner_in_bytecode = extract_embedded_owner(&bytecode);

    let owner_match = match (&owner_in_bytecode, body.eoa_address.as_str()) {
        (Some(owner), eoa) if !eoa.is_empty() => owner.to_lowercase() == eoa.to_lowercase(),
        _ => false,
    };

    Json(serde_json::json!({
        "proxy_address": proxy_address,
        "proxy_type": proxy_type,
        "suggested_signature_type": suggested_sig_type,
        "owner_embedded": owner_in_bytecode,
        "owner_matches_eoa": owner_match,
        "is_contract": !bytecode.is_empty() && bytecode != "0x",
    })).into_response()
}

#[derive(serde::Deserialize)]
pub struct SetupGenerateCredsBody {
    pub private_key: String,
    pub wallet_address: String,
    #[serde(default)]
    pub proxy_address: Option<String>,
    #[serde(default)]
    pub signature_type: Option<String>,
    /// "create" → POST /auth/api-key (first time setup)
    /// "derive" → GET /auth/derive-api-key (recover existing creds)
    /// "auto"   → try derive first, fall back to create
    #[serde(default = "default_creds_mode")]
    pub mode: String,
    /// Whether to persist credentials to config.toml. If false, credentials
    /// are returned to the caller without saving (preview mode).
    #[serde(default = "default_persist")]
    pub persist: bool,
    #[serde(default)]
    pub is_builder: Option<bool>,
}

fn default_creds_mode() -> String { "auto".to_string() }
fn default_persist() -> bool { true }

/// POST /api/polymarket/setup/generate-creds
///
/// Generates Polymarket L2 credentials (api_key, secret, passphrase) for the supplied
/// private key. Either creates new (first time) or derives existing.
/// Persists them along with wallet_address, proxy_address and signature_type to config.toml.
pub async fn handle_api_polymarket_setup_generate_creds(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SetupGenerateCredsBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    let pk = body.private_key.trim().trim_start_matches("0x").to_string();
    if pk.len() != 64 || !pk.chars().all(|c| c.is_ascii_hexdigit()) {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "Invalid private key format."
        }))).into_response();
    }

    let mode = body.mode.to_lowercase();
    let mut last_error = String::new();
    let mut method_used = "";
    let creds_result: Option<polymarket_trader::auth::PolyCredentials> = match mode.as_str() {
        "create" => {
            method_used = "create";
            polymarket_trader::auth::setup_credentials(&pk, None).await
                .map_err(|e| last_error = format!("create: {e:#}")).ok()
        }
        "derive" => {
            method_used = "derive";
            polymarket_trader::auth::derive_api_key(&pk).await
                .map_err(|e| last_error = format!("derive: {e:#}")).ok()
        }
        _ => {
            // auto: try derive first (more permissive), fall back to create
            match polymarket_trader::auth::derive_api_key(&pk).await {
                Ok(c) => { method_used = "derive"; Some(c) }
                Err(de) => {
                    last_error = format!("derive: {de:#}");
                    match polymarket_trader::auth::setup_credentials(&pk, None).await {
                        Ok(c) => { method_used = "create"; Some(c) }
                        Err(ce) => {
                            last_error = format!("derive failed ({de:#}); create failed ({ce:#})");
                            None
                        }
                    }
                }
            }
        }
    };

    let creds = match creds_result {
        Some(c) => c,
        None => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({
            "success": false,
            "error": last_error,
        }))).into_response(),
    };

    if body.persist {
        let mut config = state.config.lock().clone();
        let existing_wallets = config.polymarket.wallets.clone();
        config.polymarket = crate::config::schema::PolymarketConfig {
            api_key: Some(creds.api_key.clone()),
            secret: Some(creds.secret.clone()),
            passphrase: Some(creds.passphrase.clone()),
            wallet_address: Some(body.wallet_address.clone()),
            private_key: Some(pk),
            is_builder: body.is_builder.or(config.polymarket.is_builder),
            proxy_address: body.proxy_address.clone().filter(|p| !p.trim().is_empty()),
            signature_type: body.signature_type.clone().filter(|s| !s.trim().is_empty()),
            wallets: existing_wallets,
        };
        if let Err(e) = config.save().await {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                "success": false,
                "error": format!("Failed to save config: {e}"),
            }))).into_response();
        }
        *state.config.lock() = config;
    }

    Json(serde_json::json!({
        "success": true,
        "method_used": method_used,
        "api_key": creds.api_key,
        "api_key_masked": mask_token(&creds.api_key),
        "secret_masked": mask_token(&creds.secret),
        "passphrase_masked": mask_token(&creds.passphrase),
        "wallet_address": creds.wallet_address,
        "persisted": body.persist,
    })).into_response()
}

fn mask_token(s: &str) -> String {
    if s.len() <= 8 { "•".repeat(s.len()) }
    else { format!("{}…{}", &s[..4], &s[s.len()-4..]) }
}

/// Reads bytecode at an address from a public Polygon RPC.
/// Returns "0x" if the address has no code (pure EOA), or the deployed bytecode.
async fn fetch_polygon_bytecode(addr: &str) -> Option<String> {
    let rpcs = ["https://polygon.drpc.org", "https://1rpc.io/matic", "https://polygon-bor-rpc.publicnode.com"];
    let body = serde_json::json!({
        "jsonrpc":"2.0","method":"eth_getCode","id":1,
        "params":[addr.to_lowercase(), "latest"],
    });
    let client = reqwest::Client::new();
    for rpc in rpcs {
        let resp = match client.post(rpc).timeout(std::time::Duration::from_secs(8)).json(&body).send().await {
            Ok(r) => r, Err(_) => continue,
        };
        if !resp.status().is_success() { continue; }
        let json: serde_json::Value = match resp.json().await { Ok(v) => v, Err(_) => continue };
        if let Some(code) = json.get("result").and_then(|v| v.as_str()) {
            return Some(code.to_string());
        }
    }
    None
}

/// Classifies on-chain bytecode and returns (account_type, suggested_signature_type).
fn classify_bytecode(code: &str) -> (&'static str, &'static str) {
    if code.is_empty() || code == "0x" {
        return ("eoa", "eoa");
    }
    if code.starts_with("0xef0100") {
        // EIP-7702 delegate to a smart contract (MetaMask Smart Account, Coinbase Smart Wallet, etc.)
        return ("eip7702_smart_account", "poly1271");
    }
    if code.starts_with("0x363d3d373d3d3d363d73") {
        // Standard EIP-1167 minimal proxy clone (Magic/email Polymarket accounts).
        return ("eip1167_proxy", "proxy");
    }
    if code.starts_with("0x363d3d373d3d363d7f") {
        // Polymarket's custom proxy factory (introduced ~April 2026 for new accounts).
        // Parametrized with the owner EOA in the trailing bytecode.
        return ("polymarket_custom_proxy", "poly1271");
    }
    if code.starts_with("0x6080") {
        // Generic Solidity contract — likely Gnosis Safe or similar.
        return ("contract", "gnosis_safe");
    }
    ("contract", "poly1271")
}

/// Extracts the owner address embedded at the end of a Polymarket custom proxy bytecode.
/// Polymarket's factory templates the owner EOA into the last 20 bytes.
fn extract_embedded_owner(code: &str) -> Option<String> {
    let hex = code.trim_start_matches("0x");
    if hex.len() < 40 { return None; }
    let owner_hex = &hex[hex.len()-40..];
    if owner_hex.chars().all(|c| c == '0') { return None; }
    Some(format!("0x{}", owner_hex))
}

/// POST /api/polymarket/refresh-credentials — derive fresh API credentials via L1 EIP-712 auth.
///
/// Uses the private key from config to sign a ClobAuth message and call
/// POST /auth/api-key on the CLOB. Returns new api_key, secret, passphrase.
/// The old credentials are NOT automatically saved — caller must confirm.
pub async fn handle_api_polymarket_refresh_credentials(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let private_key = {
        let cfg = state.config.lock();
        match cfg.polymarket.private_key.clone().filter(|k| !k.is_empty()) {
            Some(pk) => pk,
            None => {
                return (axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({
                    "error": "No private_key configured in [polymarket] section. L1 auth requires the wallet private key."
                }))).into_response();
            }
        }
    };

    match polymarket_trader::auth::setup_credentials(&private_key, None).await {
        Ok(creds) => {
            Json(serde_json::json!({
                "success": true,
                "api_key": creds.api_key,
                "secret": creds.secret,
                "passphrase": creds.passphrase,
                "wallet_address": creds.wallet_address,
                "note": "These credentials are NOT saved. Copy them into Settings → Config → [polymarket] section."
            })).into_response()
        }
        Err(e) => {
            let err_str = format!("{e:#}");
            (axum::http::StatusCode::BAD_GATEWAY, Json(serde_json::json!({
                "success": false,
                "error": err_str
            }))).into_response()
        }
    }
}

/// GET /api/polymarket/balance — returns the real USDC balance of the
/// configured Polymarket wallet (Polygon RPC + CLOB API, takes the max).
/// Used by the UI to pre-populate the Initial Balance field when switching
/// a runner to live mode.
pub async fn handle_api_polymarket_balance(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let creds = match get_poly_creds(&state) {
        Some(c) if !c.api_key.is_empty() => c,
        _ => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Polymarket credentials not configured. Set api_key, secret, passphrase and wallet_address in Settings → Config."
                })),
            )
                .into_response();
        }
    };

    let client = std::sync::Arc::new(polymarket_trader::orders::ClobClient::new(creds.clone()));

    let api_bal = client.get_api_balance().await.ok();
    let rpc_bal = client.get_balance().await.ok();

    let balance = match (api_bal, rpc_bal) {
        (Some(a), Some(r)) => a.max(r),
        (Some(a), None) => a,
        (None, Some(r)) => r,
        (None, None) => {
            return (
                axum::http::StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "Could not fetch balance from Polymarket CLOB API or Polygon RPC. Check credentials and network."
                })),
            )
                .into_response();
        }
    };

    // Append a balance snapshot (hourly granularity) for the rewards-history diff.
    // Polymarket pays liquidity rewards in USDC directly to the proxy wallet at
    // midnight UTC; there is no public rewards API, so the balance delta across
    // midnight (minus realized fills P&L) is the reward proxy. See docs/GAPS_PLAN.md.
    {
        let snap_path = state.config.lock().workspace_dir
            .join("data").join("balance_snapshots.jsonl");
        let _ = std::fs::create_dir_all(snap_path.parent().unwrap());
        let now = chrono::Utc::now();
        // Only append if the last snapshot is >50 min old (avoids spamming on every poll).
        let should_append = std::fs::read_to_string(&snap_path).ok()
            .and_then(|c| c.lines().last().map(str::to_string))
            .and_then(|l| serde_json::from_str::<serde_json::Value>(&l).ok())
            .and_then(|v| v.get("ts").and_then(|t| t.as_i64()))
            .map(|last_ts| now.timestamp() - last_ts > 3000)
            .unwrap_or(true);
        if should_append {
            let row = serde_json::json!({ "ts": now.timestamp(), "balance": balance });
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&snap_path) {
                use std::io::Write;
                let _ = writeln!(f, "{}", row);
            }
        }
    }

    Json(serde_json::json!({
        "balance": balance,
        "wallet_address": creds.wallet_address,
        "currency": "USDC"
    }))
    .into_response()
}

/// GET /api/rewards/history — daily USDC balance deltas as a reward proxy.
/// Reads balance_snapshots.jsonl (appended on each balance poll) and computes the
/// change across each UTC midnight. Positive deltas not explained by fills ≈ rewards.
pub async fn handle_api_rewards_history(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    // Preferred: the OFFICIAL Polymarket rewards earnings per UTC day (last 7 days),
    // via the authenticated CLOB endpoint. Falls back to the balance-delta proxy below
    // if creds are missing or the API errors.
    if let Some(creds) = get_poly_creds(&state).filter(|c| !c.api_key.is_empty()) {
        let client = polymarket_trader::orders::ClobClient::new(creds);
        let mut official = Vec::new();
        let mut any_ok = false;
        for back in 0..7 {
            let date = (chrono::Utc::now() - chrono::Duration::days(back))
                .format("%Y-%m-%d").to_string();
            match client.get_rewards_earnings(&date).await {
                Ok(usd) => { any_ok = true; official.push(serde_json::json!({ "date": date, "earned_usdc": usd })); }
                Err(_) => {}
            }
        }
        if any_ok {
            let total: f64 = official.iter()
                .filter_map(|r| r.get("earned_usdc").and_then(|v| v.as_f64())).sum();
            return Json(serde_json::json!({
                "status": "official",
                "source": "polymarket /rewards/user/markets",
                "total_7d_usdc": total,
                "history": official,
                "note": "Official Polymarket liquidity-reward earnings per UTC day (paid at midnight UTC)."
            })).into_response();
        }
    }

    let snap_path = state.config.lock().workspace_dir
        .join("data").join("balance_snapshots.jsonl");
    let content = match std::fs::read_to_string(&snap_path) {
        Ok(c) => c,
        Err(_) => return Json(serde_json::json!({
            "status": "no_data",
            "note": "No balance snapshots yet. They accrue each time the Polymarket balance is polled. Daily reward deltas appear after the first UTC midnight crossing."
        })).into_response(),
    };
    let snaps: Vec<(i64, f64)> = content.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| Some((v.get("ts")?.as_i64()?, v.get("balance")?.as_f64()?)))
        .collect();
    if snaps.len() < 2 {
        return Json(serde_json::json!({ "status": "insufficient", "snapshots": snaps.len() })).into_response();
    }
    // Group by UTC day, take first and last balance of each day.
    use std::collections::BTreeMap;
    let mut by_day: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for (ts, bal) in &snaps {
        let day = chrono::DateTime::from_timestamp(*ts, 0)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        by_day.entry(day).and_modify(|e| e.1 = *bal).or_insert((*bal, *bal));
    }
    // Daily delta = (this day's last) - (previous day's last). Cross-midnight change.
    let days: Vec<_> = by_day.into_iter().collect();
    let mut history = Vec::new();
    for i in 1..days.len() {
        let (ref day, (_, last)) = days[i];
        let (_, (_, prev_last)) = &days[i - 1];
        history.push(serde_json::json!({
            "date": day,
            "balance_end": last,
            "delta": last - prev_last,
        }));
    }
    Json(serde_json::json!({
        "status": "ok",
        "history": history,
        "note": "delta = balance change across UTC midnight. Positive deltas not explained by fill P&L ≈ liquidity rewards. No public rewards API exists; this is the onchain proxy."
    })).into_response()
}

// ── Polymarket orders / positions helpers ────────────────────────

fn clean_optional(s: &Option<String>) -> Option<String> {
    s.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

fn legacy_poly_profile(pm: &crate::config::schema::PolymarketConfig) -> Option<crate::config::schema::PolymarketWalletProfile> {
    let has_any = clean_optional(&pm.api_key).is_some()
        || clean_optional(&pm.wallet_address).is_some()
        || clean_optional(&pm.private_key).is_some();
    if !has_any { return None; }
    Some(crate::config::schema::PolymarketWalletProfile {
        id: "default".to_string(),
        label: "Default Polymarket wallet".to_string(),
        api_key: pm.api_key.clone(),
        secret: pm.secret.clone(),
        passphrase: pm.passphrase.clone(),
        wallet_address: pm.wallet_address.clone(),
        private_key: pm.private_key.clone(),
        is_builder: pm.is_builder,
        proxy_address: pm.proxy_address.clone(),
        signature_type: pm.signature_type.clone(),
    })
}

fn list_poly_wallet_profiles(pm: &crate::config::schema::PolymarketConfig) -> Vec<crate::config::schema::PolymarketWalletProfile> {
    let mut out = Vec::new();
    if let Some(p) = legacy_poly_profile(pm) { out.push(p); }
    for (idx, profile) in pm.wallets.iter().enumerate() {
        let mut p = profile.clone();
        if p.id.trim().is_empty() { p.id = format!("wallet-{}", idx + 1); }
        if p.label.trim().is_empty() { p.label = p.id.clone(); }
        out.push(p);
    }
    out
}

fn resolve_poly_wallet_profile(
    pm: &crate::config::schema::PolymarketConfig,
    selected_id: Option<&str>,
) -> Option<crate::config::schema::PolymarketWalletProfile> {
    let profiles = list_poly_wallet_profiles(pm);
    let selected = selected_id.map(str::trim).filter(|s| !s.is_empty());
    if let Some(id) = selected {
        if let Some(p) = profiles.iter().find(|p| p.id == id).cloned() {
            return Some(p);
        }
    }
    profiles.into_iter().find(|p| p.id == "default").or_else(|| pm.wallets.first().cloned())
}

fn poly_creds_from_profile(profile: &crate::config::schema::PolymarketWalletProfile) -> Option<polymarket_trader::auth::PolyCredentials> {
    let api_key = clean_optional(&profile.api_key)?;
    Some(polymarket_trader::auth::PolyCredentials {
        api_key,
        secret: clean_optional(&profile.secret).unwrap_or_default(),
        passphrase: clean_optional(&profile.passphrase).unwrap_or_default(),
        wallet_address: clean_optional(&profile.wallet_address).unwrap_or_default().to_lowercase(),
        private_key: clean_optional(&profile.private_key),
        is_builder: profile.is_builder.unwrap_or(false),
        proxy_address: clean_optional(&profile.proxy_address).map(|s| s.to_lowercase()),
        signature_type: clean_optional(&profile.signature_type),
    })
}

fn get_poly_creds_for_wallet(state: &AppState, wallet_profile_id: Option<&str>) -> Option<polymarket_trader::auth::PolyCredentials> {
    let cfg = state.config.lock();
    let profile = resolve_poly_wallet_profile(&cfg.polymarket, wallet_profile_id)?;
    poly_creds_from_profile(&profile)
}

fn get_poly_creds(state: &AppState) -> Option<polymarket_trader::auth::PolyCredentials> {
    get_poly_creds_for_wallet(state, None)
}

fn get_poly_wallet_profile(state: &AppState, wallet_profile_id: Option<&str>) -> Option<crate::config::schema::PolymarketWalletProfile> {
    let cfg = state.config.lock();
    resolve_poly_wallet_profile(&cfg.polymarket, wallet_profile_id)
}

fn get_poly_wallet_address(state: &AppState) -> Option<String> {
    get_poly_wallet_profile(state, None).and_then(|p| clean_optional(&p.wallet_address))
}

/// Resolve (yes_token_id, no_token_id) for a fixed market by condition_id via the CLOB.
/// Used by the rewards_maker engine which quotes one market, not a rolling series.
async fn resolve_tokens_for_condition(condition_id: &str) -> anyhow::Result<(String, String)> {
    let url = format!("https://clob.polymarket.com/markets/{condition_id}");
    let v: serde_json::Value = reqwest::Client::new()
        .get(&url).header("User-Agent", "trader-claw")
        .timeout(std::time::Duration::from_secs(15))
        .send().await?.json().await?;
    let tokens = v.get("tokens").and_then(|t| t.as_array())
        .ok_or_else(|| anyhow::anyhow!("no tokens for condition {condition_id}"))?;
    let mut yes = None; let mut no = None;
    for t in tokens {
        let outcome = t.get("outcome").and_then(|o| o.as_str()).unwrap_or("").to_lowercase();
        let tid = t.get("token_id").and_then(|i| i.as_str()).map(str::to_string);
        if outcome == "yes" { yes = tid; } else if outcome == "no" { no = tid; }
    }
    Ok((yes.ok_or_else(|| anyhow::anyhow!("no YES token"))?,
        no.ok_or_else(|| anyhow::anyhow!("no NO token"))?))
}

async fn resolve_live_token_ids(series_id: Option<&str>) -> anyhow::Result<(String, String)> {
    let sid = series_id.ok_or_else(|| anyhow::anyhow!("Please select a Market Series before starting live mode."))?;
    let series = crate::tools::series::builtin_series()
        .into_iter()
        .find(|s| s.id == sid)
        .ok_or_else(|| anyhow::anyhow!("Selected Market Series is not recognized. Please refresh and choose again."))?;

    let slug_prefix = series.slug_prefix;
    let cadence = series.cadence.as_str();
    let seconds = match cadence {
        "1m" => 60,
        "5m" => 300,
        "15m" => 900,
        "1h" => 3600,
        _ => 300, // fallback to 5m
    };

    let now_utc = chrono::Utc::now();
    let now_secs = now_utc.timestamp() as u64;
    let windows = calculate_resolution_windows(now_secs, seconds);

    tracing::info!(
        "[resolve_live_token_ids] UTC now: {}, window_ts: {}, series_id={}, slug_prefix={}",
        now_utc.to_rfc3339(), windows[0], sid, slug_prefix
    );

    let mut last_err = anyhow::anyhow!("No active market found");

    for ts in &windows {
        let target_slug = format!("{}-{}", slug_prefix, ts);
        match polymarket_trader::markets::get_market(&target_slug).await {
            Ok(m) => {
                if !m.yes_token_id.trim().is_empty() && !m.no_token_id.trim().is_empty() {
                    tracing::info!("[resolve_live_token_ids] Resolved YES={} NO={} from slug {}", m.yes_token_id, m.no_token_id, target_slug);
                    return Ok((m.yes_token_id, m.no_token_id));
                }
            }
            Err(e) => {
                last_err = e;
                tracing::debug!("[resolve_live_token_ids] Slug {} not available: {}", target_slug, last_err);
            }
        }
    }

    anyhow::bail!(
        "No active market with both YES and NO tokens found for the selected series right now. (Tried windows around {}). Error: {}",
        windows[0], last_err
    );
}

/// Pure helper for resolution window selection.
/// Returns [current, next, previous, next+1, previous-1] windows.
fn calculate_resolution_windows(now_secs: u64, seconds: u64) -> Vec<u64> {
    let window_ts = now_secs - (now_secs % seconds);
    vec![
        window_ts,
        window_ts + seconds,
        window_ts - seconds,
        window_ts + (2 * seconds),
        window_ts - (2 * seconds),
    ]
}

/// Query USDC balance on Polygon via public RPC for the EOA + (optional) proxy
/// wallet, summing across native USDC and bridged USDC.e contracts.
/// Polymarket may hold trading funds in either contract / either wallet
/// depending on how the user funded the account.
async fn ensure_live_wallet_has_min_balance(
    wallet_address: &str,
    proxy_address: Option<&str>,
    min_usdc: f64,
) -> anyhow::Result<()> {
    const USDC_E:    &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174"; // bridged
    const USDC_NATIVE: &str = "0x3c499c542cef5e3811e1192ce70d8cc03d5c3359";
    const PUSD:      &str = "0xc011a7e12a19f7b1f670d46f03b03f3342e82dfb";
    let tokens = [("USDC.e", USDC_E), ("USDC", USDC_NATIVE), ("pUSD", PUSD)];

    let mut addrs: Vec<String> = Vec::with_capacity(2);
    for raw in [Some(wallet_address), proxy_address].into_iter().flatten() {
        let clean = raw.trim().trim_start_matches("0x").to_lowercase();
        if clean.len() == 40 && !addrs.contains(&clean) {
            addrs.push(clean);
        }
    }
    if addrs.is_empty() { return Ok(()); }

    let rpcs = ["https://polygon.drpc.org", "https://1rpc.io/matic", "https://polygon-bor-rpc.publicnode.com"];
    let client = reqwest::Client::new();

    async fn read_balance(client: &reqwest::Client, rpcs: &[&str], token: &str, addr: &str) -> anyhow::Result<u128> {
        let calldata = format!("0x70a08231000000000000000000000000{}", addr);
        let body = serde_json::json!({
            "jsonrpc":"2.0","method":"eth_call","id":1,
            "params":[{"to":token,"data":calldata},"latest"],
        });
        let mut last_err: Option<anyhow::Error> = None;
        for rpc in rpcs {
            let resp = match client.post(*rpc).timeout(std::time::Duration::from_secs(8)).json(&body).send().await {
                Ok(r) => r, Err(e) => { last_err = Some(e.into()); continue; }
            };
            if !resp.status().is_success() {
                last_err = Some(anyhow::anyhow!("RPC {} → {}", rpc, resp.status()));
                continue;
            }
            let json: serde_json::Value = match resp.json().await {
                Ok(v) => v, Err(e) => { last_err = Some(e.into()); continue; }
            };
            let Some(hex) = json.get("result").and_then(|v| v.as_str()) else {
                last_err = Some(anyhow::anyhow!("missing result: {}", json)); continue;
            };
            return u128::from_str_radix(hex.trim_start_matches("0x"), 16)
                .map_err(|e| anyhow::anyhow!("hex parse: {e}"));
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("all RPCs failed")))
    }

    let mut total_units: u128 = 0;
    let mut all_failed = true;
    let mut breakdown: Vec<String> = Vec::new();
    for addr in &addrs {
        for (label, token) in tokens.iter() {
            match read_balance(&client, &rpcs, token, addr).await {
                Ok(units) => {
                    all_failed = false;
                    total_units = total_units.saturating_add(units);
                    if units > 0 {
                        breakdown.push(format!("0x{}…/{}: ${:.4}", &addr[..6], label, (units as f64) / 1_000_000.0));
                    }
                }
                Err(e) => {
                    tracing::debug!("balance check 0x{}/{} failed: {}", &addr[..6], label, e);
                }
            }
        }
    }

    if all_failed {
        tracing::warn!("Skipping Polymarket wallet balance pre-check (all RPCs unreachable)");
        return Ok(());
    }

    let total_usdc = (total_units as f64) / 1_000_000.0;
    tracing::info!(
        "Polymarket wallet balance check: total ${:.4} ({})",
        total_usdc,
        if breakdown.is_empty() { "all zero".to_string() } else { breakdown.join(", ") }
    );

    if total_usdc + 1e-6 < min_usdc {
        anyhow::bail!(
            "Insufficient wallet balance for live mode. Required at least ${:.2} USDC/pUSD, detected ${:.2} across EOA+proxy on USDC + USDC.e + pUSD.",
            min_usdc, total_usdc
        );
    }
    Ok(())
}

fn friendly_live_error(e: &str) -> String {
    if e.contains("Market Series") || e.contains("No active Polymarket market") {
        format!("{e} Open Live Strategies and select a supported built-in BTC/ETH series.")
    } else if e.contains("wallet balance") || e.contains("Insufficient wallet balance") {
        format!("{e} Please fund your Polymarket wallet and try again.")
    } else if e.contains("wallet address") {
        "Live mode requires a Polymarket wallet address. Go to Settings → Config and set polymarket.wallet_address.".to_string()
    } else if e.contains("Invalid api key") || e.contains("Polymarket credentials rejected") {
        format!("{e} Go to the Polymarket page and click \"Regenerate API credentials\" so they match your wallet.")
    } else {
        e.to_string()
    }
}

/// Pre-flight validation of Polymarket L2 credentials.
///
/// Calls an authenticated CLOB endpoint (`GET /auth/api-keys`) with the supplied
/// credentials. On 401 tries all 3 secret-decoding strategies (Base64 / Raw / Hex)
/// to distinguish "wrong key" from "wrong encoding", and reports which part is
/// likely bad so the user can act.
async fn validate_live_poly_credentials(
    creds: &polymarket_trader::auth::PolyCredentials,
) -> anyhow::Result<()> {
    use polymarket_trader::auth::{create_l2_headers_with_strategy, SecretDecodeStrategy};

    let path = "/auth/api-keys";
    let client = reqwest::Client::new();

    async fn try_get(
        client: &reqwest::Client,
        creds: &polymarket_trader::auth::PolyCredentials,
        path: &str,
        strategy: SecretDecodeStrategy,
    ) -> (u16, String) {
        let headers = create_l2_headers_with_strategy(creds, "GET", path, None, strategy);
        let mut req = client
            .get(format!("https://clob.polymarket.com{}", path))
            .timeout(std::time::Duration::from_secs(10));
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                (status, body)
            }
            Err(e) => (0, format!("network error: {e}")),
        }
    }

    async fn try_post_order(
        client: &reqwest::Client,
        creds: &polymarket_trader::auth::PolyCredentials,
        strategy: SecretDecodeStrategy,
    ) -> (u16, String) {
        let body = r#"{"order":{"tokenID":"0","price":"0.5","size":"1","side":"BUY","type":"GTC"},"owner":""}"#;
        let headers = create_l2_headers_with_strategy(creds, "POST", "/order", Some(body), strategy);
        let mut req = client
            .post("https://clob.polymarket.com/order")
            .header("Content-Type", "application/json")
            .body(body)
            .timeout(std::time::Duration::from_secs(10));
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                (status, body)
            }
            Err(e) => (0, format!("network error: {e}")),
        }
    }

    // Start with the library default (Base64). This is what real order calls will use.
    let (status, body) = try_get(&client, creds, path, SecretDecodeStrategy::Base64).await;
    if status >= 200 && status < 300 {
        return Ok(());
    }
    if status == 0 {
        anyhow::bail!("Cannot reach Polymarket CLOB to validate credentials: {body}");
    }
    if status != 401 && status != 403 {
        tracing::warn!(
            "Polymarket credential pre-flight returned {status} (non-auth) — continuing: {body}"
        );
        return Ok(());
    }

    // GET /auth/api-keys returned 401/403. For Builder Keys this endpoint may
    // reject valid credentials while POST /order correctly reflects auth status.
    // Try the fallback before giving up.
    let (post_status, _post_body) = try_post_order(&client, creds, SecretDecodeStrategy::Base64).await;
    if post_status != 0 && post_status != 401 && post_status != 403 {
        tracing::info!(
            "Polymarket credential pre-flight: GET /auth/api-keys → 401, \
             but POST /order → {post_status} (auth OK, order rejected). Continuing."
        );
        return Ok(());
    }

    // 401 / 403 on both endpoints — probe the other two strategies for diagnostics.
    let (raw_status, _) = try_get(&client, creds, path, SecretDecodeStrategy::Raw).await;
    let (hex_status, _) = try_get(&client, creds, path, SecretDecodeStrategy::Hex).await;

    let detail = body.chars().take(120).collect::<String>();
    let api_key_preview = creds.api_key.chars().take(4).collect::<String>();
    let api_key_tail: String = creds
        .api_key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let secret_len = creds.secret.len();
    let passphrase_len = creds.passphrase.len();

    let hint = if raw_status == 200 || raw_status == 204 {
        " Hint: the secret works when treated as raw bytes — you may have pasted it already-decoded."
    } else if hex_status == 200 || hex_status == 204 {
        " Hint: the secret works as hex — you may have pasted a hex-encoded value by mistake."
    } else {
        " Tip: open the Polymarket page and click 'Regenerate API Credentials'; it derives a fresh L2 trío for the wallet of your saved private_key."
    };

    anyhow::bail!(
        "Polymarket credentials rejected ({status}): {detail}. api_key='{api_key_preview}…{api_key_tail}' (len={}), secret_len={secret_len}, passphrase_len={passphrase_len}, wallet={}.{hint}",
        creds.api_key.len(),
        creds.wallet_address,
    );
}

/// GET /api/polymarket/positions — open positions
pub async fn handle_api_polymarket_positions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let wallet_address = {
        let cfg = state.config.lock();
        cfg.polymarket.wallet_address.clone()
    };
    let address = match wallet_address {
        Some(a) if !a.is_empty() => a,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "No Polymarket wallet address configured. Set it via /api/polymarket/configure."})),
            )
                .into_response();
        }
    };
    // Positions endpoint is public — query by user address
    let url = format!("https://clob.polymarket.com/positions?user={address}");
    match reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<serde_json::Value>().await {
                Ok(data) => Json(serde_json::json!({"positions": data})).into_response(),
                Err(e) => (
                    StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                    Json(serde_json::json!({"error": e.to_string()})),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/polymarket/orders — open CLOB orders
pub async fn handle_api_polymarket_orders(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let creds = match get_poly_creds(&state) {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Polymarket not configured. Call /api/polymarket/configure first."})),
            )
                .into_response();
        }
    };
    let client = polymarket_trader::orders::ClobClient::new(creds);
    match client.get_open_orders().await {
        Ok(orders) => {
            let data: Vec<serde_json::Value> = orders
                .iter()
                .map(|o| serde_json::json!({
                    "id": o.id,
                    "token_id": o.token_id,
                    "side": o.side,
                    "price": o.price,
                    "size": o.size,
                    "status": o.status,
                }))
                .collect();
            Json(serde_json::json!({"orders": data})).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct PlaceOrderBody {
    pub token_id: String,
    pub side: String, // "buy" or "sell"
    pub price: f64,
    pub size: Option<f64>,
    pub amount_usdc: Option<f64>,
    pub order_type: Option<String>, // "limit" | "market"
}

/// POST /api/polymarket/order — place a limit or market order
pub async fn handle_api_polymarket_order_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PlaceOrderBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let creds = match get_poly_creds(&state) {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Polymarket not configured."})),
            )
                .into_response();
        }
    };
    let side = if body.side.to_lowercase() == "sell" {
        polymarket_trader::orders::Side::Sell
    } else {
        polymarket_trader::orders::Side::Buy
    };
    let client = polymarket_trader::orders::ClobClient::new(creds);
    let order_type = body.order_type.as_deref().unwrap_or("limit");

    let result = if order_type == "market" {
        let amount = match body.amount_usdc {
            Some(a) => a,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "amount_usdc required for market orders"})),
                )
                    .into_response();
            }
        };
        client.create_market_order(&body.token_id, side, amount, body.price).await
    } else {
        let size = match body.size {
            Some(s) => s,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "size required for limit orders"})),
                )
                    .into_response();
            }
        };
        client.create_limit_order(&body.token_id, side, body.price, size).await
    };

    match result {
        Ok(resp) => Json(serde_json::json!({
            "order_id": resp.order_id,
            "status": resp.status,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct RewardsQuoteBody {
    pub yes_token_id: String,
    pub no_token_id: String,
    /// USD size per side (each leg). Must be >= the market's min_size to earn rewards.
    pub size_usd: f64,
    /// Cents inside the eligible band to rest each side (e.g. 1.0). Lower = closer to mid
    /// (higher reward score, more adverse-selection risk).
    #[serde(default = "default_offset_c")]
    pub offset_c: f64,
}
fn default_offset_c() -> f64 { 1.0 }

/// POST /api/rewards/quote — place a two-sided maker quote for liquidity rewards.
/// Reads the live YES mid, posts a BUY YES limit at (mid - offset) and a BUY NO limit
/// at (1 - mid - offset), each sized `size_usd`. Both rest in the book (maker) → eligible
/// for rewards. Returns both order ids. This is the "add position" action for /rewards.
pub async fn handle_api_rewards_quote(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RewardsQuoteBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let creds = match get_poly_creds(&state) {
        Some(c) if !c.api_key.is_empty() => c,
        _ => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "Polymarket credentials not configured (Settings → Config)."
        }))).into_response(),
    };
    let client = polymarket_trader::orders::ClobClient::new(creds);
    use polymarket_trader::orders::Side;

    // Live YES mid from the CLOB.
    let yes_ask = polymarket_trader::markets::get_market_price(&body.yes_token_id).await.unwrap_or(0.0);
    if !(yes_ask > 0.02 && yes_ask < 0.98) {
        return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({
            "error": format!("Could not read a sane YES price (got {yes_ask:.3}). Try again.")
        }))).into_response();
    }
    let off = (body.offset_c / 100.0).max(0.0);
    let yes_px = ((yes_ask - off) * 100.0).round() / 100.0;
    let no_px = ((1.0 - yes_ask - off) * 100.0).round() / 100.0;
    if yes_px <= 0.01 || no_px <= 0.01 {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "Computed quote price <= 0.01; offset too large for this market."
        }))).into_response();
    }
    let yes_shares = (body.size_usd / yes_px).round();
    let no_shares = (body.size_usd / no_px).round();

    let yes_res = client.create_limit_order(&body.yes_token_id, Side::Buy, yes_px, yes_shares).await;
    let no_res = client.create_limit_order(&body.no_token_id, Side::Buy, no_px, no_shares).await;

    let fmt = |r: &anyhow::Result<polymarket_trader::orders::OrderResponse>| match r {
        Ok(resp) => serde_json::json!({ "order_id": resp.order_id, "status": resp.status }),
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    };
    Json(serde_json::json!({
        "yes": { "price": yes_px, "shares": yes_shares, "result": fmt(&yes_res) },
        "no": { "price": no_px, "shares": no_shares, "result": fmt(&no_res) },
        "both_placed": yes_res.is_ok() && no_res.is_ok(),
    })).into_response()
}

/// DELETE /api/polymarket/order/:id — cancel an open order
pub async fn handle_api_polymarket_order_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let creds = match get_poly_creds(&state) {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Polymarket not configured."})),
            )
                .into_response();
        }
    };
    let client = polymarket_trader::orders::ClobClient::new(creds);
    match client.cancel_order(&order_id).await {
        Ok(()) => Json(serde_json::json!({"status": "cancelled", "order_id": order_id}))
            .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/polymarket/markets/resolve?slug=... — resolve a Polymarket slug to condition_id
pub async fn handle_api_polymarket_resolve_slug(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let slug = match query.get("slug") {
        Some(s) => s.clone(),
        None => {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "missing slug parameter"}))).into_response();
        }
    };

    let url = format!("https://gamma-api.polymarket.com/markets?slug={}", slug);
    match reqwest::Client::new().get(&url).timeout(std::time::Duration::from_secs(10)).send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>().await {
                Ok(data) => {
                    if let Some(market) = data.as_array().and_then(|a| a.first()) {
                        Json(serde_json::json!({
                            "condition_id": market.get("conditionId").and_then(|v| v.as_str()),
                            "question": market.get("question").and_then(|v| v.as_str()),
                            "slug": market.get("slug").and_then(|v| v.as_str()),
                        })).into_response()
                    } else {
                        (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "market not found"}))).into_response()
                    }
                }
                Err(e) => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
            }
        }
        Ok(resp) => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": format!("Gamma API error: {}", resp.status())}))).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

/// GET /api/channels/telegram/configure — return current Telegram config (token masked)
pub async fn handle_api_telegram_get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let config = state.config.lock();
    match config.channels_config.telegram.as_ref() {
        None => Json(serde_json::json!({ "configured": false })).into_response(),
        Some(tg) => {
            // Mask all but the last 4 chars of the token so the UI can show "configured"
            // without leaking the secret.
            let masked = if tg.bot_token.len() > 4 {
                format!("{}…{}", &tg.bot_token[..8].replace(|_: char| true, "*"), &tg.bot_token[tg.bot_token.len()-4..])
            } else {
                "****".to_string()
            };
            Json(serde_json::json!({
                "configured": true,
                "bot_token_masked": masked,
                "allowed_users": tg.allowed_users,
            }))
            .into_response()
        }
    }
}

/// POST /api/channels/telegram/configure — save bot token and allowed users
pub async fn handle_api_telegram_configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TelegramConfigBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let mut config = state.config.lock().clone();

    // "__keep__" sentinel means "don't change the existing token"
    let keep_existing = body.bot_token.as_deref() == Some("__keep__");

    let token = if keep_existing {
        // Preserve the existing token
        config
            .channels_config
            .telegram
            .as_ref()
            .map(|t| t.bot_token.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| {
                return String::new(); // will be caught below
            })
    } else {
        match body.bot_token.as_deref() {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "bot_token is required"})),
                )
                    .into_response();
            }
        }
    };

    if token.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "No bot_token configured yet — provide a token"})),
        )
            .into_response();
    }

    // Update or create the telegram config, preserving existing fields
    let existing = config.channels_config.telegram.take();
    let mut tg = existing.unwrap_or_else(|| crate::config::schema::TelegramConfig {
        bot_token: String::new(),
        allowed_users: Vec::new(),
        stream_mode: Default::default(),
        draft_update_interval_ms: 1500,
        interrupt_on_new_message: false,
        mention_only: false,
        chat_id: None,
    });
    tg.bot_token = token;
    if let Some(users) = body.allowed_users {
        tg.allowed_users = users.into_iter().filter(|u| !u.is_empty()).collect();
    }
    config.channels_config.telegram = Some(tg);

    if let Err(e) = config.save().await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to save config: {e}")})),
        )
            .into_response();
    }

    *state.config.lock() = config;
    Json(serde_json::json!({"status": "ok", "message": "Telegram bot configured"})).into_response()
}

/// POST /api/channels/telegram/test — verify bot token against Telegram's getMe API
pub async fn handle_api_telegram_test(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let token = {
        let config = state.config.lock();
        config
            .channels_config
            .telegram
            .as_ref()
            .map(|t| t.bot_token.clone())
            .filter(|t| !t.is_empty())
    };

    let token = match token {
        Some(t) => t,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Telegram not configured — save a bot token first"})),
            )
                .into_response();
        }
    };

    // Call Telegram Bot API to verify the token
    let url = format!("https://api.telegram.org/bot{token}/getMe");
    match reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await
    {
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": format!("Network error: {e}")})),
        )
            .into_response(),
        Ok(resp) => {
            let status = resp.status();
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            if status.is_success() && body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                let username = body
                    .pointer("/result/username")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let name = body
                    .pointer("/result/first_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                Json(serde_json::json!({
                    "status": "ok",
                    "message": format!("Connected — bot @{username} ({name}) is active"),
                    "bot_username": username,
                    "bot_name": name,
                }))
                .into_response()
            } else {
                let description = body
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("invalid token");
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": format!("Telegram rejected the token: {description}")})),
                )
                    .into_response()
            }
        }
    }
}

/// GET /api/channels/telegram/messages — last 50 received messages (for dashboard)
pub async fn handle_api_telegram_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let messages = crate::channels::telegram::recent_telegram_messages();
    Json(serde_json::json!({ "messages": messages })).into_response()
}

/// POST /api/chat — HTTP fallback for chat (when WS unavailable)
pub async fn handle_api_chat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ChatBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let session = body.session_id.clone().unwrap_or_else(|| "default".to_string());
    let config = state.config.lock().clone();

    match crate::agent::process_message(config, &body.message).await {
        Ok(text) => Json(serde_json::json!({
            "session_id": session,
            "response": text,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Agent error: {e}")})),
        )
            .into_response(),
    }
}

// ── Helpers ─────────────────────────────────────────────────────

fn is_masked_secret(value: &str) -> bool {
    value == MASKED_SECRET
}

fn mask_optional_secret(value: &mut Option<String>) {
    if value.is_some() {
        *value = Some(MASKED_SECRET.to_string());
    }
}

fn mask_required_secret(value: &mut String) {
    if !value.is_empty() {
        *value = MASKED_SECRET.to_string();
    }
}

fn mask_vec_secrets(values: &mut [String]) {
    for value in values.iter_mut() {
        if !value.is_empty() {
            *value = MASKED_SECRET.to_string();
        }
    }
}

#[allow(clippy::ref_option)]
fn restore_optional_secret(value: &mut Option<String>, current: &Option<String>) {
    if value.as_deref().is_some_and(is_masked_secret) {
        *value = current.clone();
    }
}

fn restore_required_secret(value: &mut String, current: &str) {
    if is_masked_secret(value) {
        *value = current.to_string();
    }
}

fn restore_vec_secrets(values: &mut [String], current: &[String]) {
    for (idx, value) in values.iter_mut().enumerate() {
        if is_masked_secret(value) {
            if let Some(existing) = current.get(idx) {
                *value = existing.clone();
            }
        }
    }
}

fn normalize_route_field(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn model_route_identity_matches(
    incoming: &crate::config::schema::ModelRouteConfig,
    current: &crate::config::schema::ModelRouteConfig,
) -> bool {
    normalize_route_field(&incoming.hint) == normalize_route_field(&current.hint)
        && normalize_route_field(&incoming.provider) == normalize_route_field(&current.provider)
        && normalize_route_field(&incoming.model) == normalize_route_field(&current.model)
}

fn model_route_provider_model_matches(
    incoming: &crate::config::schema::ModelRouteConfig,
    current: &crate::config::schema::ModelRouteConfig,
) -> bool {
    normalize_route_field(&incoming.provider) == normalize_route_field(&current.provider)
        && normalize_route_field(&incoming.model) == normalize_route_field(&current.model)
}

fn embedding_route_identity_matches(
    incoming: &crate::config::schema::EmbeddingRouteConfig,
    current: &crate::config::schema::EmbeddingRouteConfig,
) -> bool {
    normalize_route_field(&incoming.hint) == normalize_route_field(&current.hint)
        && normalize_route_field(&incoming.provider) == normalize_route_field(&current.provider)
        && normalize_route_field(&incoming.model) == normalize_route_field(&current.model)
}

fn embedding_route_provider_model_matches(
    incoming: &crate::config::schema::EmbeddingRouteConfig,
    current: &crate::config::schema::EmbeddingRouteConfig,
) -> bool {
    normalize_route_field(&incoming.provider) == normalize_route_field(&current.provider)
        && normalize_route_field(&incoming.model) == normalize_route_field(&current.model)
}

fn restore_model_route_api_keys(
    incoming: &mut [crate::config::schema::ModelRouteConfig],
    current: &[crate::config::schema::ModelRouteConfig],
) {
    let mut used_current = vec![false; current.len()];
    for incoming_route in incoming {
        if !incoming_route
            .api_key
            .as_deref()
            .is_some_and(is_masked_secret)
        {
            continue;
        }

        let exact_match_idx = current
            .iter()
            .enumerate()
            .find(|(idx, current_route)| {
                !used_current[*idx] && model_route_identity_matches(incoming_route, current_route)
            })
            .map(|(idx, _)| idx);

        let match_idx = exact_match_idx.or_else(|| {
            current
                .iter()
                .enumerate()
                .find(|(idx, current_route)| {
                    !used_current[*idx]
                        && model_route_provider_model_matches(incoming_route, current_route)
                })
                .map(|(idx, _)| idx)
        });

        if let Some(idx) = match_idx {
            used_current[idx] = true;
            incoming_route.api_key = current[idx].api_key.clone();
        } else {
            // Never persist UI placeholders to disk when no safe restore target exists.
            incoming_route.api_key = None;
        }
    }
}

fn restore_embedding_route_api_keys(
    incoming: &mut [crate::config::schema::EmbeddingRouteConfig],
    current: &[crate::config::schema::EmbeddingRouteConfig],
) {
    let mut used_current = vec![false; current.len()];
    for incoming_route in incoming {
        if !incoming_route
            .api_key
            .as_deref()
            .is_some_and(is_masked_secret)
        {
            continue;
        }

        let exact_match_idx = current
            .iter()
            .enumerate()
            .find(|(idx, current_route)| {
                !used_current[*idx]
                    && embedding_route_identity_matches(incoming_route, current_route)
            })
            .map(|(idx, _)| idx);

        let match_idx = exact_match_idx.or_else(|| {
            current
                .iter()
                .enumerate()
                .find(|(idx, current_route)| {
                    !used_current[*idx]
                        && embedding_route_provider_model_matches(incoming_route, current_route)
                })
                .map(|(idx, _)| idx)
        });

        if let Some(idx) = match_idx {
            used_current[idx] = true;
            incoming_route.api_key = current[idx].api_key.clone();
        } else {
            // Never persist UI placeholders to disk when no safe restore target exists.
            incoming_route.api_key = None;
        }
    }
}

fn mask_sensitive_fields(config: &crate::config::Config) -> crate::config::Config {
    let mut masked = config.clone();

    mask_optional_secret(&mut masked.api_key);
    mask_vec_secrets(&mut masked.reliability.api_keys);
    mask_vec_secrets(&mut masked.gateway.paired_tokens);
    mask_optional_secret(&mut masked.composio.api_key);
    mask_optional_secret(&mut masked.browser.computer_use.api_key);
    mask_optional_secret(&mut masked.web_search.brave_api_key);
    mask_optional_secret(&mut masked.storage.provider.config.db_url);
    mask_optional_secret(&mut masked.memory.qdrant.api_key);
    if let Some(cloudflare) = masked.tunnel.cloudflare.as_mut() {
        mask_required_secret(&mut cloudflare.token);
    }
    if let Some(ngrok) = masked.tunnel.ngrok.as_mut() {
        mask_required_secret(&mut ngrok.auth_token);
    }

    for agent in masked.agents.values_mut() {
        mask_optional_secret(&mut agent.api_key);
    }
    for route in &mut masked.model_routes {
        mask_optional_secret(&mut route.api_key);
    }
    for route in &mut masked.embedding_routes {
        mask_optional_secret(&mut route.api_key);
    }

    if let Some(telegram) = masked.channels_config.telegram.as_mut() {
        mask_required_secret(&mut telegram.bot_token);
    }
    if let Some(discord) = masked.channels_config.discord.as_mut() {
        mask_required_secret(&mut discord.bot_token);
    }
    if let Some(slack) = masked.channels_config.slack.as_mut() {
        mask_required_secret(&mut slack.bot_token);
        mask_optional_secret(&mut slack.app_token);
    }
    if let Some(mattermost) = masked.channels_config.mattermost.as_mut() {
        mask_required_secret(&mut mattermost.bot_token);
    }
    if let Some(webhook) = masked.channels_config.webhook.as_mut() {
        mask_optional_secret(&mut webhook.secret);
    }
    if let Some(matrix) = masked.channels_config.matrix.as_mut() {
        mask_required_secret(&mut matrix.access_token);
    }
    if let Some(whatsapp) = masked.channels_config.whatsapp.as_mut() {
        mask_optional_secret(&mut whatsapp.access_token);
        mask_optional_secret(&mut whatsapp.app_secret);
        mask_optional_secret(&mut whatsapp.verify_token);
    }
    if let Some(linq) = masked.channels_config.linq.as_mut() {
        mask_required_secret(&mut linq.api_token);
        mask_optional_secret(&mut linq.signing_secret);
    }
    if let Some(nextcloud) = masked.channels_config.nextcloud_talk.as_mut() {
        mask_required_secret(&mut nextcloud.app_token);
        mask_optional_secret(&mut nextcloud.webhook_secret);
    }
    if let Some(wati) = masked.channels_config.wati.as_mut() {
        mask_required_secret(&mut wati.api_token);
    }
    if let Some(irc) = masked.channels_config.irc.as_mut() {
        mask_optional_secret(&mut irc.server_password);
        mask_optional_secret(&mut irc.nickserv_password);
        mask_optional_secret(&mut irc.sasl_password);
    }
    if let Some(lark) = masked.channels_config.lark.as_mut() {
        mask_required_secret(&mut lark.app_secret);
        mask_optional_secret(&mut lark.encrypt_key);
        mask_optional_secret(&mut lark.verification_token);
    }
    if let Some(feishu) = masked.channels_config.feishu.as_mut() {
        mask_required_secret(&mut feishu.app_secret);
        mask_optional_secret(&mut feishu.encrypt_key);
        mask_optional_secret(&mut feishu.verification_token);
    }
    if let Some(dingtalk) = masked.channels_config.dingtalk.as_mut() {
        mask_required_secret(&mut dingtalk.client_secret);
    }
    if let Some(qq) = masked.channels_config.qq.as_mut() {
        mask_required_secret(&mut qq.app_secret);
    }
    if let Some(nostr) = masked.channels_config.nostr.as_mut() {
        mask_required_secret(&mut nostr.private_key);
    }
    masked
}

fn restore_masked_sensitive_fields(
    incoming: &mut crate::config::Config,
    current: &crate::config::Config,
) {
    restore_optional_secret(&mut incoming.api_key, &current.api_key);
    restore_vec_secrets(
        &mut incoming.gateway.paired_tokens,
        &current.gateway.paired_tokens,
    );
    restore_vec_secrets(
        &mut incoming.reliability.api_keys,
        &current.reliability.api_keys,
    );
    restore_optional_secret(&mut incoming.composio.api_key, &current.composio.api_key);
    restore_optional_secret(
        &mut incoming.browser.computer_use.api_key,
        &current.browser.computer_use.api_key,
    );
    restore_optional_secret(
        &mut incoming.web_search.brave_api_key,
        &current.web_search.brave_api_key,
    );
    restore_optional_secret(
        &mut incoming.storage.provider.config.db_url,
        &current.storage.provider.config.db_url,
    );
    restore_optional_secret(
        &mut incoming.memory.qdrant.api_key,
        &current.memory.qdrant.api_key,
    );
    if let (Some(incoming_tunnel), Some(current_tunnel)) = (
        incoming.tunnel.cloudflare.as_mut(),
        current.tunnel.cloudflare.as_ref(),
    ) {
        restore_required_secret(&mut incoming_tunnel.token, &current_tunnel.token);
    }
    if let (Some(incoming_tunnel), Some(current_tunnel)) = (
        incoming.tunnel.ngrok.as_mut(),
        current.tunnel.ngrok.as_ref(),
    ) {
        restore_required_secret(&mut incoming_tunnel.auth_token, &current_tunnel.auth_token);
    }

    for (name, agent) in &mut incoming.agents {
        if let Some(current_agent) = current.agents.get(name) {
            restore_optional_secret(&mut agent.api_key, &current_agent.api_key);
        }
    }
    restore_model_route_api_keys(&mut incoming.model_routes, &current.model_routes);
    restore_embedding_route_api_keys(&mut incoming.embedding_routes, &current.embedding_routes);

    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.telegram.as_mut(),
        current.channels_config.telegram.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.bot_token, &current_ch.bot_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.discord.as_mut(),
        current.channels_config.discord.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.bot_token, &current_ch.bot_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.slack.as_mut(),
        current.channels_config.slack.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.bot_token, &current_ch.bot_token);
        restore_optional_secret(&mut incoming_ch.app_token, &current_ch.app_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.mattermost.as_mut(),
        current.channels_config.mattermost.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.bot_token, &current_ch.bot_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.webhook.as_mut(),
        current.channels_config.webhook.as_ref(),
    ) {
        restore_optional_secret(&mut incoming_ch.secret, &current_ch.secret);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.matrix.as_mut(),
        current.channels_config.matrix.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.access_token, &current_ch.access_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.whatsapp.as_mut(),
        current.channels_config.whatsapp.as_ref(),
    ) {
        restore_optional_secret(&mut incoming_ch.access_token, &current_ch.access_token);
        restore_optional_secret(&mut incoming_ch.app_secret, &current_ch.app_secret);
        restore_optional_secret(&mut incoming_ch.verify_token, &current_ch.verify_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.linq.as_mut(),
        current.channels_config.linq.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.api_token, &current_ch.api_token);
        restore_optional_secret(&mut incoming_ch.signing_secret, &current_ch.signing_secret);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.nextcloud_talk.as_mut(),
        current.channels_config.nextcloud_talk.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.app_token, &current_ch.app_token);
        restore_optional_secret(&mut incoming_ch.webhook_secret, &current_ch.webhook_secret);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.wati.as_mut(),
        current.channels_config.wati.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.api_token, &current_ch.api_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.irc.as_mut(),
        current.channels_config.irc.as_ref(),
    ) {
        restore_optional_secret(
            &mut incoming_ch.server_password,
            &current_ch.server_password,
        );
        restore_optional_secret(
            &mut incoming_ch.nickserv_password,
            &current_ch.nickserv_password,
        );
        restore_optional_secret(&mut incoming_ch.sasl_password, &current_ch.sasl_password);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.lark.as_mut(),
        current.channels_config.lark.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.app_secret, &current_ch.app_secret);
        restore_optional_secret(&mut incoming_ch.encrypt_key, &current_ch.encrypt_key);
        restore_optional_secret(
            &mut incoming_ch.verification_token,
            &current_ch.verification_token,
        );
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.feishu.as_mut(),
        current.channels_config.feishu.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.app_secret, &current_ch.app_secret);
        restore_optional_secret(&mut incoming_ch.encrypt_key, &current_ch.encrypt_key);
        restore_optional_secret(
            &mut incoming_ch.verification_token,
            &current_ch.verification_token,
        );
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.dingtalk.as_mut(),
        current.channels_config.dingtalk.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.client_secret, &current_ch.client_secret);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.qq.as_mut(),
        current.channels_config.qq.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.app_secret, &current_ch.app_secret);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.nostr.as_mut(),
        current.channels_config.nostr.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.private_key, &current_ch.private_key);
    }
}

fn hydrate_config_for_save(
    mut incoming: crate::config::Config,
    current: &crate::config::Config,
) -> crate::config::Config {
    restore_masked_sensitive_fields(&mut incoming, current);
    // These are runtime-computed fields skipped from TOML serialization.
    incoming.config_path = current.config_path.clone();
    incoming.workspace_dir = current.workspace_dir.clone();
    incoming
}

// ── Agent Skills ─────────────────────────────────────────────────

/// GET /api/skills — list installed agent skills
pub async fn handle_api_skills_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let workspace_dir = state.config.lock().workspace_dir.clone();
    let skills = crate::skills::load_skills(&workspace_dir);

    let skills_json: Vec<serde_json::Value> = skills
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "description": s.description,
                "version": s.version,
                "author": s.author,
                "tags": s.tags,
                "location": s.location.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
            })
        })
        .collect();

    Json(serde_json::json!({ "skills": skills_json })).into_response()
}

#[derive(serde::Deserialize)]
pub struct SkillContentQuery {
    pub path: String,
}

/// GET /api/skills/content — read skill file content
pub async fn handle_api_skills_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SkillContentQuery>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let path = std::path::Path::new(&query.path);

    // Security: only allow reading from skills directory
    let workspace_dir = state.config.lock().workspace_dir.clone();
    let skills_dir = workspace_dir.join("skills");

    // Also check open-skills directory if enabled
    let is_valid_path = path.starts_with(&skills_dir)
        || path.ancestors().any(|p| p.file_name().map(|n| n == "skills").unwrap_or(false));

    if !is_valid_path {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Can only read files from skills directories" })),
        )
            .into_response();
    }

    if !path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Skill file not found" })),
        )
            .into_response();
    }

    // Only allow reading SKILL.md or SKILL.toml files
    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if filename != "SKILL.md" && filename != "SKILL.toml" {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Can only read SKILL.md or SKILL.toml files" })),
        )
            .into_response();
    }

    match std::fs::read_to_string(path) {
        Ok(content) => Json(serde_json::json!({ "content": content })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to read: {e}") })),
        )
            .into_response(),
    }
}

// ── TradingView Screener ─────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct ScreenerQuery {
    pub symbols: Option<String>,
}

/// GET /api/tradingview/scan — fetch indicators from TradingView Screener
pub async fn handle_api_tradingview_scan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<ScreenerQuery>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let symbols_str = params.symbols.unwrap_or_default();
    let explicit_symbols: Vec<&str> = symbols_str
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    // When no symbols given, fetch top-20 by volume live instead of hardcoded list
    let data_result = if explicit_symbols.is_empty() {
        market_analyzer::screener::fetch_top_by_volume(20).await
    } else {
        market_analyzer::screener::fetch_indicators(&explicit_symbols).await
    };

    match data_result {
        Ok(data) => {
            let rows: Vec<serde_json::Value> = data
                .into_iter()
                .map(|d| {
                    serde_json::json!({
                        "symbol": d.symbol,
                        "exchange": d.exchange,
                        "price": d.price,
                        "volume": d.volume,
                        "rsi": d.rsi,
                        "macd": d.macd,
                        "macd_signal": d.macd_signal,
                    })
                })
                .collect();
            Json(serde_json::json!({
                "data": rows,
                "fetched_at": chrono::Utc::now().to_rfc3339(),
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("TradingView screener error: {e}") })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct RewardsQuery {
    /// Max markets to return (after ranking). Default 50.
    pub limit: Option<usize>,
    /// Include toxic (crypto / UP-DOWN / hourly) markets. Default false.
    pub include_toxic: Option<bool>,
    /// CLOB pages to fetch (1000 markets each). Default 3.
    pub max_pages: Option<usize>,
}

/// GET /api/rewards/markets — scan Polymarket liquidity-reward markets, ranked by a
/// reward/adverse-selection-risk heuristic. Toxic (crypto/UP-DOWN/hourly) markets are
/// excluded by default since their fast-moving fair value makes maker quoting unsafe.
pub async fn handle_api_rewards_markets(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<RewardsQuery>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let max_pages = params.max_pages.unwrap_or(3).clamp(1, 20);
    let include_toxic = params.include_toxic.unwrap_or(false);
    let limit = params.limit.unwrap_or(50).min(500);

    match market_analyzer::rewards::scan_reward_markets(max_pages).await {
        Ok(mut markets) => {
            let total = markets.len();
            let toxic_count = markets.iter().filter(|m| m.is_toxic).count();
            if !include_toxic {
                markets.retain(|m| !m.is_toxic);
            }
            let eligible = markets.len();
            markets.truncate(limit);
            Json(serde_json::json!({
                "markets": markets,
                "total_incentivized": total,
                "toxic_excluded": toxic_count,
                "eligible": eligible,
                "fetched_at": chrono::Utc::now().to_rfc3339(),
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("Rewards scan error: {e}") })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct ArbScanQuery {
    /// How many open events to scan (each pulls books for its markets). Default 80.
    pub max_events: Option<usize>,
    /// Minimum gross edge in cents to surface. Default 0.5¢ (filters CLOB minutiae).
    pub threshold_c: Option<f64>,
}

/// GET /api/arb/scan — structural-arb scanner across multi-market events.
/// Detects (1) disjoint bucket sets where YES asks sum < $1 (or NO asks < $1) and (2)
/// monotonicity violations on date-ordered cumulative strikes. Both are PRE-FEE gross
/// edges; verify book depth and NegRisk fees before posting.
pub async fn handle_api_arb_scan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<ArbScanQuery>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let max_events = params.max_events.unwrap_or(80).clamp(5, 500);
    let threshold_c = params.threshold_c.unwrap_or(0.5);
    match market_analyzer::arb_scanner::scan_arb_opportunities(max_events, threshold_c).await {
        Ok(candidates) => Json(serde_json::json!({
            "candidates": candidates,
            "scanned_events_max": max_events,
            "threshold_c": threshold_c,
            "fetched_at": chrono::Utc::now().to_rfc3339(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("Arb scan error: {e}") })),
        )
            .into_response(),
    }
}

/// GET /api/capital/allocator — honest capital allocation across runners.
/// For each runner with ≥30 official-resolution trades, runs the 3-leg validator and
/// assigns weight ∝ max(0, EV/trade) / CI_width (fractional-Kelly-style: reward scaled by
/// confidence). Runners that fail validation (NO_EDGE / INSUFFICIENT) get weight 0 — capital
/// flows ONLY to statistically-confirmed edge, not to raw P&L. This closes the loop between
/// validation and sizing (the Rec-3 allocator).
pub async fn handle_api_capital_allocator(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let runners = state.strategy_runner.list();

    #[derive(serde::Serialize)]
    struct Alloc {
        name: String,
        n: usize,
        verdict: String,
        ev_per_trade_pct: f64,
        ci_lo: f64,
        ci_hi: f64,
        raw_weight: f64,
        weight_pct: f64,
    }

    let mut allocs: Vec<Alloc> = Vec::new();
    for r in &runners {
        // Aggregate this runner's official-resolution trades
        let (mut entries, mut wons) = (Vec::new(), Vec::new());
        if let Some(res) = r.result.as_ref() {
            for o in &res.live_orders {
                if o.resolution_source.as_deref() != Some("polymarket") { continue; }
                let Some(rs) = o.result.as_deref() else { continue; };
                let p = crate::strategy_runner::settle_price(o.entry_price, o.fill_price);
                if p > 0.01 && p < 0.99 { entries.push(p); wons.push(rs.trim_end_matches('*') == "WIN"); }
            }
        }
        if entries.len() < 30 { continue; }
        let v = crate::tools::edge_validator::validate(&entries, &wons, 3000);
        // Weight only confirmed edge; reward ∝ EV, confidence ∝ 1/CI-width.
        let ci_width = (v.ci_hi - v.ci_lo).max(1.0);
        let raw_weight = if v.verdict == "EDGE" && v.ev_per_trade_pct > 0.0 {
            (v.ev_per_trade_pct / ci_width).max(0.0)
        } else { 0.0 };
        allocs.push(Alloc {
            name: r.config.name.clone(),
            n: v.n,
            verdict: v.verdict.clone(),
            ev_per_trade_pct: v.ev_per_trade_pct,
            ci_lo: v.ci_lo,
            ci_hi: v.ci_hi,
            raw_weight,
            weight_pct: 0.0,
        });
    }
    // Normalize weights to percentages
    let total: f64 = allocs.iter().map(|a| a.raw_weight).sum();
    if total > 0.0 {
        for a in &mut allocs { a.weight_pct = a.raw_weight / total * 100.0; }
    }
    allocs.sort_by(|a, b| b.weight_pct.partial_cmp(&a.weight_pct).unwrap_or(std::cmp::Ordering::Equal));

    Json(serde_json::json!({
        "allocations": allocs,
        "note": "weight ∝ validated EV / CI-width. Only EDGE-verdict runners get capital. \
                 NO_EDGE/INSUFFICIENT → 0%. This sizes on confirmed edge, never raw P&L.",
    })).into_response()
}

/// GET /api/fear-index/status — returns the most recent fear index reading and
/// z-score from the rolling 2h window, computed from the Python collector log.
/// Acts as a defensive gate signal: z > 2 → pause rewards-maker quotes.
pub async fn handle_api_fear_index_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let log_path = state.config.lock().workspace_dir
        .join("data").join("fear_index.jsonl");
    let content = match std::fs::read_to_string(&log_path) {
        Ok(c) => c,
        Err(_) => return Json(serde_json::json!({
            "status": "no_data",
            "note": "Fear Index collector not running. Start with: python3 scripts/ml/fear_index.py --collect --cluster politics --hours 24"
        })).into_response(),
    };
    let rows: Vec<serde_json::Value> = content.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if rows.is_empty() {
        return Json(serde_json::json!({ "status": "no_data" })).into_response();
    }
    // Latest row + rolling 2h z-score
    let latest = rows.last().unwrap();
    let ts_now = latest.get("ts").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let two_hours_ago = ts_now - 7200.0;
    let window: Vec<f64> = rows.iter()
        .filter_map(|r| {
            let ts = r.get("ts")?.as_f64()?;
            if ts >= two_hours_ago { r.get("index")?.as_f64() } else { None }
        })
        .collect();
    let (z_score, state_label) = if window.len() >= 10 {
        let mu = window.iter().sum::<f64>() / window.len() as f64;
        let sd = (window.iter().map(|x| (x - mu).powi(2)).sum::<f64>() / window.len() as f64).sqrt();
        let idx = latest.get("index").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let z = if sd > 1e-9 { (idx - mu) / sd } else { 0.0 };
        let lbl = if z > 2.0 { "FEAR_SPIKE" } else if z > 1.0 { "ELEVATED" } else { "CALM" };
        (z, lbl)
    } else {
        (0.0, "INSUFFICIENT_DATA")
    };
    Json(serde_json::json!({
        "status": state_label,
        "z_score": (z_score * 100.0).round() / 100.0,
        "index": latest.get("index"),
        "cluster": latest.get("cluster"),
        "n_window_samples": window.len(),
        "latest_ts": ts_now as i64,
        "gate_recommendation": if z_score > 2.0 { "PAUSE quotes" } else { "OK to quote" },
    })).into_response()
}

// ── Backtesting ──────────────────────────────────────────────────

/// GET /api/backtest/scripts — list .rhai files in /scripts/
pub async fn handle_api_backtest_scripts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let scripts_dir = state.config.lock().workspace_dir.join("scripts");
    // Create the directory if it doesn't exist yet
    let _ = std::fs::create_dir_all(&scripts_dir);
    let mut scripts = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&scripts_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rhai") {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let path_str = path.to_string_lossy().to_string();

                // Read first comment line as fallback description
                let file_description = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|content| {
                        content
                            .lines()
                            .next()
                            .map(|l| l.trim_start_matches("//").trim().to_string())
                    })
                    .filter(|s| !s.is_empty());

                // Read meta file for description and stats
                let meta_path = path.with_extension("rhai.meta.json");
                let meta: serde_json::Value = std::fs::read_to_string(&meta_path)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_else(|| serde_json::json!({}));

                // Prefer meta description over file comment
                let description = meta.get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or(file_description);

                let last_run_stats = meta.get("last_run_stats").cloned();

                scripts.push(serde_json::json!({
                    "name": name,
                    "path": path_str,
                    "description": description,
                    "last_run_stats": last_run_stats,
                }));
            }
        }
    }

    Json(serde_json::json!({ "scripts": scripts })).into_response()
}

/// GET /api/backtest/series — list all built-in (and future user-defined) recurring market series
pub async fn handle_api_backtest_series(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let series = crate::tools::series::builtin_series();
    Json(serde_json::json!({ "series": series })).into_response()
}

/// GET /api/backtest/tick-slugs — list available CLOB 1 HZ tick slugs with date coverage.
///
/// Returns slugs that have at least one JSONL file under `data/ticks/<slug>/`.
/// Each entry includes the slug name, available dates, and total tick count.
pub async fn handle_api_backtest_tick_slugs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let workspace_dir = state.config.lock().workspace_dir.clone();
    let slugs = crate::tools::backtest::list_tick_slugs(&workspace_dir);
    let response: Vec<serde_json::Value> = slugs.into_iter().map(|(slug, dates, tick_count)| {
        serde_json::json!({
            "slug": slug,
            "dates": dates,
            "tick_count": tick_count,
            "from_date": dates.first().cloned().unwrap_or_default(),
            "to_date": dates.last().cloned().unwrap_or_default(),
        })
    }).collect();
    Json(serde_json::json!({ "slugs": response })).into_response()
}

/// GET /api/backtest/event-slugs — list available clob_events stream slugs.
///
/// Returns slugs with at least one `.jsonl.gz` under `data/events/<slug>/`,
/// each with its date coverage. Feeds the `clob_events` market type in the UI.
pub async fn handle_api_backtest_event_slugs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let workspace_dir = state.config.lock().workspace_dir.clone();
    let slugs = crate::tools::backtest::list_event_slugs(&workspace_dir);
    let response: Vec<serde_json::Value> = slugs.into_iter().map(|(slug, dates, _)| {
        serde_json::json!({
            "slug": slug,
            "dates": dates,
            "from_date": dates.first().cloned().unwrap_or_default(),
            "to_date": dates.last().cloned().unwrap_or_default(),
        })
    }).collect();
    Json(serde_json::json!({ "slugs": response })).into_response()
}

#[derive(serde::Deserialize)]
pub struct BacktestRunBody {
    /// Rhai script path — required for rhai_candle, ignored for other engine kinds.
    #[serde(default)]
    pub script: String,
    #[serde(default = "default_market_type")]
    pub market_type: String,
    pub symbol: String,
    #[serde(default = "default_interval")]
    pub interval: String,
    pub from_date: String,
    pub to_date: String,
    pub initial_balance: f64,
    pub fee_pct: f64,
    /// Optional series identifier u2014 if provided, overrides symbol/interval/resolution_logic
    pub series_id: Option<String>,
    /// Resolution logic override: "price_up" | "threshold_above" | "threshold_below"
    pub resolution_logic: Option<String>,
    /// Threshold for threshold_above/below resolution (e.g. 25.0 for u00b0C)
    pub threshold: Option<f64>,
    /// Maximum stake per trade in USD — enforces Polymarket per-market liquidity limits.
    /// Polymarket recurring 5-min binary windows have ~$500-$3,000 liquidity each.
    /// Default (None) = no cap (use for crypto backtests).
    pub max_position_usd: Option<f64>,
    /// Maximum entry price threshold. If the current price (crypto) or token price
    /// (binary) exceeds this value, the trade/bet is skipped.
    pub max_entry_price: Option<f64>,
    /// Position sizing mode: 'fixed' = fixed USD amount, 'percent' = cap fraction of balance.
    pub sizing_mode: Option<String>,
    /// Sizing value: USD amount for fixed mode, or max fraction (0.0-1.0) for percent mode.
    pub sizing_value: Option<f64>,
    /// Price mode for Polymarket binary entry: 'historical' = real scraped price,
    /// 'mid' = average of buy/sell (mid-price).
    pub price_mode: Option<String>,
    /// Hour gate: only trade during these UTC hours (0-23). Empty = no restriction.
    #[serde(default)]
    pub allowed_hours: Vec<u8>,
    /// Spread guard: skip windows where CLOB spread at decision time exceeds this fraction.
    /// Mirrors the live runner's max_spread_pct gate (default 3% = 0.03).
    /// Only applied in archive_candles mode (real bid/ask required). None = use default 3%.
    #[serde(default)]
    pub max_spread_pct: Option<f64>,
    /// RV floor: skip windows where BTC 1h realized-vol < this value. 0 = disabled.
    #[serde(default)]
    pub rv_min_btc: Option<f64>,
    /// Engine kind for strategy-core engines. When set to anything other than
    /// "rhai_candle" (or absent), the Rhai script path is ignored and the engine
    /// is driven directly by the normalised Binance OHLCV feed.
    #[serde(default)]
    pub kind: Option<String>,
    /// Per-engine tunable parameters from the UI EngineParamsForm.
    /// Merged over each engine's default config at runtime.
    #[serde(default)]
    pub engine_params: Option<serde_json::Value>,
    // ── Guardrail parameters (mirrored from RunnerConfig) ─────────────────────
    /// Maximum kelly multiplier applied to script kelly_size. Default 1.5.
    #[serde(default)]
    pub kelly_size_cap: Option<f64>,
    /// Minimum entry price: skip bets when token ask < this value. Default 0.05.
    #[serde(default)]
    pub min_entry_price: Option<f64>,
    /// Auto-stop after N consecutive losses in backtest simulation. 0 = disabled.
    #[serde(default)]
    pub max_consecutive_losses: Option<u32>,
    /// Stop-loss per trade: exit early if token drops this fraction from entry.
    #[serde(default)]
    pub stop_loss_pct: Option<f64>,
    /// Simulated order latency in milliseconds. 0 = same-tick fill (default, backward-compat).
    /// clob_1hz: fill at the first tick where ts_ms >= signal_ts + latency_ms.
    /// archive_candles: shifts the entry price to the tick latency_ms after the decision candle.
    #[serde(default)]
    pub latency_ms: Option<u64>,
    /// Fee model: "pct" = flat fee_pct% (default), "crypto_taker" = 1.8%×p×(1-p).
    #[serde(default)]
    pub fee_model: Option<String>,
    /// clob_events only: feed latency in ms — how late the strategy PERCEIVES each
    /// event (separate from latency_ms, which is the ORDER arrival latency).
    #[serde(default)]
    pub feed_latency_ms: Option<u64>,
    /// When true, run the native 3-leg edge_validator on the backtest's trades and
    /// attach the verdict (CI, random-null, shuffle-null) to the response.
    #[serde(default)]
    pub validate_edge: bool,
    /// Walk-forward split: when set (0.0-1.0), the backtest is run twice — on the
    /// first `walk_forward_train_frac` of the date range (train) and the remainder
    /// (test) — and both result sets are returned so the user can check the edge
    /// holds out-of-sample. None/0 = single full-range run (default).
    #[serde(default)]
    pub walk_forward_train_frac: Option<f64>,
}

fn default_market_type() -> String {
    "crypto".to_string()
}

fn default_interval() -> String {
    "1m".to_string()
}

/// POST /api/backtest/run — run a real backtest using Binance OHLCV + Rhai engine
pub async fn handle_api_backtest_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BacktestRunBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let workspace_dir = state.config.lock().workspace_dir.clone();

    // ── Strategy-core engine backtest (non-Rhai path) ─────────────────────────
    let engine_kind = body.kind.as_deref().unwrap_or("rhai_candle");
    if engine_kind != "rhai_candle" && !engine_kind.is_empty() {
        // MAKER backtest (Fase D): rewards_maker / minting_mm are resting-quote
        // makers — they don't fit the taker on_book path. Replay the ms event
        // stream through the maker fill model (fills, adverse selection, uptime).
        if body.market_type == "clob_events"
            && matches!(engine_kind, "rewards_maker" | "minting_mm")
        {
            let slug = body.symbol.trim();
            if slug.is_empty() {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                    "error": "maker backtest requires an event-stream slug in `symbol` (e.g. btc_5m_ev)."
                }))).into_response();
            }
            // engine_params overrides: offset_cents, reprice_threshold, size_usd.
            let ep = body.engine_params.as_ref();
            let getf = |k: &str, d: f64| ep.and_then(|v| v.get(k)).and_then(|x| x.as_f64()).unwrap_or(d);
            let params = crate::tools::engine_backtest::MakerBacktestParams {
                slug,
                offset_cents: getf("offset_cents", 1.0),
                reprice_threshold: getf("reprice_threshold", 0.02),
                size_usd: body.max_position_usd.unwrap_or_else(|| getf("size_usd", 50.0)),
                from_date: &body.from_date,
                to_date: &body.to_date,
                initial_balance: body.initial_balance,
                workspace_dir: &workspace_dir,
            };
            let metrics = crate::tools::engine_backtest::run_maker_backtest(params).await;
            let all_trades: Vec<serde_json::Value> = metrics.all_trades.iter().map(|t| {
                serde_json::json!({
                    "timestamp": t.timestamp, "side": t.side, "price": t.price,
                    "size": t.size, "pnl": t.pnl, "balance": t.balance,
                })
            }).collect();
            return Json(serde_json::json!({
                "script":           format!("engine:{engine_kind}"),
                "market_type":      "clob_events",
                "symbol":           slug,
                "total_return_pct": metrics.total_return_pct,
                "win_rate_pct":     metrics.win_rate_pct,
                "total_trades":     metrics.total_trades,
                "analysis":         metrics.analysis,
                "maker_stats":      metrics.kv_state,
                "worst_trades":     serde_json::Value::Array(vec![]),
                "all_trades":       all_trades,
            })).into_response();
        }

        // CLOB EVENTS (sub-second): feed the engine a REAL two-sided book from
        // the ms event stream + official resolution (Fase E). Book-driven engines
        // (arb_binary, fair_value, fv_momentum, arb_hedge) via on_book.
        if body.market_type == "clob_events" {
            let slug = body.symbol.trim();
            if slug.is_empty() {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                    "error": "clob_events market type requires an event-stream slug in `symbol` (e.g. btc_5m_ev)."
                }))).into_response();
            }
            let params = crate::tools::engine_backtest::EngineClobEventsParams {
                kind: engine_kind,
                slug,
                threshold: body.threshold,
                engine_params: body.engine_params.clone(),
                from_date: &body.from_date,
                to_date: &body.to_date,
                initial_balance: body.initial_balance,
                fee_pct: body.fee_pct,
                workspace_dir: &workspace_dir,
            };
            let metrics = crate::tools::engine_backtest::run_engine_clob_events_backtest(params).await;
            let all_trades: Vec<serde_json::Value> = metrics.all_trades.iter().map(|t| {
                serde_json::json!({
                    "timestamp": t.timestamp, "side": t.side, "price": t.price,
                    "size": t.size, "pnl": t.pnl, "balance": t.balance,
                })
            }).collect();
            return Json(serde_json::json!({
                "script":           format!("engine:{engine_kind}"),
                "market_type":      "clob_events",
                "symbol":           slug,
                "total_return_pct": metrics.total_return_pct,
                "sharpe_ratio":     metrics.sharpe_ratio,
                "max_drawdown_pct": metrics.max_drawdown_pct,
                "win_rate_pct":     metrics.win_rate_pct,
                "total_trades":     metrics.total_trades,
                "analysis":         metrics.analysis,
                "markets_tested":   metrics.markets_tested,
                "worst_trades":     serde_json::Value::Array(vec![]),
                "all_trades":       all_trades,
            })).into_response();
        }

        // CLOB 1 HZ tick replay: route engine kinds to the recorded-tick
        // backtester so they get real Polymarket YES/NO order book data
        // instead of synthetic candles.
        if body.market_type == "clob_1hz" {
            let slug = body.symbol.trim();
            if slug.is_empty() {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                    "error": "clob_1hz market type requires a tick slug in `symbol` (e.g. btc_5m)."
                }))).into_response();
            }
            let params = crate::tools::engine_backtest::EngineClobBacktestParams {
                kind: engine_kind,
                slug,
                threshold: body.threshold,
                engine_params: body.engine_params.clone(),
                from_date: &body.from_date,
                to_date: &body.to_date,
                initial_balance: body.initial_balance,
                fee_pct: body.fee_pct,
                workspace_dir: &workspace_dir,
            };
            let metrics = crate::tools::engine_backtest::run_engine_clob_1hz_backtest(params).await;
            let all_trades: Vec<serde_json::Value> = metrics.all_trades.iter().map(|t| {
                serde_json::json!({
                    "timestamp": t.timestamp,
                    "side":      t.side,
                    "price":     t.price,
                    "size":      t.size,
                    "pnl":       t.pnl,
                    "balance":   t.balance,
                })
            }).collect();
            return Json(serde_json::json!({
                "script":           format!("engine:{engine_kind}"),
                "market_type":      "clob_1hz",
                "symbol":           slug,
                "total_return_pct": metrics.total_return_pct,
                "sharpe_ratio":     metrics.sharpe_ratio,
                "max_drawdown_pct": metrics.max_drawdown_pct,
                "win_rate_pct":     metrics.win_rate_pct,
                "total_trades":     metrics.total_trades,
                "analysis":         metrics.analysis,
                "markets_tested":   metrics.markets_tested,
                "worst_trades":     serde_json::Value::Array(vec![]),
                "all_trades":       all_trades,
            })).into_response();
        }

        // For engine kinds we accept either a recurring `series_id` (preferred:
        // resolves to the current Polymarket window slug) or a comma-separated
        // legacy `symbol` field. The synthetic backtester only uses the slug
        // as a label so falling back to `series_id` itself is safe when Gamma
        // doesn't have an active window yet.
        let markets: Vec<String> = if let Some(sid) = body.series_id
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            match crate::engines::series_helper::resolve_current_slug(sid).await {
                Ok(slug) => vec![slug],
                Err(_) => vec![sid.to_string()],
            }
        } else {
            body.symbol
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        let params = crate::tools::engine_backtest::EngineBacktestParams {
            kind: engine_kind,
            markets,
            threshold: body.threshold,
            engine_params: body.engine_params.clone(),
            from_date: &body.from_date,
            to_date: &body.to_date,
            initial_balance: body.initial_balance,
            workspace_dir: &workspace_dir,
        };
        let metrics = crate::tools::engine_backtest::run_engine_backtest(params).await;
        return Json(serde_json::json!({
            "script": format!("engine:{engine_kind}"),
            "symbol": body.symbol,
            "total_return_pct":  metrics.total_return_pct,
            "sharpe_ratio":      metrics.sharpe_ratio,
            "max_drawdown_pct":  metrics.max_drawdown_pct,
            "win_rate_pct":      metrics.win_rate_pct,
            "total_trades":      metrics.total_trades,
            "analysis":          metrics.analysis,
            "markets_tested":    metrics.markets_tested,
            "worst_trades":      serde_json::Value::Array(vec![]),
            "all_trades":        serde_json::Value::Array(vec![]),
        })).into_response();
    }

    // Resolve script path: try as-is, then relative to scripts/ dir
    let script_path = {
        let p = std::path::Path::new(&body.script);
        if p.is_absolute() || p.exists() {
            p.to_path_buf()
        } else {
            workspace_dir.join("scripts").join(&body.script)
        }
    };

    if !script_path.exists() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Script not found: {}", script_path.display()) })),
        )
            .into_response();
    }

    // For archive modes the `symbol` field already holds the tick slug (e.g. "btc_5m").
    // Do NOT resolve via series_id — that would replace the slug with a Binance symbol
    // ("BTCUSDT") which doesn't match any tick directory and causes 0 trades.
    let (symbol, interval, resolution_logic, threshold) =
    if body.market_type == "archive_candles" || body.market_type == "clob_1hz" {
        // Use body.symbol directly as the tick slug; ignore series_id for archive modes.
        (body.symbol.clone(), body.interval.clone(),
         body.resolution_logic.clone().unwrap_or_else(|| "price_up".into()),
         body.threshold)
    } else if let Some(ref sid) = body.series_id {
        let series = crate::tools::series::builtin_series();
        if let Some(s) = series.iter().find(|s| s.id == *sid) {
            let rl = match s.resolution_logic {
                crate::tools::series::ResolutionLogic::PriceUp        => "price_up",
                crate::tools::series::ResolutionLogic::ThresholdAbove => "threshold_above",
                crate::tools::series::ResolutionLogic::ThresholdBelow => "threshold_below",
            };
            (s.symbol.clone(), s.cadence.clone(), rl.to_string(), s.threshold)
        } else {
            (body.symbol.clone(), body.interval.clone(),
             body.resolution_logic.clone().unwrap_or_else(|| "price_up".into()),
             body.threshold)
        }
    } else {
        (body.symbol.clone(), body.interval.clone(),
         body.resolution_logic.clone().unwrap_or_else(|| "price_up".into()),
         body.threshold)
    };

    let sizing_mode = body.sizing_mode.as_deref().unwrap_or("percent");
    let sizing_value = body.sizing_value.unwrap_or(1.0);
    let price_mode = body.price_mode.as_deref().unwrap_or("historical");

    let metrics = crate::tools::backtest::run_backtest_engine(
        &script_path,
        &body.market_type,
        &symbol,
        &interval,
        &body.from_date,
        &body.to_date,
        body.initial_balance,
        body.fee_pct,
        &resolution_logic,
        threshold,
        body.max_position_usd,
        body.max_entry_price,
        sizing_mode,
        sizing_value,
        price_mode,
        &workspace_dir,
        &body.allowed_hours,
        body.max_spread_pct,
        body.rv_min_btc,
        body.kelly_size_cap,
        body.min_entry_price,
        body.max_consecutive_losses,
        body.stop_loss_pct,
        body.latency_ms,
        body.fee_model.as_deref(),
        body.feed_latency_ms,
    )
    .await;

    let worst_trades: Vec<serde_json::Value> = metrics
        .worst_trades
        .iter()
        .map(|t| serde_json::json!({
            "timestamp": t.timestamp,
            "side": t.side,
            "price": t.price,
            "pnl": t.pnl,
        }))
        .collect();

    let all_trades: Vec<serde_json::Value> = metrics
        .all_trades
        .iter()
        .map(|t| serde_json::json!({
            "timestamp": t.timestamp,
            "side": t.side,
            "price": t.price,
            "size": t.size,
            "pnl": t.pnl,
            "balance": t.balance,
        }))
        .collect();

    // ── Fase F: optional 3-leg edge validation on the backtest's own trades ──────
    // Uses the shared extractor so it recognizes EVERY engine's side labels
    // (bet_yes/bet_no, yes/no, yes_win/no_loss…) — see BUG-2.
    let edge_validation = if body.validate_edge {
        let (entries, wons) = crate::tools::edge_validator::extract_binary_trades(&metrics.all_trades);
        Some(crate::tools::edge_validator::validate(&entries, &wons, 5000))
    } else {
        None
    };

    Json(serde_json::json!({
        "script": body.script,
        "market_type": body.market_type,
        "edge_validation": edge_validation,
        "symbol": body.symbol,
        "interval": body.interval,
        "from_date": body.from_date,
        "to_date": body.to_date,
        "initial_balance": body.initial_balance,
        "fee_pct": body.fee_pct,
        "series_id": body.series_id,
        "resolution_logic": resolution_logic,
        "threshold": threshold,
        "total_return_pct": metrics.total_return_pct,
        "sharpe_ratio": metrics.sharpe_ratio,
        "max_drawdown_pct": metrics.max_drawdown_pct,
        "win_rate_pct": metrics.win_rate_pct,
        "total_trades": metrics.total_trades,
        "worst_trades": worst_trades,
        "all_trades": all_trades,
        "analysis": metrics.analysis,
        "avg_token_price": metrics.avg_token_price,
        "correct_direction_pct": metrics.correct_direction_pct,
        "break_even_win_rate": metrics.break_even_win_rate,
        "markets_tested": metrics.markets_tested,
        "windows_with_real_price": metrics.windows_with_real_price,
        "windows_with_estimated_price": metrics.windows_with_estimated_price,
        "historical_data_coverage_pct": metrics.historical_data_coverage_pct,
        "recommended_max_stake_usd": metrics.recommended_max_stake_usd,
        "flat_debugs": metrics.flat_debugs,
        "final_balance": body.initial_balance * (1.0 + metrics.total_return_pct / 100.0),
        "latency_ms": body.latency_ms.unwrap_or(0),
        "fee_model": body.fee_model.as_deref().unwrap_or("pct"),
    }))
    .into_response()
}

/// POST /api/backtest/latency-sweep — run the same backtest at multiple latency values.
/// Returns a table of results keyed by latency_ms.
#[derive(serde::Deserialize)]
pub struct LatencySweepBody {
    // Core params (same as BacktestRunBody)
    #[serde(default)]
    pub script: String,
    #[serde(default = "default_market_type")]
    pub market_type: String,
    pub symbol: String,
    #[serde(default = "default_interval")]
    pub interval: String,
    pub from_date: String,
    pub to_date: String,
    pub initial_balance: f64,
    pub fee_pct: f64,
    pub series_id: Option<String>,
    pub resolution_logic: Option<String>,
    pub threshold: Option<f64>,
    pub max_position_usd: Option<f64>,
    pub max_entry_price: Option<f64>,
    pub sizing_mode: Option<String>,
    pub sizing_value: Option<f64>,
    pub price_mode: Option<String>,
    #[serde(default)]
    pub allowed_hours: Vec<u8>,
    #[serde(default)]
    pub max_spread_pct: Option<f64>,
    #[serde(default)]
    pub rv_min_btc: Option<f64>,
    #[serde(default)]
    pub kelly_size_cap: Option<f64>,
    #[serde(default)]
    pub min_entry_price: Option<f64>,
    #[serde(default)]
    pub max_consecutive_losses: Option<u32>,
    #[serde(default)]
    pub stop_loss_pct: Option<f64>,
    #[serde(default)]
    pub fee_model: Option<String>,
    /// clob_events only: constant feed-perception latency (ms) held across the sweep.
    #[serde(default)]
    pub feed_latency_ms: Option<u64>,
    /// Latency values (ms) to sweep. E.g. [0, 100, 250, 500, 1000].
    pub latency_values: Vec<u64>,
}

pub async fn handle_api_backtest_latency_sweep(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<LatencySweepBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    if body.latency_values.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "latency_values must be a non-empty array of u64 milliseconds."
        }))).into_response();
    }

    let workspace_dir = state.config.lock().workspace_dir.clone();

    let scripts_dir = workspace_dir.join("scripts");
    let script_path = if body.script.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "script field is required for latency sweep."
        }))).into_response();
    } else if std::path::Path::new(&body.script).is_absolute() {
        std::path::PathBuf::from(&body.script)
    } else {
        scripts_dir.join(&body.script)
    };

    if !script_path.exists() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({
            "error": format!("Script not found: {}", script_path.display())
        }))).into_response();
    }

    let sizing_mode = body.sizing_mode.as_deref().unwrap_or("percent");
    let sizing_value = body.sizing_value.unwrap_or(1.0);
    let price_mode = body.price_mode.as_deref().unwrap_or("historical");
    let resolution_logic = body.resolution_logic.clone().unwrap_or_else(|| "price_up".into());

    let mut rows: Vec<crate::tools::backtest::LatencySweepRow> = Vec::new();
    for &lat_ms in &body.latency_values {
        let metrics = crate::tools::backtest::run_backtest_engine(
            &script_path,
            &body.market_type,
            &body.symbol,
            &body.interval,
            &body.from_date,
            &body.to_date,
            body.initial_balance,
            body.fee_pct,
            &resolution_logic,
            body.threshold,
            body.max_position_usd,
            body.max_entry_price,
            sizing_mode,
            sizing_value,
            price_mode,
            &workspace_dir,
            &body.allowed_hours,
            body.max_spread_pct,
            body.rv_min_btc,
            body.kelly_size_cap,
            body.min_entry_price,
            body.max_consecutive_losses,
            body.stop_loss_pct,
            Some(lat_ms),
            body.fee_model.as_deref(),
            body.feed_latency_ms,
        )
        .await;

        let ev_per_trade_usd = if metrics.total_trades > 0 {
            metrics.total_return_pct / 100.0 * body.initial_balance / metrics.total_trades as f64
        } else {
            0.0
        };

        rows.push(crate::tools::backtest::LatencySweepRow {
            latency_ms: lat_ms,
            total_return_pct: metrics.total_return_pct,
            win_rate_pct: metrics.win_rate_pct,
            total_trades: metrics.total_trades,
            ev_per_trade_usd,
        });
    }

    Json(serde_json::json!({
        "symbol": body.symbol,
        "market_type": body.market_type,
        "from_date": body.from_date,
        "to_date": body.to_date,
        "initial_balance": body.initial_balance,
        "fee_model": body.fee_model.as_deref().unwrap_or("pct"),
        "rows": rows,
    }))
    .into_response()
}

/// Split [from_date, to_date] (inclusive, YYYY-MM-DD) at `train_frac` of the day
/// span. Returns ((train_from, train_to), (test_from, test_to)). The test range
/// starts the day AFTER train_to so the two windows never overlap.
fn split_date_range(from: &str, to: &str, train_frac: f64) -> Option<((String, String), (String, String))> {
    let f = chrono::NaiveDate::parse_from_str(from, "%Y-%m-%d").ok()?;
    let t = chrono::NaiveDate::parse_from_str(to, "%Y-%m-%d").ok()?;
    if t < f { return None; }
    let total_days = (t - f).num_days();
    if total_days < 1 { return None; }
    let frac = train_frac.clamp(0.1, 0.9);
    let train_days = ((total_days as f64) * frac).round() as i64;
    let train_to = f + chrono::Duration::days(train_days);
    let test_from = train_to + chrono::Duration::days(1);
    if test_from > t { return None; }
    Some((
        (from.to_string(), train_to.format("%Y-%m-%d").to_string()),
        (test_from.format("%Y-%m-%d").to_string(), to.to_string()),
    ))
}

/// POST /api/backtest/walk-forward — run the SAME backtest on a train split and a
/// held-out test split, validating each with the 3-leg edge_validator. The point
/// is to catch overfit: an edge that only survives in-sample is not an edge.
/// Reuses `LatencySweepBody` (same fields); `latency_values[0]` (if any) sets the
/// order latency, and `walk_forward_train_frac` is read from the query string.
pub async fn handle_api_backtest_walk_forward(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
    Json(body): Json<LatencySweepBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let train_frac = q.get("train_frac").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.7);
    let (train, test) = match split_date_range(&body.from_date, &body.to_date, train_frac) {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "date range too short to split (need ≥2 days)."
        }))).into_response(),
    };

    let workspace_dir = state.config.lock().workspace_dir.clone();
    let scripts_dir = workspace_dir.join("scripts");
    let script_path = if body.script.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "script field is required for walk-forward."
        }))).into_response();
    } else if std::path::Path::new(&body.script).is_absolute() {
        std::path::PathBuf::from(&body.script)
    } else {
        scripts_dir.join(&body.script)
    };
    if !script_path.exists() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({
            "error": format!("Script not found: {}", script_path.display())
        }))).into_response();
    }

    let sizing_mode = body.sizing_mode.as_deref().unwrap_or("percent");
    let sizing_value = body.sizing_value.unwrap_or(1.0);
    let price_mode = body.price_mode.as_deref().unwrap_or("historical");
    let resolution_logic = body.resolution_logic.clone().unwrap_or_else(|| "price_up".into());
    let latency_ms = body.latency_values.first().copied();

    // Run one split → (metrics, edge_validation JSON).
    async fn run_split(
        script_path: &std::path::Path, body: &LatencySweepBody, from: &str, to: &str,
        resolution_logic: &str, sizing_mode: &str, sizing_value: f64, price_mode: &str,
        workspace_dir: &std::path::Path, latency_ms: Option<u64>,
    ) -> serde_json::Value {
        let m = crate::tools::backtest::run_backtest_engine(
            script_path, &body.market_type, &body.symbol, &body.interval, from, to,
            body.initial_balance, body.fee_pct, resolution_logic, body.threshold,
            body.max_position_usd, body.max_entry_price, sizing_mode, sizing_value, price_mode,
            workspace_dir, &body.allowed_hours, body.max_spread_pct, body.rv_min_btc,
            body.kelly_size_cap, body.min_entry_price, body.max_consecutive_losses,
            body.stop_loss_pct, latency_ms, body.fee_model.as_deref(), body.feed_latency_ms,
        ).await;
        let (entries, wons) = crate::tools::edge_validator::extract_binary_trades(&m.all_trades);
        let validation = crate::tools::edge_validator::validate(&entries, &wons, 5000);
        serde_json::json!({
            "from_date": from, "to_date": to,
            "total_return_pct": m.total_return_pct,
            "sharpe_ratio": m.sharpe_ratio,
            "max_drawdown_pct": m.max_drawdown_pct,
            "win_rate_pct": m.win_rate_pct,
            "total_trades": m.total_trades,
            "analysis": m.analysis,
            "edge_validation": validation,
        })
    }

    let train_res = run_split(&script_path, &body, &train.0, &train.1, &resolution_logic,
        sizing_mode, sizing_value, price_mode, &workspace_dir, latency_ms).await;
    let test_res = run_split(&script_path, &body, &test.0, &test.1, &resolution_logic,
        sizing_mode, sizing_value, price_mode, &workspace_dir, latency_ms).await;

    // The headline: did the edge survive out-of-sample?
    let train_edge = train_res.get("edge_validation").and_then(|v| v.get("verdict")).and_then(|v| v.as_str()).unwrap_or("");
    let test_edge = test_res.get("edge_validation").and_then(|v| v.get("verdict")).and_then(|v| v.as_str()).unwrap_or("");
    let holds_oos = train_edge == "EDGE" && test_edge == "EDGE";

    Json(serde_json::json!({
        "symbol": body.symbol,
        "market_type": body.market_type,
        "train_frac": train_frac,
        "train": train_res,
        "test": test_res,
        "holds_out_of_sample": holds_oos,
        "verdict": if holds_oos { "EDGE survives out-of-sample" }
                   else if train_edge == "EDGE" { "OVERFIT — edge in train only, gone in test" }
                   else { "NO EDGE in train" },
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
pub struct DeleteScriptQuery {
    pub path: String,
}

/// DELETE /api/backtest/scripts — delete a .rhai script
pub async fn handle_api_backtest_scripts_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DeleteScriptQuery>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let path = std::path::Path::new(&query.path);
    if !path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Script not found" })),
        )
            .into_response();
    }

    // Only allow deleting .rhai files in scripts directory
    if path.extension().and_then(|e| e.to_str()) != Some("rhai") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Can only delete .rhai files" })),
        )
            .into_response();
    }

    match std::fs::remove_file(path) {
        Ok(_) => {
            // Also remove the meta file if exists
            let meta_path = path.with_extension("rhai.meta.json");
            let _ = std::fs::remove_file(meta_path);
            Json(serde_json::json!({ "success": true })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to delete: {e}") })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct RenameScriptBody {
    pub old_path: String,
    pub new_name: String,
}

/// POST /api/backtest/scripts/rename — rename a .rhai script
pub async fn handle_api_backtest_scripts_rename(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RenameScriptBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let old_path = std::path::Path::new(&body.old_path);
    if !old_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Script not found" })),
        )
            .into_response();
    }

    // Ensure new name has .rhai extension
    let new_name = if body.new_name.ends_with(".rhai") {
        body.new_name.clone()
    } else {
        format!("{}.rhai", body.new_name)
    };

    let new_path = old_path.parent().unwrap_or(old_path).join(&new_name);

    match std::fs::rename(old_path, &new_path) {
        Ok(_) => {
            // Also rename meta file if exists
            let old_meta = old_path.with_extension("rhai.meta.json");
            if old_meta.exists() {
                let new_meta = new_path.with_extension("rhai.meta.json");
                let _ = std::fs::rename(old_meta, new_meta);
            }
            Json(serde_json::json!({ "success": true, "new_path": new_path.to_string_lossy() })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to rename: {e}") })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct UpdateDescriptionBody {
    pub path: String,
    pub description: String,
}

/// POST /api/backtest/scripts/description — update script description (stored in meta file)
pub async fn handle_api_backtest_scripts_description(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpdateDescriptionBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let script_path = std::path::Path::new(&body.path);
    if !script_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Script not found" })),
        )
            .into_response();
    }

    // Store description in a sidecar .meta.json file
    let meta_path = script_path.with_extension("rhai.meta.json");
    let mut meta: serde_json::Value = std::fs::read_to_string(&meta_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    meta["description"] = serde_json::json!(body.description);

    match std::fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap()) {
        Ok(_) => Json(serde_json::json!({ "success": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to save: {e}") })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct UpdateStatsBody {
    pub path: String,
    pub stats: serde_json::Value,
}

/// POST /api/backtest/scripts/stats — save last run stats to meta file
pub async fn handle_api_backtest_scripts_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpdateStatsBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let script_path = std::path::Path::new(&body.path);
    if !script_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Script not found" })),
        )
            .into_response();
    }

    // Store stats in a sidecar .meta.json file
    let meta_path = script_path.with_extension("rhai.meta.json");
    let mut meta: serde_json::Value = std::fs::read_to_string(&meta_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    meta["last_run_stats"] = body.stats;

    match std::fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap()) {
        Ok(_) => Json(serde_json::json!({ "success": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to save: {e}") })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct GetScriptContentQuery {
    pub path: String,
}

/// GET /api/backtest/scripts/content — read script content
pub async fn handle_api_backtest_scripts_content_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GetScriptContentQuery>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let script_path = std::path::Path::new(&query.path);
    if !script_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Script not found" })),
        )
            .into_response();
    }

    // Only allow reading .rhai files
    if script_path.extension().and_then(|e| e.to_str()) != Some("rhai") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Can only read .rhai files" })),
        )
            .into_response();
    }

    match std::fs::read_to_string(script_path) {
        Ok(content) => Json(serde_json::json!({ "content": content })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to read: {e}") })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct SaveScriptContentBody {
    pub path: String,
    pub content: String,
}

/// POST /api/backtest/scripts/content — save script content
pub async fn handle_api_backtest_scripts_content_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SaveScriptContentBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let script_path = std::path::Path::new(&body.path);

    // Only allow writing .rhai files
    if script_path.extension().and_then(|e| e.to_str()) != Some("rhai") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Can only write .rhai files" })),
        )
            .into_response();
    }

    // Create parent directories if needed
    if let Some(parent) = script_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match std::fs::write(script_path, &body.content) {
        Ok(_) => Json(serde_json::json!({ "success": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to save: {e}") })),
        )
            .into_response(),
    }
}

// ── Polymarket historical dataset sync (dashboard) ────────────────────────

#[derive(serde::Deserialize)]
pub struct PolyHistSyncBody {
    /// Series to sync. Defaults to "btc_5m".
    #[serde(default)]
    pub series_id: Option<String>,
    /// Rolling window in days ending today (UTC). Defaults to 60.
    #[serde(default)]
    pub days_back: Option<u32>,
}

/// POST /api/backtest/polymarket-historical/sync
///
/// Starts a background scrape of the last `days_back` days of Polymarket
/// data for the given series. Fetches both minute-4 (P4, main dataset) and
/// minute-3 (P3, drift signal) token prices via CLOB `/prices-history`.
///
/// The request returns immediately. Progress is exposed at
/// `GET /api/backtest/polymarket-historical/status`.
///
/// If a sync is already running, returns `started: false` plus the current
/// progress snapshot — callers should just poll `/status` in that case.
pub async fn handle_api_backtest_polymarket_historical_sync(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PolyHistSyncBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let series_id = body.series_id.unwrap_or_else(|| "btc_5m".to_string());
    let days_back = body.days_back.unwrap_or(60).clamp(1, 365) as i64;

    // Reject unknown series early so the UI gets a clean error.
    let series_known = crate::tools::series::builtin_series()
        .into_iter()
        .any(|s| s.id == series_id);
    if !series_known {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Unknown series_id: {}", series_id) })),
        )
            .into_response();
    }

    // Guard: at most one sync at a time.
    {
        let prog = state.poly_sync_progress.lock();
        if prog.running {
            return Json(serde_json::json!({
                "started": false,
                "progress": *prog,
            }))
            .into_response();
        }
    }

    let to_dt = chrono::Utc::now();
    let from_dt = to_dt - chrono::Duration::days(days_back);
    let from_date = from_dt.format("%Y-%m-%d").to_string();
    let to_date = to_dt.format("%Y-%m-%d").to_string();
    let workspace_dir = state.config.lock().workspace_dir.clone();

    // Initialise progress + reset cancel flag from any previous run.
    {
        let mut prog = state.poly_sync_progress.lock();
        *prog = crate::tools::polymarket_historical::SyncProgress {
            running: true,
            series_id: series_id.clone(),
            from_date: from_date.clone(),
            to_date: to_date.clone(),
            stage: "min4".to_string(),
            windows_total: 0,
            windows_fetched: 0,
            min4_count: 0,
            min3_count: 0,
            error: None,
            started_at: Some(chrono::Utc::now().to_rfc3339()),
            completed_at: None,
        };
    }
    state.poly_sync_cancel.store(false, std::sync::atomic::Ordering::SeqCst);
    let cancel_flag = state.poly_sync_cancel.clone();

    // Spawn the scrape task. Minute-4 first (main dataset), then minute-3.
    //
    // Internally uses a SINGLE poll task across both stages with an explicit
    // AtomicBool stop flag. The previous design had two poll tasks whose
    // break condition (`running==false || stage∈{error,done}`) never tripped
    // during the stage-1 → stage-2 transition (stage was "min3", running
    // still true) — so the outer task hung forever on `poll_handle.await`
    // and `running` never flipped back to false in the dashboard.
    let progress = state.poly_sync_progress.clone();
    tokio::spawn(async move {
        use crate::tools::polymarket_historical::{scrape_series_with_options, ScrapeOptions};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        // Shared between stages: counters reset between stages, stop flag
        // signals the poll task to exit at the very end.
        let fetched = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let stop_flag = Arc::new(AtomicBool::new(false));

        let poll_fetched = fetched.clone();
        let poll_total = total.clone();
        let poll_stop = stop_flag.clone();
        let poll_progress = progress.clone();
        let poll_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            loop {
                interval.tick().await;
                if poll_stop.load(Ordering::SeqCst) { break; }
                let mut p = poll_progress.lock();
                p.windows_fetched = poll_fetched.load(Ordering::SeqCst);
                p.windows_total = poll_total.load(Ordering::SeqCst);
            }
        });

        // ── Stage 1: minute-4 (P4 decision) ──
        let min4_result = scrape_series_with_options(
            &series_id,
            &from_date,
            &to_date,
            &workspace_dir,
            ScrapeOptions {
                decision_offset_secs: None,     // default = minute 4 for 5m
                file_prefix: None,              // main dataset
                fetched_counter: Some(fetched.clone()),
                total_counter: Some(total.clone()),
                cancel_flag: Some(cancel_flag.clone()),
            },
        )
        .await;

        match min4_result {
            Ok(n) => {
                // If cancellation was requested during stage 1, stop here
                // instead of starting stage 2. Mark as cancelled and exit.
                if cancel_flag.load(Ordering::SeqCst) {
                    {
                        let mut p = progress.lock();
                        p.min4_count = n;
                        p.running = false;
                        p.stage = "cancelled".to_string();
                        p.completed_at = Some(chrono::Utc::now().to_rfc3339());
                    }
                    stop_flag.store(true, Ordering::SeqCst);
                    let _ = poll_handle.await;
                    return;
                }
                {
                    let mut p = progress.lock();
                    p.min4_count = n;
                    p.stage = "min3".to_string();
                    p.windows_fetched = 0;
                    p.windows_total = 0;
                }
                // Reset counters for stage 2.
                fetched.store(0, Ordering::SeqCst);
                total.store(0, Ordering::SeqCst);
            }
            Err(e) => {
                {
                    let mut p = progress.lock();
                    p.running = false;
                    p.stage = "error".to_string();
                    p.error = Some(format!("min4 scrape failed: {}", e));
                    p.completed_at = Some(chrono::Utc::now().to_rfc3339());
                }
                stop_flag.store(true, Ordering::SeqCst);
                let _ = poll_handle.await;
                return;
            }
        }

        // ── Stage 2: minute-3 (P3 drift signal) ──
        // Decision offset for minute-3: (window_minutes - 2) * 60s.
        let series = crate::tools::series::builtin_series()
            .into_iter()
            .find(|s| s.id == series_id);
        let window_minutes: i64 = match series {
            Some(s) => {
                let c = &s.cadence;
                if let Some(m) = c.strip_suffix('m') {
                    m.parse().unwrap_or(5)
                } else if let Some(h) = c.strip_suffix('h') {
                    h.parse::<i64>().unwrap_or(1) * 60
                } else {
                    5
                }
            }
            None => 5,
        };
        let min3_offset = (window_minutes - 2) * 60;

        let min3_result = scrape_series_with_options(
            &series_id,
            &from_date,
            &to_date,
            &workspace_dir,
            ScrapeOptions {
                decision_offset_secs: Some(min3_offset),
                file_prefix: Some("min3_".to_string()),
                fetched_counter: Some(fetched.clone()),
                total_counter: Some(total.clone()),
                cancel_flag: Some(cancel_flag.clone()),
            },
        )
        .await;

        {
            let mut p = progress.lock();
            p.running = false;
            p.completed_at = Some(chrono::Utc::now().to_rfc3339());
            match min3_result {
                Ok(n) => {
                    p.min3_count = n;
                    // If cancel flag was raised during stage 2, mark as
                    // cancelled (we still flushed whatever we had).
                    if cancel_flag.load(Ordering::SeqCst) {
                        p.stage = "cancelled".to_string();
                    } else {
                        p.stage = "done".to_string();
                    }
                }
                Err(e) => {
                    p.stage = "error".to_string();
                    p.error = Some(format!("min3 scrape failed: {}", e));
                }
            }
        }
        // Stop the single poll task and join it cleanly.
        stop_flag.store(true, Ordering::SeqCst);
        let _ = poll_handle.await;
    });

    let snapshot = state.poly_sync_progress.lock().clone();
    Json(serde_json::json!({
        "started": true,
        "progress": snapshot,
    }))
    .into_response()
}

/// POST /api/backtest/polymarket-historical/cancel
///
/// Sets the shared cancel flag so the active sync task exits early.
/// Returns immediately. The task may take a few seconds to wind down
/// (it finishes any in-flight worker tasks and flushes already-fetched
/// records to disk before marking stage = "cancelled").
pub async fn handle_api_backtest_polymarket_historical_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let was_running = state.poly_sync_progress.lock().running;
    state.poly_sync_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    Json(serde_json::json!({
        "ok": true,
        "was_running": was_running,
        "message": if was_running {
            "Cancel signal sent. Sync will stop within a few seconds."
        } else {
            "No sync was running; cancel flag set anyway."
        }
    }))
    .into_response()
}

/// GET /api/backtest/polymarket-historical/status
///
/// Returns the current sync progress snapshot plus a lightweight summary of
/// cached datasets on disk so the dashboard can render "last synced" info
/// even across server restarts.
pub async fn handle_api_backtest_polymarket_historical_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let progress = state.poly_sync_progress.lock().clone();
    let workspace_dir = state.config.lock().workspace_dir.clone();
    let hist_dir = workspace_dir.join("data").join("polymarket_historical");

    // Summarise available datasets (main + min3 pairs per series).
    let mut datasets: Vec<serde_json::Value> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&hist_dir) {
        use std::collections::BTreeMap;
        let mut by_series: BTreeMap<String, (Option<u64>, Option<u64>, Option<std::time::SystemTime>)> = BTreeMap::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) if n.ends_with(".jsonl") => n.to_string(),
                _ => continue,
            };
            let (series_key, is_min3) = if let Some(rest) = name.strip_prefix("min3_") {
                (rest.trim_end_matches(".jsonl").to_string(), true)
            } else {
                (name.trim_end_matches(".jsonl").to_string(), false)
            };
            let meta = std::fs::metadata(&path).ok();
            let modified = meta.as_ref().and_then(|m| m.modified().ok());
            // Approx record count = line count. Cheap enough for small files.
            let line_count = std::fs::read_to_string(&path)
                .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count() as u64)
                .ok();
            let slot = by_series.entry(series_key).or_default();
            if is_min3 {
                slot.1 = line_count;
            } else {
                slot.0 = line_count;
            }
            if let Some(m) = modified {
                slot.2 = Some(match slot.2 {
                    Some(prev) if prev > m => prev,
                    _ => m,
                });
            }
        }
        for (series_key, (min4, min3, modified)) in by_series {
            let modified_rfc = modified.and_then(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                Some(dt.to_rfc3339())
            });
            datasets.push(serde_json::json!({
                "series_id": series_key,
                "min4_count": min4,
                "min3_count": min3,
                "last_modified": modified_rfc,
            }));
        }
    }

    Json(serde_json::json!({
        "progress": progress,
        "datasets": datasets,
    }))
    .into_response()
}

// ── Live Strategy Runner API ──────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct CreateRunnerBody {
    pub name: Option<String>,
    #[serde(default)]
    pub script: String,
    pub market_type: String,
    /// Comma-separated Polymarket slugs OR a CEX symbol. Optional when
    /// `series_id` is provided — engines resolve the slug per-window then.
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub interval: String,
    pub mode: String,
    /// Polymarket wallet profile id for this runner (default = legacy profile).
    #[serde(default)]
    pub polymarket_wallet_id: Option<String>,
    pub initial_balance: f64,
    pub fee_pct: Option<f64>,
    pub warmup_days: Option<u32>,
    pub auto_restart: Option<bool>,
    /// Override the Rec-1 validation gate when going Live with a NO_EDGE strategy.
    #[serde(default)]
    pub force_live: Option<bool>,
    pub series_id: Option<String>,
    pub resolution_logic: Option<String>,
    pub threshold: Option<f64>,
    #[serde(default)]
    pub live_sizing_mode: Option<String>,
    #[serde(default)]
    pub live_sizing_value: Option<f64>,
    #[serde(default)]
    pub stop_loss_pct: Option<f64>,
    #[serde(default)]
    pub early_fire_secs: Option<u32>,
    #[serde(default)]
    pub max_entry_price: Option<f64>,
    #[serde(default)]
    pub price_mode: Option<String>,
    #[serde(default)]
    pub max_spread_pct: Option<f64>,
    #[serde(default)]
    pub max_slippage_pct: Option<f64>,
    #[serde(default)]
    pub allowed_hours: Option<Vec<u8>>,
    #[serde(default)]
    pub rv_min_btc: Option<f64>,
    #[serde(default)]
    pub kind: Option<String>,
    /// Wallet password for decrypting EVM key (live CEX modes only). Never persisted.
    #[serde(default)]
    pub wallet_password: Option<String>,
    /// Binance Futures API credentials (live funding_arb / cex). Never persisted.
    #[serde(default)]
    pub binance_api_key: Option<String>,
    #[serde(default)]
    pub binance_api_secret: Option<String>,
    /// Funding-arb watchlist + tunables.
    #[serde(default)]
    pub funding_watchlist: Option<Vec<String>>,
    #[serde(default)]
    pub min_apr_diff: Option<f64>,
    #[serde(default)]
    pub force_close_diff: Option<f64>,
    #[serde(default)]
    pub max_open_pairs: Option<usize>,
    #[serde(default)]
    pub max_pos_pct: Option<f64>,
    #[serde(default)]
    pub funding_poll_secs: Option<u64>,
    #[serde(default)]
    pub fee_buffer_bps: Option<f64>,
    #[serde(default)]
    pub chainlink_endpoint_url: Option<String>,
    #[serde(default)]
    pub chainlink_interval_secs: Option<u64>,
    /// Per-engine tunable parameters from the UI EngineParamsForm.
    #[serde(default)]
    pub engine_params: Option<serde_json::Value>,
    /// Maximum kelly multiplier (default 1.5). Prevents scripts from over-sizing bets.
    #[serde(default)]
    pub kelly_size_cap: Option<f64>,
    /// Auto-stop when cumulative live P&L loss exceeds this % of initial_balance. 0 = disabled.
    #[serde(default)]
    pub max_runner_loss_pct: Option<f64>,
    /// Auto-stop after N consecutive losses. 0 = disabled.
    #[serde(default)]
    pub max_consecutive_losses: Option<u32>,
    /// Minimum entry price. Skip orders when token ask < this value. Default 0.05.
    #[serde(default)]
    pub min_entry_price: Option<f64>,
    /// Polymarket condition_id — used by the rewards_maker engine to resolve the
    /// YES/NO token ids at startup (the maker quotes a single fixed market, not a series).
    #[serde(default)]
    pub poly_condition_id: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct PatchRunnerBody {
    pub auto_restart: Option<bool>,
    pub live_sizing_mode: Option<String>,
    pub live_sizing_value: Option<f64>,
    /// Hide / unhide a runner in the Live Strategies UI.
    pub hidden: Option<bool>,
    // These use Option<Option<T>> so we can distinguish:
    //   absent → None (skip)   |   null → Some(None) (clear)   |   value → Some(Some(v)) (set)
    #[serde(default, deserialize_with = "nullable::deserialize")]
    pub max_entry_price: Option<Option<f64>>,
    pub price_mode: Option<String>,
    #[serde(default, deserialize_with = "nullable::deserialize")]
    pub max_spread_pct: Option<Option<f64>>,
    #[serde(default, deserialize_with = "nullable::deserialize")]
    pub max_slippage_pct: Option<Option<f64>>,
    #[serde(default, deserialize_with = "nullable::deserialize")]
    pub early_fire_secs: Option<Option<u32>>,
    /// UTC hours (0-23) where trading is allowed. Empty array = no restriction.
    pub allowed_hours: Option<Vec<u8>>,
    /// Minimum BTC RV-1h threshold; null/0 = disabled.
    #[serde(default, deserialize_with = "nullable::deserialize")]
    pub rv_min_btc: Option<Option<f64>>,
    /// Maximum kelly multiplier override.
    #[serde(default)]
    pub kelly_size_cap: Option<f64>,
    /// Auto-stop loss percentage (0 = disabled).
    #[serde(default)]
    pub max_runner_loss_pct: Option<f64>,
    /// Auto-stop consecutive losses (0 = disabled).
    #[serde(default)]
    pub max_consecutive_losses: Option<u32>,
    /// Minimum entry price (0 = disabled, use default 0.05).
    #[serde(default)]
    pub min_entry_price: Option<f64>,
    /// Stop-loss per trade: exit early if token drops this fraction from entry.
    /// null/0 = disabled.
    #[serde(default, deserialize_with = "nullable::deserialize")]
    pub stop_loss_pct: Option<Option<f64>>,
}

async fn hydrate_live_runtime_config(
    state: &AppState,
    config: &mut crate::strategy_runner::RunnerConfig,
    wallet_password: Option<&str>,
) -> anyhow::Result<()> {
    if config.mode != "live" {
        return Ok(());
    }

    // CEX live mode: build signer from wallet-manager
    if config.market_type != "polymarket_binary" {
        let hl_cfg = state.config.lock().hyperliquid.clone();
        let label = hl_cfg.wallet_label.as_deref().unwrap_or("");
        if label.is_empty() {
            anyhow::bail!("hyperliquid.wallet_label is not set in config.");
        }

        let wallets = state.wallets.lock();
        let wallet = wallets
            .iter()
            .find(|w| w.chain == "evm" && (label == "*" || w.label == label))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No EVM wallet found with label '{}' for Hyperliquid trading.",
                    label
                )
            })?;

        let password = wallet_password.ok_or_else(|| {
            anyhow::anyhow!("Wallet password required for live CEX trading.")
        })?;

        let pk_bytes = wallet_manager::evm::export_private_key(&wallet.encrypted_key, password)
            .map_err(|e| anyhow::anyhow!("Failed to decrypt wallet: {e}"))?;

        let signer = hyperliquid_trader::exchange::Signer::from_pk_bytes(pk_bytes)
            .map_err(|e| anyhow::anyhow!("Invalid private key: {e}"))?;

        config.wallet_address = Some(signer.address().to_string());
        config.hl_signer = Some(signer);
        // Pass risk gate reference for live trading
        config.risk_gate = state.trading_risk_gate.clone();
        return Ok(());
    }

    let profile = get_poly_wallet_profile(state, config.polymarket_wallet_id.as_deref())
        .ok_or_else(|| anyhow::anyhow!("Live mode requires a configured Polymarket wallet profile."))?;
    let creds = poly_creds_from_profile(&profile)
        .ok_or_else(|| anyhow::anyhow!("Live mode requires polymarket api_key, secret, passphrase, wallet_address, and private_key in the selected wallet profile."))?;
    if creds.secret.is_empty() || creds.passphrase.is_empty() {
        anyhow::bail!("Live mode requires polymarket.secret and polymarket.passphrase in the selected wallet profile.");
    }
    if creds.wallet_address.is_empty() {
        anyhow::bail!("Live mode requires a wallet address in the selected Polymarket wallet profile.");
    }
    if creds.private_key.as_deref().unwrap_or("").is_empty() {
        anyhow::bail!("Live mode requires private_key for EIP-712 order signing in the selected Polymarket wallet profile.");
    }
    let wallet_address = creds.wallet_address.clone();

    // The rewards_maker engine resolves its own YES/NO tokens from the condition_id
    // (stored in `symbol`/`poly_condition_id`) — it quotes one fixed market, not a
    // rolling series, so it must NOT go through the series-based token resolver.
    let (yes_token_id, no_token_id) = if config.kind.as_deref() == Some(strategy_core::engines::REWARDS_MAKER) {
        let cid = config.poly_condition_id.clone()
            .or_else(|| if config.symbol.starts_with("0x") { Some(config.symbol.clone()) } else { None })
            .ok_or_else(|| anyhow::anyhow!("rewards_maker live mode needs poly_condition_id"))?;
        match polymarket_trader::markets::get_resolution_by_condition_id(&cid).await {
            // get_resolution returns Option<bool>; we need the token ids, so fetch the
            // CLOB market directly here for YES/NO.
            _ => resolve_tokens_for_condition(&cid).await?,
        }
    } else {
        resolve_live_token_ids(config.series_id.as_deref()).await?
    };
    let min_live_usdc = 1.0;
    if config.mode == "live" {
        let proxy_for_check = clean_optional(&profile.proxy_address);
        ensure_live_wallet_has_min_balance(&wallet_address, proxy_for_check.as_deref(), min_live_usdc).await?;
    }

    // Pre-flight: verify L2 auth actually works before starting the runner.
    // Catches mismatched api_key/secret/passphrase so we don't discover the
    // problem only when the first order is submitted.
    validate_live_poly_credentials(&creds).await?;

    config.poly_creds = Some(creds);
    config.poly_token_id = Some(yes_token_id);
    config.poly_no_token_id = Some(no_token_id);
    config.wallet_address = Some(wallet_address);

    // Populate Chainlink price feed config from global settings
    let cl = state.config.lock().chainlink.clone();
    if cl.enabled {
        config.chainlink_endpoint_url = cl.endpoint_url;
        config.chainlink_api_key = cl.api_key;
        config.chainlink_interval_secs = cl.interval_secs;
    }
    Ok(())
}

async fn rehydrate_live_runner_config(state: &AppState, config: &mut crate::strategy_runner::RunnerConfig) -> anyhow::Result<()> {
    if config.mode != "live" {
        return Ok(());
    }
    // CEX live runners require wallet password which is not persisted.
    // They must be restarted manually through the UI.
    if config.market_type != "polymarket_binary" {
        return Ok(());
    }
    hydrate_live_runtime_config(state, config, None).await
}

pub async fn restart_stored_runners(state: &AppState) {
    let configs = state.strategy_runner.list_restartable_configs();
    if configs.is_empty() {
        return;
    }

    let mut restarted = 0usize;
    for mut config in configs {
        if let Err(e) = rehydrate_live_runner_config(state, &mut config).await {
            let msg = friendly_live_error(&e.to_string());
            let id = config.id.clone();
            let _ = state.strategy_runner.set_starting(&id);
            if let Some(mut r) = state.strategy_runner.get(&id) {
                r.status.status = "error".to_string();
                r.status.error = Some(msg);
                state.strategy_runner.upsert(r);
            }
            continue;
        }

        let id = config.id.clone();
        let Some(creds) = config.poly_creds.clone() else {
            continue;
        };
        if !state.strategy_runner.hydrate_live_creds_for_runner(&id, creds) {
            continue;
        }
        if let (Some(yes), Some(no)) = (config.poly_token_id.clone(), config.poly_no_token_id.clone()) {
            let _ = state.strategy_runner.set_poly_token_ids(&id, yes, no);
        }
        if let Some(addr) = config.wallet_address.clone() {
            let _ = state.strategy_runner.set_wallet_address(&id, addr);
        }
        let _ = state.strategy_runner.set_starting(&id);

        let (workspace_dir, cfg_path) = {
            let c = state.config.lock();
            (c.workspace_dir.clone(), c.config_path.clone())
        };
        let _ = crate::strategy_runner::start_runner(
            state.strategy_runner.clone(),
            config,
            workspace_dir,
            Some(cfg_path),
        );
        restarted += 1;
    }

    if restarted > 0 {
        tracing::info!("Auto-restarted {restarted} strategy runner(s) after startup");
    }
}

pub async fn handle_api_live_patch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<PatchRunnerBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    if let Some(auto_restart) = body.auto_restart {
        match state.strategy_runner.set_auto_restart(&id, auto_restart) {
            Some(runner) => return Json(serde_json::json!({ "runner": runner })).into_response(),
            None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "runner not found" }))).into_response(),
        }
    }

    if let Some(hidden) = body.hidden {
        match state.strategy_runner.set_hidden(&id, hidden) {
            Some(runner) => return Json(serde_json::json!({ "runner": runner })).into_response(),
            None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "runner not found" }))).into_response(),
        }
    }

    if body.live_sizing_mode.is_some()
        || body.live_sizing_value.is_some()
        || body.max_entry_price.is_some()
        || body.price_mode.is_some()
        || body.max_spread_pct.is_some()
        || body.max_slippage_pct.is_some()
        || body.stop_loss_pct.is_some()
        || body.early_fire_secs.is_some()
        || body.allowed_hours.is_some()
        || body.rv_min_btc.is_some()
        || body.kelly_size_cap.is_some()
        || body.max_runner_loss_pct.is_some()
        || body.max_consecutive_losses.is_some()
        || body.min_entry_price.is_some()
    {
        let mode = body.live_sizing_mode.map(|m| match m.as_str() {
            "fixed" => crate::strategy_runner::LiveSizingMode::Fixed,
            _ => crate::strategy_runner::LiveSizingMode::Percent,
        });
        match state.strategy_runner.update_runner_config(
            &id,
            mode,
            body.live_sizing_value,
            body.max_entry_price,
            body.allowed_hours,
            body.rv_min_btc,
            body.price_mode,
            body.max_spread_pct,
            body.max_slippage_pct,
            body.early_fire_secs,
            body.kelly_size_cap,
            body.max_runner_loss_pct,
            body.max_consecutive_losses,
            body.min_entry_price,
            body.stop_loss_pct,
        ) {
            Some(runner) => return Json(serde_json::json!({ "runner": runner })).into_response(),
            None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "runner not found" }))).into_response(),
        }
    }

    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": "No valid fields to patch" })),
    ).into_response()
}

/// GET /api/live/strategies — list all strategy runners
/// GET /api/live/strategies — list all strategy runners
pub async fn handle_api_live_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let runners = state.strategy_runner.list();
    Json(serde_json::json!({ "runners": runners })).into_response()
}

#[derive(Deserialize)]
pub struct ValidateQuery {
    /// Runner name substring to validate (aggregates all matching runners).
    pub name: String,
}

/// GET /api/validate/runner?name=<substring> — the trusted edge check. Runs the 3-leg
/// validation (bootstrap CI + random-outcome null + shuffle null) on the matching
/// runner(s)' OFFICIAL-resolution trades, priced at the realistic settle (fill) price.
/// Replaces the misleading backtest "edge" numbers from the synthetic/stale engines.
pub async fn handle_api_validate_runner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<ValidateQuery>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let needle = params.name.to_lowercase();
    let runners = state.strategy_runner.list();
    let mut entries: Vec<f64> = Vec::new();
    let mut wons: Vec<bool> = Vec::new();
    let mut matched = 0usize;
    for r in &runners {
        if !r.config.name.to_lowercase().contains(&needle) { continue; }
        matched += 1;
        if let Some(res) = r.result.as_ref() {
            for o in &res.live_orders {
                // Only OFFICIAL Polymarket resolution — never binance_provisional.
                if o.resolution_source.as_deref() != Some("polymarket") { continue; }
                let Some(result) = o.result.as_deref() else { continue; };
                let price = crate::strategy_runner::settle_price(o.entry_price, o.fill_price);
                if !(price > 0.01 && price < 0.99) { continue; }
                entries.push(price);
                wons.push(result.trim_end_matches('*') == "WIN");
            }
        }
    }
    let result = crate::tools::edge_validator::validate(&entries, &wons, 5000);
    Json(serde_json::json!({
        "name": params.name,
        "runners_matched": matched,
        "result": result,
    }))
    .into_response()
}

/// POST /api/live/strategies — create & start a new runner
pub async fn handle_api_live_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateRunnerBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    let id = uuid::Uuid::new_v4().to_string();

    let mut symbol = body.symbol;
    let mut interval = body.interval;
    let mut resolution_logic = body.resolution_logic;
    let mut threshold = body.threshold;

    if let Some(ref sid) = body.series_id {
        if let Some(s) = crate::tools::series::builtin_series().into_iter().find(|s| s.id == *sid) {
            symbol = s.symbol;
            interval = s.cadence;
            resolution_logic = Some(match s.resolution_logic {
                crate::tools::series::ResolutionLogic::PriceUp => "price_up".to_string(),
                crate::tools::series::ResolutionLogic::ThresholdAbove => "threshold_above".to_string(),
                crate::tools::series::ResolutionLogic::ThresholdBelow => "threshold_below".to_string(),
            });
            threshold = s.threshold;
        }
    }

    let is_live = body.mode == "live";
    if is_live {
        if body.market_type == "polymarket_binary" {
            // Rec 1 — validation-first gate. If a paper/live runner with this script
            // already has official-resolution history, run the 3-leg validator and
            // BLOCK the live start when it shows NO_EDGE, unless the body carries
            // `force_live: true`. This stops un-validated strategies from reaching
            // real capital (the root cause of the May incident).
            if !body.force_live.unwrap_or(false) {
                let script_needle = body.script.rsplit('/').next().unwrap_or(&body.script).to_string();
                let (mut entries, mut wons) = (Vec::new(), Vec::new());
                for r in state.strategy_runner.list() {
                    if !r.config.script.contains(&script_needle) { continue; }
                    if let Some(res) = r.result.as_ref() {
                        for o in &res.live_orders {
                            if o.resolution_source.as_deref() != Some("polymarket") { continue; }
                            let Some(rs) = o.result.as_deref() else { continue; };
                            let p = crate::strategy_runner::settle_price(o.entry_price, o.fill_price);
                            if p > 0.01 && p < 0.99 { entries.push(p); wons.push(rs.trim_end_matches('*') == "WIN"); }
                        }
                    }
                }
                if entries.len() >= 30 {
                    let v = crate::tools::edge_validator::validate(&entries, &wons, 5000);
                    if v.verdict == "NO_EDGE" {
                        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                            "error": format!(
                                "Validation gate: '{}' shows NO EDGE on {} official trades (EV {:.1}%/trade, random-null p={:.2}). \
                                 Live blocked. Pass force_live:true to override.",
                                script_needle, v.n, v.ev_per_trade_pct, v.p_random),
                            "verdict": v,
                        }))).into_response();
                    }
                }
            }
        } else if body.market_type == "funding_arb" {
            // Funding arb requires BOTH Hyperliquid wallet AND Binance credentials
            let hl_cfg = state.config.lock().hyperliquid.clone();
            if !hl_cfg.enabled || hl_cfg.wallet_label.is_none() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "Live funding arb requires hyperliquid.enabled=true and hyperliquid.wallet_label set in config."
                    })),
                ).into_response();
            }
            let has_binance = body.binance_api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false)
                && body.binance_api_secret.as_ref().map(|s| !s.is_empty()).unwrap_or(false);
            if !has_binance {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "Live funding arb requires Binance API key and secret."
                    })),
                ).into_response();
            }
        } else {
            // CEX live — requires Hyperliquid wallet configuration
            let hl_cfg = state.config.lock().hyperliquid.clone();
            if !hl_cfg.enabled || hl_cfg.wallet_label.is_none() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "Live mode for CEX requires hyperliquid.enabled=true and hyperliquid.wallet_label set in config."
                    })),
                ).into_response();
            }
        }
    }

    let mut config = crate::strategy_runner::RunnerConfig {
        id: id.clone(),
        name: body.name.unwrap_or_else(|| format!("{} on {}", body.script, symbol)),
        script: body.script,
        market_type: body.market_type,
        symbol,
        interval,
        mode: body.mode,
        polymarket_wallet_id: body.polymarket_wallet_id,
        initial_balance: body.initial_balance,
        fee_pct: body.fee_pct.unwrap_or(0.1),
        warmup_days: body.warmup_days.unwrap_or(90),
        auto_restart: body.auto_restart.unwrap_or(true),
        series_id: body.series_id,
        resolution_logic: Some(resolution_logic.unwrap_or_else(|| "price_up".to_string())),
        threshold,
        poly_creds: None,
        poly_token_id: None,
        poly_no_token_id: None,
        poly_condition_id: body.poly_condition_id,
        wallet_address: None,
        chainlink_endpoint_url: None,
        chainlink_api_key: None,
        chainlink_interval_secs: 5,
        live_sizing_mode: match body.live_sizing_mode.as_deref() {
            Some("fixed") => crate::strategy_runner::LiveSizingMode::Fixed,
            _ => crate::strategy_runner::LiveSizingMode::Percent,
        },
        live_sizing_value: body.live_sizing_value.unwrap_or(5.0), // stored as 0–100 percent
        stop_loss_pct: body.stop_loss_pct.filter(|&v| v > 0.0),
        early_fire_secs: body.early_fire_secs.or_else(|| {
            let v = state.config.lock().live_strategy.early_fire_secs;
            if v > 0 { Some(v) } else { None }
        }),
        max_entry_price: body.max_entry_price,
        price_mode: body.price_mode,
        max_spread_pct: body.max_spread_pct,
        max_slippage_pct: body.max_slippage_pct,
        allowed_hours: body.allowed_hours.unwrap_or_default(),
        rv_min_btc: body.rv_min_btc.filter(|&v| v > 0.0),
        kelly_size_cap: body.kelly_size_cap.unwrap_or(1.5),
        max_runner_loss_pct: body.max_runner_loss_pct.filter(|&v| v > 0.0),
        max_consecutive_losses: body.max_consecutive_losses.filter(|&v| v > 0),
        min_entry_price: body.min_entry_price.unwrap_or(0.05),
        hl_signer: None,
        risk_gate: state.trading_risk_gate.clone(),
        binance_creds: None,
        funding_watchlist: body.funding_watchlist.unwrap_or_else(|| {
            ["BTC", "ETH", "SOL", "AVAX"].iter().map(|s| s.to_string()).collect()
        }),
        min_apr_diff: body.min_apr_diff.unwrap_or(0.10),
        force_close_diff: body.force_close_diff.unwrap_or(0.02),
        max_open_pairs: body.max_open_pairs.unwrap_or(4),
        max_pos_pct: body.max_pos_pct.unwrap_or(0.15),
        funding_poll_secs: body.funding_poll_secs.unwrap_or(60),
        fee_buffer_bps: body.fee_buffer_bps.unwrap_or(12.0),
        kind: body.kind,
        engine_params: body.engine_params,
    };

    // Populate Binance credentials if provided (live mode only, never persisted)
    if let (Some(key), Some(secret)) = (body.binance_api_key, body.binance_api_secret) {
        if !key.is_empty() && !secret.is_empty() {
            config.binance_creds = Some(crate::tools::binance_perps::BinanceCredentials {
                api_key: key,
                api_secret: secret,
            });
        }
    }

    if let Err(e) = hydrate_live_runtime_config(&state, &mut config, body.wallet_password.as_deref()).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": friendly_live_error(&e.to_string()) })),
        ).into_response();
    }

    let (workspace_dir, cfg_path) = {
        let c = state.config.lock();
        (c.workspace_dir.clone(), c.config_path.clone())
    };
    let runner = crate::strategy_runner::start_runner(
        state.strategy_runner.clone(),
        config,
        workspace_dir,
        Some(cfg_path),
    );

    (StatusCode::CREATED, Json(serde_json::json!({ "runner": runner }))).into_response()
}

/// GET /api/live/strategies/{id} — get single runner details
pub async fn handle_api_live_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    match state.strategy_runner.get(&id) {
        Some(r) => Json(serde_json::json!({ "runner": r })).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "not found" }))).into_response(),
    }
}

/// DELETE /api/live/strategies/{id} — stop and delete a runner
pub async fn handle_api_live_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    if state.strategy_runner.delete(&id) {
        Json(serde_json::json!({ "success": true })).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "runner not found" }))).into_response()
    }
}

/// POST /api/live/strategies/{id}/stop — stop a runner (keep it in list)
pub async fn handle_api_live_stop(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    state.strategy_runner.stop(&id);
    Json(serde_json::json!({ "success": true })).into_response()
}

/// POST /api/live/strategies/{id}/restart — restart a stopped runner
pub async fn handle_api_live_restart(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    let mut config = match state.strategy_runner.get(&id) {
        Some(r) => r.config.clone(),
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "runner not found" }))).into_response(),
    };

    if let Err(e) = rehydrate_live_runner_config(&state, &mut config).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": friendly_live_error(&e.to_string()) })),
        ).into_response();
    }

    let (workspace_dir, cfg_path) = {
        let c = state.config.lock();
        (c.workspace_dir.clone(), c.config_path.clone())
    };
    let runner = crate::strategy_runner::start_runner(
        state.strategy_runner.clone(),
        config,
        workspace_dir,
        Some(cfg_path),
    );
    Json(serde_json::json!({ "runner": runner })).into_response()
}

/// POST /api/live/strategies/{id}/sync-onchain — reconcile untracked onchain
/// transactions against the runner's live_orders log. Inserts any TRADE events
/// from the last 48 h that are missing from the log as UNTRACKED records.
pub async fn handle_api_live_sync_onchain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    // Need the wallet address — fall back to global polymarket config
    let wallet = {
        let gcfg = state.config.lock();
        gcfg.polymarket.proxy_address.clone()
            .unwrap_or_else(|| gcfg.polymarket.wallet_address.clone().unwrap_or_default())
    };
    // Verify runner exists
    if state.strategy_runner.get(&id).is_none() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"runner not found"}))).into_response();
    }

    if wallet.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"no wallet address found for this runner"}))).into_response();
    }

    crate::strategy_runner::reconcile_untracked_onchain_pub(
        &state.strategy_runner,
        &id,
        &wallet,
    ).await;

    Json(serde_json::json!({"success": true, "wallet": wallet})).into_response()
}

/// POST /api/live/stop-all-live — emergency stop all running live-mode runners.
pub async fn handle_api_live_stop_all(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let stopped = state.strategy_runner.stop_all_live();
    Json(serde_json::json!({
        "success": true,
        "stopped_count": stopped.len(),
        "stopped_ids": stopped,
    })).into_response()
}

// ── Export / Import ──────────────────────────────────────────────────────────

/// GET /api/export — download a ZIP with config, wallets, and scripts
pub async fn handle_api_export(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();

    let zip_bytes = match build_export_zip(&config).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Export failed: {e}")})),
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/zip"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"traderclaw-export.zip\"",
            ),
        ],
        zip_bytes,
    )
        .into_response()
}

async fn build_export_zip(config: &crate::config::Config) -> anyhow::Result<Vec<u8>> {
    // Collect all file content first (async), then build zip (sync/blocking)
    let masked = mask_sensitive_fields(config);
    let toml_str = toml::to_string_pretty(&masked).unwrap_or_default();

    let wallets_path = super::wallets_file_path(&config.config_path);
    let wallets_bytes = if wallets_path.exists() {
        tokio::fs::read(&wallets_path).await.unwrap_or_default()
    } else {
        vec![]
    };

    let mut script_files: Vec<(String, Vec<u8>)> = vec![];
    let scripts_dir = config.workspace_dir.join("scripts");
    if scripts_dir.is_dir() {
        if let Ok(mut entries) = tokio::fs::read_dir(&scripts_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("rhai") {
                    if let Ok(content) = tokio::fs::read(&path).await {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("script.rhai")
                            .to_owned();
                        script_files.push((name, content));
                    }
                }
            }
        }
    }

    // Build zip synchronously
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let cursor = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(cursor);

        zip.start_file("config.toml", opts)?;
        zip.write_all(toml_str.as_bytes())?;

        if !wallets_bytes.is_empty() {
            zip.start_file("wallets.json", opts)?;
            zip.write_all(&wallets_bytes)?;
        }

        for (name, content) in script_files {
            zip.start_file(format!("scripts/{name}"), opts)?;
            zip.write_all(&content)?;
        }

        let cursor = zip.finish()?;
        Ok::<Vec<u8>, anyhow::Error>(cursor.into_inner())
    })
    .await?
}

/// POST /api/import — upload a ZIP to restore config, wallets, and scripts
pub async fn handle_api_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();

    let b64 = match body.get("data").and_then(|v| v.as_str()) {
        Some(s) => s.to_owned(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'data' field (base64 zip)"})),
            )
                .into_response();
        }
    };

    use base64::Engine as _;
    let zip_bytes = match base64::engine::general_purpose::STANDARD.decode(&b64) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid base64: {e}")})),
            )
                .into_response();
        }
    };

    match apply_import_zip(&config, zip_bytes).await {
        Ok(imported) => Json(serde_json::json!({ "status": "ok", "imported": imported })).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("Import failed: {e}")})),
        )
            .into_response(),
    }
}

async fn apply_import_zip(config: &crate::config::Config, bytes: Vec<u8>) -> anyhow::Result<Vec<String>> {
    // Parse zip synchronously, collect files to write
    let wallets_path = super::wallets_file_path(&config.config_path);
    let scripts_dir = config.workspace_dir.join("scripts");

    let extracted: Vec<(String, Vec<u8>)> = tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let cursor = std::io::Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(cursor)?;
        let mut files = Vec::new();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let name = file.name().to_owned();
            let mut content = Vec::new();
            file.read_to_end(&mut content)?;
            files.push((name, content));
        }
        Ok::<_, anyhow::Error>(files)
    })
    .await??;

    let mut imported = Vec::new();
    for (name, content) in extracted {
        if name == "wallets.json" {
            if let Some(parent) = wallets_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(&wallets_path, &content).await?;
            imported.push("wallets.json".to_string());
        } else if name.starts_with("scripts/") && name.ends_with(".rhai") {
            tokio::fs::create_dir_all(&scripts_dir).await?;
            let filename = std::path::Path::new(&name)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("script.rhai");
            tokio::fs::write(scripts_dir.join(filename), &content).await?;
            imported.push(name.clone());
        }
    }

    Ok(imported)
}

/// GET /api/logs — return recent gateway log lines (last ~500)
pub async fn handle_api_logs(
    _headers: HeaderMap,
) -> impl IntoResponse {
    // Public endpoint — no auth required so logs can be viewed during troubleshooting
    let log_dir = directories::BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .unwrap_or_else(std::env::temp_dir)
        .join(".traderclaw")
        .join("logs");
    // Find the newest gateway log file
    let mut entries: Vec<_> = match tokio::fs::read_dir(&log_dir).await {
        Ok(mut rd) => {
            let mut v = Vec::new();
            while let Ok(Some(entry)) = rd.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("gateway") {
                    if let Ok(meta) = entry.metadata().await {
                        if let Ok(modified) = meta.modified() {
                            v.push((modified, entry.path()));
                        }
                    }
                }
            }
            v
        }
        Err(_) => Vec::new(),
    };
    entries.sort_by(|a, b| b.0.cmp(&a.0)); // newest first
    let lines: Vec<String> = if let Some((_, path)) = entries.first() {
        match tokio::fs::read_to_string(path).await {
            Ok(content) => {
                let all: Vec<String> = content.lines().map(String::from).collect();
                all.into_iter().rev().take(500).collect::<Vec<_>>().into_iter().rev().collect()
            }
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };
    Json(serde_json::json!({ "lines": lines, "file": entries.first().map(|e| e.1.to_string_lossy().to_string()).unwrap_or_default() })).into_response()
}

// ── Copy Trading handlers ─────────────────────────────────────────

/// GET /api/copy/leaders — list active watched leaders
pub async fn handle_api_copy_leaders(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let watchlist = state.copy_orchestrator.watchlist.lock().await;
    let leaders: Vec<_> = watchlist.list().iter().map(|e| serde_json::json!({
        "address": e.address,
        "venue": e.venue,
        "category": e.category,
        "mirror_enabled": e.mirror_enabled,
        "consensus_weight": e.consensus_weight,
        "wallet_score": e.wallet_score,
        "size_factor": e.size_factor,
        "live_mode": e.live_mode,
        "max_notional_per_trade": e.max_notional_per_trade,
        "max_daily_loss": e.max_daily_loss,
        "max_open_positions": e.max_open_positions,
    })).collect();

    Json(serde_json::json!({ "leaders": leaders })).into_response()
}

/// POST /api/copy/leaders/{addr}/toggle — toggle mirror for a leader
pub async fn handle_api_copy_leader_toggle(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(addr): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let mut watchlist = state.copy_orchestrator.watchlist.lock().await;
    let new_state = watchlist.toggle_mirror(&addr);
    Json(serde_json::json!({ "address": addr, "mirror_enabled": new_state })).into_response()
}

/// Request body for adding a leader directly to the watchlist.
#[derive(Debug, Deserialize)]
pub struct AddLeaderRequest {
    pub address: String,
    #[serde(default = "default_venue")]
    pub venue: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default = "default_size_factor")]
    pub size_factor: f64,
    #[serde(default = "default_consensus_weight")]
    pub consensus_weight: f64,
    #[serde(default)]
    pub wallet_score: Option<f64>,
    #[serde(default)]
    pub mirror_enabled: bool,
}

fn default_venue() -> String {
    "polymarket".to_string()
}
fn default_size_factor() -> f64 {
    0.5
}
fn default_consensus_weight() -> f64 {
    1.0
}

/// POST /api/copy/leaders — add a leader directly to the watchlist (bypasses Discovery)
pub async fn handle_api_copy_leader_add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AddLeaderRequest>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let addr = req.address.trim().to_lowercase();
    if !is_valid_evm_address(&addr) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid wallet address — expected 0x + 40 hex chars" })),
        )
            .into_response();
    }

    let entry = copy_orchestrator::watchlist::WatchlistEntry {
        address: addr.clone(),
        venue: req.venue,
        category: req.category,
        mirror_enabled: req.mirror_enabled,
        consensus_weight: req.consensus_weight,
        size_factor: req.size_factor,
        wallet_score: req.wallet_score.unwrap_or(0.0),
        added_at: chrono::Utc::now().to_rfc3339(),
        ..Default::default()  // live_mode=false (Dry Run), guardrails None
    };

    {
        let mut watchlist = state.copy_orchestrator.watchlist.lock().await;
        watchlist.add(entry);
    }
    refresh_polymarket_tracker(&state).await;
    Json(serde_json::json!({ "added": addr })).into_response()
}

/// Patch body for a leader.
#[derive(Debug, Deserialize)]
pub struct PatchLeaderRequest {
    #[serde(default)]
    pub size_factor: Option<f64>,
    #[serde(default)]
    pub consensus_weight: Option<f64>,
    #[serde(default)]
    pub category: Option<Option<String>>,
    #[serde(default)]
    pub mirror_enabled: Option<bool>,
    // Dry Run / Live mode + per-leader guardrails (only enforced in Live)
    #[serde(default)]
    pub live_mode: Option<bool>,
    #[serde(default)]
    pub max_notional_per_trade: Option<f64>,
    #[serde(default)]
    pub max_daily_loss: Option<f64>,
    #[serde(default)]
    pub max_open_positions: Option<u32>,
}

/// PATCH /api/copy/leaders/{addr} — edit leader knobs
pub async fn handle_api_copy_leader_patch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(addr): Path<String>,
    Json(req): Json<PatchLeaderRequest>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let mut watchlist = state.copy_orchestrator.watchlist.lock().await;
    let updated = watchlist.update(
        &addr,
        req.size_factor,
        req.consensus_weight,
        req.category,
        req.mirror_enabled,
    );
    // Apply Dry Run / Live mode + guardrails if provided
    if req.live_mode.is_some() || req.max_notional_per_trade.is_some()
        || req.max_daily_loss.is_some() || req.max_open_positions.is_some() {
        watchlist.set_mode(&addr, req.live_mode, req.max_notional_per_trade,
            req.max_daily_loss, req.max_open_positions);
    }
    if !updated {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Leader not found" })),
        )
            .into_response();
    }
    Json(serde_json::json!({ "address": addr, "updated": true })).into_response()
}

/// POST /api/copy/leaders/{addr}/validate — run the 3-leg edge validator on this
/// wallet's onchain fills (via data-api), tag as HFT if >100 trades/hour, and
/// persist the score back to the watchlist. Score scale: 0 = no_edge/hft, 100 = edge.
/// Returns the full ValidationResult + HFT flag so the UI can show the breakdown.
pub async fn handle_api_copy_leader_validate(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(addr): axum::extract::Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    let trades = match fetch_wallet_trades(&addr, 2_500).await {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": format!("Activity fetch failed: {e}") })))
            .into_response(),
    };

    let entries: Vec<f64> = trades.iter().map(|(p, _)| *p).collect();
    let wons: Vec<bool> = trades.iter().map(|(_, w)| *w).collect();
    let result = crate::tools::edge_validator::validate(&entries, &wons, 5_000);

    // Rough HFT detection from n_trades / observed span
    let n = trades.len();
    let is_hft = n > 500; // simplified: if we got 500+ resolved pairs in 2500 events, HFT-like

    // Score: EDGE = 80, NO_EDGE with n >= 30 = 10, HFT = 0, INSUFFICIENT = 30 (unknown)
    let score: f64 = if is_hft { 0.0 }
        else { match result.verdict.as_str() {
            "EDGE" => 80.0,
            "NO_EDGE" => 10.0,
            _ => 30.0, // INSUFFICIENT — not enough data, neutral
        }};

    // Persist score
    {
        let mut watchlist = state.copy_orchestrator.watchlist.lock().await;
        watchlist.update_score(&addr, score);
    }

    Json(serde_json::json!({
        "address": addr,
        "n_resolved_trades": n,
        "is_hft": is_hft,
        "score": score,
        "result": result,
    })).into_response()
}

/// DELETE /api/copy/leaders/{addr} — remove a leader from the watchlist
pub async fn handle_api_copy_leader_remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(addr): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let removed = {
        let mut watchlist = state.copy_orchestrator.watchlist.lock().await;
        watchlist.remove(&addr).is_some()
    };
    if !removed {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Leader not found" })),
        )
            .into_response();
    }
    refresh_polymarket_tracker(&state).await;
    Json(serde_json::json!({ "address": addr, "removed": true })).into_response()
}

/// Validate a 0x-prefixed 20-byte EVM address.
fn is_valid_evm_address(addr: &str) -> bool {
    if !addr.starts_with("0x") || addr.len() != 42 {
        return false;
    }
    addr[2..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Re-seed the Polymarket wallet tracker from the current watchlist + the
/// SQLite candidate list.  Call this after any add/remove on either store so
/// that the background poller picks up the change on its next tick.
async fn refresh_polymarket_tracker(state: &AppState) {
    let candidates = match state.wallet_indexer.list_candidates(None).await {
        Ok(c) => c
            .into_iter()
            .filter(|c| c.venue.eq_ignore_ascii_case("polymarket"))
            .filter(|c| c.status == "candidate" || c.status == "graduated")
            .map(|c| c.wallet_address)
            .collect::<Vec<_>>(),
        Err(e) => {
            tracing::warn!("[copy] tracker refresh: failed to list candidates: {e}");
            Vec::new()
        }
    };
    state
        .copy_orchestrator
        .refresh_polymarket_tracker(candidates)
        .await;
}

/// Request body for adding a discovery candidate manually.
#[derive(Debug, Deserialize)]
pub struct AddCandidateRequest {
    pub address: String,
    #[serde(default = "default_venue")]
    pub venue: String,
    #[serde(default)]
    pub discovery_score: Option<f64>,
}

/// POST /api/copy/discovery — manually add a wallet to the candidate list
pub async fn handle_api_copy_discovery_add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AddCandidateRequest>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let addr = req.address.trim().to_lowercase();
    if !is_valid_evm_address(&addr) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid wallet address — expected 0x + 40 hex chars" })),
        )
            .into_response();
    }
    let score = req.discovery_score.unwrap_or(0.0);
    match state.wallet_indexer.add_candidate(&addr, &req.venue, score).await {
        Ok(()) => {
            refresh_polymarket_tracker(&state).await;
            Json(serde_json::json!({ "added": addr, "venue": req.venue, "discovery_score": score })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// POST /api/copy/discovery/refresh — trigger the nightly Polymarket indexer on demand
pub async fn handle_api_copy_discovery_refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let indexer = state.wallet_indexer.clone();
    tokio::spawn(async move {
        if let Err(e) = indexer.run_polymarket_nightly(50).await {
            tracing::warn!("Manual Polymarket indexer refresh failed: {}", e);
        }
    });
    Json(serde_json::json!({ "started": true })).into_response()
}

/// GET /api/copy/discovery — list discovery candidates
pub async fn handle_api_copy_discovery(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    match state.wallet_indexer.list_candidates(None).await {
        Ok(candidates) => Json(serde_json::json!({ "candidates": candidates })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// POST /api/copy/discovery/{addr}/graduate — promote candidate to watchlist
pub async fn handle_api_copy_discovery_graduate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(addr): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    // Fetch candidate info from indexer
    let candidates = match state.wallet_indexer.list_candidates(None).await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let candidate = match candidates.into_iter().find(|c| c.wallet_address == addr) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Candidate not found" })),
            )
                .into_response();
        }
    };

    // Add to watchlist
    let entry = copy_orchestrator::watchlist::WatchlistEntry {
        address: candidate.wallet_address.clone(),
        venue: candidate.venue.clone(),
        category: None,
        mirror_enabled: false,
        consensus_weight: 1.0,
        size_factor: 0.5,
        wallet_score: candidate.discovery_score,
        added_at: chrono::Utc::now().to_rfc3339(),
        ..Default::default()
    };

    {
        let mut watchlist = state.copy_orchestrator.watchlist.lock().await;
        watchlist.add(entry);
    }

    // Reflect the new status on the candidate row so it disappears from the
    // candidate filter in the UI.
    if let Err(e) = state.wallet_indexer.set_candidate_status(&addr, "graduated").await {
        tracing::warn!("Failed to update candidate status for {}: {}", addr, e);
    }

    refresh_polymarket_tracker(&state).await;
    Json(serde_json::json!({ "graduated": addr })).into_response()
}

/// POST /api/copy/discovery/{addr}/blacklist — reject a candidate
pub async fn handle_api_copy_discovery_blacklist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(addr): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    match state.wallet_indexer.set_candidate_status(&addr, "blacklisted").await {
        Ok(true) => {
            refresh_polymarket_tracker(&state).await;
            Json(serde_json::json!({ "blacklisted": addr })).into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Candidate not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// DELETE /api/copy/discovery/{addr} — remove a candidate entirely
pub async fn handle_api_copy_discovery_remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(addr): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    match state.wallet_indexer.remove_candidate(&addr).await {
        Ok(true) => {
            refresh_polymarket_tracker(&state).await;
            Json(serde_json::json!({ "removed": addr })).into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Candidate not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /api/copy/discovery/{addr}/stats — tracked trade stats + recent fills for a candidate.
/// Powers the Discovery page per-wallet detail panel.
pub async fn handle_api_copy_discovery_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(addr): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    match state.wallet_indexer.get_discovery_stats(&addr, 10).await {
        Ok(stats) => Json(serde_json::to_value(&stats).unwrap_or_default()).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /api/copy/positions — list open mirror positions
pub async fn handle_api_copy_positions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let tracker = state.copy_orchestrator.mirror.lock().await;
    let positions: Vec<_> = tracker.list_open().iter().map(|p| serde_json::json!({
        "leader_address": p.leader_address,
        "leader_fill_id": p.leader_fill_id,
        "venue": p.venue,
        "symbol": p.symbol,
        "side": p.side,
        "notional": p.notional,
        "entry_price": p.entry_price,
        "status": format!("{:?}", p.status),
        "opened_at": p.opened_at,
    })).collect();

    Json(serde_json::json!({ "positions": positions })).into_response()
}

// ── Copy Trading – capital / sizing / consensus / history ─────────

/// GET /api/copy/capital
pub async fn handle_api_copy_get_capital(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let capital = *state.copy_orchestrator.capital.lock().await;
    Json(serde_json::json!({ "capital_usd": capital })).into_response()
}

/// POST /api/copy/capital  { capital_usd: f64 }
pub async fn handle_api_copy_set_capital(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let Some(capital) = body.get("capital_usd").and_then(|v| v.as_f64()) else {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "capital_usd required"}))).into_response();
    };
    state.copy_orchestrator.set_capital(capital).await;
    Json(serde_json::json!({ "capital_usd": capital, "ok": true })).into_response()
}

/// GET /api/copy/positions/history — closed mirror positions
pub async fn handle_api_copy_positions_history(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let tracker = state.copy_orchestrator.mirror.lock().await;
    let positions: Vec<_> = tracker.list_all().iter()
        .filter(|p| !matches!(p.status, copy_orchestrator::mirror::PositionStatus::Open))
        .map(|p| serde_json::json!({
            "leader_address": p.leader_address,
            "symbol": p.symbol,
            "side": p.side,
            "notional": p.notional,
            "entry_price": p.entry_price,
            "status": format!("{:?}", p.status),
            "opened_at": p.opened_at,
            "closed_at": p.closed_at,
            "pnl": p.pnl,
        }))
        .collect();
    Json(serde_json::json!({ "positions": positions })).into_response()
}

/// GET /api/copy/consensus
pub async fn handle_api_copy_consensus(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let acc = state.copy_orchestrator.consensus.lock().await;
    let windows: Vec<_> = acc.list_active_windows(300).iter().map(|w| serde_json::json!({
        "symbol": w.symbol,
        "side": w.side,
        "leader_count": w.leaders.len(),
        "first_seen": w.first_seen.to_rfc3339(),
        "last_seen": w.last_seen.to_rfc3339(),
    })).collect();
    Json(serde_json::json!({ "windows": windows })).into_response()
}

/// PATCH /api/copy/sizing  { max_single_trade_pct?: f64, liquidity_floor_factor?: f64 }
/// Returns current values (live patching not supported without mutex; adjust and restart).
pub async fn handle_api_copy_patch_sizing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(_body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let sizing = &state.copy_orchestrator.sizing;
    Json(serde_json::json!({
        "max_single_trade_pct": sizing.max_single_trade_pct,
        "liquidity_floor_factor": sizing.liquidity_floor_factor,
        "note": "Live patching not yet supported; values shown are current."
    })).into_response()
}

/// GET /api/copy/score/{addr}
pub async fn handle_api_copy_score(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(addr): axum::extract::Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    match state.wallet_indexer.get_wallet_score(&addr).await {
        Ok(Some(score)) => Json(serde_json::to_value(&score).unwrap_or_default()).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "score not found"}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

/// GET /api/copy/leaders/{addr}/trades
pub async fn handle_api_copy_leader_trades(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(addr): axum::extract::Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    match state.wallet_indexer.get_leader_trades(&addr).await {
        Ok(trades) => Json(serde_json::json!({ "trades": trades })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

/// GET /api/copy/tracker/activity — recent fills observed by the Polymarket
/// tracker plus their dispatch outcome.  Surfaces Discovery activity so the
/// user can confirm wallets are being polled even when the dispatcher drops
/// the fill due to a low score / not-in-watchlist.
pub async fn handle_api_copy_tracker_activity(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }
    let entries = state.copy_orchestrator.recent_activity(200).await;
    Json(serde_json::json!({ "activity": entries })).into_response()
}

// ── Hyperliquid ─────────────────────────────────────────────────

/// GET /api/health/hyperliquid — Hyperliquid API connectivity check
pub async fn handle_api_health_hyperliquid(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let Some(ref client) = state.hyperliquid_client else {
        return Json(serde_json::json!({
            "status": "disabled",
            "message": "Hyperliquid is not enabled in config"
        })).into_response();
    };

    match client.mids().await {
        Ok(mids) => {
            let btc = mids.get("BTC").copied();
            Json(serde_json::json!({
                "status": "ok",
                "connected": true,
                "assets_tracked": mids.len(),
                "btc_mid": btc,
            })).into_response()
        }
        Err(e) => {
            tracing::warn!("Hyperliquid health check failed: {}", e);
            Json(serde_json::json!({
                "status": "error",
                "connected": false,
                "message": format!("{}", e),
            })).into_response()
        }
    }
}

/// GET /api/hyperliquid/mids — current mid prices for all assets
pub async fn handle_api_hyperliquid_mids(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let Some(ref client) = state.hyperliquid_client else {
        return Json(serde_json::json!({
            "status": "disabled",
            "message": "Hyperliquid is not enabled in config"
        })).into_response();
    };

    match client.mids().await {
        Ok(mids) => {
            Json(serde_json::json!({
                "status": "ok",
                "mids": mids,
            })).into_response()
        }
        Err(e) => {
            tracing::warn!("Hyperliquid mids failed: {}", e);
            Json(serde_json::json!({
                "status": "error",
                "message": format!("{}", e),
            })).into_response()
        }
    }
}

/// GET /api/hyperliquid/funding — funding rate for a coin or all predicted
pub async fn handle_api_hyperliquid_funding(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let Some(ref client) = state.hyperliquid_client else {
        return Json(serde_json::json!({
            "status": "disabled",
            "message": "Hyperliquid is not enabled in config"
        })).into_response();
    };

    if let Some(coin) = params.get("coin") {
        match client.funding_rate(coin).await {
            Ok(rate) => {
                Json(serde_json::json!({
                    "status": "ok",
                    "coin": coin,
                    "funding_rate": rate.funding_rate,
                    "next_funding_time": rate.next_funding_time,
                })).into_response()
            }
            Err(e) => {
                tracing::warn!("Hyperliquid funding rate failed for {}: {}", coin, e);
                Json(serde_json::json!({
                    "status": "error",
                    "message": format!("{}", e),
                })).into_response()
            }
        }
    } else {
        match client.predicted_funding().await {
            Ok(rates) => {
                Json(serde_json::json!({
                    "status": "ok",
                    "predicted_funding": rates,
                })).into_response()
            }
            Err(e) => {
                tracing::warn!("Hyperliquid predicted funding failed: {}", e);
                Json(serde_json::json!({
                    "status": "error",
                    "message": format!("{}", e),
                })).into_response()
            }
        }
    }
}

/// GET /api/funding/comparison — cross-venue funding rate comparison
pub async fn handle_api_funding_comparison(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let watchlist: Vec<String> = params.get("symbols")
        .map(|s| s.split(',').map(|c| c.trim().to_uppercase()).collect())
        .unwrap_or_else(|| vec!["BTC".into(), "ETH".into(), "SOL".into(), "AVAX".into()]);

    // Fetch Hyperliquid predicted funding
    let hl_client = state.hyperliquid_client.clone()
        .unwrap_or_else(|| Arc::new(hyperliquid_trader::HyperliquidClient::new_mainnet()));

    let hl_rates = match hl_client.predicted_funding().await {
        Ok(rates) => rates,
        Err(e) => {
            tracing::warn!("Funding comparison: HL predicted funding failed: {}", e);
            std::collections::HashMap::new()
        }
    };

    // Fetch Binance funding rates
    let binance_rates = match fetch_binance_funding_for_comparison(&watchlist).await {
        Ok(rates) => rates,
        Err(e) => {
            tracing::warn!("Funding comparison: Binance funding failed: {}", e);
            std::collections::HashMap::new()
        }
    };

    let mut results = Vec::new();
    for coin in &watchlist {
        let hl_raw = hl_rates.get(coin).copied().unwrap_or(0.0);
        let bin_raw = binance_rates.get(coin).copied().unwrap_or(0.0);

        let hl_apr = hl_raw * 24.0 * 365.0;
        let bin_apr = bin_raw * 3.0 * 365.0;
        let diff_apr = (hl_apr - bin_apr).abs();

        let recommendation = if diff_apr < 0.02 {
            "hold"
        } else if hl_apr > bin_apr {
            "short_hl_long_binance"
        } else {
            "long_hl_short_binance"
        };

        results.push(serde_json::json!({
            "symbol": coin,
            "hyperliquid": {
                "rate": hl_raw,
                "apr": hl_apr,
            },
            "binance": {
                "rate": bin_raw,
                "apr": bin_apr,
            },
            "diff_apr": diff_apr,
            "recommendation": recommendation,
        }));
    }

    Json(serde_json::json!({
        "status": "ok",
        "timestamp": chrono::Utc::now().timestamp_millis(),
        "rates": results,
    })).into_response()
}

async fn fetch_binance_funding_for_comparison(
    watchlist: &[String],
) -> anyhow::Result<std::collections::HashMap<String, f64>> {
    let client = reqwest::Client::new();
    let url = "https://fapi.binance.com/fapi/v1/premiumIndex";
    let resp = client.get(url).send().await?;
    let arr: Vec<serde_json::Value> = resp.json().await?;

    let mut rates = std::collections::HashMap::new();
    for item in &arr {
        let symbol = item["symbol"].as_str().unwrap_or("");
        let rate_str = item["lastFundingRate"].as_str().unwrap_or("0");
        let rate: f64 = rate_str.parse().unwrap_or(0.0);
        for coin in watchlist {
            if symbol == format!("{}USDT", coin) {
                rates.insert(coin.clone(), rate);
            }
        }
    }
    Ok(rates)
}

// ── Live CEX Positions ─────────────────────────────────────────

/// GET /api/live/positions — list open Hyperliquid positions for an address.
/// Query `address` overrides the configured hyperliquid.wallet_address.
pub async fn handle_api_live_positions(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let address = params.get("address").cloned().or_else(|| {
        let label = { state.config.lock().hyperliquid.wallet_label.clone()? };
        let addr = {
            let wallets = state.wallets.lock();
            wallets.iter()
                .find(|w| w.chain == "evm" && w.label == label)
                .map(|w| w.address.clone())
        };
        addr
    });
    let Some(address) = address else {
        return Json(serde_json::json!({
            "status": "error",
            "message": "No address provided and no hyperliquid wallet configured"
        })).into_response();
    };

    let client = state.hyperliquid_client.clone().unwrap_or_else(|| {
        Arc::new(hyperliquid_trader::HyperliquidClient::new_mainnet())
    });

    match client.clearinghouse_state(&address).await {
        Ok(chs) => {
            let positions: Vec<serde_json::Value> = chs.asset_positions.iter().map(|ap| {
                serde_json::json!({
                    "coin": ap.position.coin,
                    "size": ap.position.szi,
                    "entry_price": ap.position.entry_px,
                    "position_value": ap.position.position_value,
                    "unrealized_pnl": ap.position.unrealized_pnl,
                    "leverage": ap.position.leverage,
                    "margin_used": ap.position.margin_used,
                })
            }).collect();
            Json(serde_json::json!({
                "status": "ok",
                "address": address,
                "account_value": chs.margin_summary.account_value.parse::<f64>().unwrap_or(0.0),
                "positions": positions,
            })).into_response()
        }
        Err(e) => {
            tracing::warn!("Hyperliquid clearinghouse_state failed for {}: {}", address, e);
            Json(serde_json::json!({
                "status": "error",
                "message": format!("{}", e),
            })).into_response()
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct ClosePositionBody {
    pub wallet_password: String,
}

/// POST /api/live/positions/{symbol}/close — manually close a Hyperliquid position.
#[axum::debug_handler]
pub async fn handle_api_live_position_close(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(symbol): Path<String>,
    Json(body): Json<ClosePositionBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let label = match { state.config.lock().hyperliquid.wallet_label.clone() } {
        Some(l) => l,
        None => {
            return Json(serde_json::json!({
                "status": "error",
                "message": "hyperliquid.wallet_label is not configured"
            })).into_response();
        }
    };

    let wallet = {
        let wallets = state.wallets.lock();
        match wallets.iter().find(|w| w.chain == "evm" && w.label == label) {
            Some(w) => w.clone(),
            None => {
                return Json(serde_json::json!({
                    "status": "error",
                    "message": format!("EVM wallet with label '{}' not found", label)
                })).into_response();
            }
        }
    };

    let pk_hex = match wallet_manager::evm::export_private_key(&wallet.encrypted_key, &body.wallet_password
    ) {
        Ok(k) => k,
        Err(e) => {
            return Json(serde_json::json!({
                "status": "error",
                "message": format!("Failed to decrypt wallet: {e}")
            })).into_response();
        }
    };

    let signer = match hyperliquid_trader::exchange::Signer::from_pk_bytes(pk_hex) {
        Ok(s) => s,
        Err(e) => {
            return Json(serde_json::json!({
                "status": "error",
                "message": format!("Invalid private key: {e}")
            })).into_response();
        }
    };

    let client = hyperliquid_trader::HyperliquidClient::new_mainnet_with_signer(signer);
    match client.close_position(&symbol).await {
        Ok(resp) => {
            Json(serde_json::json!({
                "status": "ok",
                "symbol": symbol,
                "order_id": resp.order_id,
            })).into_response()
        }
        Err(e) => {
            tracing::warn!("Hyperliquid close_position failed for {}: {}", symbol, e);
            Json(serde_json::json!({
                "status": "error",
                "message": format!("{}", e),
            })).into_response()
        }
    }
}

// ── General Trading Risk ────────────────────────────────────────

/// POST /api/risk/halt — manual kill-switch
pub async fn handle_api_risk_halt(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let Some(ref gate) = state.trading_risk_gate else {
        return Json(serde_json::json!({
            "status": "disabled",
            "message": "Trading risk gate is not initialized"
        })).into_response();
    };

    gate.halt_all();
    Json(serde_json::json!({
        "status": "halted",
        "message": "All trading halted via kill-switch"
    })).into_response()
}

/// POST /api/risk/resume — clear manual halt
pub async fn handle_api_risk_resume(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let Some(ref gate) = state.trading_risk_gate else {
        return Json(serde_json::json!({
            "status": "disabled",
            "message": "Trading risk gate is not initialized"
        })).into_response();
    };

    gate.resume_all();
    Json(serde_json::json!({
        "status": "resumed",
        "message": "Trading resumed"
    })).into_response()
}

/// GET /api/risk/status — current risk gate state
pub async fn handle_api_risk_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let Some(ref gate) = state.trading_risk_gate else {
        return Json(serde_json::json!({
            "status": "disabled",
            "message": "Trading risk gate is not initialized"
        })).into_response();
    };

    let st = gate.status();
    Json(serde_json::json!({
        "status": if gate.is_halted() { "halted" } else { "ok" },
        "total_capital": st.total_capital,
        "drawdown_pct": st.drawdown_pct,
        "daily_pnl_pct": st.daily_pnl_pct,
        "open_positions": st.total_positions,
    })).into_response()
}

/// GET /api/portfolio-guard/status — Polymarket wallet-level guard state for the /live
/// widget. Reads the REAL balance, compares to the earliest balance snapshot (baseline),
/// and reports drawdown + how many live runners are running. The guard halts all live
/// runners at -50% (see spawn_portfolio_guard).
pub async fn handle_api_portfolio_guard_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    // Live runner count
    let runners = state.strategy_runner.list();
    let live_running = runners.iter()
        .filter(|r| r.config.mode == "live" && r.status.status == "running")
        .count();

    // Current + baseline balance from snapshots
    let snap_path = state.config.lock().workspace_dir.join("data").join("balance_snapshots.jsonl");
    let (baseline, current) = std::fs::read_to_string(&snap_path).ok()
        .map(|c| {
            let bals: Vec<f64> = c.lines()
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .filter_map(|v| v.get("balance").and_then(|b| b.as_f64()))
                .collect();
            (bals.first().copied().unwrap_or(0.0), bals.last().copied().unwrap_or(0.0))
        })
        .unwrap_or((0.0, 0.0));
    let drawdown_pct = if baseline > 0.0 { (baseline - current) / baseline * 100.0 } else { 0.0 };

    Json(serde_json::json!({
        "live_runners_running": live_running,
        "baseline_usdc": baseline,
        "current_usdc": current,
        "drawdown_pct": drawdown_pct,
        "halt_threshold_pct": 50.0,
        "status": if drawdown_pct >= 50.0 { "BREACH" } else if drawdown_pct >= 30.0 { "WARNING" } else { "OK" },
    })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masking_keeps_toml_valid_and_preserves_api_keys_type() {
        let mut cfg = crate::config::Config::default();
        cfg.api_key = Some("sk-live-123".to_string());
        cfg.reliability.api_keys = vec!["rk-1".to_string(), "rk-2".to_string()];
        cfg.gateway.paired_tokens = vec!["pair-token-1".to_string()];
        cfg.tunnel.cloudflare = Some(crate::config::schema::CloudflareTunnelConfig {
            token: "cf-token".to_string(),
        });
        cfg.memory.qdrant.api_key = Some("qdrant-key".to_string());
        cfg.channels_config.wati = Some(crate::config::schema::WatiConfig {
            api_token: "wati-token".to_string(),
            api_url: "https://live-mt-server.wati.io".to_string(),
            tenant_id: None,
            allowed_numbers: vec![],
        });
        cfg.channels_config.feishu = Some(crate::config::schema::FeishuConfig {
            app_id: "cli_aabbcc".to_string(),
            app_secret: "feishu-secret".to_string(),
            encrypt_key: Some("feishu-encrypt".to_string()),
            verification_token: Some("feishu-verify".to_string()),
            allowed_users: vec!["*".to_string()],
            receive_mode: crate::config::schema::LarkReceiveMode::Websocket,
            port: None,
        });
        cfg.model_routes = vec![crate::config::schema::ModelRouteConfig {
            hint: "reasoning".to_string(),
            provider: "openrouter".to_string(),
            model: "anthropic/claude-sonnet-4.6".to_string(),
            api_key: Some("route-model-key".to_string()),
        }];
        cfg.embedding_routes = vec![crate::config::schema::EmbeddingRouteConfig {
            hint: "semantic".to_string(),
            provider: "openai".to_string(),
            model: "text-embedding-3-small".to_string(),
            dimensions: Some(1536),
            api_key: Some("route-embed-key".to_string()),
        }];

        let masked = mask_sensitive_fields(&cfg);
        let toml = toml::to_string_pretty(&masked).expect("masked config should serialize");
        let parsed: crate::config::Config =
            toml::from_str(&toml).expect("masked config should remain valid TOML for Config");

        assert_eq!(parsed.api_key.as_deref(), Some(MASKED_SECRET));
        assert_eq!(
            parsed.reliability.api_keys,
            vec![MASKED_SECRET.to_string(), MASKED_SECRET.to_string()]
        );
        assert_eq!(
            parsed.gateway.paired_tokens,
            vec![MASKED_SECRET.to_string()]
        );
        assert_eq!(
            parsed.tunnel.cloudflare.as_ref().map(|v| v.token.as_str()),
            Some(MASKED_SECRET)
        );
        assert_eq!(
            parsed
                .channels_config
                .wati
                .as_ref()
                .map(|v| v.api_token.as_str()),
            Some(MASKED_SECRET)
        );
        assert_eq!(parsed.memory.qdrant.api_key.as_deref(), Some(MASKED_SECRET));
        assert_eq!(
            parsed
                .channels_config
                .feishu
                .as_ref()
                .map(|v| v.app_secret.as_str()),
            Some(MASKED_SECRET)
        );
        assert_eq!(
            parsed
                .channels_config
                .feishu
                .as_ref()
                .and_then(|v| v.encrypt_key.as_deref()),
            Some(MASKED_SECRET)
        );
        assert_eq!(
            parsed
                .channels_config
                .feishu
                .as_ref()
                .and_then(|v| v.verification_token.as_deref()),
            Some(MASKED_SECRET)
        );
        assert_eq!(
            parsed
                .model_routes
                .first()
                .and_then(|v| v.api_key.as_deref()),
            Some(MASKED_SECRET)
        );
        assert_eq!(
            parsed
                .embedding_routes
                .first()
                .and_then(|v| v.api_key.as_deref()),
            Some(MASKED_SECRET)
        );
    }

    #[test]
    fn hydrate_config_for_save_restores_masked_secrets_and_paths() {
        let mut current = crate::config::Config::default();
        current.config_path = std::path::PathBuf::from("/tmp/current/config.toml");
        current.workspace_dir = std::path::PathBuf::from("/tmp/current/workspace");
        current.api_key = Some("real-key".to_string());
        current.reliability.api_keys = vec!["r1".to_string(), "r2".to_string()];
        current.gateway.paired_tokens = vec!["pair-1".to_string(), "pair-2".to_string()];
        current.tunnel.cloudflare = Some(crate::config::schema::CloudflareTunnelConfig {
            token: "cf-token-real".to_string(),
        });
        current.tunnel.ngrok = Some(crate::config::schema::NgrokTunnelConfig {
            auth_token: "ngrok-token-real".to_string(),
            domain: None,
        });
        current.memory.qdrant.api_key = Some("qdrant-real".to_string());
        current.channels_config.wati = Some(crate::config::schema::WatiConfig {
            api_token: "wati-real".to_string(),
            api_url: "https://live-mt-server.wati.io".to_string(),
            tenant_id: None,
            allowed_numbers: vec![],
        });
        current.channels_config.feishu = Some(crate::config::schema::FeishuConfig {
            app_id: "cli_current".to_string(),
            app_secret: "feishu-secret-real".to_string(),
            encrypt_key: Some("feishu-encrypt-real".to_string()),
            verification_token: Some("feishu-verify-real".to_string()),
            allowed_users: vec!["*".to_string()],
            receive_mode: crate::config::schema::LarkReceiveMode::Websocket,
            port: None,
        });
        current.model_routes = vec![
            crate::config::schema::ModelRouteConfig {
                hint: "reasoning".to_string(),
                provider: "openrouter".to_string(),
                model: "anthropic/claude-sonnet-4.6".to_string(),
                api_key: Some("route-model-key-1".to_string()),
            },
            crate::config::schema::ModelRouteConfig {
                hint: "fast".to_string(),
                provider: "openrouter".to_string(),
                model: "openai/gpt-4.1-mini".to_string(),
                api_key: Some("route-model-key-2".to_string()),
            },
        ];
        current.embedding_routes = vec![
            crate::config::schema::EmbeddingRouteConfig {
                hint: "semantic".to_string(),
                provider: "openai".to_string(),
                model: "text-embedding-3-small".to_string(),
                dimensions: Some(1536),
                api_key: Some("route-embed-key-1".to_string()),
            },
            crate::config::schema::EmbeddingRouteConfig {
                hint: "archive".to_string(),
                provider: "custom:https://emb.example.com/v1".to_string(),
                model: "bge-m3".to_string(),
                dimensions: Some(1024),
                api_key: Some("route-embed-key-2".to_string()),
            },
        ];

        let mut incoming = mask_sensitive_fields(&current);
        incoming.default_model = Some("gpt-4.1-mini".to_string());
        // Simulate UI changing only one key and keeping the first masked.
        incoming.reliability.api_keys = vec![MASKED_SECRET.to_string(), "r2-new".to_string()];
        incoming.gateway.paired_tokens = vec![MASKED_SECRET.to_string(), "pair-2-new".to_string()];
        if let Some(cloudflare) = incoming.tunnel.cloudflare.as_mut() {
            cloudflare.token = MASKED_SECRET.to_string();
        }
        if let Some(ngrok) = incoming.tunnel.ngrok.as_mut() {
            ngrok.auth_token = MASKED_SECRET.to_string();
        }
        incoming.memory.qdrant.api_key = Some(MASKED_SECRET.to_string());
        if let Some(wati) = incoming.channels_config.wati.as_mut() {
            wati.api_token = MASKED_SECRET.to_string();
        }
        if let Some(feishu) = incoming.channels_config.feishu.as_mut() {
            feishu.app_secret = MASKED_SECRET.to_string();
            feishu.encrypt_key = Some(MASKED_SECRET.to_string());
            feishu.verification_token = Some("feishu-verify-new".to_string());
        }
        incoming.model_routes[1].api_key = Some("route-model-key-2-new".to_string());
        incoming.embedding_routes[1].api_key = Some("route-embed-key-2-new".to_string());

        let hydrated = hydrate_config_for_save(incoming, &current);

        assert_eq!(hydrated.config_path, current.config_path);
        assert_eq!(hydrated.workspace_dir, current.workspace_dir);
        assert_eq!(hydrated.api_key, current.api_key);
        assert_eq!(hydrated.default_model.as_deref(), Some("gpt-4.1-mini"));
        assert_eq!(
            hydrated.reliability.api_keys,
            vec!["r1".to_string(), "r2-new".to_string()]
        );
        assert_eq!(
            hydrated.gateway.paired_tokens,
            vec!["pair-1".to_string(), "pair-2-new".to_string()]
        );
        assert_eq!(
            hydrated
                .tunnel
                .cloudflare
                .as_ref()
                .map(|v| v.token.as_str()),
            Some("cf-token-real")
        );
        assert_eq!(
            hydrated
                .tunnel
                .ngrok
                .as_ref()
                .map(|v| v.auth_token.as_str()),
            Some("ngrok-token-real")
        );
        assert_eq!(
            hydrated.memory.qdrant.api_key.as_deref(),
            Some("qdrant-real")
        );
        assert_eq!(
            hydrated
                .channels_config
                .wati
                .as_ref()
                .map(|v| v.api_token.as_str()),
            Some("wati-real")
        );
        assert_eq!(
            hydrated
                .channels_config
                .feishu
                .as_ref()
                .map(|v| v.app_secret.as_str()),
            Some("feishu-secret-real")
        );
        assert_eq!(
            hydrated
                .channels_config
                .feishu
                .as_ref()
                .and_then(|v| v.encrypt_key.as_deref()),
            Some("feishu-encrypt-real")
        );
        assert_eq!(
            hydrated
                .channels_config
                .feishu
                .as_ref()
                .and_then(|v| v.verification_token.as_deref()),
            Some("feishu-verify-new")
        );
        assert_eq!(
            hydrated.model_routes[0].api_key.as_deref(),
            Some("route-model-key-1")
        );
        assert_eq!(
            hydrated.model_routes[1].api_key.as_deref(),
            Some("route-model-key-2-new")
        );
        assert_eq!(
            hydrated.embedding_routes[0].api_key.as_deref(),
            Some("route-embed-key-1")
        );
        assert_eq!(
            hydrated.embedding_routes[1].api_key.as_deref(),
            Some("route-embed-key-2-new")
        );
    }

    #[test]
    fn hydrate_config_for_save_restores_route_keys_by_identity_and_clears_unmatched_masks() {
        let mut current = crate::config::Config::default();
        current.model_routes = vec![
            crate::config::schema::ModelRouteConfig {
                hint: "reasoning".to_string(),
                provider: "openrouter".to_string(),
                model: "anthropic/claude-sonnet-4.6".to_string(),
                api_key: Some("route-model-key-1".to_string()),
            },
            crate::config::schema::ModelRouteConfig {
                hint: "fast".to_string(),
                provider: "openrouter".to_string(),
                model: "openai/gpt-4.1-mini".to_string(),
                api_key: Some("route-model-key-2".to_string()),
            },
        ];
        current.embedding_routes = vec![
            crate::config::schema::EmbeddingRouteConfig {
                hint: "semantic".to_string(),
                provider: "openai".to_string(),
                model: "text-embedding-3-small".to_string(),
                dimensions: Some(1536),
                api_key: Some("route-embed-key-1".to_string()),
            },
            crate::config::schema::EmbeddingRouteConfig {
                hint: "archive".to_string(),
                provider: "custom:https://emb.example.com/v1".to_string(),
                model: "bge-m3".to_string(),
                dimensions: Some(1024),
                api_key: Some("route-embed-key-2".to_string()),
            },
        ];

        let mut incoming = mask_sensitive_fields(&current);
        incoming.model_routes.swap(0, 1);
        incoming.embedding_routes.swap(0, 1);
        incoming
            .model_routes
            .push(crate::config::schema::ModelRouteConfig {
                hint: "new".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4.1".to_string(),
                api_key: Some(MASKED_SECRET.to_string()),
            });
        incoming
            .embedding_routes
            .push(crate::config::schema::EmbeddingRouteConfig {
                hint: "new-embed".to_string(),
                provider: "custom:https://emb2.example.com/v1".to_string(),
                model: "bge-small".to_string(),
                dimensions: Some(768),
                api_key: Some(MASKED_SECRET.to_string()),
            });

        let hydrated = hydrate_config_for_save(incoming, &current);

        assert_eq!(
            hydrated.model_routes[0].api_key.as_deref(),
            Some("route-model-key-2")
        );
        assert_eq!(
            hydrated.model_routes[1].api_key.as_deref(),
            Some("route-model-key-1")
        );
        assert_eq!(hydrated.model_routes[2].api_key, None);
        assert_eq!(
            hydrated.embedding_routes[0].api_key.as_deref(),
            Some("route-embed-key-2")
        );
        assert_eq!(
            hydrated.embedding_routes[1].api_key.as_deref(),
            Some("route-embed-key-1")
        );
        assert_eq!(hydrated.embedding_routes[2].api_key, None);
        assert!(hydrated
            .model_routes
            .iter()
            .all(|route| route.api_key.as_deref() != Some(MASKED_SECRET)));
        assert!(hydrated
            .embedding_routes
            .iter()
            .all(|route| route.api_key.as_deref() != Some(MASKED_SECRET)));
    }

    #[test]
    fn test_calculate_resolution_windows() {
        // Test 5m cadence (300 seconds)
        let now = 1776871250; // Some random timestamp
        let window_ts = now - (now % 300); // 1776871200

        let windows = calculate_resolution_windows(now, 300);

        assert_eq!(windows.len(), 5);
        assert_eq!(windows[0], window_ts); // current
        assert_eq!(windows[1], window_ts + 300); // next
        assert_eq!(windows[2], window_ts - 300); // prev
        assert_eq!(windows[3], window_ts + 600); // next+1
        assert_eq!(windows[4], window_ts - 600); // prev-1

        // Ensure rounding behaves properly exactly on the boundary
        let exact = 1776871200;
        let windows_exact = calculate_resolution_windows(exact, 300);
        assert_eq!(windows_exact[0], exact);
        assert_eq!(windows_exact[1], exact + 300);
        assert_eq!(windows_exact[2], exact - 300);
    }
}

// ── Active Polymarket token lookup ────────────────────────────────────────────

/// GET /api/polymarket/active-token?series_id=btc_5m
///
/// Returns the YES token condition_id, yes_token_id, and no_token_id for the
/// currently-active window of the given market series. Used by the UI to
/// auto-populate the condition_id field in the tick recorder form.
pub async fn handle_api_polymarket_active_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    let series_id = params.get("series_id").map(|s| s.as_str());

    let sid = match series_id {
        Some(s) => s,
        None => return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "series_id query param required" })),
        ).into_response(),
    };

    let series = crate::tools::series::builtin_series()
        .into_iter()
        .find(|s| s.id == sid);

    let series = match series {
        Some(s) => s,
        None => return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Unknown series_id '{sid}'") })),
        ).into_response(),
    };

    let seconds: u64 = match series.cadence.as_str() {
        "1m" => 60, "5m" => 300, "15m" => 900, "1h" => 3600, _ => 300,
    };

    let now_secs = chrono::Utc::now().timestamp() as u64;
    let windows = calculate_resolution_windows(now_secs, seconds);
    let slug_prefix = &series.slug_prefix;

    for ts in &windows {
        let target_slug = format!("{slug_prefix}-{ts}");
        match polymarket_trader::markets::get_market(&target_slug).await {
            Ok(m) if !m.yes_token_id.trim().is_empty() => {
                return Json(serde_json::json!({
                    "ok": true,
                    "series_id": sid,
                    "slug": target_slug,
                    "condition_id": m.condition_id,
                    "yes_token_id": m.yes_token_id,
                    "no_token_id": m.no_token_id,
                    "window_ts": ts,
                })).into_response();
            }
            _ => continue,
        }
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": format!("No active market found for series '{sid}' right now. Try again in a moment.")
        })),
    ).into_response()
}

// ── Tick Recorder REST API ────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct TickRecorderStartBody {
    pub slug: String,
    pub condition_id: String,
    #[serde(default = "default_binance_symbol")]
    pub binance_symbol: String,
    pub chainlink_url: Option<String>,
    /// Days of historical files to keep. Default 7. Pass 0 to DISABLE pruning entirely
    /// — required when the directory contains regenerated historical ticks from
    /// `to-ticks-multi` that must not be deleted.
    pub retain_days: Option<u64>,
}

fn default_binance_symbol() -> String { "BTCUSDT".to_string() }

/// POST /api/tick-recorder/start — start a 1-Hz CLOB tick recorder for a market.
/// Body: { slug, condition_id, binance_symbol?, chainlink_url?, retain_days? }
pub async fn handle_api_tick_recorder_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TickRecorderStartBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    let workspace_dir = state.config.lock().workspace_dir.clone();
    let mut cfg = crate::tick_recorder::TickRecorderConfig::new(
        &body.slug,
        &body.condition_id,
        &body.binance_symbol,
        &workspace_dir,
    );
    cfg.chainlink_url = body.chainlink_url;
    if let Some(rd) = body.retain_days {
        cfg.retain_days = rd;
    }

    crate::tick_recorder::start_recorder(cfg).await;

    Json(serde_json::json!({
        "ok": true,
        "slug": body.slug,
        "message": format!(
            "Tick recorder started for '{}'. Writing to {}/data/ticks/{}/",
            body.slug, workspace_dir.display(), body.slug
        ),
    })).into_response()
}

#[derive(serde::Deserialize)]
pub struct TickRecorderSlugBody {
    pub slug: String,
}

/// POST /api/tick-recorder/stop — stop a running tick recorder.
/// Body: { slug }
pub async fn handle_api_tick_recorder_stop(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TickRecorderSlugBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    let stopped = crate::tick_recorder::stop_recorder(&body.slug).await;
    Json(serde_json::json!({
        "ok": true,
        "stopped": stopped,
        "message": if stopped {
            format!("Tick recorder for '{}' stopped.", body.slug)
        } else {
            format!("No active tick recorder found for '{}'.", body.slug)
        },
    })).into_response()
}

/// GET /api/tick-recorder/status — list all running tick recorders.
pub async fn handle_api_tick_recorder_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    let running = crate::tick_recorder::running_recorders().await;
    Json(serde_json::json!({ "running": running })).into_response()
}

// ────────────────────────────────────────────────────────────────────────────
// Orderbook Archive API handlers
// ────────────────────────────────────────────────────────────────────────────

/// POST /api/orderbook/query — remote DuckDB query (no local download needed).
/// Body: { days, mode, market?, freq?, window_secs? }
pub async fn handle_api_orderbook_query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<crate::tools::orderbook::QueryRequest>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    let workspace_dir = state.config.lock().workspace_dir.clone();
    let days = body.days.clamp(1, 30).to_string();

    // For remote-query modes, limit sample hours to avoid downloading hundreds of 400MB files.
    // Each hourly file is 100–400 MB; users should "Download" for full coverage.
    let sample_hours = body.sample_hours.unwrap_or(1).clamp(1, 6).to_string();

    let result = match body.mode.as_str() {
        "summary" => {
            crate::tools::orderbook::run_parser(
                &workspace_dir,
                "summary",
                &["--days", &days, "--hours", &sample_hours],
            ).await
        }
        "top-markets" => {
            crate::tools::orderbook::run_parser(
                &workspace_dir,
                "top-markets",
                &["--days", &days, "--hours", &sample_hours],
            ).await
        }
        "price-series" => {
            let Some(ref market) = body.market else {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                    "error": "market is required for price-series mode"
                }))).into_response();
            };
            let freq = body.freq.as_deref().unwrap_or("5min");
            crate::tools::orderbook::run_parser(
                &workspace_dir,
                "price-series",
                &["--market", market, "--days", &days, "--freq", freq],
            ).await
        }
        "spread-stats" => {
            let Some(ref market) = body.market else {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                    "error": "market is required for spread-stats mode"
                }))).into_response();
            };
            crate::tools::orderbook::run_parser(
                &workspace_dir,
                "spread-stats",
                &["--market", market, "--days", &days],
            ).await
        }
        "drift" => {
            let Some(ref market) = body.market else {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                    "error": "market is required for drift mode"
                }))).into_response();
            };
            let window = body.window_secs.unwrap_or(300).to_string();
            crate::tools::orderbook::run_parser(
                &workspace_dir,
                "drift",
                &["--market", market, "--days", &days, "--window", &window],
            ).await
        }
        other => {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "error": format!("Unknown mode '{}'. Use: summary | top-markets | price-series | spread-stats | drift", other)
            }))).into_response();
        }
    };

    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "error": e.to_string()
        }))).into_response(),
    }
}

/// POST /api/orderbook/download — start a background download job.
/// Body: { days, market? }
pub async fn handle_api_orderbook_download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<crate::tools::orderbook::DownloadRequest>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    // Reject if a download is already running
    {
        let p = state.orderbook.progress.lock().await;
        if p.running {
            return (StatusCode::CONFLICT, Json(serde_json::json!({
                "error": "A download is already in progress. Cancel it first."
            }))).into_response();
        }
    }

    let days = body.days.clamp(1, 30);
    let workspace_dir = state.config.lock().workspace_dir.clone();

    crate::tools::orderbook::spawn_download(
        workspace_dir,
        days,
        body.market.clone(),
        state.orderbook.progress.clone(),
        state.orderbook.cancel.clone(),
    );

    Json(serde_json::json!({
        "ok": true,
        "days": days,
        "market": body.market,
        "message": format!("Download started for {} day(s). Poll /api/orderbook/download/status for progress.", days)
    })).into_response()
}

/// GET /api/orderbook/download/status — poll download progress.
pub async fn handle_api_orderbook_download_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let p = state.orderbook.progress.lock().await.clone();
    Json(serde_json::to_value(&p).unwrap_or_default()).into_response()
}

/// POST /api/orderbook/download/cancel — cancel ongoing download.
pub async fn handle_api_orderbook_download_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    state.orderbook.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    Json(serde_json::json!({ "ok": true, "message": "Cancel signal sent." })).into_response()
}

/// GET /api/orderbook/files — list locally downloaded Parquet files.
pub async fn handle_api_orderbook_files(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }
    let workspace_dir = state.config.lock().workspace_dir.clone();
    let files = crate::tools::orderbook::list_local_files(&workspace_dir);
    let total_mb: f64 = files.iter().map(|f| f.size_mb).sum();
    Json(serde_json::json!({
        "file_count": files.len(),
        "total_mb": total_mb,
        "files": files,
    })).into_response()
}

/// POST /api/orderbook/ingest-multi — download ALL parquets + auto-convert all 5-min series.
/// Body: { days, slugs? }
pub async fn handle_api_orderbook_ingest_multi(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<crate::tools::orderbook::IngestMultiRequest>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    {
        let p = state.orderbook.progress.lock().await;
        if p.running {
            return (StatusCode::CONFLICT, Json(serde_json::json!({
                "error": "A download/ingest job is already in progress. Cancel it first."
            }))).into_response();
        }
    }

    let days = body.days.clamp(1, 30);
    let workspace_dir = state.config.lock().workspace_dir.clone();
    let slugs_label = body.slugs.clone().unwrap_or_else(|| "btc_5m,eth_5m,sol_5m,xrp_5m,bnb_5m".to_string());

    crate::tools::orderbook::spawn_ingest_multi(
        workspace_dir,
        days,
        body.slugs.clone(),
        state.orderbook.progress.clone(),
        state.orderbook.cancel.clone(),
    );

    Json(serde_json::json!({
        "ok": true,
        "days": days,
        "slugs": slugs_label,
        "message": format!(
            "Multi-ingest started: downloading {} day(s) → auto-converting series [{}]. Poll /api/orderbook/download/status.",
            days, slugs_label
        )
    })).into_response()
}

/// POST /api/orderbook/ingest — download + auto-convert to ticks in one job.
/// Body: { days, market, slug, binance_symbol }
pub async fn handle_api_orderbook_ingest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<crate::tools::orderbook::IngestRequest>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) { return e.into_response(); }

    // Reject if already running
    {
        let p = state.orderbook.progress.lock().await;
        if p.running {
            return (StatusCode::CONFLICT, Json(serde_json::json!({
                "error": "A download/ingest job is already in progress. Cancel it first."
            }))).into_response();
        }
    }

    let days = body.days.clamp(1, 30);
    let workspace_dir = state.config.lock().workspace_dir.clone();

    crate::tools::orderbook::spawn_ingest(
        workspace_dir,
        days,
        body.market.clone(),
        body.slug.clone(),
        body.binance_symbol.clone(),
        state.orderbook.progress.clone(),
        state.orderbook.cancel.clone(),
    );

    Json(serde_json::json!({
        "ok": true,
        "days": days,
        "market": body.market,
        "slug": body.slug,
        "binance_symbol": body.binance_symbol,
        "message": format!(
            "Ingest started: downloading {} day(s) for market {} → converting to ticks/{} slug. Poll /api/orderbook/download/status for progress.",
            days, body.market, body.slug
        )
    })).into_response()
}

// ── Wallet Validator ──────────────────────────────────────────────────────────

/// A single activity event returned by the Polymarket data API.
#[derive(serde::Deserialize, Debug)]
struct PolyActivity {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    timestamp: Option<i64>,
    #[serde(rename = "conditionId", default)]
    condition_id: Option<String>,
    #[serde(default)]
    side: Option<String>,
    #[serde(default)]
    price: Option<f64>,
    #[serde(rename = "usdcSize", default)]
    usdc_size: Option<f64>,
}

/// Fetch up to `max_events` activity events for `proxy_wallet` from the Polymarket
/// data API, paginating in batches of 500 until no more pages or the limit is hit.
/// Returns `Vec<(entry_price, won)>` where entry_price is the average BUY price per
/// conditionId and won is determined by whether a REDEEM exists for that conditionId
/// with usdcSize exceeding the total USDC spent on BUYs.
pub async fn fetch_wallet_trades(
    proxy_wallet: &str,
    max_events: usize,
) -> anyhow::Result<Vec<(f64, bool)>> {
    let client = reqwest::Client::builder()
        .user_agent("trader-claw/wallet-validator")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let mut all_events: Vec<PolyActivity> = Vec::new();
    let mut offset = 0usize;
    let page_size = 500usize;

    loop {
        if all_events.len() >= max_events {
            break;
        }
        let url = format!(
            "https://data-api.polymarket.com/activity?user={proxy_wallet}&limit={page_size}&offset={offset}"
        );
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Polymarket activity fetch failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Polymarket activity API returned {status}: {body}");
        }

        let page: Vec<PolyActivity> = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse activity JSON: {e}"))?;

        let page_len = page.len();
        all_events.extend(page);
        if page_len < page_size {
            break; // last page
        }
        offset += page_size;
    }

    // Group BUYs and REDEEMs by conditionId.
    // buys: conditionId -> (sum_price_weighted, total_usdc_spent)
    // redeems: conditionId -> total_usdc_received
    use std::collections::HashMap;
    let mut buy_price_sum: HashMap<String, f64> = HashMap::new();
    let mut buy_price_count: HashMap<String, usize> = HashMap::new();
    let mut buy_usdc: HashMap<String, f64> = HashMap::new();
    let mut redeem_usdc: HashMap<String, f64> = HashMap::new();

    for event in &all_events {
        let Some(ref cid) = event.condition_id else { continue; };
        // data-api uses type="TRADE" with side="BUY"/"SELL", and type="REDEEM"
        let is_buy = event.event_type == "TRADE"
            && event.side.as_deref().map(|s| s.eq_ignore_ascii_case("buy")).unwrap_or(false);
        let is_redeem = event.event_type == "REDEEM";
        if is_buy {
            let price = event.price.unwrap_or(0.5);
            if price > 0.0 && price < 1.0 {
                let w = event.usdc_size.unwrap_or(1.0).max(1e-9);
                // weighted mean price accumulation
                *buy_price_sum.entry(cid.clone()).or_insert(0.0) += price * w;
                *buy_usdc.entry(cid.clone()).or_insert(0.0) += w;
                *buy_price_count.entry(cid.clone()).or_insert(0) += 1;
            }
        } else if is_redeem {
            let usdc = event.usdc_size.unwrap_or(0.0);
            *redeem_usdc.entry(cid.clone()).or_insert(0.0) += usdc;
        }
    }

    // Pair each conditionId that has BUYs with an outcome.
    let mut result: Vec<(f64, bool)> = Vec::new();
    for (cid, count) in &buy_price_count {
        if *count == 0 { continue; }
        let total_usdc = buy_usdc.get(cid).copied().unwrap_or(1.0).max(1e-9);
        // weighted average price (price * usdcSize / total usdcSize)
        let avg_price = buy_price_sum.get(cid).copied().unwrap_or(0.5) / total_usdc;
        let price = crate::strategy_runner::settle_price(Some(avg_price), None);
        if !(price > 0.01 && price < 0.99) { continue; }

        let spent = total_usdc;
        let received = redeem_usdc.get(cid).copied().unwrap_or(0.0);
        // Won if REDEEM exists and returned more than was spent (net positive).
        let won = received > spent && received > 0.0;

        result.push((price, won));
    }

    Ok(result)
}

/// Query parameters for GET /api/validate/wallet
#[derive(Deserialize)]
pub struct ValidateWalletQuery {
    pub address: String,
}

/// GET /api/validate/wallet?address=<0x...>
/// Fetches onchain activity for a Polymarket proxy wallet, measures trade
/// frequency (HFT detection), and runs the 3-leg edge validator.
pub async fn handle_api_validate_wallet(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<ValidateWalletQuery>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let address = params.address.trim().to_lowercase();
    if address.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "address query param is required"})),
        )
            .into_response();
    }

    // Fetch up to 10k events
    // data-api.polymarket.com: max offset is 3000, max usable pages = 3000/500 = 6 pages
    let trades = match fetch_wallet_trades(&address, 2_500).await {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": format!("Failed to fetch wallet activity: {e}")
                })),
            )
                .into_response();
        }
    };

    let n_trades = trades.len();

    // Count trades per hour for HFT detection: fetch raw events separately
    // by counting timestamps. We approximate using n_trades over the observed
    // time span from the events. Re-use trades here as a proxy: fetch
    // timestamps via a second call for a quick time-span estimate.
    // For efficiency we reuse the trades list and estimate window duration.
    // Since we paginate up to 10k we approximate the span as n_trades / observed_rate.
    // A simpler proxy: if n_trades > 100 in the first 500 (one page) → likely HFT.
    // We'll re-fetch a single page to count timestamps for a proper per-hour rate.
    let trades_per_hour: f64;
    let is_hft: bool;
    {
        // Quick time-span estimate: fetch the first and last activity pages
        // (already fetched as part of the up-to-10k loop above, but we don't
        // have timestamps there). Do a minimal re-fetch of 2 pages for ts.
        let ts_first = fetch_activity_timestamps(&address, 0, 500).await;
        let ts_last = fetch_activity_timestamps(&address, n_trades.saturating_sub(500), 500).await;
        if let (Some(first_page), Some(last_page)) = (ts_first.ok(), ts_last.ok()) {
            let newest = first_page.iter().copied().max().unwrap_or(0);
            let oldest = last_page.iter().copied().min().unwrap_or(0);
            let span_hours = if newest > oldest {
                (newest - oldest) as f64 / 3600.0
            } else {
                1.0 // default to 1h to avoid div-by-zero
            };
            let span_hours = span_hours.max(1.0);
            trades_per_hour = n_trades as f64 / span_hours;
        } else {
            trades_per_hour = 0.0;
        }
        is_hft = trades_per_hour > 100.0;
    }

    let entries: Vec<f64> = trades.iter().map(|(p, _)| *p).collect();
    let wons: Vec<bool> = trades.iter().map(|(_, w)| *w).collect();
    let validation = crate::tools::edge_validator::validate(&entries, &wons, 5000);

    Json(serde_json::json!({
        "address": address,
        "n_trades": n_trades,
        "trades_per_hour": trades_per_hour,
        "is_hft": is_hft,
        "result": validation,
    }))
    .into_response()
}

/// Helper: fetch timestamps from a single activity page (offset + limit).
async fn fetch_activity_timestamps(
    proxy_wallet: &str,
    offset: usize,
    limit: usize,
) -> anyhow::Result<Vec<i64>> {
    let client = reqwest::Client::builder()
        .user_agent("trader-claw/wallet-validator")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let url = format!(
        "https://data-api.polymarket.com/activity?user={proxy_wallet}&limit={limit}&offset={offset}"
    );
    let events: Vec<PolyActivity> = client.get(&url).send().await?.json().await?;
    Ok(events.into_iter().filter_map(|e| e.timestamp).collect())
}
