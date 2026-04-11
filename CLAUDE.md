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
