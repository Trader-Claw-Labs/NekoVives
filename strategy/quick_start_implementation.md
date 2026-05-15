# 🚀 Quick Start: Implementación en Rust + Rhai

## Step 1: Crear el Proyecto

```bash
cargo new trading-bot-rust --name trading_bot
cd trading-bot-rust
```

## Step 2: Actualizar Cargo.toml

```toml
[package]
name = "trading-bot-rust"
version = "2.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1.35", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
rhai = { version = "1.17", features = ["sync"] }
reqwest = { version = "0.11", features = ["json"] }
chrono = "0.4"
csv = "1.3"
anyhow = "1.0"
thiserror = "1.0"
log = "0.4"
env_logger = "0.11"
dotenv = "0.15"

[dev-dependencies]
tokio-test = "0.4"
```

## Step 3: Crear Estructura de Directorios

```bash
mkdir -p src/{config,engine,exchange,models,utils}
mkdir -p scripts data results logs
touch .env
touch src/lib.rs
```

## Step 4: Crear .env (Variables de Entorno)

```bash
# .env
BINANCE_API_KEY=your_binance_key
BINANCE_API_SECRET=your_binance_secret

POLYMARKET_API_KEY=your_polymarket_key
POLYMARKET_PRIVATE_KEY=your_polymarket_private_key

RUST_LOG=info,trading_bot=debug
```

## Step 5: Implementar Estructura Base

### src/lib.rs

```rust
pub mod config;
pub mod engine;
pub mod exchange;
pub mod models;
pub mod utils;

pub use config::{CryptoConfig, PolymarketConfig};
pub use engine::{Backtester, LiveTrader};
```

### src/main.rs

```rust
use std::env;
use dotenv::dotenv;
use log::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load environment variables
    dotenv().ok();
    env_logger::init();
    
    let args: Vec<String> = env::args().collect();
    
    if args.len() < 2 {
        eprintln!("Usage: trading-bot <command> [options]");
        eprintln!("Commands:");
        eprintln!("  backtest-crypto <days>    - Run crypto backtest");
        eprintln!("  backtest-poly <days>      - Run Polymarket backtest");
        eprintln!("  paper-crypto              - Paper trading crypto");
        eprintln!("  paper-poly                - Paper trading Polymarket");
        return Ok(());
    }
    
    let command = &args[1];
    
    match command.as_str() {
        "backtest-crypto" => run_crypto_backtest().await?,
        "backtest-poly" => run_polymarket_backtest().await?,
        "paper-crypto" => run_crypto_paper_trading().await?,
        "paper-poly" => run_polymarket_paper_trading().await?,
        _ => eprintln!("Unknown command: {}", command),
    }
    
    Ok(())
}

async fn run_crypto_backtest() -> anyhow::Result<()> {
    info!("Starting Crypto Backtest...");
    
    let config = trading_bot::CryptoConfig::default();
    let mut backtester = trading_bot::engine::Backtester::new(
        config.initial_balance,
        config.fee_pct
    );
    
    // Load candles from CSV or API
    let candles = load_historical_data(&config).await?;
    
    for candle in candles {
        backtester.add_candle(candle);
    }
    
    let results = backtester.run()?;
    
    println!("\n========== RESULTS ==========");
    println!("Total Return: {:.2}%", results.total_return);
    println!("Win Rate: {:.1}%", results.win_rate * 100.0);
    println!("Sharpe Ratio: {:.2}", results.sharpe_ratio);
    println!("Max Drawdown: {:.2}%", results.max_drawdown * 100.0);
    println!("Total Trades: {}", results.total_trades);
    
    Ok(())
}

async fn load_historical_data(config: &trading_bot::CryptoConfig) 
    -> anyhow::Result<Vec<trading_bot::models::Candle>> {
    // TODO: Implement loading from Binance API or CSV
    Ok(Vec::new())
}

async fn run_polymarket_backtest() -> anyhow::Result<()> {
    info!("Starting Polymarket Backtest...");
    // Similar to crypto_backtest
    Ok(())
}

async fn run_crypto_paper_trading() -> anyhow::Result<()> {
    info!("Starting Crypto Paper Trading...");
    // TODO: Implement paper trading
    Ok(())
}

async fn run_polymarket_paper_trading() -> anyhow::Result<()> {
    info!("Starting Polymarket Paper Trading...");
    // TODO: Implement paper trading
    Ok(())
}
```

