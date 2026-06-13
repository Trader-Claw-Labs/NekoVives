# CLAUDE.md — Trader Claw

## What is this project?
Fork of TraderClaw (traderclaw-labs/traderclaw). A Rust crypto trading agent
for EVM (Uniswap), Solana (Raydium/PumpFun), TON (STON.fi), and
Polymarket prediction markets.

Full specifications are in: trader_agent_research.docx


## Live Strategies stats

To generate new strategies, always read the files 
strategy/BTC_UPDOWN_5M_FINDINGS.md 
strategy/POLYMARKET_UPDOWN_5M_CROSS_ASSET.md
it contains the stats & findings on Live Strategies & logic

## Build & Test
```bash
# Full build (embeds web/dist into binary)
cargo build --release

# Rebuild web dashboard only
cd web && npm run build && cd ..

# Both steps required after any frontend change
cd web && npm run build && cd .. && cargo build --release

# Run
./target/release/trader-claw gateway

cargo test
cargo clippy -- -D warnings
docker compose up -d
```

## Binary
- Name: `trader-claw` (was `trader-agent` / `degen-agent`)
- Config dir: `~/.config/trader-claw/`
- Config file: `~/.traderclaw/config.toml`

## Architecture
Workspace crates:
- `.` (src/)                — main binary, gateway, channels, tools, agent loop
- `crates/wallet-manager`  — EVM BIP44, Solana ED25519, TON v4R2 — AES-256-GCM + Argon2id keystore
- `crates/evm-trader`      — Uniswap V2/V3/V4 via alloy + uniswap-sdk-core
- `crates/solana-trader`   — PumpFun, Raydium via sol-trade-sdk
- `crates/ton-trader`      — STON.fi via tonlib-rs
- `crates/polymarket-trader` — Gamma + CLOB API, L1/L2 auth, WebSocket
- `crates/market-analyzer` — TradingView Screener HTTP client (fetch_indicators, top_crypto_symbols)

## Web Dashboard (web/)
React + Vite + TanStack Query + Tailwind. Assets embedded into binary via rust-embed.
Rebuild: `cd web && npm run build` then `cargo build --release`.

Pages and routes:
- `/`                — Dashboard (status cards, system health, market scanner widget)
- `/wallets`         — Web3 Wallets (EVM · Solana · TON)
- `/polymarket`      — Polymarket prediction market trading
- `/telegram`        — Telegram bot config
- `/skills`          — Cron strategies (scheduled jobs)
- `/chat`            — Multi-chat parallel AI sessions
- `/tradingview`     — TradingView Screener: live RSI, MACD, price table + active signals panel
- `/backtesting`     — Strategy backtesting: .rhai script runner, metrics, worst trades, AI analysis
- `/orderbook`       — Orderbook Archive: remote DuckDB queries, dataset download, local file browser
- `/settings/llm`    — LLM provider/model config
- `/settings/config` — Advanced config

## Gateway API Routes
All `/api/*` require Bearer token auth (pair via `POST /pair` with `X-Pairing-Code` header).
Public: `GET /health`, `GET /metrics`, `POST /pair`.

Key routes:
- `GET  /api/status`                          — system overview
- `GET  /api/tradingview/scan`                — TradingView Screener indicators (?symbols=BTCUSDT,ETHUSDT)
- `GET  /api/backtest/scripts`                — list .rhai files from /scripts/
- `POST /api/backtest/run`                    — run backtest (Rhai engine)
- `GET  /api/backtest/tick-slugs`             — list available tick JSONL slugs for archive backtesting
- `GET  /api/wallets`                         — list wallets
- `POST /api/wallets/create`                  — create wallet (EVM/Solana/TON)
- `GET  /api/polymarket/markets`              — list markets
- `GET  /api/cron`                            — list cron jobs
- `POST /api/cron`                            — add cron job
- `GET  /api/memory`                          — list memories
- `POST /api/orderbook/ingest`                — download archive parquets + convert to ticks (combined job)
- `POST /api/orderbook/download`              — download parquets only
- `GET  /api/orderbook/download/status`       — poll download/ingest progress
- `POST /api/orderbook/download/cancel`       — cancel running job
- `GET  /api/orderbook/files`                 — list local parquet files
- `POST /api/orderbook/query`                 — remote DuckDB query (summary/top-markets/price-series/spread-stats/drift)
- `GET  /api/polymarket/balance`              — real USDC wallet balance (CLOB API + Polygon RPC, max of both)
- `POST /api/live/strategies/{id}/sync-onchain` — reconcile untracked onchain trades into runner log
- `POST /api/live/stop-all-live`              — emergency stop ALL running live runners

