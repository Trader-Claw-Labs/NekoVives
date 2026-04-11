# CLAUDE.md — Trader Claw


Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.
 

## What is this project?
A Rust crypto trading agent
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

## Backtesting Engine (spec)
- `/scripts/`  — .rhai strategy files generated by the agent
- `/data/`     — Parquet historical data (crypto + Polymarket)
- `/results/`  — JSON metric outputs for Claude to analyze
- Engine: Rust + Rhai (sandboxed, memory/time limits)
- Parallelism: Rayon for multi-param sweeps
- Metrics reported: PnL, Sharpe Ratio, Max Drawdown, Win Rate, 5 worst trades

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

 
