# Trader Claw — Stub Implementation Roadmap

> Generated 2026-04-07. Every item below is a confirmed stub or placeholder in the current codebase.

---

## Phase 1 — Trading Execution (Core Value)

These stubs block the primary use case: executing actual trades.

### 1.1 Solana swap transaction broadcasting
**File:** `src/tools/trade_swap.rs:326–338`  
**Current:** After verifying the Jupiter quote and decrypting the wallet keypair, returns a warning message instead of broadcasting.  
**Fix:**
1. POST to `https://quote-api.jup.ag/v6/swap` with the quote response + user public key to get a serialized transaction.
2. Deserialize the base64 transaction with `solana_sdk::transaction::VersionedTransaction`.
3. Sign it with the decrypted `Keypair`.
4. Send via `solana_client::rpc_client::RpcClient::send_and_confirm_transaction`.
5. Return the transaction signature.

**Dependencies to add:** `solana-sdk`, `solana-client`  
**Complexity:** Medium (2–4 days)

---

### 1.2 Solana trader crate
**File:** `crates/solana-trader/src/lib.rs`  
**Current:** Single `// TODO: implement SolanaTrader` comment.  
**Fix:** Implement a `SolanaTrader` struct with:
- `get_balance(address) -> f64` — SOL balance via `getBalance` RPC
- `get_token_balance(address, mint) -> f64` — SPL token balance via `getTokenAccountsByOwner`
- `swap(keypair, input_mint, output_mint, amount_lamports, slippage_bps) -> Signature` — delegates to Jupiter v6 (shares logic with trade_swap tool)
- `get_recent_transactions(address, limit) -> Vec<TxSummary>`

**Complexity:** Medium (3–5 days)

---

### 1.3 TON trader crate
**File:** `crates/ton-trader/src/lib.rs`  
**Current:** Single `// TODO: implement TonTrader` comment.  
**Fix:** Implement a `TonTrader` struct backed by `tonlib-rs` with:
- `get_balance(address) -> f64`
- `swap_on_stonfi(keypair, from_token, to_token, amount) -> TxHash` — STON.fi swap
- `get_jetton_balance(wallet_address, jetton_master) -> f64`

**Dependencies:** `tonlib-rs` (already in Cargo.toml)  
**Complexity:** Complex (5–8 days, TON API is non-trivial)

---

### 1.4 EVM trader crate
**File:** `crates/evm-trader/src/` — no swap/trade functions found  
**Current:** Wallet creation exists in `wallet-manager` but no actual Uniswap swap execution.  
**Fix:** Add to evm-trader:
- `swap_uniswap_v3(signer, token_in, token_out, amount, fee_tier) -> TxHash`
- `get_erc20_balance(address, token_contract) -> U256`

**Dependencies:** `alloy`, `uniswap-v3-sdk` (already in Cargo.toml)  
**Complexity:** Complex (5–8 days)

---

## Phase 2 — Real Backtesting Engine

### 2.1 Historical price data fetcher
**File:** `src/tools/backtest.rs` — `run_backtest_engine()` function  
**Current:** Generates deterministic fake metrics from a `sin()` hash of the script filename. No real OHLCV data is used at all.  
**Fix:**
1. Fetch OHLCV data from Binance REST API (`GET /api/v3/klines`) for the given symbol and date range. Cache to `/data/<symbol>_<interval>.parquet` (as per CLAUDE.md spec).
2. Feed candles into a Rhai script execution loop:
   ```
   for each candle:
     call rhai fn on_candle(open, high, low, close, volume, timestamp)
     collect buy/sell signals returned
     simulate position P&L with fee deduction
   ```
3. Compute real metrics: PnL, Sharpe, max drawdown, win rate from actual trade log.

**Rhai API to expose to scripts:**
- `get_price()`, `get_rsi(period)`, `get_macd()`, `get_volume()`
- `buy(amount)`, `sell(amount)`, `get_position()`

**Complexity:** Complex (1–2 weeks)

---

### 2.2 Sync API backtest handler with tool
**File:** `src/gateway/api.rs:1647–1690` (`handle_api_backtest_run`)  
**Current:** Returns completely hardcoded metrics (`12.4%`, `1.32`, `8.7%`, `54%`, `87 trades`) — ignores the script content entirely.  
**Fix:** Replace hardcoded JSON with a call to the same `run_backtest_engine()` used by the agent tool. Both paths should produce consistent results.

**Complexity:** Simple (1 day — depends on 2.1 being done first)

---

## Phase 3 — Wallet & Balance Data

### 3.1 API wallet balance endpoint
**File:** `src/gateway/api.rs` — `handle_api_wallets_balance` (around line 783)  
**Current:** Always returns `{"balance": "0.00", "currency": "USD"}` regardless of chain or address.  
**Fix:**
- Solana: reuse `wallet_balance.rs` tool logic (`getBalance` RPC)
- EVM: `eth_getBalance` via Cloudflare ETH RPC (already in `wallet_balance.rs`)
- TON: tonlib-rs balance query