## Backtesting Engine
- `<workspace>/scripts/`  — .rhai strategy files (agent-written + bundled defaults)
- `<workspace>/data/`     — candle cache (JSON, auto-fetched from Binance or Polymarket CLOB)
- `<workspace>/data/ticks/<slug>/` — 1-Hz Polymarket tick JSONL (see Tick Recorder below)
- Engine: `src/tools/backtest.rs` — real Rhai execution, no stubs
- Data sources: Binance REST (`/api/v3/klines`, paginated) for crypto; Polymarket CLOB
  (`/prices-history`) for prediction markets
- Metrics: Total Return %, Sharpe Ratio (annualised), Max Drawdown %, Win Rate, Trade Count,
  5 Worst Trades, AI analysis text
- `ensure_default_scripts()` writes bundled scripts to `scripts/` on first use

### Bundled default strategies (embedded in binary via `include_str!`)
| File | API | Description |
|------|-----|-------------|
| `polymarket_4min.rhai` | ctx-based | Polymarket 4-min strategy v2: RSI + 4-candle momentum + volume (3-of-4), ATR stop/take |
| `strategy_reference.rhai` | array-based (reference) | Original strategy.rhai — documents the 2-param `on_candle(candle_data, capital)` pattern |
| `polymarket_btc_updown_5m_drift_v2_combo.rhai` | ctx-based | BTC 5m UP/DOWN: §1 mid-band drift fade (setups 1-8), §2 extreme zones with RSI+win_pct |
| `polymarket_eth_updown_5m_btcproxy.rhai` | ctx-based | ETH 5m: BTC-proxy drift, setups 2(0.5×)/4/6/8/9 |
| `polymarket_sol_updown_5m_favorite_fade.rhai` | ctx-based | SOL 5m: NO-only, setups 2/4/8/10(1.5×) — bear-tilted asset |
| `polymarket_xrp_updown_5m_lowvol_drift.rhai` | ctx-based | XRP 5m: low-vol drift, setups 1/4/8/10 all at 0.7× stake |
| `polymarket_doge_updown_5m_meanrev.rhai` | ctx-based | DOGE 5m: mean-reversion, 2× thresholds, spike-fade setups 11/12 |
| `polymarket_hype_updown_5m_thinmkt.rhai` | ctx-based | HYPE 5m: drift-only (no Binance candles), setup 7 unique to HYPE |
| `polymarket_all_updown_5m_adaptive.rhai` | ctx-based | Universal ATR-adaptive, NO-side-only (setup 9 YES disabled after dry-run) |
| `clob_1hz_spread_scalper.rhai` | on_tick-based | CLOB 1 HZ: spread fade — bets NO/YES at extreme prices in final 60 s of each window |
| `clob_1hz_late_certainty.rhai` | on_tick-based | CLOB 1 HZ: fade uncertainty in final 30-45s. Setups A/B follow clear BTC move; C/D fade wrong-side favourite/underdog. EV-positive on 33d real data |
| `clob_1hz_early_oracle.rhai` | on_tick-based | CLOB 1 HZ: early oracle-divergence entry |
| `clob_1hz_volatility_regime.rhai` | on_tick-based | CLOB 1 HZ: regime-gated tick entries |

Cross-asset analysis and setup-by-setup edge tables:
`src/tools/scripts/POLYMARKET_UPDOWN_5M_CROSS_ASSET.md`

