# 🚀 Polymarket Trading Agent - Rust + Rhai

Agente de trading completamente embebido en Rust que ejecuta la estrategia de Polymarket 4-minutos mediante scripts Rhai.

## 📋 Tabla de Contenidos

1. [Características](#características)
2. [Requisitos](#requisitos)
3. [Instalación](#instalación)
4. [Compilación](#compilación)
5. [Uso](#uso)
6. [Arquitectura](#arquitectura)
7. [Ejemplos](#ejemplos)
8. [Configuración](#configuración)
9. [Troubleshooting](#troubleshooting)

---

## ✨ Características

- ✅ **Script Rhai embebido**: Ejecuta estrategia sin recompilar
- ✅ **Hot-reload**: Actualiza estrategia sin detener el bot
- ✅ **Type-safe**: Sistema de tipos Rust robusto
- ✅ **Async/await**: Runtime tokio para operaciones no-bloqueantes
- ✅ **Multi-market**: Ejecuta múltiples mercados simultáneamente
- ✅ **Persistencia**: Guarda/carga estado del bot
- ✅ **Event logging**: Auditoría completa de operaciones
- ✅ **Zero-copy**: Acceso eficiente a datos
- ✅ **Backtesting**: Simula operaciones históricas
- ✅ **JSON export**: Exporta estadísticas fácilmente

---

## 📦 Requisitos

### Sistema
- Rust 1.70+ (instalado vía [rustup.rs](https://rustup.rs/))
- Cargo (incluido con Rust)
- 2GB RAM mínimo
- 500MB disco para compilación

### Linux/Mac
```bash
# Verificar Rust instalado
rustc --version
cargo --version

# Si no está instalado:
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

### Windows
```powershell
# Descargar desde https://rustup.rs/
# O usar scoop:
scoop install rustup
rustup install stable
```

---

## 🔧 Instalación

### 1. Clonar/Descargar Proyecto

```bash
# Crear directorio del proyecto
mkdir polymarket-trading-agent
cd polymarket-trading-agent

# Copiar archivos:
# - strategy.rhai
# - Cargo.toml
# - src/lib.rs
# - src/agent.rs
# - src/main.rs
```

### 2. Estructura de Carpetas

```
polymarket-trading-agent/
├── Cargo.toml                    # Configuración del proyecto
├── strategy.rhai                 # Script de la estrategia
├── src/
│   ├── main.rs                   # Aplicación principal
│   ├── lib.rs                    # Librería compartida
│   └── agent.rs                  # Motor del agente
├── tests/
│   └── integration_tests.rs      # Tests (opcional)
├── bot_state.json               # Estado guardado (generado)
├── trades.csv                    # Log de trades (generado)
└── README.md                     # Este archivo
```

### 3. Verificar Instalación

```bash
cd polymarket-trading-agent
cargo --version
cargo build --release 2>&1 | head -20
```

---

## 🏗️ Compilación

### Compilación Debug (desarrollo)

```bash
# Rápido, pero no optimizado
cargo build

# Salida:
# target/debug/trading_agent
```

**Ventajas:**
- Compilación muy rápida (segundos)
- Mejor para debugging
- Símbolos de debug incluidos

**Desventajas:**
- Ejecutable más lento
- Uso mayor de memoria

### Compilación Release (producción)

```bash
# Más lento, pero optimizado al máximo
cargo build --release

# Salida:
# target/release/trading_agent
```

**Ventajas:**
- Ejecutable 10-50x más rápido
- Mejor optimización de código
- Menor footprint de memoria

**Desventajas:**
- Compilación más lenta (1-5 minutos)
- Símbolos de debug removidos

### Compilación Condicional

```bash
# Compilar solo librería (sin binario)
cargo build --lib

# Compilar con features específicas
cargo build --features "database"

# Compilar ejemplo específico
cargo build --example live_trading
```

---

## ▶️ Uso

### Ejecución Básica

```bash
# Ejecución debug
cargo run

# Ejecución release (recomendado)
cargo run --release

# Salida esperada:
# ╔═══════════════════════════════════════════════════════╗
# ║  POLYMARKET TRADING AGENT - RHAI SCRIPT ENGINE      ║
# ╚═══════════════════════════════════════════════════════╝
# 
# ✓ Agente creado
# ✓ Configuración actualizada
# ...
```

### Pasos de Ejecución

1. **Inicializa el agente** con path al script Rhai
2. **Carga la estrategia** desde `strategy.rhai`
3. **Compila** el script Rhai en el engine
4. **Procesa velas** según datos de entrada
5. **Genera señales** basadas en indicadores
6. **Gestiona posiciones** (entrada/salida)
7. **Registra trades** en log
8. **Exporta estadísticas** al final

### Argumentos de Línea de Comandos

```bash
# Ejecutar con script personalizado
cargo run --release -- --script mi_estrategia.rhai

# Configurar capital inicial
cargo run --release -- --capital 50000

# Activar modo verbose
cargo run --release -- --verbose

# Usar archivo de configuración
cargo run --release -- --config config.toml
```

---

## 🏛️ Arquitectura

### Diagrama de Flujo

```
┌─────────────────────────────────────────────────────────┐
│              POLYMARKET TRADING AGENT                   │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  ┌──────────────┐         ┌─────────────────────┐     │
│  │ Market Data  │────────>│   TradingAgent      │     │
│  │  (OHLCV)     │         │   (Rust)            │     │
│  └──────────────┘         ├─────────────────────┤     │
│                           │ - Load Script       │     │
│  ┌──────────────┐         │ - Manage State      │     │
│  │ Rhai Engine  │────────>│ - Execute Callbacks │     │
│  │ (Strategy)   │         │ - Log Events        │     │
│  └──────────────┘         └─────────────────────┘     │
│                                   │                    │
│                                   ▼                    │
│                          ┌──────────────────┐         │
│                          │   Signals        │         │
│                          │   (Bullish/Bear) │         │
│                          └──────────────────┘         │
│                                   │                    │
│                   ┌───────────────┴──────────────┐    │
│                   ▼                              ▼    │
│            ┌──────────────┐            ┌──────────────┐
│            │ Position     │            │  Trade Log   │
│            │ Management   │            │              │
│            └──────────────┘            └──────────────┘
│                   │                              │    │
│                   └───────────────┬──────────────┘    │
│                                   ▼                    │
│                          ┌──────────────────┐         │
│                          │   Statistics     │         │
│                          │   Win/Loss Rate  │         │
│                          │   P&L Percent    │         │
│                          └──────────────────┘         │
└─────────────────────────────────────────────────────────┘
```

### Componentes Principales

| Componente | Función | Ubicación |
|-----------|---------|-----------|
| **TradingAgent** | Orquestador principal | `agent.rs` |
| **Rhai Engine** | Intérprete de scripts | Rhai crate |
| **CandleData** | OHLCV normalizado | `lib.rs` |
| **BotState** | Estado persistente | `lib.rs` |
| **StrategyConfig** | Parámetros dinámicos | `lib.rs` |
| **TradingEvent** | Auditoría de eventos | `lib.rs` |

---

## 📚 Ejemplos

### Ejemplo 1: Backtest Simple

```rust
use trading_agent_lib::{CandleData, MarketConfig, TradingAgent};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Crear agente
    let market_config = MarketConfig {
        market_id: "BTC-5M".to_string(),
        market_name: "Bitcoin".to_string(),
        timeframe: "5m".to_string(),
        asset: "BTC".to_string(),
        initial_capital: 10000.0,
    };
    
    let mut agent = TradingAgent::new("strategy.rhai", market_config)?;
    
    // Crear datos de prueba
    let mut candles = CandleData::new();
    candles.add_candle(40000.0, 40100.0, 39900.0, 40050.0, 1000.0);
    // ... más velas ...
    
    // Procesar
    agent.on_candle(&candles, 10000.0)?;
    
    // Obtener stats
    let stats = agent.get_stats()?;
    println!("Win Rate: {:.2}%", stats.win_rate);
    
    Ok(())
}
```

### Ejemplo 2: Live Trading con Polymarket API

```rust
// Conectar a API de Polymarket
async fn fetch_market_data(market_id: &str) -> Result<CandleData, Box<dyn Error>> {
    let url = format!("https://api.polymarket.com/markets/{}/price-history", market_id);
    let resp = reqwest::get(&url).await?;
    let data = resp.json::<Vec<Candle>>().await?;
    
    let mut candles = CandleData::new();
    for candle in data {
        candles.add_candle(candle.open, candle.high, candle.low, candle.close, candle.volume);
    }
    
    Ok(candles)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut agent = TradingAgent::new("strategy.rhai", market_config)?;
    
    // Polling loop cada 5 segundos
    loop {
        let candles = fetch_market_data("BTC-5M").await?;
        agent.on_candle(&candles, 10000.0)?;
        
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
```

### Ejemplo 3: Múltiples Mercados

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let markets = vec!["BTC-5M", "ETH-5M", "SOL-5M"];
    let mut agents = Vec::new();
    
    for market_id in &markets {
        let config = MarketConfig {
            market_id: market_id.to_string(),
            // ... más config ...
        };
        
        let agent = TradingAgent::new("strategy.rhai", config)?;
        agents.push(agent);
    }
    
    // Procesar en paralelo con tokio::join_all
    loop {
        let tasks: Vec<_> = agents.iter_mut()
            .zip(&markets)
            .map(|(agent, market)| {
                let market = market.to_string();
                async move {
                    let candles = fetch_market_data(&market).await.ok()?;
                    agent.on_candle(&candles, 10000.0).ok()?;
                    Some(())
                }
            })
            .collect();
        
        futures::future::join_all(tasks).await;
        
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
```

### Ejemplo 4: Personalizar Configuración

```rust
let mut agent = TradingAgent::new("strategy.rhai", market_config)?;

// Cambiar parámetros en tiempo real
let custom_config = StrategyConfig {
    momentum_threshold: 1.0,      // Más restrictivo
    take_profit_percent: 5.0,     // Mayor TP
    max_loss_percent: 1.5,        // Menor SL
    required_confirmations: 4,     // Más confirmaciones
    ..Default::default()
};

agent.set_config(custom_config)?;

// Procesar con nueva configuración
agent.on_candle(&candles, 10000.0)?;
```

---

## ⚙️ Configuración

### Archivo de Configuración (config.toml)

```toml
[market]
market_id = "BTC-5M-POLYMARKET"
market_name = "Bitcoin 5-Minute"
timeframe = "5m"
asset = "BTC"
initial_capital = 10000.0

[strategy]
momentum_threshold = 0.8
rsi_threshold = 30.0
atr_multiplier = 1.5
max_position_size = 0.2
max_loss_percent = 2.0
take_profit_percent = 3.0
max_positions = 3
required_confirmations = 3
max_hold_bars = 5

[polymarket]
api_url = "https://api.polymarket.com"
api_key = "tu_api_key_aqui"
timeout_seconds = 10

[trading]
enable_live_trading = false
paper_trading = true
log_all_events = true
```

### Variables de Entorno

```bash
# .env file
POLYMARKET_API_KEY=sk_live_...
STRATEGY_PATH=./strategy.rhai
MARKET_ID=BTC-5M
INITIAL_CAPITAL=10000
LOG_LEVEL=INFO
```

---

## 🐛 Troubleshooting

### 1. Error: "cannot find file `strategy.rhai`"

**Problema:** El script no está en el directorio correcto.

**Solución:**
```bash
# Asegurar que strategy.rhai está en la raíz del proyecto
ls -la strategy.rhai

# Si falta, crear un link
ln -s ruta/a/strategy.rhai ./strategy.rhai
```

### 2. Compilación Lenta

**Problema:** Primera compilación toma mucho tiempo.

**Solución:**
```bash
# Usar caché de compilación
cargo build -j $(nproc)  # Linux/Mac
cargo build -j %NUMBER_OF_PROCESSORS%  # Windows

# O usar sccache
cargo install sccache
RUSTC_WRAPPER=sccache cargo build --release
```

### 3. Error de Rhai Script

**Problema:** "Error in Rhai script" cuando ejecuta `on_candle`

**Solución:**
```bash
# Verificar sintaxis del script
# Rhai es similar a JavaScript/Rust

# Comprobar que la función existe
grep "fn on_candle" strategy.rhai

# Validar tipos de datos
# En Rhai: closes es Array<f64>, no Vec<f64>
```

### 4. Out of Memory

**Problema:** Bot consume demasiada memoria.

**Solución:**
```bash
# Limitar tamaño de historiales
# En strategy.rhai, limpiar trades_log periódicamente

# Usar release build (más eficiente)
cargo run --release

# Monitorear memoria
/usr/bin/time -v cargo run --release
```

### 5. Performance Lento

**Problema:** Bot no puede procesar datos en tiempo real.

**Solución:**
```bash
# Profile el código
cargo flamegraph --release

# Aumentar workers async
TOKIO_WORKER_THREADS=8 cargo run --release

# Usar compilación optimizada
cargo build --release -C opt-level=3 -C lto=true
```

### 6. Datos Incorrectos

**Problema:** Señales no coinciden con backtest.

**Solución:**
```bash
# Validar formato de datos en CandleData
// En Rust
assert_eq!(candles.len(), expected_len);
println!("Close: {:?}", candles.closes);

# Verificar que los índices son correctos
# Rhai usa indexación 0-based como Rust
```

---

## 📊 Testing

### Tests Unitarios

```bash
# Ejecutar todos los tests
cargo test

# Ejecutar test específico
cargo test test_agent_creation

# Ejecutar con output
cargo test -- --nocapture

# Ejecutar tests de librería
cargo test --lib
```

### Tests de Integración

```bash
# Ejecutar tests de integración
cargo test --test '*'

# Crear archivo tests/integration_test.rs
# Colocar tests grandes ahí
```

### Benchmarking

```bash
# Instalar criterión para benchmarks
cargo install cargo-criterion

# Crear archivo benches/strategy_bench.rs
# Ejecutar benchmarks
cargo criterion
```

---

## 🚀 Despliegue a Producción

### Checklist Pre-Deployment

- [ ] Todos los tests pasan: `cargo test --release`
- [ ] Sin warnings: `cargo clippy --release`
- [ ] Formateo correcto: `cargo fmt`
- [ ] Documentación completa: `cargo doc --open`
- [ ] Backtest exitoso con datos reales
- [ ] Paper trading validado 500+ trades
- [ ] Configuración guardada en archivo
- [ ] Logs habilitados y monitoreados
- [ ] Alert system configurado
- [ ] Rollback procedure documentado

### Build para Producción

```bash
# Compilación optimizada final
cargo build --release

# Crear binario standalone
# El ejecutable está en:
# target/release/trading_agent

# Crear tarball para distribución
tar czf trading_agent-v0.1.0.tar.gz \
    target/release/trading_agent \
    strategy.rhai \
    config.toml \
    bot_state.json
```

### Ejecutar en Servidor

```bash
# En servidor Linux (ejemplo)
scp trading_agent-v0.1.0.tar.gz user@server:/opt/

ssh user@server
cd /opt
tar xzf trading_agent-v0.1.0.tar.gz

# Ejecutar con systemd
sudo nano /etc/systemd/system/trading-agent.service
```

---

## 📚 Recursos Adicionales

- [Rhai Documentation](https://rhai.rs/)
- [Tokio Guide](https://tokio.rs/)
- [Rust Book](https://doc.rust-lang.org/book/)
- [Cargo Manual](https://doc.rust-lang.org/cargo/)

---

## 📝 Licencia

MIT / Apache 2.0

---

## 🤝 Contribuciones

Las pull requests son bienvenidas. Para cambios principales, primero abre un issue.

---

## ⚠️ Disclaimer

Este software es para propósitos educativos. Trading de criptomonedas es altamente riesgoso. 
Usa con caución y nunca inviertas más de lo que puedas perder.
