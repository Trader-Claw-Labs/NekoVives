# CLAUDE.md — Trader Claw

## What is this project?
Fork of TraderClaw (traderclaw-labs/traderclaw). A Rust crypto trading agent
for EVM (Uniswap), Solana (Raydium/PumpFun), TON (STON.fi), and
Polymarket prediction markets.

Full specifications are in: trader_agent_research.docx

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

## CLI subcommands
- `trader-claw gateway` u2014 start web dashboard + REST/WS API
- `trader-claw daemon`  u2014 gateway + Telegram + cron scheduler
- `trader-claw onboard` / `onboard --interactive` u2014 first-time setup wizard
- `trader-claw update [--prerelease]` u2014 self-update binary from GitHub Releases

## Distribution & CI
Pre-built releases ship for 5 targets via `.github/workflows/release.yml` on `v*` tags:
- Linux x86_64 / ARM64 (`.tar.gz`)
- macOS x86_64 / ARM64 (`.tar.gz` + `.dmg`)
- Windows x86_64 (`.zip` + `.msi` via cargo-wix)

Release artifacts are signed with cosign keyless signing (Sigstore) and accompanied by SBOM (syft: `.spdx.json` + `.cdx.json`).

Install scripts:
- `install.sh` u2014 Linux/macOS one-liner (detects OS+arch, installs to `/usr/local/bin/`)
- `install.ps1` u2014 Windows PowerShell (installs to `%LOCALAPPDATA%\trader-claw\`, adds to PATH)

Package managers: Homebrew (`Trader-Claw-Labs/trader-claw` tap), Scoop (`scoop-trader-claw` bucket), AUR (`trader-claw-bin`).

Security CI (`.github/workflows/security.yml`):
- `cargo audit` + `npm audit` on push/PR/weekly schedule
- `cargo deny` (license policy in `deny.toml`), gitleaks secret scanning
- CodeQL (JS/TS), Trivy filesystem scan, cargo-geiger unsafe report

OSSF Scorecard (`.github/workflows/scorecard.yml`) u2014 weekly, publishes to GitHub Security tab.

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
- `/backtesting`     — Strategy backtesting: .rhai script runner, MarketSeries selector, binary recurrence engine, metrics + AI analysis
- `/strategy-builder`— No-code strategy templates (crypto, Polymarket BTC UP/DOWN, weather binary) -> saves .rhai scripts
- `/settings/llm`    — LLM provider/model config
- `/settings/config` — Advanced config

## Gateway API Routes
All `/api/*` require Bearer token auth (pair via `POST /pair` with `X-Pairing-Code` header).
Public: `GET /health`, `GET /metrics`, `POST /pair`.

Key routes:
- `GET  /api/status`                   — system overview
- `GET  /api/tradingview/scan`         — TradingView Screener indicators (?symbols=BTCUSDT,ETHUSDT)
- `GET  /api/backtest/scripts`         — list .rhai files from /scripts/
- `GET  /api/backtest/series`          — list MarketSeries registry (builtin recurrent binary markets)
- `POST /api/backtest/run`             — run backtest (real Rhai engine; supports series_id + resolution_logic + threshold)
- `GET  /api/wallets`                  — list wallets
- `POST /api/wallets/create`           — create wallet (EVM/Solana/TON)
- `GET  /api/polymarket/markets`       — list markets
- `GET  /api/cron`                     — list cron jobs
- `POST /api/cron`                     — add cron job
- `GET  /api/memory`                   — list memories

## Backtesting Engine
- `<workspace>/scripts/`  — .rhai strategy files (builder + agent + bundled defaults)
- `<workspace>/data/`     — candle cache (JSON)
- Engine: `src/tools/backtest.rs` — real Rhai execution, no stubs
- Supports AST compile-once execution for recurring binary windows (single compile reused across windows)
- Data sources:
  - Binance REST (`/api/v3/klines`) for crypto candles and BTC/ETH recurrent binary series
  - Open-Meteo archive API (`/v1/archive`) for weather daily threshold markets
- Resolution logic:
  - `price_up`          (resolution candle close > window open)
  - `threshold_above`   (resolution value > threshold)
  - `threshold_below`   (resolution value < threshold)
- Metrics: Total Return %, Final Balance, Sharpe Ratio (annualised), Max Drawdown %, Win Rate, Trade Count,
  Avg Ticket, 5 Worst Trades, AI analysis text
- `ensure_default_scripts()` writes bundled scripts to `scripts/` on first use

### MarketSeries registry
- Source: `src/tools/series.rs`
- Exposed by: `GET /api/backtest/series`
- Fields include: `id`, `label`, `slug_prefix`, `data_source`, `symbol`, `cadence`,
  `resolution_logic`, `threshold`, `unit`, `description`, `default_script`, `builtin`
- Builtin crypto recurrent series: `btc_5m`, `btc_4m`, `btc_15m`, `btc_1h`, `eth_5m`, `eth_15m`
- Builtin weather series (Open-Meteo): `munich_temp_daily`, `london_temp_daily`, `nyc_temp_daily`

### Bundled default strategies (embedded in binary via `include_str!`)
| File | API | Description |
|------|-----|-------------|
| `polymarket_4min.rhai` | ctx-based | Polymarket recurrent binary strategy (momentum + RSI + volume confirmations) |
| `weather_binary.rhai` | ctx-based | Weather threshold strategy (EMA/threshold scoring for YES/NO bets) |
| `strategy_reference.rhai` | array-based (reference) | Original strategy.rhai — documents legacy 2-param `on_candle(candle_data, capital)` pattern |

### Strategy Builder templates
- Polymarket BTC UP/DOWN template aligned with recurring binary engine (series-aware, no manual stop/take)
- Polymarket Weather Binary template for threshold markets (Open-Meteo series)
- Generated scripts are saved to `/scripts/` and run directly in Backtesting

### Frontend backtesting UX updates
- Removed non-binary Polymarket option from market selector (all Polymarket flows are binary)
- Dynamic series selector now loads from `/api/backtest/series`
- Added threshold override input for threshold-based series
- Added KPIs: `Final Balance` and `Avg Ticket` in results panel
- Added migration from legacy `polymarket` state to `polymarket_binary`
- Local state supports: `series_id`, `resolution_logic`, `threshold`

### Binary engine runtime context additions
- `ctx.threshold`
- `ctx.resolution_value`
- `ctx.value` (alias)
- For weather/threshold markets, token pricing defaults to flat `0.50`
- For price-up markets, token price remains momentum-derived

### Performance note
- Recurring binary backtests no longer recompile Rhai per window; engine/AST are reused across the full run
  to reduce runtime overhead on large ranges (e.g. 3 months of 5m windows).

### Known implementation files touched recently
- `src/tools/backtest.rs`
- `src/tools/series.rs`
- `src/tools/mod.rs`
- `src/gateway/api.rs`
- `src/gateway/mod.rs`
- `src/strategy_runner.rs`
- `web/src/hooks/useBacktestState.ts`
- `web/src/pages/Backtesting.tsx`
- `web/src/pages/StrategyBuilder.tsx`
- `src/tools/scripts/weather_binary.rhai`
- `src/tools/scripts/polymarket_btc_binary.rhai`
- `web/index.html`, `web/dist/index.html`
- `Cargo.toml`, `Cargo.lock`
- `src/config/schema.rs`
- `src/security/pairing.rs`
- `src/gateway/api.rs`
- `src/tools/backtest.rs`
- `src/gateway/mod.rs`
- `src/strategy_runner.rs`
- `web/src/components/Sidebar.tsx`
- `web/src/pages/Polymarket.tsx`
- `web/src/pages/Config.tsx`
- `web/src/hooks/useApi.ts`
- `web/src/App.tsx`
- `web/src/pages/Backtesting.tsx`
- `web/src/hooks/useBacktestState.ts`
- `web/src/pages/StrategyBuilder.tsx`
- `strategy/` (new workspace folder in progress)

### Notes
- The gateway route map and UI continue evolving; prefer checking current source for the exact latest status.
- This file captures architectural intent and current expected behavior for the backtesting stack.
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
}
```

**Legacy signal-based API** — script sets `signal = "buy"/"sell"/"hold"` as a variable;
pre-injected scope vars: `open, high, low, close, volume, rsi, macd, signal, macd_hist,
balance, position`.

**Note on 2-param array API** (`on_candle(candle_data, capital)`): Rhai functions cannot
access module-level `let` variables (bot_state, config), so this pattern cannot run as-is
in the backtester. Use the ctx-based API for new strategies.

## Pairing / Auth
Gateway requires pairing when `require_pairing = true` in config.
1. Start gateway — one-time code printed to terminal
2. Open dashboard → pairing modal appears automatically
3. Enter code → `POST /pair` → Bearer token saved to localStorage
4. Token persisted to `config.toml` for future restarts

## Channels (active)
- Telegram (`/poly` commands: markets, price, buy, sell, positions, orders, cancel)
- CLI

## Agent Chat (WebSocket)
- WebSocket endpoint: `GET /ws/chat?token=<bearer>`
- Protocol: `{"type":"message","content":"..."}` → chunks/tool_call/tool_result/done/error events
- Cancel a running turn: send `{"type":"cancel"}` u2014 server aborts agent via `tokio::select!`, responds `{"type":"cancelled"}`
- Hard timeout: `agent_turn_timeout_secs` in `[channels_config]` (default 1800s = 30 min; `0` = no limit)
- UI features: actions/rounds expanded by default; Stop button (red square) replaces Send while agent is running

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
