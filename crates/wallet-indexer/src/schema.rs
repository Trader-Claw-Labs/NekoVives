//! SQLite schema and migrations for the copy-trading indexer.

use anyhow::Result;
use rusqlite::Connection;

/// Run all migrations to bring the database up to date.
pub fn run_migrations(conn: &Connection) -> Result<()> {
    // v2: add fill_id column (deduplication key). Silently ignore if already exists.
    let _ = conn.execute_batch("ALTER TABLE wallet_trades ADD COLUMN fill_id TEXT;");
    // Unique index allows NULL fill_ids from old rows to coexist.
    let _ = conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_wallet_trades_fill_id \
         ON wallet_trades(fill_id) WHERE fill_id IS NOT NULL;",
    );
    // Remove duplicates that accumulated before the fill_id guard was in place.
    // Keep the row with the lowest rowid for each (address, fill_id) group.
    let _ = conn.execute_batch(
        "DELETE FROM wallet_trades
         WHERE fill_id IS NOT NULL
           AND rowid NOT IN (
               SELECT MIN(rowid) FROM wallet_trades
               WHERE fill_id IS NOT NULL
               GROUP BY fill_id
           );",
    );

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wallet_scores (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            address TEXT NOT NULL,
            venue TEXT NOT NULL,
            category TEXT,
            pnl_norm REAL,
            winrate_score REAL,
            drawdown_score REAL,
            sharpe_score REAL,
            consistency_score REAL,
            diversity_score REAL,
            wallet_score REAL,
            tracked_since TEXT,
            last_updated TEXT,
            UNIQUE(address, venue, category)
        );

        CREATE TABLE IF NOT EXISTS wallet_trades (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            address TEXT NOT NULL,
            venue TEXT NOT NULL,
            market_id TEXT,
            side TEXT,
            notional REAL,
            price REAL,
            timestamp TEXT,
            pnl REAL
        );

        CREATE INDEX IF NOT EXISTS idx_wallet_trades_address ON wallet_trades(address);
        CREATE INDEX IF NOT EXISTS idx_wallet_trades_timestamp ON wallet_trades(timestamp);

        -- Migration v2: add fill_id for deduplication (idempotent)
        -- SQLite ignores duplicate-column errors so we swallow them at the call site.

        CREATE TABLE IF NOT EXISTS mirror_positions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            leader_address TEXT NOT NULL,
            leader_fill_id TEXT NOT NULL UNIQUE,
            my_order_id TEXT,
            venue TEXT NOT NULL,
            symbol TEXT,
            side TEXT,
            notional REAL,
            entry_price REAL,
            status TEXT,
            opened_at TEXT,
            closed_at TEXT,
            pnl REAL
        );

        CREATE TABLE IF NOT EXISTS shadow_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            wallet_address TEXT NOT NULL,
            venue TEXT NOT NULL,
            started_at TEXT,
            ended_at TEXT,
            simulated_pnl REAL,
            trade_count INTEGER,
            score_at_start REAL
        );

        CREATE TABLE IF NOT EXISTS candidate_list (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            wallet_address TEXT NOT NULL,
            venue TEXT NOT NULL,
            discovery_score REAL,
            shadow_pnl REAL,
            shadow_sharpe REAL,
            status TEXT,
            discovered_at TEXT,
            graduated_at TEXT,
            UNIQUE(wallet_address, venue)
        );

        CREATE INDEX IF NOT EXISTS idx_candidate_status ON candidate_list(status);
        ",
    )?;
    Ok(())
}
