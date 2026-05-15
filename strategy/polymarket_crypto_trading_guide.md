# 🤖 Guía de Implementación: Estrategia 4-5 Min Optimizada para Crypto y Polymarket
## Bot de Trading en Rust + Rhai

---

## 📋 Tabla de Contenidos

1. [Comparación de Escenarios](#comparación-de-escenarios)
2. [Parámetros Optimizados](#parámetros-optimizados)
3. [Scripts Rhai Completos](#scripts-rhai-completos)
4. [Guía de Implementación Rust](#guía-de-implementación-rust)
5. [Testing y Validación](#testing-y-validación)
6. [Monitoreo en Vivo](#monitoreo-en-vivo)

---

## Comparación de Escenarios

### 📊 Análisis Comparativo

```
┌─────────────────────────────────────────────────────────────────────────┐
│                      CRYPTO (BINANCE)         vs      POLYMARKET 5-MIN  │
├─────────────────────────────────────────────────────────────────────────┤
│ Volatilidad          │ Alta (3-5x)           vs      Baja/Media         │
│ Spread               │ 0.01%-0.05%           vs      1-2% (round-trip)  │
│ Slippage             │ Variable              vs      Mínimo              │
│ Timeframe Efectivo   │ 4-5 min recomendado   vs      5 min (critical)   │
│ Ruido en Señal       │ Alto                  vs      Moderado           │
│ Liquidez             │ Excelente             vs      Variable por evento │
│ Fees                 │ ~0.1% (Maker/Taker)   vs      1-2% (Polymarket)  │
│ Correlación Assets   │ BTC/ETH correlados    vs      Binario (no corr.) │
│ Best Pair            │ BTC/USDT, ETH/USDT    vs      BTC_UP, BTC_DOWN   │
└─────────────────────────────────────────────────────────────────────────┘
```

### ⚠️ Implicaciones para Estrategia

| Aspecto | Crypto | Polymarket |
|--------|--------|-----------|
| **Sensibilidad Momentum** | Mayor (necesita threshold alto) | Menor (umbrales moderados) |
| **Filtro Tendencia** | Crítico (SMA-50 esencial) | Flexible |
| **RSI Settings** | Más sueltos (25/75) | Más ajustados (30/70) |
| **Stop Loss** | ATR × 2.0-2.5 (amplio) | ATR × 1.5-2.0 (moderado) |
| **Max Hold** | 5-8 bars (mercados rápidos) | 10-15 bars (más estables) |
| **Volume Threshold** | 1.5-2.0× MA | 1.2-1.5× MA |

---

## Parámetros Optimizados

### 🔧 ESCENARIO 1: CRYPTO (BINANCE 4-MIN)

**Objetivo**: Win rate 55-65%, Retorno +2% a +8%, Sharpe > 0.8

```javascript
// ============================================================================
// PARÁMETROS CRYPTO (BINANCE 4-MIN)
// ============================================================================

// Lookback periods
let lookback_4      = 4;      // Momentum rápido (4 velas)
let lookback_14     = 14;     // RSI + EMA medium
let lookback_50     = 50;     // SMA trend filter (CRITICAL)

// MOMENTUM THRESHOLDS (aumentados para reducir ruido)
let mom_threshold   = 1.8;    // Min 4-bar momentum % (UP: >1.8%, DOWN: <-1.8%)
let mom1_threshold  = 0.5;    // Min 1-bar confirmation (UP: >0.5%, DOWN: <-0.5%)

// RSI SETTINGS (sueltos para volatilidad alta)
let rsi_oversold    = 25.0;   // Bullish rebound (was 30)
let rsi_overbought  = 75.0;   // Bearish correction (was 70)
let rsi_period      = 14;

// ATR-BASED RISK (amplios para movimientos bruscos)
let atr_period      = 14;
let atr_sl_mult     = 2.0;    // Stop loss = ATR × 2.0 (was 1.5)
let atr_tp_mult     = 3.0;    // Take profit = ATR × 3.0 (was 2.0)

// PROFIT/LOSS TARGETS (más amplios)
let max_loss_pct    = 3.5;    // Hard exit if loss ≥ 3.5%
let take_pft_pct    = 6.0;    // Hard exit if profit ≥ 6.0%

// VOLUME CONFIRMATION (más estricto)
let min_vol_mult    = 1.5;    // Vol must exceed 1.5× rolling average
let vol_ema_period  = 20;

// TIME-BASED EXIT
let max_hold        = 8;      // Max 8 bars (32 minutes)

// WARMUP & VALIDATION
let min_idx         = 100;    // Need 100+ bars before first trade
let confirmation_req= 4;      // Require ALL 4 signals (was 3-of-4)

// Fee consideration
let fee_pct         = 0.2;    // Binance ~0.1-0.2% (rounded up)

// ──────────────────────────────────────────────────────────────────────────
// WHY THESE VALUES FOR CRYPTO?
// ──────────────────────────────────────────────────────────────────────────
// • mom_threshold 1.8%: Binance 4-min moves fast; 0.8% triggers too many false signals
// • atr_sl_mult 2.0: Volatility is 3-5x higher; 1.5× too tight → stopped out prematurely
// • atr_tp_mult 3.0: Wider TP to capture extended moves without over-exiting
// • RSI 25/75: Oversold/overbought conditions less meaningful in high-volatility markets
// • SMA-50 filter: CRITICAL—only trade trends where close > EMA14 AND EMA14 > SMA50
// • max_hold 8: Quick in-and-out; 5 bars too restrictive given move length
// • confirmation_req 4: ALL signals needed to reduce whipsaws (vol, RSI, mom4, mom1)
```

---

### 🎯 ESCENARIO 2: POLYMARKET (5-MIN, BTC UP/DOWN)

**Objetivo**: Win rate 60-70%, Retorno +3% a +10%, Sharpe > 1.0

```javascript
// ============================================================================
// PARÁMETROS POLYMARKET (5-MIN, BINARIO BTC UP/DOWN)
// ============================================================================

// Lookback periods
let lookback_4      = 4;      // Momentum rápido
let lookback_14     = 14;     // RSI + EMA
let lookback_50     = 50;     // SMA trend filter (useful but not critical)

// MOMENTUM THRESHOLDS (moderate—less noise in binary markets)
let mom_threshold   = 1.2;    // Min 4-bar momentum % (UP: >1.2%, DOWN: <-1.2%)
let mom1_threshold  = 0.3;    // Min 1-bar confirmation (UP: >0.3%, DOWN: <-0.3%)

// RSI SETTINGS (standard—cleaner signals in binaries)
let rsi_oversold    = 30.0;   // Bullish rebound (standard)
let rsi_overbought  = 70.0;   // Bearish correction (standard)
let rsi_period      = 14;

// ATR-BASED RISK (tighter than crypto—less volatility)
let atr_period      = 14;
let atr_sl_mult     = 1.5;    // Stop loss = ATR × 1.5
let atr_tp_mult     = 2.5;    // Take profit = ATR × 2.5

// PROFIT/LOSS TARGETS (Polymarket fees are higher ~1-2%)
let max_loss_pct    = 2.5;    // Hard exit if loss ≥ 2.5%
let take_pft_pct    = 5.0;    // Hard exit if profit ≥ 5.0%

// VOLUME CONFIRMATION (moderate)
let min_vol_mult    = 1.2;    // Vol must exceed 1.2× rolling average
let vol_ema_period  = 15;

// TIME-BASED EXIT
let max_hold        = 12;     // Max 12 bars (60 minutes) - binary markets more stable

// WARMUP & VALIDATION
let min_idx         = 80;     // Need 80 bars before trading (less warmup than crypto)
let confirmation_req= 3;      // 3-of-4 OK (less strict than crypto)

// Fee consideration (Polymarket round-trip fees)
let fee_pct         = 1.5;    // Polymarket ~1-2% round-trip

// ──────────────────────────────────────────────────────────────────────────
// WHY THESE VALUES FOR POLYMARKET?
// ──────────────────────────────────────────────────────────────────────────
// • mom_threshold 1.2%: Binary markets less volatile; 1.8% too strict = missed signals
// • atr_sl_mult 1.5: Lower volatility = tighter stops acceptable
// • atr_tp_mult 2.5: Moderate; binary moves more predictable
// • RSI 30/70: Standard settings work well (cleaner signals than crypto)
// • Confirmation 3-of-4: Binary nature provides some "noise immunity"
// • max_hold 12: More time to develop; markets stable during events
// • SMA-50: Less critical; but still useful for trend filtering
// • fee_pct 1.5%: Account for Polymarket's 1-2% round-trip fees in P&L calc
```

---

## Scripts Rhai Completos

### ✅ SCRIPT 1: CRYPTO (BINANCE 4-MIN)

```javascript
// ============================================================================
// CRYPTO Trading Bot - Binance 4-Minute Strategy v2.1
// Platform: Rust + Rhai
// ============================================================================

fn on_candle(ctx) {
    // ── CONFIGURATION: CRYPTO (BINANCE) ──────────────────────────────────
    
    // Lookback periods
    let lookback_4      = 4;
    let lookback_14     = 14;
    let lookback_50     = 50;

    // Momentum thresholds
    let mom_threshold   = 1.8;
    let mom1_threshold  = 0.5;

    // RSI settings
    let rsi_oversold    = 25.0;
    let rsi_overbought  = 75.0;

    // ATR-based risk
    let atr_sl_mult     = 2.0;
    let atr_tp_mult     = 3.0;

    // Profit/loss targets
    let max_loss_pct    = 3.5;
    let take_pft_pct    = 6.0;

    // Volume confirmation
    let min_vol_mult    = 1.5;
    
    // Time-based exit
    let max_hold        = 8;

    // Warmup
    let min_idx         = 100;

    // ── PRE-CONDITIONS ────────────────────────────────────────────────────
    
    let idx = ctx.index;
    if idx < min_idx {
        return;
    }

    let close   = ctx.close;
    let volume  = ctx.volume;
    let atr     = ctx.atr(14);

    // ── EXIT LOGIC (FIRST) ────────────────────────────────────────────────
    // Close existing positions based on profit/loss or time
    
    if ctx.position != 0.0 {
        let ep          = ctx.entry_price;
        let bars_held   = idx - ctx.entry_index;
        let pnl_pct     = if ep > 0.0 { 
            (close - ep) / ep * 100.0 
        } else { 
            0.0 
        };

        // Adjust for shorts
        let effective_pnl = if ctx.position < 0.0 { -pnl_pct } else { pnl_pct };

        if effective_pnl >= take_pft_pct || 
           effective_pnl <= -max_loss_pct || 
           bars_held >= max_hold {
            
            ctx.sell(1.0);
            ctx.log("EXIT: PnL=" + effective_pnl + "% | Bars=" + bars_held);
        }
        return;
    }

    // ── TECHNICAL INDICATORS ──────────────────────────────────────────────

    // 4-candle momentum (short-term velocity)
    let close_4ago = ctx.close_at(idx - lookback_4);
    let mom4 = if close_4ago > 0.0 {
        (close - close_4ago) / close_4ago * 100.0
    } else {
        0.0
    };

    // 1-candle momentum (confirmation)
    let close_1ago = ctx.close_at(idx - 1);
    let mom1 = if close_1ago > 0.0 {
        (close - close_1ago) / close_1ago * 100.0
    } else {
        0.0
    };

    // EMA-14 (trend direction filter)
    let ema14 = ctx.ema(lookback_14);

    // SMA-50 (primary trend filter) ← CRITICAL FOR CRYPTO
    let sma50 = ctx.sma(lookback_50);

    // RSI-14 (overbought/oversold)
    let rsi = ctx.rsi(lookback_14);

    // Rolling volume average (exponential moving average)
    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.85 + volume * 0.15);
    let vol_ok = volume > avg_vol * min_vol_mult;

    // ATR-based stop distance
    let stop_dist = atr * atr_sl_mult;
    let tp_dist   = atr * atr_tp_mult;

    // ── ENTRY SIGNALS ─────────────────────────────────────────────────────

    // BULLISH: Count confirmations (0-4)
    let bull_count = 0;
    if mom4 > mom_threshold    { bull_count += 1; }  // Signal 1: 4-bar momentum
    if mom1 > mom1_threshold   { bull_count += 1; }  // Signal 2: 1-bar momentum
    if rsi < rsi_oversold      { bull_count += 1; }  // Signal 3: RSI oversold
    if vol_ok                  { bull_count += 1; }  // Signal 4: Volume spike

    // BEARISH: Count confirmations (0-4)
    let bear_count = 0;
    if mom4 < -mom_threshold   { bear_count += 1; }  // Signal 1: 4-bar momentum down
    if mom1 < -mom1_threshold  { bear_count += 1; }  // Signal 2: 1-bar momentum down
    if rsi > rsi_overbought    { bear_count += 1; }  // Signal 3: RSI overbought
    if vol_ok                  { bear_count += 1; }  // Signal 4: Volume spike

    // ── EXECUTE ENTRIES ───────────────────────────────────────────────────

    // LONG: All 4 signals + price > EMA-14 + EMA-14 > SMA-50
    if bull_count == 4 && close > ema14 && ema14 > sma50 {
        ctx.buy(1.0);
        ctx.set_stop_loss(close - stop_dist);
        ctx.set_take_profit(close + tp_dist);
        ctx.log("BUY: mom4=" + mom4 + "% | rsi=" + rsi + " | vol_ok=" + vol_ok);
    }

    // SHORT: All 4 signals + price < EMA-14 + EMA-14 < SMA-50
    else if bear_count == 4 && close < ema14 && ema14 < sma50 {
        ctx.sell(1.0);
        ctx.set_stop_loss(close + stop_dist);
        ctx.set_take_profit(close - tp_dist);
        ctx.log("SELL: mom4=" + mom4 + "% | rsi=" + rsi + " | vol_ok=" + vol_ok);
    }
}

// ============================================================================
// END CRYPTO SCRIPT
// ============================================================================
```

---

### ✅ SCRIPT 2: POLYMARKET (5-MIN, BINARIO)

```javascript
// ============================================================================
// POLYMARKET Trading Bot - BTC UP/DOWN 5-Minute Strategy v2.1
// Platform: Rust + Rhai
// Markets: BTC_UP, BTC_DOWN (binary prediction markets)
// ============================================================================

fn on_candle(ctx) {
    // ── CONFIGURATION: POLYMARKET (BINARY) ────────────────────────────────
    
    // Lookback periods
    let lookback_4      = 4;
    let lookback_14     = 14;
    let lookback_50     = 50;

    // Momentum thresholds (less strict than crypto)
    let mom_threshold   = 1.2;
    let mom1_threshold  = 0.3;

    // RSI settings (standard)
    let rsi_oversold    = 30.0;
    let rsi_overbought  = 70.0;

    // ATR-based risk (tighter than crypto)
    let atr_sl_mult     = 1.5;
    let atr_tp_mult     = 2.5;

    // Profit/loss targets (account for 1-2% Polymarket fees)
    let max_loss_pct    = 2.5;
    let take_pft_pct    = 5.0;

    // Volume confirmation (moderate)
    let min_vol_mult    = 1.2;
    
    // Time-based exit (binary markets more stable)
    let max_hold        = 12;

    // Warmup (less than crypto)
    let min_idx         = 80;

    // ── PRE-CONDITIONS ────────────────────────────────────────────────────
    
    let idx = ctx.index;
    if idx < min_idx {
        return;
    }

    let close   = ctx.close;
    let volume  = ctx.volume;
    let atr     = ctx.atr(14);

    // ── EXIT LOGIC (FIRST) ────────────────────────────────────────────────
    // Close existing positions based on profit/loss or time
    
    if ctx.position != 0.0 {
        let ep          = ctx.entry_price;
        let bars_held   = idx - ctx.entry_index;
        let pnl_pct     = if ep > 0.0 { 
            (close - ep) / ep * 100.0 
        } else { 
            0.0 
        };

        // Adjust for shorts
        let effective_pnl = if ctx.position < 0.0 { -pnl_pct } else { pnl_pct };

        if effective_pnl >= take_pft_pct || 
           effective_pnl <= -max_loss_pct || 
           bars_held >= max_hold {
            
            ctx.sell(1.0);
            ctx.log("EXIT: PnL=" + effective_pnl + "% | Bars=" + bars_held);
        }
        return;
    }

    // ── TECHNICAL INDICATORS ──────────────────────────────────────────────

    // 4-candle momentum
    let close_4ago = ctx.close_at(idx - lookback_4);
    let mom4 = if close_4ago > 0.0 {
        (close - close_4ago) / close_4ago * 100.0
    } else {
        0.0
    };

    // 1-candle momentum (confirmation)
    let close_1ago = ctx.close_at(idx - 1);
    let mom1 = if close_1ago > 0.0 {
        (close - close_1ago) / close_1ago * 100.0
    } else {
        0.0
    };

    // EMA-14 (trend direction)
    let ema14 = ctx.ema(lookback_14);

    // SMA-50 (secondary trend filter)
    let sma50 = ctx.sma(lookback_50);

    // RSI-14 (overbought/oversold)
    let rsi = ctx.rsi(lookback_14);

    // Rolling volume average
    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.9 + volume * 0.1);
    let vol_ok = volume > avg_vol * min_vol_mult;

    // ATR-based stop distance
    let stop_dist = atr * atr_sl_mult;
    let tp_dist   = atr * atr_tp_mult;

    // ── ENTRY SIGNALS ─────────────────────────────────────────────────────

    // BULLISH: Count confirmations (requires 3 of 4)
    let bull_count = 0;
    if mom4 > mom_threshold    { bull_count += 1; }
    if mom1 > mom1_threshold   { bull_count += 1; }
    if rsi < rsi_oversold      { bull_count += 1; }
    if vol_ok                  { bull_count += 1; }

    // BEARISH: Count confirmations (requires 3 of 4)
    let bear_count = 0;
    if mom4 < -mom_threshold   { bear_count += 1; }
    if mom1 < -mom1_threshold  { bear_count += 1; }
    if rsi > rsi_overbought    { bear_count += 1; }
    if vol_ok                  { bear_count += 1; }

    // ── EXECUTE ENTRIES ───────────────────────────────────────────────────

    // LONG (BTC_UP): 3-of-4 signals + trend filter
    if bull_count >= 3 && close > ema14 {
        ctx.buy(1.0);
        ctx.set_stop_loss(close - stop_dist);
        ctx.set_take_profit(close + tp_dist);
        ctx.log("BUY: bull_count=" + bull_count + " | mom4=" + mom4 + "% | rsi=" + rsi);
    }

    // SHORT (BTC_DOWN): 3-of-4 signals + trend filter
    else if bear_count >= 3 && close < ema14 {
        ctx.sell(1.0);
        ctx.set_stop_loss(close + stop_dist);
        ctx.set_take_profit(close - tp_dist);
        ctx.log("SELL: bear_count=" + bear_count + " | mom4=" + mom4 + "% | rsi=" + rsi);
    }
}

// ============================================================================
// END POLYMARKET SCRIPT
// ============================================================================
```

---

## Guía de Implementación Rust

### 📁 Estructura de Proyecto

```
trading-bot-rust/
├── Cargo.toml
├── src/
│   ├── main.rs                    # Entry point
│   ├── lib.rs
│   ├── config/
│   │   ├── mod.rs
│   │   ├── crypto.rs              # Binance config
│   │   └── polymarket.rs          # Polymarket config
│   ├── engine/
│   │   ├── mod.rs
│   │   ├── backtester.rs          # Backtest engine
│   │   ├── live_trader.rs         # Live trading
│   │   └── script_executor.rs     # Rhai executor
│   ├── exchange/
│   │   ├── mod.rs
│   │   ├── binance.rs
│   │   └── polymarket.rs
│   ├── models/
│   │   ├── mod.rs
│   │   ├── candle.rs
│   │   └── position.rs
│   └── utils/
│       ├── mod.rs
│       └── logger.rs
├── scripts/
│   ├── crypto_4min.rhai           # Script CRYPTO
│   └── polymarket_5min.rhai       # Script POLYMARKET
└── data/
    ├── crypto_4m_sample.csv       # Sample data
    └── polymarket_5m_sample.csv   # Sample data
```

---

### 🔨 Cargo.toml

```toml
[package]
name = "trading-bot-rust"
version = "2.1.0"
edition = "2021"

[dependencies]
# Core
tokio = { version = "1.35", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Scripting
rhai = { version = "1.17", features = ["sync"] }

# Exchange APIs
reqwest = { version = "0.11", features = ["json"] }
binance = "0.12"  # Binance API

# Data & Time
chrono = "0.4"
csv = "1.3"
futures = "0.3"

# Utilities
thiserror = "1.0"
tracing = "0.1"
tracing-subscriber = "0.3"
log = "0.4"
anyhow = "1.0"

[dev-dependencies]
tokio-test = "0.4"
```

---

### 🎯 config/crypto.rs

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoConfig {
    // Exchange settings
    pub exchange: String,          // "binance"
    pub symbol: String,            // "BTCUSDT", "ETHUSDT", etc.
    pub interval: String,          // "4m"
    
    // Account settings
    pub api_key: String,
    pub api_secret: String,
    pub initial_balance: f64,      // USDT
    pub leverage: f64,             // 1.0 for spot, 2.0-125.0 for futures
    
    // Strategy parameters
    pub lookback_4: usize,
    pub lookback_14: usize,
    pub lookback_50: usize,
    
    pub mom_threshold: f64,        // 1.8%
    pub mom1_threshold: f64,       // 0.5%
    pub rsi_oversold: f64,         // 25.0
    pub rsi_overbought: f64,       // 75.0
    
    pub atr_sl_mult: f64,          // 2.0
    pub atr_tp_mult: f64,          // 3.0
    pub max_loss_pct: f64,         // 3.5%
    pub take_pft_pct: f64,         // 6.0%
    pub min_vol_mult: f64,         // 1.5
    pub max_hold: usize,           // 8 bars
    pub min_idx: usize,            // 100
    
    // Fee
    pub fee_pct: f64,              // 0.2%
    
    // Script
    pub script_path: String,       // Path to crypto_4min.rhai
}

impl Default for CryptoConfig {
    fn default() -> Self {
        Self {
            exchange: "binance".to_string(),
            symbol: "BTCUSDT".to_string(),
            interval: "4m".to_string(),
            
            api_key: std::env::var("BINANCE_API_KEY").unwrap_or_default(),
            api_secret: std::env::var("BINANCE_API_SECRET").unwrap_or_default(),
            initial_balance: 1000.0,
            leverage: 1.0,
            
            lookback_4: 4,
            lookback_14: 14,
            lookback_50: 50,
            
            mom_threshold: 1.8,
            mom1_threshold: 0.5,
            rsi_oversold: 25.0,
            rsi_overbought: 75.0,
            
            atr_sl_mult: 2.0,
            atr_tp_mult: 3.0,
            max_loss_pct: 3.5,
            take_pft_pct: 6.0,
            min_vol_mult: 1.5,
            max_hold: 8,
            min_idx: 100,
            
            fee_pct: 0.2,
            
            script_path: "scripts/crypto_4min.rhai".to_string(),
        }
    }
}
```

---

### 🎯 config/polymarket.rs

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolymarketConfig {
    // Market settings
    pub market_type: String,       // "polymarket"
    pub symbol: String,            // "BTC_UP", "BTC_DOWN", etc.
    pub interval: String,          // "5m"
    
    // Account settings
    pub api_key: String,
    pub private_key: String,
    pub initial_balance: f64,      // USDC
    
    // Strategy parameters
    pub lookback_4: usize,
    pub lookback_14: usize,
    pub lookback_50: usize,
    
    pub mom_threshold: f64,        // 1.2%
    pub mom1_threshold: f64,       // 0.3%
    pub rsi_oversold: f64,         // 30.0
    pub rsi_overbought: f64,       // 70.0
    
    pub atr_sl_mult: f64,          // 1.5
    pub atr_tp_mult: f64,          // 2.5
    pub max_loss_pct: f64,         // 2.5%
    pub take_pft_pct: f64,         // 5.0%
    pub min_vol_mult: f64,         // 1.2
    pub max_hold: usize,           // 12 bars
    pub min_idx: usize,            // 80
    
    // Fee
    pub fee_pct: f64,              // 1.5%
    
    // Script
    pub script_path: String,       // Path to polymarket_5min.rhai
}

impl Default for PolymarketConfig {
    fn default() -> Self {
        Self {
            market_type: "polymarket".to_string(),
            symbol: "BTC_UP".to_string(),
            interval: "5m".to_string(),
            
            api_key: std::env::var("POLYMARKET_API_KEY").unwrap_or_default(),
            private_key: std::env::var("POLYMARKET_PRIVATE_KEY").unwrap_or_default(),
            initial_balance: 1000.0,
            
            lookback_4: 4,
            lookback_14: 14,
            lookback_50: 50,
            
            mom_threshold: 1.2,
            mom1_threshold: 0.3,
            rsi_oversold: 30.0,
            rsi_overbought: 70.0,
            
            atr_sl_mult: 1.5,
            atr_tp_mult: 2.5,
            max_loss_pct: 2.5,
            take_pft_pct: 5.0,
            min_vol_mult: 1.2,
            max_hold: 12,
            min_idx: 80,
            
            fee_pct: 1.5,
            
            script_path: "scripts/polymarket_5min.rhai".to_string(),
        }
    }
}
```

---

### 🎬 engine/script_executor.rs

```rust
use rhai::{Engine, Scope};
use std::sync::{Arc, Mutex};

pub struct ScriptContext {
    pub index: usize,
    pub close: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub volume: f64,
    
    pub position: f64,         // 1.0 = long, -1.0 = short, 0.0 = flat
    pub entry_price: f64,
    pub entry_index: usize,
    
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
}

pub struct ScriptExecutor {
    engine: Engine,
    script: String,
}

impl ScriptExecutor {
    pub fn new(script_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut engine = Engine::new();
        
        // Register custom functions for context
        engine.register_fn("buy", |ctx: &mut ScriptContext| {
            ctx.position = 1.0;
        });
        
        engine.register_fn("sell", |ctx: &mut ScriptContext| {
            ctx.position = -1.0;
        });
        
        engine.register_fn("set_stop_loss", |ctx: &mut ScriptContext, sl: f64| {
            ctx.stop_loss = Some(sl);
        });
        
        engine.register_fn("set_take_profit", |ctx: &mut ScriptContext, tp: f64| {
            ctx.take_profit = Some(tp);
        });
        
        let script = std::fs::read_to_string(script_path)?;
        
        Ok(Self { engine, script })
    }
    
    pub fn execute(&self, ctx: &mut ScriptContext) -> Result<(), Box<dyn std::error::Error>> {
        // Create scope with context data
        let mut scope = Scope::new();
        scope.push("ctx", ctx.clone());
        
        // Execute script
        self.engine.run_with_scope(&mut scope, &self.script)?;
        
        Ok(())
    }
}
```

---

### 📊 engine/backtester.rs (ejemplo base)

```rust
use crate::models::Candle;
use std::collections::VecDeque;

pub struct Backtester {
    candles: VecDeque<Candle>,
    initial_balance: f64,
    current_balance: f64,
    positions: Vec<Position>,
    fee_pct: f64,
}

#[derive(Clone)]
pub struct Position {
    pub entry_price: f64,
    pub entry_index: usize,
    pub size: f64,  // 1.0 for long, -1.0 for short
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
}

pub struct BacktestResult {
    pub total_return: f64,
    pub win_rate: f64,
    pub total_trades: usize,
    pub max_drawdown: f64,
    pub sharpe_ratio: f64,
}

impl Backtester {
    pub fn new(initial_balance: f64, fee_pct: f64) -> Self {
        Self {
            candles: VecDeque::new(),
            initial_balance,
            current_balance: initial_balance,
            positions: Vec::new(),
            fee_pct,
        }
    }
    
    pub fn add_candle(&mut self, candle: Candle) {
        self.candles.push_back(candle);
    }
    
    pub fn run(&mut self) -> BacktestResult {
        let mut trades = Vec::new();
        
        for (idx, candle) in self.candles.iter().enumerate() {
            // Execute script for this candle
            // Update positions based on SL/TP
            // Track P&L
        }
        
        // Calculate metrics
        BacktestResult {
            total_return: ((self.current_balance - self.initial_balance) / self.initial_balance) * 100.0,
            win_rate: self.calculate_win_rate(&trades),
            total_trades: trades.len(),
            max_drawdown: self.calculate_max_drawdown(&trades),
            sharpe_ratio: self.calculate_sharpe_ratio(&trades),
        }
    }
}
```

---

## Testing y Validación

### ✅ Checklist de Testing

```markdown
## FASE 1: Validación Local (Backtesting)

### Crypto (Binance 4-min)
- [ ] Ejecutar en 3 meses de datos históricos
- [ ] Verificar Win Rate > 50%
- [ ] Verificar Sharpe Ratio > 0.8
- [ ] Verificar Max Drawdown < 15%
- [ ] Verificar # trades entre 20-50 por mes
- [ ] Validar en 2-3 símbolos distintos (BTC, ETH, SOL)
- [ ] Comprobar parámetros:
  - [ ] mom_threshold: 1.8 debe reducir signals falsos
  - [ ] SMA-50 filter: elimina trades en mercados laterales
  - [ ] ATR × 2.0: stops no demasiado ajustados

### Polymarket (5-min Binarios)
- [ ] Ejecutar en datos de 2-3 eventos distintos
- [ ] Verificar Win Rate > 55%
- [ ] Verificar Sharpe Ratio > 1.0
- [ ] Verificar Max Drawdown < 10%
- [ ] Validar en BTC_UP y BTC_DOWN
- [ ] Comprobar parámetros:
  - [ ] mom_threshold: 1.2 OK para binarios
  - [ ] Confirmation 3-of-4: balancea selectividad y oportunidad
  - [ ] ATR × 1.5: apropiado para volatilidad baja

---

## FASE 2: Paper Trading (Simulación en Vivo)

### Setup
```bash
# Crypto (Binance)
MARKET=crypto SYMBOL=BTCUSDT INTERVAL=4m PAPER_TRADING=true cargo run

# Polymarket
MARKET=polymarket SYMBOL=BTC_UP INTERVAL=5m PAPER_TRADING=true cargo run
```

### Métricas a Monitorear (2-3 semanas)
- [ ] Drawdown intraday (¿se comporta como esperado?)
- [ ] Timing de entries (¿se abren en puntos lógicos?)
- [ ] Timing de exits (¿SL/TP se disparan correctamente?)
- [ ] Slippage vs backtesting (¿diferencia aceptable?)
- [ ] Distribucion de wins/losses

---

## FASE 3: Live Trading (Capital Real - PEQUEÑO)

### Setup Inicial
```bash
# Start with 50 USDT / 0.001 BTC
LIVE_TRADING=true ACCOUNT_SIZE=50 LEVERAGE=1.0 cargo run
```

### Métricas a Monitorear (4 semanas mínimo)
- [ ] Performance vs paper trading
- [ ] Drawdown máximo tolerado
- [ ] Consistencia semana a semana
- [ ] Slippage/fees impacto en PnL
- [ ] Logs: cada entrada, SL, TP perfectamente documentado

---

## Criterios para Escalar Capital

| Métrica | Escalar | Revisar |
|---------|---------|---------|
| Win Rate | > 55% | < 45% |
| Sharpe | > 1.0 | < 0.5 |
| Max DD | < 12% | > 20% |
| Consistency | Ganancia 3/4 sem. | Pérdida 2+ sem. |
```

---

### 🧪 Script de Testing en Bash

```bash
#!/bin/bash
# test_strategy.sh - Ejecuta backtests y genera reportes

set -e

CRYPTO_SYMBOLS=("BTCUSDT" "ETHUSDT" "BNBUSDT")
POLYMARKET_SYMBOLS=("BTC_UP" "BTC_DOWN")

echo "=========================================="
echo "BACKTEST: CRYPTO (4-MIN)"
echo "=========================================="

for symbol in "${CRYPTO_SYMBOLS[@]}"; do
    echo "Testing $symbol..."
    cargo run --release -- \
        --market crypto \
        --symbol "$symbol" \
        --interval 4m \
        --backtest true \
        --days 90 \
        --output "results/crypto_${symbol}_90d.json"
done

echo ""
echo "=========================================="
echo "BACKTEST: POLYMARKET (5-MIN)"
echo "=========================================="

for symbol in "${POLYMARKET_SYMBOLS[@]}"; do
    echo "Testing $symbol..."
    cargo run --release -- \
        --market polymarket \
        --symbol "$symbol" \
        --interval 5m \
        --backtest true \
        --days 30 \
        --output "results/polymarket_${symbol}_30d.json"
done

echo ""
echo "=========================================="
echo "ANALYSIS COMPLETE"
echo "=========================================="
echo "Results saved to /results directory"
```

---

## Monitoreo en Vivo

### 📊 Dashboard Recomendado

```
╔═══════════════════════════════════════════════════════════════════╗
║                    TRADING BOT DASHBOARD                         ║
╠═══════════════════════════════════════════════════════════════════╣
║ ACCOUNT STATUS                                                    ║
║  Balance: $1,250.43  | Open P&L: +$45.23 (+3.7%) | Risk: 2.1%   ║
║                                                                   ║
║ CURRENT POSITION                                                  ║
║  Type: LONG BTC  | Entry: $42,150  | Current: $42,340           ║
║  Size: 0.025 BTC | P&L: +$4.75 (+1.1%) | Time: 12m              ║
║  SL: $41,500  | TP: $43,100                                      ║
║                                                                   ║
║ SESSION STATISTICS (Last 7 days)                                 ║
║  Trades: 15 | Wins: 10 | Losses: 5 | Win Rate: 66.7%            ║
║  Avg Win: +1.8% | Avg Loss: -1.2% | Profit Factor: 2.45         ║
║  Max Drawdown: 2.3%                                              ║
║                                                                   ║
║ RECENT TRADES                                                    ║
║  [CLOSE] 14:23 LONG ETHUSDT @ 2,235 +2.1% ✓                     ║
║  [CLOSE] 13:15 SHORT BNBUSDT @ 612 -0.8% ✗                      ║
║  [OPEN]  12:47 LONG BTCUSDT @ 42,150 +1.1% ...                  ║
║                                                                   ║
║ ALERTS                                                            ║
║  ⚠️  High volatility detected (VIX equiv: 35)                    ║
║  ℹ️  Next event: FOMC decision in 2h 15m                         ║
╚═══════════════════════════════════════════════════════════════════╝
```

### 📝 Logging Estructura

```rust
// Cada operación debe ser logeada con este formato:

[2024-01-15 14:23:45] [CRYPTO] [BTC_4M]
  Signal: BULLISH
  mom4=2.1% | mom1=0.7% | RSI=28 | vol=1.8x
  ├─ Entry: LONG @ 42,150 (size: 0.025)
  ├─ SL: 41,500 (-1.5% ATR)
  ├─ TP: 43,100 (+3.0% ATR)
  └─ Expected RRR: 1:2.0

[2024-01-15 14:35:23] [CRYPTO] [BTC_4M]
  Position Update: +1.1% (P&L: +$4.75)
  Price: 42,340 | Bars held: 3
  
[2024-01-15 14:47:12] [CRYPTO] [BTC_4M]
  CLOSE POSITION: PROFIT TARGET HIT
  Exit: 43,100 | P&L: +1.8% ($7.63)
  Win Rate impact: 10/15 trades (66.7%)
```

---

## Resumen de Diferencias Clave

### 📈 CRYPTO (Binance 4-min)
```
✓ Alta volatilidad → Parámetros ajustados
✓ SMA-50 CRÍTICO → Filtra mercados laterales
✓ Confirmation 4-of-4 → Reduce false signals
✓ Wide stops (ATR × 2.0) → Permite movimientos normales
✓ Más tiempo entre trades → Mejor análisis
```

### 📊 POLYMARKET (5-min Binarios)
```
✓ Baja volatilidad → Parámetros moderados
✓ Confirmación 3-of-4 → Aprovecha más oportunidades
✓ Tight stops (ATR × 1.5) → Aceptable dado volatilidad baja
✓ Fee awareness 1.5% → Ajustar P&L targets
✓ Mayor tiempo de hold → Markets estables
```

---

## Próximos Pasos

1. **Implementar Config System** → Cargue parameters desde JSON/ENV
2. **Buildear Backtester** → 90 días Binance, 30 días Polymarket
3. **Paper Trading 2 semanas** → Valide antes de live
4. **Live Trading Minimal** → $50 initial, track religiosamente
5. **Escalamiento gradual** → 2x capital cada 4 semanas si Sharpe > 1.0

---

**Autor**: Trading Bot v2.1  
**Última actualización**: 2024-01-15  
**Status**: Production Ready (después de Phase 1-2)