### Backtesting guardrails (live-parity)
`BacktestRunBody` / `BacktestGuardrails` mirror the live runner's risk controls so backtest
P&L reflects what would actually happen live. UI-exposed in the Backtesting page Guardrails
section (polymarket_binary + archive_candles):
- `kelly_size_cap`, `min_entry_price`, `max_consecutive_losses`, `stop_loss_pct`
Threaded through `run_backtest_engine` → `run_polymarket_slug_backtest` (applied in the
trade loop: skip extreme entries, halt sim on loss streak).

### Rhai script APIs

**ctx-based API** (`on_candle(ctx)`) — fully supported, recommended:
```rhai
fn on_candle(ctx) {
    // Scalars
    ctx.close / ctx.open / ctx.high / ctx.low / ctx.volume
    ctx.index / ctx.position / ctx.entry_price / ctx.entry_index
    ctx.balance / ctx.open_positions

    // Historical lookups
    ctx.close_at(i)  ctx.high_at(i)  ctx.low_at(i)  ctx.volume_at(i)

    // Indicators (computed inline in Rust)
    ctx.rsi(period)  ctx.ema(period)  ctx.atr(period)

    // Trade actions
    ctx.buy(size)   ctx.sell(size)

    // Stop / take profit (enforced by engine each candle)
    ctx.set_stop_loss(price)   ctx.set_take_profit(price)

    // Key-value persistence across candles
    ctx.set("key", val)   ctx.get("key", default)

    // Polymarket binary market fields (live signal only)
    ctx.token_price       // YES token price P4 (0-1), real CLOB ask
    ctx.token_price_prev  // YES token price P3 (60s before), or 0 if unavailable
    ctx.token_drift       // P4 - P3 drift signal
    ctx.window_open       // underlying asset price at window open
    ctx.window_minutes    // window duration in minutes (5 for UP/DOWN 5m)

    // Oracle comparison (live signal only; 0 in backtests)
    ctx.binance_mark      // Binance spot price via miniTicker WS at decision time
    ctx.chainlink_mark    // Chainlink oracle price (0 if not configured)
    ctx.oracle_lag_secs   // seconds since Chainlink last updated

    // Early-fire support (live signal only)
    ctx.minute_offset     // 0 = standard decision; N = N minutes before window close
}
```

**CLOB 1 HZ API** (`on_tick(ctx)`) — for tick-replay backtesting from recorded JSONL data:
```rhai
fn on_tick(ctx) {
    ctx.ts_ms             // Unix timestamp of this tick (ms)
    ctx.yes_bid / ctx.yes_ask / ctx.yes_mid  // YES token bid/ask/mid (0-1)
    ctx.no_bid  / ctx.no_ask               // NO token bid/ask (0-1)
    ctx.spread_pct        // (yes_ask - yes_bid) × 100 (in ¢)
    ctx.binance_price     // Binance spot price at this tick
    ctx.window_ts         // UNIX ts of current window open
    ctx.window_secs_left  // seconds until window resolves
    ctx.second_in_window  // seconds elapsed in current window (0 = first tick)
    ctx.balance / ctx.position / ctx.entry_price
    ctx.window_open_price  // binance price anchored at window start (engine-set, authoritative)
    ctx.ask_depth_usd / ctx.bid_depth_usd  // book liquidity within 2% (0 if unavailable)
    ctx.window_yes_won     // official Polymarket resolution: true/false, or () if unknown

    ctx.bet_yes(size)     // size = fraction of balance (0-1). Enters YES bet at yes_ask.
    ctx.bet_no(size)      // Enters NO bet at no_ask.

    ctx.set("key", val)   // Key-value persistence across ticks
    ctx.get("key", def)
}
```
Resolution: at `window_secs_left == 0` (and on window-change), prefer the official
`window_yes_won` from the tick file; fall back to comparing `binance_price` to
`window_open_price` only when it's None. YES wins → payout = stake/entry_price; else NO wins.
Fill price = `sim_fill_vwap` (walks 2-level book from `ask_depth_usd`) when depth > 0,
else best ask. Positions auto-resolve each window; only one open position per window.

**Legacy signal-based API** — script sets `signal = "buy"/"sell"/"hold"` as a variable;
pre-injected scope vars: `open, high, low, close, volume, rsi, macd, signal, macd_hist,
balance, position`.

