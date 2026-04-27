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

const MASKED_SECRET: &str = "***MASKED***";

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
        ..Default::default()
    };
    match polymarket_trader::markets::list_markets(filter).await {
        Ok(markets) => {
            let mut sorted = markets;
            sorted.sort_by(|a, b| b.volume.partial_cmp(&a.volume).unwrap_or(std::cmp::Ordering::Equal));
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
                        "question": m.question,
                        "yes_price": price_res.ok(),
                        "volume": m.volume,
                        "end_date": m.end_date_iso,
                        "yes_token_id": m.yes_token_id,
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

    match polymarket_trader::auth::setup_credentials(&private_key).await {
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

// ── Polymarket orders / positions helpers ────────────────────────

fn get_poly_creds(state: &AppState) -> Option<polymarket_trader::auth::PolyCredentials> {
    let cfg = state.config.lock();
    let pm = &cfg.polymarket;
    let api_key = pm.api_key.clone().filter(|k| !k.is_empty())?;
    Some(polymarket_trader::auth::PolyCredentials {
        api_key,
        secret: pm.secret.clone().unwrap_or_default(),
        passphrase: pm.passphrase.clone().unwrap_or_default(),
        wallet_address: pm.wallet_address.clone().unwrap_or_default().to_lowercase(),
        private_key: pm.private_key.clone().filter(|k| !k.is_empty()),
        is_builder: pm.is_builder.unwrap_or(false),
        proxy_address: pm.proxy_address.clone().filter(|k| !k.is_empty()).map(|s| s.to_lowercase()),
    })
}

fn get_poly_wallet_address(state: &AppState) -> Option<String> {
    let cfg = state.config.lock();
    cfg.polymarket
        .wallet_address
        .clone()
        .filter(|w| !w.trim().is_empty())
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

async fn ensure_live_wallet_has_min_balance(wallet_address: &str, min_usdc: f64) -> anyhow::Result<()> {
    let url = format!("https://clob.polymarket.com/positions?user={wallet_address}");
    let resp = reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(12))
        .send()
        .await?;

    if !resp.status().is_success() {
        let code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!(
            "Skipping Polymarket wallet balance pre-check (endpoint unavailable: {}): {}",
            code,
            body
        );
        return Ok(());
    }

    let data: serde_json::Value = resp.json().await?;

    fn as_num(v: Option<&serde_json::Value>) -> Option<f64> {
        let x = v?;
        if let Some(n) = x.as_f64() { return Some(n); }
        x.as_str().and_then(|s| s.parse::<f64>().ok())
    }

    let candidates = [
        as_num(data.get("availableBalance")),
        as_num(data.get("available_balance")),
        as_num(data.get("cash")),
        as_num(data.get("balance")),
        as_num(data.get("usdc_balance")),
    ];

    let available = candidates.into_iter().flatten().next().unwrap_or(0.0);
    if available < min_usdc {
        anyhow::bail!(
            "Insufficient wallet balance for live mode. Required at least ${:.2} USDC, detected ${:.2}.",
            min_usdc,
            available
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

/// GET /api/backtest/series u2014 list all built-in (and future user-defined) recurring market series
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

#[derive(serde::Deserialize)]
pub struct BacktestRunBody {
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

    // If a series_id was provided, resolve it to symbol/interval/resolution_logic/threshold
    let (symbol, interval, resolution_logic, threshold) = if let Some(ref sid) = body.series_id {
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
        &workspace_dir,
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

    Json(serde_json::json!({
        "script": body.script,
        "market_type": body.market_type,
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
        "final_balance": body.initial_balance * (1.0 + metrics.total_return_pct / 100.0),
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

// ── Live Strategy Runner API ──────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct CreateRunnerBody {
    pub name: Option<String>,
    pub script: String,
    pub market_type: String,
    pub symbol: String,
    pub interval: String,
    pub mode: String,
    pub initial_balance: f64,
    pub fee_pct: Option<f64>,
    pub warmup_days: Option<u32>,
    pub auto_restart: Option<bool>,
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
}

#[derive(serde::Deserialize)]
pub struct PatchRunnerBody {
    pub auto_restart: Option<bool>,
}

async fn hydrate_live_runtime_config(state: &AppState, config: &mut crate::strategy_runner::RunnerConfig) -> anyhow::Result<()> {
    if config.mode != "live" || config.market_type != "polymarket_binary" {
        return Ok(());
    }

    let poly = state.config.lock().polymarket.clone();
    let api_key = poly.api_key.unwrap_or_default();
    let secret = poly.secret.unwrap_or_default();
    let passphrase = poly.passphrase.unwrap_or_default();
    if api_key.is_empty() || secret.is_empty() || passphrase.is_empty() {
        anyhow::bail!("Live mode requires polymarket.api_key, polymarket.secret, and polymarket.passphrase in config.");
    }

    let wallet_address = get_poly_wallet_address(state)
        .ok_or_else(|| anyhow::anyhow!("Live mode requires polymarket.wallet_address. Go to Settings → Config and set your Polymarket wallet address."))?;

    let private_key = poly.private_key.filter(|k| !k.is_empty());
    if private_key.is_none() {
        anyhow::bail!("Live mode requires polymarket.private_key for EIP-712 order signing. Go to Settings → Polymarket and set your wallet private key.");
    }

    let (yes_token_id, no_token_id) = resolve_live_token_ids(config.series_id.as_deref()).await?;
    let min_live_usdc = 10.0;
    if config.mode == "live" {
        ensure_live_wallet_has_min_balance(&wallet_address, min_live_usdc).await?;
    }

    let creds = polymarket_trader::auth::PolyCredentials {
        api_key,
        secret,
        passphrase,
        wallet_address: wallet_address.clone().to_lowercase(),
        private_key,
        is_builder: poly.is_builder.unwrap_or(false),
        proxy_address: poly.proxy_address.clone().filter(|k| !k.is_empty()).map(|s| s.to_lowercase()),
    };

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
    hydrate_live_runtime_config(state, config).await
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

        let workspace_dir = state.config.lock().workspace_dir.clone();
        let _ = crate::strategy_runner::start_runner(
            state.strategy_runner.clone(),
            config,
            workspace_dir,
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

    let auto_restart = match body.auto_restart {
        Some(v) => v,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Missing auto_restart field" })),
            ).into_response();
        }
    };

    match state.strategy_runner.set_auto_restart(&id, auto_restart) {
        Some(runner) => Json(serde_json::json!({ "runner": runner })).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "runner not found" }))).into_response(),
    }
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
    if is_live && body.market_type != "polymarket_binary" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Live mode is only supported for market_type=polymarket_binary"
            })),
        ).into_response();
    }

    let mut config = crate::strategy_runner::RunnerConfig {
        id: id.clone(),
        name: body.name.unwrap_or_else(|| format!("{} on {}", body.script, symbol)),
        script: body.script,
        market_type: body.market_type,
        symbol,
        interval,
        mode: body.mode,
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
        poly_condition_id: None,
        wallet_address: None,
        chainlink_endpoint_url: None,
        chainlink_api_key: None,
        chainlink_interval_secs: 5,
        live_sizing_mode: match body.live_sizing_mode.as_deref() {
            Some("fixed") => crate::strategy_runner::LiveSizingMode::Fixed,
            _ => crate::strategy_runner::LiveSizingMode::Percent,
        },
        live_sizing_value: body.live_sizing_value.unwrap_or(0.05),
        stop_loss_pct: body.stop_loss_pct.filter(|&v| v > 0.0),
        early_fire_secs: body.early_fire_secs.or_else(|| {
            let v = state.config.lock().live_strategy.early_fire_secs;
            if v > 0 { Some(v) } else { None }
        }),
    };

    if let Err(e) = hydrate_live_runtime_config(&state, &mut config).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": friendly_live_error(&e.to_string()) })),
        ).into_response();
    }

    let workspace_dir = state.config.lock().workspace_dir.clone();
    let runner = crate::strategy_runner::start_runner(
        state.strategy_runner.clone(),
        config,
        workspace_dir,
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

    let workspace_dir = state.config.lock().workspace_dir.clone();
    let runner = crate::strategy_runner::start_runner(
        state.strategy_runner.clone(),
        config,
        workspace_dir,
    );
    Json(serde_json::json!({ "runner": runner })).into_response()
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
