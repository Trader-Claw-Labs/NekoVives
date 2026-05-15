use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

pub mod polymarket;
pub mod schema;
pub mod scoring;

use scoring::{passes_hard_filters, score_wallet, WalletMetrics, WalletScore};

/// SQLite-backed wallet indexer.
pub struct Indexer {
    conn: Arc<Mutex<Connection>>,
    db_path: PathBuf,
    http_client: reqwest::Client,
}

impl Indexer {
    pub fn new(workspace_dir: &Path) -> Result<Self> {
        let db_path = workspace_dir.join("copy_trading.db");
        let conn = Connection::open(&db_path)?;
        schema::run_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path,
            http_client: reqwest::Client::new(),
        })
    }

    /// Run the nightly indexer for Polymarket.
    pub async fn run_polymarket_nightly(&self,
        leaderboard_limit: usize,
    ) -> Result<Vec<WalletScore>> {
        let entries = polymarket::fetch_leaderboard(&self.http_client,
            leaderboard_limit,
        ).await?;

        let mut scores = Vec::new();
        for entry in entries {
            // Fetch recent trades to compute metrics
            let trades = match polymarket::fetch_wallet_trades(
                &self.http_client,
                &entry.address,
                500,
            ).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("Failed to fetch trades for {}: {}", entry.address, e);
                    continue;
                }
            };

            // Build basic metrics (simplified — full metrics need 90d historical data)
            let metrics = WalletMetrics {
                pnl_90d: entry.profit,
                winrate: 0.50, // placeholder — compute from trade history
                trades_90d: trades.len() as i64,
                max_drawdown_pct: 0.20, // placeholder
                sharpe_ratio: 1.0,      // placeholder
                cv_monthly_pnl: 0.20,   // placeholder
                unique_tickers: trades.iter().map(|t| t.market_slug.clone()).collect::<std::collections::HashSet<_>>().len() as i64,
                capital_usd: entry.volume.max(50_000.0),
            };

            if !passes_hard_filters(&metrics) {
                continue;
            }

            // Simple percentile: rank by profit relative to leaderboard
            let pnl_norm = (entry.profit / entry.volume.max(1.0)).min(1.0).max(0.0);

            let score = score_wallet(
                entry.address.clone(),
                "polymarket".into(),
                None, // category determined per-market later
                &metrics,
                pnl_norm,
            );

            self.persist_score(&score).await?;
            scores.push(score);
        }

        Ok(scores)
    }

    async fn persist_score(&self,
        score: &WalletScore,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO wallet_scores
             (address, venue, category, pnl_norm, winrate_score, drawdown_score,
              sharpe_score, consistency_score, diversity_score, wallet_score,
              tracked_since, last_updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(address, venue, category) DO UPDATE SET
              pnl_norm = excluded.pnl_norm,
              winrate_score = excluded.winrate_score,
              drawdown_score = excluded.drawdown_score,
              sharpe_score = excluded.sharpe_score,
              consistency_score = excluded.consistency_score,
              diversity_score = excluded.diversity_score,
              wallet_score = excluded.wallet_score,
              last_updated = excluded.last_updated",
            params![
                &score.address,
                &score.venue,
                score.category.as_ref(),
                score.pnl_norm,
                score.winrate_score,
                score.drawdown_score,
                score.sharpe_score,
                score.consistency_score,
                score.diversity_score,
                score.total_score,
                Utc::now().to_rfc3339(),
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Add a candidate to the discovery list.
    pub async fn add_candidate(
        &self,
        address: &str,
        venue: &str,
        score: f64,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO candidate_list
             (wallet_address, venue, discovery_score, status, discovered_at)
             VALUES (?1, ?2, ?3, 'candidate', ?4)
             ON CONFLICT(wallet_address, venue) DO UPDATE SET
              discovery_score = excluded.discovery_score",
            params![address, venue, score, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Update a candidate's status (e.g. 'candidate', 'blacklisted', 'graduated').
    pub async fn set_candidate_status(&self, address: &str, status: &str) -> Result<bool> {
        let conn = self.conn.lock().await;
        let changed = conn.execute(
            "UPDATE candidate_list SET status = ?1,
                    graduated_at = CASE WHEN ?1 = 'graduated' THEN ?2 ELSE graduated_at END
             WHERE wallet_address = ?3",
            params![status, Utc::now().to_rfc3339(), address],
        )?;
        Ok(changed > 0)
    }

    /// Remove a candidate from the discovery list.
    pub async fn remove_candidate(&self, address: &str) -> Result<bool> {
        let conn = self.conn.lock().await;
        let changed = conn.execute(
            "DELETE FROM candidate_list WHERE wallet_address = ?1",
            params![address],
        )?;
        Ok(changed > 0)
    }

    /// List candidates by status.
    pub async fn list_candidates(
        &self,
        status: Option<&str>,
    ) -> Result<Vec<CandidateRecord>> {
        let conn = self.conn.lock().await;
        let sql = if let Some(_s) = status {
            "SELECT wallet_address, venue, discovery_score, shadow_pnl, shadow_sharpe,
                    status, discovered_at, graduated_at
             FROM candidate_list WHERE status = ?1 ORDER BY discovery_score DESC"
        } else {
            "SELECT wallet_address, venue, discovery_score, shadow_pnl, shadow_sharpe,
                    status, discovered_at, graduated_at
             FROM candidate_list ORDER BY discovery_score DESC"
        };
        let mut stmt = conn.prepare(sql)?;

        fn map_row(row: &rusqlite::Row) -> rusqlite::Result<CandidateRecord> {
            Ok(CandidateRecord {
                wallet_address: row.get(0)?,
                venue: row.get(1)?,
                discovery_score: row.get(2)?,
                shadow_pnl: row.get(3)?,
                shadow_sharpe: row.get(4)?,
                status: row.get(5)?,
                discovered_at: row.get(6)?,
                graduated_at: row.get(7)?,
            })
        }

        let rows = if let Some(s) = status {
            stmt.query_map([s], map_row)?
        } else {
            stmt.query_map([], map_row)?
        };

        let mut records = Vec::new();
        for r in rows {
            records.push(r?);
        }
        Ok(records)
    }

    /// Query score sub-fields for a specific address.
    pub async fn get_wallet_score(&self, address: &str) -> Result<Option<WalletScoreRecord>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT address, venue, pnl_norm, winrate_score, drawdown_score,
                    sharpe_score, consistency_score, diversity_score, wallet_score
             FROM wallet_scores WHERE address = ?1 LIMIT 1"
        )?;
        let mut rows = stmt.query_map([address], |row| Ok(WalletScoreRecord {
            address: row.get(0)?,
            venue: row.get(1)?,
            pnl_norm: row.get(2)?,
            winrate_score: row.get(3)?,
            drawdown_score: row.get(4)?,
            sharpe_score: row.get(5)?,
            consistency_score: row.get(6)?,
            diversity_score: row.get(7)?,
            wallet_score: row.get(8)?,
        }))?;
        Ok(rows.next().transpose()?)
    }

    /// List recent trades for a specific address (newest first, limit 100).
    pub async fn get_leader_trades(&self, address: &str) -> Result<Vec<WalletTradeRecord>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT address, venue, market_id, side, notional, price, timestamp, pnl
             FROM wallet_trades WHERE address = ?1
             ORDER BY timestamp DESC LIMIT 100"
        )?;
        let rows = stmt.query_map([address], |row| Ok(WalletTradeRecord {
            address: row.get(0)?,
            venue: row.get(1)?,
            market_id: row.get(2)?,
            side: row.get(3)?,
            notional: row.get(4)?,
            price: row.get(5)?,
            timestamp: row.get(6)?,
            pnl: row.get(7)?,
        }))?;
        let mut records = Vec::new();
        for r in rows { records.push(r?); }
        Ok(records)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WalletScoreRecord {
    pub address: String,
    pub venue: String,
    pub pnl_norm: Option<f64>,
    pub winrate_score: Option<f64>,
    pub drawdown_score: Option<f64>,
    pub sharpe_score: Option<f64>,
    pub consistency_score: Option<f64>,
    pub diversity_score: Option<f64>,
    pub wallet_score: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WalletTradeRecord {
    pub address: String,
    pub venue: String,
    pub market_id: Option<String>,
    pub side: Option<String>,
    pub notional: Option<f64>,
    pub price: Option<f64>,
    pub timestamp: Option<String>,
    pub pnl: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CandidateRecord {
    pub wallet_address: String,
    pub venue: String,
    pub discovery_score: f64,
    pub shadow_pnl: Option<f64>,
    pub shadow_sharpe: Option<f64>,
    pub status: String,
    pub discovered_at: String,
    pub graduated_at: Option<String>,
}
