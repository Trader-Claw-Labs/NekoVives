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
- `/settings/llm`    — LLM provider/model config
- `/settings/config` — Advanced config

## Gateway API Routes
All `/api/*` require Bearer token auth (pair via `POST /pair` with `X-Pairing-Code` header).
Public: `GET /health`, `GET /metrics`, `POST /pair`.

Key routes:
- `GET  /api/status`                   — system overview
- `GET  /api/tradingview/scan`         — TradingView Screener indicators (?symbols=BTCUSDT,ETHUSDT)
- `GET  /api/backtest/scripts`         — list .rhai files from /scripts/
- `POST /api/backtest/run`             — run backtest (Rhai engine, stub returns metrics)
- `GET  /api/wallets`                  — list wallets
- `POST /api/wallets/create`           — create wallet (EVM/Solana/TON)
- `GET  /api/polymarket/markets`       — list markets
- `GET  /api/cron`                     — list cron jobs
- `POST /api/cron`                     — add cron job
- `GET  /api/memory`                   — list memories

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

Cross-asset analysis and setup-by-setup edge tables:
`src/tools/scripts/POLYMARKET_UPDOWN_5M_CROSS_ASSET.md`

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

    ctx.bet_yes(size)     // size = fraction of balance (0-1). Enters YES bet at yes_ask.
    ctx.bet_no(size)      // Enters NO bet at no_ask.

    ctx.set("key", val)   // Key-value persistence across ticks
    ctx.get("key", def)
}
```
Resolution: at `window_secs_left == 0`, compare `binance_price` to `window_open_price`.
If price went up → YES wins → payout = stake/entry_price; else NO wins (or vice versa).
Positions auto-resolve each window; only one open position allowed per window.

**Legacy signal-based API** — script sets `signal = "buy"/"sell"/"hold"` as a variable;
pre-injected scope vars: `open, high, low, close, volume, rsi, macd, signal, macd_hist,
balance, position`.

**Note on 2-param array API** (`on_candle(candle_data, capital)`): Rhai functions cannot
access module-level `let` variables (bot_state, config), so this pattern cannot run as-is
in the backtester. Use the ctx-based API for new strategies.

## Live Strategy Runner (Polymarket)
`src/strategy_runner.rs` → `polymarket_runner_loop()`

Key fields in `RunnerConfig`:
- `chainlink_endpoint_url` — optional Chainlink REST endpoint for oracle comparison
- `chainlink_api_key`      — optional Bearer token for Chainlink endpoint
- `chainlink_interval_secs` — Chainlink poll interval (default 5s)
- `early_fire_secs`         — fire order N seconds before decision candle closes (0 = disabled)

At each decision candle the runner injects into ctx:
- `ctx.binance_mark` from Binance miniTicker WS (live, ~1s updates)
- `ctx.chainlink_mark` / `ctx.oracle_lag_secs` from configured Chainlink endpoint
- `ctx.minute_offset` = 0 for standard fire; >0 for early-fire

Every resolved trade is auto-recorded to the dynamic asset selector
(`src/tools/asset_selector.rs`) for rolling WR tracking.

## Tick Recorder (`src/tick_recorder.rs`)
1-Hz recorder for Polymarket binary markets. Writes JSONL rows to
`<workspace>/data/ticks/<slug>/<YYYY-MM-DD>.jsonl` (7-day retention).

Each row: `ts_ms, yes_bid, yes_ask, no_bid, no_ask, yes_mid, binance_price,
chainlink_price, oracle_lag_ms, window_ts, window_secs_left`

Global registry accessible from any tool. Tool: `tick_recorder`
- `action=start slug=btc_5m condition_id=0x... binance_symbol=BTCUSDT`
- `action=stop  slug=btc_5m`
- `action=status`
- `action=read  slug=btc_5m last_n=60`

## CLOB 1 HZ Backtesting (`market_type = "clob_1hz"`)
Replays recorded tick JSONL files through `on_tick(ctx)` Rhai scripts.
Enables testing of intra-window strategies (spread scalping, timing, entry windows) that
the 1m-candle engine cannot evaluate.

**Workflow:**
1. Record ticks with `tick_recorder action=start slug=btc_5m condition_id=0x...`
2. Let it run for at least 1 day
3. In the Backtesting page, select **Market = "CLOB 1 Hz (tick replay)"**
4. Pick the slug from the dropdown (auto-loaded from `GET /api/backtest/tick-slugs`)
5. Select a script that uses `on_tick(ctx)` (e.g. `clob_1hz_spread_scalper.rhai`)
6. Run — engine replays every recorded tick and resolves each 5m window

**Key functions:** `run_clob_1hz_backtest()`, `list_tick_slugs()`,
`run_clob_1hz_backtest_from_files()` in `src/tools/backtest.rs`

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

## Key dependencies
alloy = "1", uniswap-v3-sdk = "5", sol-trade-sdk = "3",
polymarket-client-sdk (Polymarket/rs-clob-client), tonlib-rs (ston-fi),
market-analyzer (path = "crates/market-analyzer"), chrono = "0.4"