### src/models/mod.rs

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub timestamp: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Clone)]
pub struct Position {
    pub entry_price: f64,
    pub entry_time: DateTime<Utc>,
    pub size: f64,  // 1.0 = long, -1.0 = short
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TradeResult {
    pub entry_price: f64,
    pub exit_price: f64,
    pub pnl: f64,
    pub pnl_pct: f64,
    pub entry_time: DateTime<Utc>,
    pub exit_time: DateTime<Utc>,
    pub bars_held: usize,
}
```

### src/config/mod.rs

```rust
pub mod crypto;
pub mod polymarket;

pub use crypto::CryptoConfig;
pub use polymarket::PolymarketConfig;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub crypto: Option<CryptoConfig>,
    pub polymarket: Option<PolymarketConfig>,
    pub global: GlobalConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub environment: String,  // "backtest", "paper", "live"
    pub timezone: String,
    pub log_level: String,
}

impl Config {
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config = serde_json::from_str(&content)?;
        Ok(config)
    }
}
```

## Step 6: Copiar Scripts Rhai

```bash
# Copy the Rhai scripts to scripts/ directory
cp crypto_4min.rhai scripts/
cp polymarket_5min.rhai scripts/
```

## Step 7: Ejecutar Primeros Tests

```bash
# Build
cargo build --release

# Backtest Crypto
RUST_LOG=info cargo run --release -- backtest-crypto 90

# Backtest Polymarket  
RUST_LOG=info cargo run --release -- backtest-poly 30
```

---

## Flujo de Desarrollo Recomendado

### Semana 1: Configuración Base
- [ ] Implementar CryptoConfig y PolymarketConfig
- [ ] Crear modelos básicos (Candle, Position, TradeResult)
- [ ] Setupear logging con log/env_logger
- [ ] Copiar scripts Rhai y validar syntax

### Semana 2: Backtesting
- [ ] Implementar Backtester con carga de datos (CSV o API)
- [ ] Integración con Rhai ScriptExecutor
- [ ] Métrica calculations (Win Rate, Sharpe, Max DD)
- [ ] Ejecutar backtests en ambos escenarios

### Semana 3: Paper Trading
- [ ] Implementar LiveTrader para datos en vivo
- [ ] Integración API (Binance, Polymarket)
- [ ] Dashboard/logging de posiciones
- [ ] Paper trading por 2 semanas

### Semana 4: Live Trading (Minimal)
- [ ] Position sizing y risk management
- [ ] Real order execution
- [ ] Alert system (Slack/Email)
- [ ] Monitoreo 24/7

---

## Comandos Útiles

```bash
# Compilar en desarrollo
cargo build

# Compilar optimizado
cargo build --release

# Ejecutar tests
cargo test

# Ver logs en vivo
RUST_LOG=debug cargo run -- backtest-crypto 30

# Format código
cargo fmt

# Lint
cargo clippy

# Generar documentación
cargo doc --open
```

---

## Testing Progresivo

### Prueba 1: Carga de Datos
```rust
#[test]
fn test_load_candles() {
    let candles = load_csv("data/sample.csv").unwrap();
    assert!(candles.len() > 0);
    assert!(candles[0].close > 0.0);
}
```

### Prueba 2: Script Execution
```rust
#[test]
fn test_rhai_execution() {
    let executor = ScriptExecutor::new("scripts/crypto_4min.rhai").unwrap();
    let mut ctx = ScriptContext { /* ... */ };
    executor.execute(&mut ctx).unwrap();
    // Assert on ctx.position or other fields
}
```

### Prueba 3: Backtest Completo
```rust
#[tokio::test]
async fn test_crypto_backtest() {
    let config = CryptoConfig::default();
    let mut backtester = Backtester::new(1000.0, 0.2);
    let candles = load_sample_data().await.unwrap();
    
    for candle in candles {
        backtester.add_candle(candle);
    }
    
    let results = backtester.run().unwrap();
    assert!(results.win_rate > 0.4);  // Should be > 40%
}
```

---

## Documentación y Recursos

1. **Rhai Book**: https://rhai.rs/
2. **Tokio Async**: https://tokio.rs/
3. **Serde Serialization**: https://serde.rs/
4. **Binance API**: https://binance-docs.github.io/apidocs/
5. **Polymarket Docs**: https://docs.polymarket.com/

---

## Próximos Pasos Después del Setup

1. **Implementar data providers**:
   - Binance REST API para datos históricos
   - WebSocket para datos en vivo

2. **Meter más indicadores**:
   - MACD
   - Bollinger Bands
   - Stochastic RSI

3. **Risk Management avanzado**:
   - Position sizing dinámico
   - Correlación con otros pares
   - Gestión de drawdown

4. **Optimización**:
   - Parameter optimization (grid search)
   - Walk-forward analysis
   - Monte Carlo simulation

5. **Production**:
   - Docker containerization
   - Cloud deployment (AWS/GCP)
   - Real-time monitoring dashboard