**Note on 2-param array API** (`on_candle(candle_data, capital)`): Rhai functions cannot
access module-level `let` variables (bot_state, config), so this pattern cannot run as-is
in the backtester. Use the ctx-based API for new strategies.

## Live Strategy Runner (Polymarket)
`src/strategy_runner.rs`

Two engine loops:
- `polymarket_runner_loop()` — on_candle engines (drift, midband, adaptive). Decision
  once per window at the decision candle. Balance = `initial + sum(order.pnl)` (derived).
- `tick_runner_loop()` — `rhai_tick` engine (e.g. `clob_1hz_late_certainty`). Runs
  `on_tick(ctx)` every second. Balance lives in `TickRunnerState` (mutated in place);
  the loop copies it to `RunnerResult.balance` each second for the dashboard.

Key fields in `RunnerConfig`:
- `chainlink_endpoint_url` — optional Chainlink REST endpoint for oracle comparison
- `chainlink_api_key`      — optional Bearer token for Chainlink endpoint
- `chainlink_interval_secs` — Chainlink poll interval (default 5s)
- `early_fire_secs`         — fire order N seconds before decision candle closes (0 = disabled)
- `price_mode`             — "historical" (real CLOB ask) | "mid" ((bid+ask)/2). UI-editable.
- `max_slippage_pct`       — worst_price cap for live market orders (default 5%)

At each decision candle the runner injects into ctx:
- `ctx.binance_mark` from Binance miniTicker WS (live, ~1s updates)
- `ctx.chainlink_mark` / `ctx.oracle_lag_secs` from configured Chainlink endpoint
- `ctx.minute_offset` = 0 for standard fire; >0 for early-fire

Every resolved trade is auto-recorded to the dynamic asset selector
(`src/tools/asset_selector.rs`) for rolling WR tracking.

### Guardrails (risk controls — `RunnerConfig` fields, all UI-editable)
After the May-2026 incident (BNB runner lost $3.6k via uncapped kelly, XRP lost $3k via
no auto-stop), the following guardrails apply to live AND paper runners:
- `kelly_size_cap` (default 1.5) — caps the `kelly_size` multiplier a script can emit.
  Was hardcoded 2.0; an uncapped script scaled bets 6.6× → $330 each on BNB.
- `max_runner_loss_pct` (None=off) — auto-stop + switch to paper when cumulative loss
  exceeds X% of `initial_balance`.
- `max_consecutive_losses` (None=off) — auto-stop after N consecutive losses.
- `min_entry_price` (default 0.05) — skip bets when token ask < this (blocks 3-5¢
  long-shots; the late-certainty runner was buying NO at 0.04 = 96% against).
- Regressive sizing — after 3+ consecutive losses, bet is halved (×0.5) until a win.
- `PortfolioGuard` (`src/portfolio_guard.rs`) — global watchdog; `stop_all_live()` halts
  every running live runner when the wallet drops past the configured % from baseline.

### Order execution flow (`execute_live_polymarket_signal`)
- 3 attempts: 2 market orders (`worst_price = mid × (1 + max_slippage_pct)`), then 1
  limit order fallback at decision mid-price on the final attempt.
- `spawn_order_monitor` — background task: polls `/data/trades` up to 120s to capture
  the real fill (`fill_price`/`fill_size`/`tx_hash`); for limit orders it also cancels
  the order if still unfilled at window close (avoids stale open positions).
- Immediate `store.persist()` after every order push (was lost on restart before flush).
- Per-series order queue (`ORDER_QUEUES`, Semaphore(1) per slug) serializes execution so
  parallel runners on the same slug don't race the book.
- Global CLOB rate limiter (`CLOB_RATE_LIMIT`, Semaphore(5)) on all CLOB HTTP calls.

### Window resolution — provisional + official oracle monitor
Polymarket settles via Chainlink ~60-180s AFTER the window closes, but the runner must
settle a position immediately (first tick of next window). Both engines now:
1. Settle provisionally with the Binance price comparison (`resolution_source =
   "binance_provisional"`).
