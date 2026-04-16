//! Live Strategy Runner — manages paper/live trading sessions
//! Each runner fetches live candles on an interval, runs the Rhai strategy,
//! and tracks paper trading P&L.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use tokio::task::AbortHandle;

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerConfig {
    pub id: String,
    pub name: String,
    pub script: String,
    pub market_type: String,
    pub symbol: String,
    pub interval: String,
    pub mode: String,
    pub initial_balance: f64,
    pub fee_pct: f64,
    pub warmup_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerStatus {
    pub id: String,
    pub status: String,
    pub started_at: String,
    pub last_tick_at: Option<String>,
    pub next_tick_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerResult {
    pub total_return_pct: f64,
    pub balance: f64,
    pub position: f64,
    pub total_trades: u32,
    pub win_rate_pct: f64,
    pub sharpe_ratio: f64,
    pub max_drawdown_pct: f64,
    pub all_trades: Vec<crate::tools::backtest::AllTrade>,
    pub last_signal: String,
    pub analysis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRunner {
    pub config: RunnerConfig,
    pub status: RunnerStatus,
    pub result: Option<RunnerResult>,
}

pub struct StrategyRunnerStore {
    runners: Arc<Mutex<HashMap<String, StoredRunner>>>,
    handles: Arc<Mutex<HashMap<String, AbortHandle>>>,
    workspace_dir: PathBuf,
}

// ── Store impl ───────────────────────────────────────────────────────────────

impl StrategyRunnerStore {
    pub fn new(workspace_dir: PathBuf) -> Self {
        let store = Self {
            runners: Arc::new(Mutex::new(HashMap::new())),
            handles: Arc::new(Mutex::new(HashMap::new())),
            workspace_dir,
        };
        store.load_from_disk();
        store
    }

    fn runners_file(&self) -> PathBuf {
        self.workspace_dir.join("live_strategies.json")
    }

    fn load_from_disk(&self) {
        if let Ok(data) = std::fs::read_to_string(self.runners_file()) {
            if let Ok(runners) = serde_json::from_str::<Vec<StoredRunner>>(&data) {
                let mut map = self.runners.lock().unwrap();
                for mut r in runners {
                    if r.status.status == "running" || r.status.status == "starting" {
                        r.status.status = "stopped".to_string();
                    }
                    map.insert(r.config.id.clone(), r);
                }
            }
        }
    }

    pub fn persist(&self) {
        let runners: Vec<StoredRunner> = self.runners.lock().unwrap().values().cloned().collect();
        if let Ok(json) = serde_json::to_string_pretty(&runners) {
            let _ = std::fs::write(self.runners_file(), json);
        }
    }

    pub fn list(&self) -> Vec<StoredRunner> {
        let mut runners: Vec<StoredRunner> = self.runners.lock().unwrap().values().cloned().collect();
        runners.sort_by(|a, b| a.status.started_at.cmp(&b.status.started_at));
        runners
    }

    pub fn get(&self, id: &str) -> Option<StoredRunner> {
        self.runners.lock().unwrap().get(id).cloned()
    }

    pub fn upsert(&self, runner: StoredRunner) {
        self.runners.lock().unwrap().insert(runner.config.id.clone(), runner);
        self.persist();
    }

    pub fn stop(&self, id: &str) -> bool {
        if let Some(handle) = self.handles.lock().unwrap().remove(id) {
            handle.abort();
        }
        let mut map = self.runners.lock().unwrap();
        if let Some(r) = map.get_mut(id) {
            r.status.status = "stopped".to_string();
            drop(map);
            self.persist();
            true
        } else {
            false
        }
    }

    pub fn delete(&self, id: &str) -> bool {
        self.stop(id);
        let removed = self.runners.lock().unwrap().remove(id).is_some();
        self.persist();
        removed
    }

    pub fn register_handle(&self, id: String, handle: AbortHandle) {
        self.handles.lock().unwrap().insert(id, handle);
    }
}

// ── Start a new runner ───────────────────────────────────────────────────────

pub fn start_runner(
    store: Arc<StrategyRunnerStore>,
    config: RunnerConfig,
    workspace_dir: PathBuf,
) -> StoredRunner {
    let id = config.id.clone();
    let now = chrono::Utc::now().to_rfc3339();

    let status = RunnerStatus {
        id: id.clone(),
        status: "starting".to_string(),
        started_at: now.clone(),
        last_tick_at: None,
        next_tick_at: None,
        error: None,
    };

    let runner = StoredRunner { config: config.clone(), status: status.clone(), result: None };
    store.upsert(runner.clone());

    let store_clone = store.clone();
    let ws_dir = workspace_dir.clone();
    let task = tokio::spawn(async move {
        runner_loop(store_clone, config, ws_dir).await;
    });
    store.register_handle(id, task.abort_handle());

    runner
}

// ── Background runner loop ───────────────────────────────────────────────────

async fn runner_loop(
    store: Arc<StrategyRunnerStore>,
    config: RunnerConfig,
    workspace_dir: PathBuf,
) {
    let id = config.id.clone();
    let interval_secs = interval_to_secs(&config.interval).max(60);

    loop {
        let tick_start = chrono::Utc::now();

        {
            let mut map = store.runners.lock().unwrap();
            if let Some(r) = map.get_mut(&id) {
                r.status.status = "running".to_string();
                r.status.last_tick_at = Some(tick_start.to_rfc3339());
                let next = tick_start + chrono::Duration::seconds(interval_secs as i64);
                r.status.next_tick_at = Some(next.to_rfc3339());
                r.status.error = None;
            }
        }
        store.persist();

        let warmup_days = config.warmup_days.max(30) as i64;
        let from_date = (tick_start - chrono::Duration::days(warmup_days))
            .format("%Y-%m-%d").to_string();
        let to_date = tick_start.format("%Y-%m-%d").to_string();

        let script_path = {
            let p = std::path::Path::new(&config.script);
            if p.is_absolute() || p.exists() {
                p.to_path_buf()
            } else {
                workspace_dir.join("scripts").join(&config.script)
            }
        };

        if !script_path.exists() {
            let mut map = store.runners.lock().unwrap();
            if let Some(r) = map.get_mut(&id) {
                r.status.status = "error".to_string();
                r.status.error = Some(format!("Script not found: {}", script_path.display()));
            }
            store.persist();
            break;
        }

        let metrics = crate::tools::backtest::run_backtest_engine(
            &script_path,
            &config.market_type,
            &config.symbol,
            &config.interval,
            &from_date,
            &to_date,
            config.initial_balance,
            config.fee_pct,
            &workspace_dir,
        ).await;

        let last_signal = metrics.all_trades.last()
            .map(|t| t.side.clone())
            .unwrap_or_else(|| "flat".to_string());

        let result = RunnerResult {
            total_return_pct: metrics.total_return_pct,
            balance: config.initial_balance * (1.0 + metrics.total_return_pct / 100.0),
            position: 0.0,
            total_trades: metrics.total_trades,
            win_rate_pct: metrics.win_rate_pct,
            sharpe_ratio: metrics.sharpe_ratio,
            max_drawdown_pct: metrics.max_drawdown_pct,
            all_trades: metrics.all_trades,
            last_signal,
            analysis: metrics.analysis,
        };

        {
            let mut map = store.runners.lock().unwrap();
            if let Some(r) = map.get_mut(&id) {
                r.status.status = "running".to_string();
                r.result = Some(result);
            }
        }
        store.persist();

        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
    }
}

fn interval_to_secs(interval: &str) -> u64 {
    let s = interval.trim().to_lowercase();
    if s.ends_with('s') {
        s.trim_end_matches('s').parse::<u64>().unwrap_or(60)
    } else if s.ends_with('m') {
        s.trim_end_matches('m').parse::<u64>().unwrap_or(1) * 60
    } else if s.ends_with('h') {
        s.trim_end_matches('h').parse::<u64>().unwrap_or(1) * 3600
    } else if s.ends_with('d') {
        s.trim_end_matches('d').parse::<u64>().unwrap_or(1) * 86400
    } else {
        s.parse::<u64>().unwrap_or(60) * 60
    }
}
