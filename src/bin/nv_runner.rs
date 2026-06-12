//! nv-runner — headless single-strategy runner for the VPS (Fase 1 del
//! docs/VPS_EXECUTION_PLAN.md).
//!
//! Runs exactly ONE engine + config on a small box (4GB/1vCPU): no web
//! dashboard, no LLM, no gateway. Reuses the battle-tested `strategy_runner`
//! loops with all guardrails (kelly cap, min_entry_price, order queues,
//! resolution monitor). State persists to `<workspace>/live_strategies.json`,
//! same file the dashboard uses — copy it back to the laptop to inspect runs.
//!
//! Config: a JSON `RunnerConfig` (same shape the dashboard persists).
//! Secrets come ONLY from environment variables, never from the config file.
//!
//! Usage:
//!   nv-runner --config rewards_maker.json [--workspace ~/.traderclaw/workspace]
//!
//! Env (live mode):
//!   POLY_API_KEY / POLY_SECRET / POLY_PASSPHRASE     CLOB L2 credentials
//!   POLY_WALLET_ADDRESS / POLY_PRIVATE_KEY           EIP-712 order signer
//!   POLY_PROXY_ADDRESS / POLY_SIGNATURE_TYPE         optional (proxy wallets)
//!   POLY_IS_BUILDER=true                             optional
//! Env (optional):
//!   NV_WORKSPACE                                     workspace dir override
//!   NV_CHAINLINK_API_KEY                             oracle feed auth
//!   NV_PORTFOLIO_GUARD_PCT                           global stop-loss % (e.g. 20)
//!   RUST_LOG                                         log filter (default info)
//!
//! V1 targets Polymarket engines (rewards_maker, rhai_tick, rhai_candle,
//! arb_binary…). CEX-live engines need wallet-manager decryption that this
//! binary intentionally does not ship.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use trader_claw::strategy_runner::{
    self, RunnerConfig, RunnerStatus, StoredRunner, StrategyRunnerStore,
};

#[derive(Parser)]
#[command(name = "nv-runner", about = "Headless Neko Vives strategy runner (one engine, no dashboard)")]
struct Args {
    /// Path to the RunnerConfig JSON file.
    #[arg(long)]
    config: PathBuf,
    /// Workspace dir (state + data). Default: $NV_WORKSPACE or ~/.traderclaw/workspace
    #[arg(long)]
    workspace: Option<PathBuf>,
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn hydrate_live_creds(config: &mut RunnerConfig) -> anyhow::Result<()> {
    let wallet_address = env_opt("POLY_WALLET_ADDRESS")
        .ok_or_else(|| anyhow::anyhow!("live mode requires POLY_WALLET_ADDRESS"))?;
    let private_key = env_opt("POLY_PRIVATE_KEY")
        .ok_or_else(|| anyhow::anyhow!("live mode requires POLY_PRIVATE_KEY (EIP-712 signing)"))?;
    let creds = polymarket_trader::auth::PolyCredentials {
        api_key: env_opt("POLY_API_KEY").unwrap_or_default(),
        secret: env_opt("POLY_SECRET").unwrap_or_default(),
        passphrase: env_opt("POLY_PASSPHRASE").unwrap_or_default(),
        wallet_address: wallet_address.to_lowercase(),
        private_key: Some(private_key),
        is_builder: env_opt("POLY_IS_BUILDER").map(|v| v == "true" || v == "1").unwrap_or(false),
        proxy_address: env_opt("POLY_PROXY_ADDRESS").map(|s| s.to_lowercase()),
        signature_type: env_opt("POLY_SIGNATURE_TYPE"),
    };
    config.poly_creds = Some(creds);
    config.wallet_address = Some(wallet_address);
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let raw = std::fs::read_to_string(&args.config)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", args.config.display()))?;
    let mut config: RunnerConfig = serde_json::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("invalid RunnerConfig JSON: {e}"))?;

    if let Some(kind) = config.kind.as_deref() {
        if !strategy_core::engines::is_known(kind) {
            anyhow::bail!("unknown engine kind '{kind}'");
        }
    }
    if config.id.trim().is_empty() {
        config.id = format!("nv-{}", uuid::Uuid::new_v4());
    }
    // Process supervision (systemd Restart=) replaces the in-process auto-restart
    // wrapper; leaving it on would mark the runner "starting" forever on errors.
    config.auto_restart = false;

    let workspace = args
        .workspace
        .or_else(|| env_opt("NV_WORKSPACE").map(PathBuf::from))
        .unwrap_or_else(|| {
            directories::UserDirs::new()
                .map(|u| u.home_dir().join(".traderclaw").join("workspace"))
                .unwrap_or_else(|| PathBuf::from("./nv-workspace"))
        });
    std::fs::create_dir_all(&workspace)?;

    if config.mode == "live" {
        hydrate_live_creds(&mut config)?;
    }
    if config.chainlink_api_key.is_none() {
        config.chainlink_api_key = env_opt("NV_CHAINLINK_API_KEY");
    }

    let kind = config.kind.clone().unwrap_or_else(|| strategy_core::engines::default_kind().to_string());
    tracing::info!(
        "nv-runner starting: id={} kind={} mode={} symbol={} workspace={}",
        config.id, kind, config.mode, config.symbol, workspace.display()
    );

    let store = Arc::new(StrategyRunnerStore::new(workspace.clone()));
    // Preserve accumulated stats across restarts, same as start_runner does.
    let existing_result = store.get(&config.id).and_then(|r| r.result);
    store.upsert(StoredRunner {
        config: config.clone(),
        status: RunnerStatus {
            id: config.id.clone(),
            status: "starting".to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            last_tick_at: None,
            next_tick_at: None,
            error: None,
        },
        result: existing_result,
        hidden: false,
    });
    store.persist();

    // Official-resolution sweep patches provisional Binance settlements.
    strategy_runner::spawn_resolution_sweep(store.clone());
    if let Some(pct) = env_opt("NV_PORTFOLIO_GUARD_PCT").and_then(|v| v.parse::<f64>().ok()) {
        strategy_runner::spawn_portfolio_guard(store.clone(), pct);
    }

    let mut task = tokio::spawn(strategy_runner::runner_loop(
        store.clone(),
        config.clone(),
        workspace.clone(),
        None,
    ));

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => tracing::info!("SIGINT — shutting down"),
        _ = sigterm.recv() => tracing::info!("SIGTERM — shutting down"),
        _ = &mut task => {
            let status = store.get(&config.id).map(|r| r.status.status).unwrap_or_default();
            store.persist();
            tracing::info!("engine loop exited on its own (status={status})");
            return if status == "error" { std::process::exit(1) } else { Ok(()) };
        }
    }

    // Graceful stop: mark stopped so status-polling engines (rewards_maker)
    // cancel their resting quotes before we abort the task.
    strategy_runner::set_runner_status(&store, &config.id, "stopped");
    store.persist();
    let grace_secs = if kind == strategy_core::engines::REWARDS_MAKER { 75 } else { 5 };
    tracing::info!("waiting up to {grace_secs}s for the engine to clean up…");
    match tokio::time::timeout(Duration::from_secs(grace_secs), &mut task).await {
        Ok(_) => tracing::info!("engine exited cleanly"),
        Err(_) => {
            tracing::warn!("grace period expired — aborting engine task");
            task.abort();
        }
    }
    store.persist();
    tracing::info!("nv-runner stopped");
    Ok(())
}