2. Spawn `spawn_resolution_monitor` — retries the official Gamma resolution every 30s for
   up to 10 min. When the oracle settles, it patches `order.result` / `order.pnl` /
   `resolution_source = "polymarket"`, adjusts `live_wins`, and corrects the balance.
   - on_candle: only patches `order.pnl` (balance is derived → `tick_state = None`).
   - rhai_tick: patches `TickRunnerState.balance` directly (the loop overwrites
     `RunnerResult.balance` each second → must fix the internal source of truth).
Applies to both Live and Dry Run modes.

### Paper / Dry Run mode realism
- Entry price uses the real CLOB ask via `get_market_price()` (`/price?side=buy`), then
  `simulate_book_fill()` walks the `/book` ask levels to compute a VWAP that exposes
  slippage beyond top-of-book. `entry_price` = real ask; `fill_price` = book VWAP.
- On startup a live runner runs `reconcile_untracked_onchain()` — queries
  `data-api.polymarket.com/activity` (48h) and inserts any onchain trade missing from
  `live_orders` as an UNTRACKED record (fixes dashboard-vs-onchain drift from crashes
  before disk flush).

## Tick Recorder (`src/tick_recorder.rs`)
1-Hz recorder for Polymarket binary markets. Writes JSONL rows to
`<workspace>/data/ticks/<slug>/<YYYY-MM-DD>.jsonl` (7-day retention).

Each row: `ts_ms, yes_bid, yes_ask, no_bid, no_ask, yes_mid, binance_price,
chainlink_price, oracle_lag_ms, window_ts, window_secs_left,
ask_depth_usd, bid_depth_usd, window_yes_won`

New tick fields (all `#[serde(default)]` — backward compatible with old files):
- `ask_depth_usd` / `bid_depth_usd` — USD liquidity within 2% of best ask/bid. The live
  recorder fetches CLOB `/book` every 10s; `to-ticks` estimates it from trade-event volume.
  Used by the on_tick backtester's `sim_fill_vwap` for realistic market-order fills.
- `window_yes_won` — `Option<bool>`: official Polymarket resolution (True=YES/UP won,
  False=NO/DOWN, None=not resolved). Written by `to-ticks` / `backfill-resolutions` from the
  Gamma API. The on_tick backtester prefers this over Binance price comparison when present.
  Live recorder always writes None (resolution unknown until the market settles).

Global registry accessible from any tool. Tool: `tick_recorder`
- `action=start slug=btc_5m condition_id=0x... binance_symbol=BTCUSDT`
- `action=stop  slug=btc_5m`
- `action=status`
- `action=read  slug=btc_5m last_n=60`

## Orderbook Archive Dataset (`tools/orderbook_parser.py`)

Historical Polymarket CLOB data from the **pmxt.dev v2** public archive.
Hourly Parquet files (~100–400 MB each) on Cloudflare R2:
`https://r2v2.pmxt.dev/polymarket_orderbook_YYYY-MM-DDTHH.parquet`

Columns: `timestamp_received, market (condition_id), event_type, asset_id,
best_bid, best_ask, price, size, side, transaction_hash`