**Complexity:** Simple (1–2 days — logic already exists in `src/tools/wallet_balance.rs`)

---

### 3.2 SPL token balances
**File:** `src/tools/wallet_balance.rs`  
**Current:** Only reports native SOL balance. Shows no information about USDC, USDT, or other SPL tokens.  
**Fix:** Add `get_spl_token_balances(address) -> Vec<(mint, symbol, balance)>` using `getTokenAccountsByOwner` RPC call with `programId = TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA`.

**Complexity:** Simple (1 day)

---

## Phase 4 — Polymarket Integration

### 4.1 Polymarket configure endpoint
**File:** `src/gateway/api.rs:839–849`  
**Current:** `handle_api_polymarket_configure` accepts the body but does nothing with it — ignores all fields, returns `{"status": "ok"}`.  
**Fix:**
1. Validate the API key against Polymarket CLOB API (`GET /profile` or `GET /order`).
2. Encrypt and save `api_key`, `secret`, `passphrase` to config via `config.save()`.
3. Return error if validation fails.

**Complexity:** Simple (1 day)

---

### 4.2 Polymarket live positions and orders
**File:** `web/src/pages/Polymarket.tsx` + missing API routes  
**Current:** The frontend shows markets from the API, but open positions and active orders are not fetched from the CLOB API.  
**Fix:** Add routes:
- `GET /api/polymarket/positions` — open positions via CLOB `/positions`
- `GET /api/polymarket/orders` — open orders via CLOB `/orders`
- `POST /api/polymarket/order` — place limit/market order
- `DELETE /api/polymarket/order/:id` — cancel order

**Complexity:** Medium (2–3 days)

---

## Phase 5 — Agent Tool Polish

### 5.1 `shell` tool security on ReadOnly mode
**File:** `src/tools/shell_exec.rs:60–65`  
**Current:** Calls `is_command_allowed()` but `ReadOnly` autonomy will block all commands — the error message doesn't suggest what to do.  
**Fix:** Return a clear message: `"Shell execution requires autonomy level AutoEdit or higher. Current: ReadOnly. Change in Settings > Config."` 

**Complexity:** Trivial (30 min)

---

### 5.2 Trade swap — EVM support
**File:** `src/tools/trade_swap.rs`  
**Current:** Only handles Solana/Jupiter. Silently fails for EVM wallets.  
**Fix:** Add EVM swap path using Uniswap V3 Router when `from_token` is an EVM address or the wallet is on the `evm` chain.

**Complexity:** Medium (depends on Phase 1.4)

---

### 5.3 Trade swap — TON support
**File:** `src/tools/trade_swap.rs`  
**Current:** No TON handling.  
**Fix:** Add TON swap path via STON.fi when wallet chain is `ton`.

**Complexity:** Medium (depends on Phase 1.3)

---

## Summary Table

| # | Feature | File | Complexity | Phase |
|---|---------|------|-----------|-------|
| 1.1 | Solana swap broadcast | `src/tools/trade_swap.rs:326` | Medium | 1 |
| 1.2 | SolanaTrader crate | `crates/solana-trader/src/lib.rs` | Medium | 1 |
| 1.3 | TonTrader crate | `crates/ton-trader/src/lib.rs` | Complex | 1 |
| 1.4 | EVM swap execution | `crates/evm-trader/src/` | Complex | 1 |
| 2.1 | Real backtest engine | `src/tools/backtest.rs` | Complex | 2 |
| 2.2 | API backtest sync | `src/gateway/api.rs:1647` | Simple | 2 |
| 3.1 | Wallet balance API | `src/gateway/api.rs:783` | Simple | 3 |
| 3.2 | SPL token balances | `src/tools/wallet_balance.rs` | Simple | 3 |
| 4.1 | Polymarket configure | `src/gateway/api.rs:839` | Simple | 4 |
| 4.2 | Polymarket orders/positions | missing routes | Medium | 4 |
| 5.1 | Shell tool error message | `src/tools/shell_exec.rs:60` | Trivial | 5 |
| 5.2 | EVM swap tool | `src/tools/trade_swap.rs` | Medium | 5 |
| 5.3 | TON swap tool | `src/tools/trade_swap.rs` | Medium | 5 |

**Total: 13 items** — 1 trivial · 4 simple · 5 medium · 3 complex

---

## Suggested Order

```
Phase 1.1  →  Phase 1.2  →  Phase 3.1 + 3.2 (parallel)
     ↓
Phase 2.1  →  Phase 2.2
     ↓
Phase 4.1  →  Phase 4.2
     ↓
Phase 1.3 + 1.4 (parallel)  →  Phase 5.2 + 5.3
     ↓
Phase 5.1 (anytime)
```