**Parser CLI** (`tools/orderbook_parser.py`):
```bash
# Remote DuckDB queries (no download)
python3 tools/orderbook_parser.py summary --days 1
python3 tools/orderbook_parser.py top-markets --days 1
python3 tools/orderbook_parser.py price-series --days 3 --market 0x...
python3 tools/orderbook_parser.py spread-stats --days 3 --market 0x...

# Download parquet files locally
python3 tools/orderbook_parser.py download --days 15 --market 0x... --out ~/.traderclaw/workspace/data/orderbook/

# Convert downloaded parquets → CLOB 1Hz JSONL (for backtesting)
python3 tools/orderbook_parser.py to-ticks --market 0x... --slug btc_5m \
  --binance-symbol BTCUSDT \
  --in ~/.traderclaw/workspace/data/orderbook/ \
  --out ~/.traderclaw/workspace/data/ticks/btc_5m/

# Convert downloaded parquets → OHLC candle JSON (for Backtesting page)
python3 tools/orderbook_parser.py to-candles --market 0x... --slug ob_test \
  --in ~/.traderclaw/workspace/data/orderbook/ \
  --out /tmp/candles_test/ --freq 5min

# Auto-detect all recurring 5/15/60-min series and convert in one shot
python3 tools/orderbook_parser.py to-ticks-multi --days 40

# Convert downloaded parquets → sub-second (ms) EVENT stream for the clob_events
# engine (BACKTEST_ENGINE_PLAN.md Fase A). Keeps every event at ms resolution
# (no 1Hz decimation) and BOTH the YES and NO books separately (real two-sided
# book, not no = 1 − yes). Output: data/events/<slug>/YYYY-MM-DD.jsonl.gz.
# Single arbitrary market:
python3 tools/orderbook_parser.py to-events --market 0x... --slug btc_5m_ev \
  --binance-symbol BTCUSDT \
  --in ~/.traderclaw/workspace/data/orderbook/ \
  --out ~/.traderclaw/workspace/data/events/
# Rolling updown series (discovers each window's condition_id):
python3 tools/orderbook_parser.py to-events --series-prefix btc-updown-5m --slug btc_5m_ev \
  --in ~/.traderclaw/workspace/data/orderbook/ --out ~/.traderclaw/workspace/data/events/

# Backfill official Polymarket resolution (window_yes_won) into EXISTING tick JSONL files.
# Queries Gamma API by slug "{series_prefix}-{window_ts}", rewrites JSONL in place.
# ~5 min per slug (30 concurrent requests/batch). Safe to re-run.
python3 tools/orderbook_parser.py backfill-resolutions \
  --slug btc_5m --series btc-updown-5m --window-minutes 5
```

**Window resolution (`window_yes_won`):** `to-ticks` and `to-ticks-multi` now call
`fetch_polymarket_window_resolutions()` (Gamma API) to attach the official outcome per
window. `to-ticks-multi` derives `window_ts` from each market's `end_ts` and batch-fetches
in parallel. Series slug prefixes use `-updown-` (e.g. `btc-updown-5m`), NOT `-up-or-down-`.
Only BTC and ETH have 15m series; 1h exists for BTC only.

**Important quirks:**
- Cloudflare R2 returns 403 on HEAD requests — use Range GET with User-Agent header
- DuckDB `custom_user_agent` must be set at connection creation (not via `SET` after open)
- Archive files have gaps (missing hours) — `filter_available_urls()` pre-checks each URL
- NO price = 1 − YES price: `no_bid = 1 − yes_ask`, `no_ask = 1 − yes_bid`

**Gateway API routes** (all require Bearer auth):
- `POST /api/orderbook/query`            — remote DuckDB query (modes: summary, top-markets, price-series, spread-stats, drift)
- `POST /api/orderbook/download`         — start background parquet download job
- `GET  /api/orderbook/download/status`  — poll download/ingest progress
- `POST /api/orderbook/download/cancel`  — cancel ongoing job
- `GET  /api/orderbook/files`            — list locally downloaded parquet files
- `POST /api/orderbook/ingest`           — **combined** download + to-ticks conversion in one job

**Ingest endpoint body:** `{ days, market, slug, binance_symbol }`

**Dashboard page:** `/orderbook` — three tabs: Remote Query, Download, Local Files

## Orderbook Archive Backtesting

Tick JSONL files produced by the archive converter (or the live tick recorder) live at
`<workspace>/data/ticks/<slug>/` and feed two backtesting modes.

### `clob_1hz` — "Orderbook Archive (on_tick)"
Replays raw 1-second ticks through `on_tick(ctx)` Rhai scripts.
For intra-window strategies: spread scalping, timing, entry windows.

**Workflow:**
1. In Backtesting UI, click **Archive Dataset** button → fill in condition ID, slug, days → **Start Download + Convert**
   — OR — run `to-ticks` from CLI manually
2. Select **Market = "Orderbook Archive (on_tick)"**
3. Pick the slug from the dropdown (auto-loaded from `GET /api/backtest/tick-slugs`)
4. Select a script with `on_tick(ctx)` (e.g. `clob_1hz_spread_scalper.rhai`)
5. Run

### `archive_candles` — "Orderbook Archive (on_candle)"
Aggregates tick JSONL → 1-minute Binance-style OHLC candles from `binance_price`,
then injects real CLOB YES token prices at decision time via `HistoricalMarketWindow`.
Runs `on_candle(ctx)` scripts through the same engine as `polymarket_binary`.

**Use this mode for existing drift strategies** (e.g. `polymarket_btc_updown_5m_drift_v2_combo.rhai`).
`ctx.token_price` gets the real CLOB ask from the archive instead of a synthetic momentum estimate.

**Workflow:**
1. Same as above — ingest ticks for the target slug
2. Select **Market = "Orderbook Archive (on_candle)"**
3. Pick slug, pick any `on_candle(ctx)` script
4. Run

**Key functions in `src/tools/backtest.rs`:**
- `run_clob_1hz_backtest()` / `run_clob_1hz_backtest_from_files()` — on_tick path
- `run_archive_candles_backtest()` — on_candle path (new)
- `load_ticks_for_range()` — shared tick loader used by both paths

**Gateway API:** `GET /api/backtest/tick-slugs` — returns available slugs with date ranges.

## Dynamic Asset Selector (`src/tools/asset_selector.rs`)
Rolling 30-day win-rate tracker per (script × symbol). Automatically records
every resolved live trade. Capital allocation weights are proportional to rolling WR.

Storage: `<workspace>/data/asset_selector_stats.json`

Tool: `asset_selector`
- `action=summary`          — performance table for all tracked pairs
- `action=weights`          — normalized allocation weights (min_trades=10)
- `action=record`           — manually record a trade outcome
- `action=clear`            — reset all history

## Pairing / Auth
Gateway requires pairing when `require_pairing = true` in config.
1. Start gateway — one-time code printed to terminal
2. Open dashboard → pairing modal appears automatically
3. Enter code → `POST /pair` → Bearer token saved to localStorage
4. Token persisted to `config.toml` for future restarts

## Channels (active)
- Telegram (`/poly` commands: markets, price, buy, sell, positions, orders, cancel)
- CLI

## Security — NEVER violate these
- NEVER log private keys, mnemonics, or Polymarket L2 credentials
- ALWAYS encrypt secrets at rest (AES-256-GCM + Argon2id)
- ALWAYS validate amounts before signing any tx or order
- Polymarket wallet must be a dedicated Polygon wallet
- Rhai scripts run sandboxed — enforce memory + execution time limits

## Live trading risk — lessons from the May-2026 incident
The Polymarket wallet drained from real losses. Root causes, all now mitigated:
- **Uncapped kelly** — a BTC-calibrated script run on BNB scaled bets to $330 (kelly 6.6×).
  → `kelly_size_cap` (default 1.5).
- **No auto-stop** — an XRP runner placed 219 trades at 17% WR without halting.
  → `max_runner_loss_pct`, `max_consecutive_losses`, regressive sizing, `PortfolioGuard`.
- **Extreme entries** — late-certainty runner bought NO at 0.04 (96% against).
  → `min_entry_price` (default 0.05).
- **Dashboard ≠ onchain** — crashes before disk flush lost ~64 real trades from the log;
  resolution mismatch (Binance vs Chainlink oracle) inflated simulated P&L by ~$1.5k.
  → immediate `store.persist()`, `reconcile_untracked_onchain`, `spawn_resolution_monitor`.
- **Parallel-runner contention** — 15 runners on the same slug raced the CLOB book.
  → per-series `ORDER_QUEUES`, global `CLOB_RATE_LIMIT`.
When in doubt about a runner config, verify against onchain via
`data-api.polymarket.com/activity?user=<proxy_wallet>` before trusting the dashboard.

New files from this work: `src/portfolio_guard.rs` (global loss watchdog).

## Key dependencies
alloy = "1", uniswap-v3-sdk = "5", sol-trade-sdk = "3",
polymarket-client-sdk (Polymarket/rs-clob-client), tonlib-rs (ston-fi),
market-analyzer (path = "crates/market-analyzer"), chrono = "0.4"
