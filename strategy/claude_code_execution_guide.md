# 🎬 Claude Code: Guía de Ejecución Paso-a-Paso

## Setup Visual: Cómo Lucirá Claude Code

```
┌─────────────────────────────────────────────────────────────────────┐
│  VS Code - trading-bot-claude-code                          [≡][-][✕]│
├─────────┬───────────────────────────────────────────────────────────┤
│  Files  │                                                             │
│  ≡ cli… │  Claude Code Chat Interface                               │
├─────────┤                                                             │
│ ┌─ src  │  📌 "Grid Trading Implementation"                         │
│ │ ┌─ st…│                                                             │
│ │ │ ├─ …│  You:                                                      │
│ │ │ └─ …│  [Pega el PROMPT 1 aquí]                                 │
│ │ └─ ex…│                                                             │
│ │ ├─ ba…│  Claude Code:                                             │
│ │ └─ li…│  ✓ Creando src/strategies/grid_trading.rs                │
│ ├─ scri…│  ✓ Implementando GridStrategy struct                     │
│ ├─ test…│  ✓ Agregando métodos: place_orders, on_fill, get_pnl    │
│ └─ .env │  ✓ Creando tests en tests/grid_trading_test.rs           │
│         │  ✓ Actualizando src/strategies/mod.rs                     │
│         │  ✓ Build successful! ✅                                    │
│         │                                                             │
│         │  [Mostrará el código generado]                            │
│         │                                                             │
│         │  Next: "Ahora crea la estrategia de Spread Trading..."   │
│         │                                                             │
└─────────┴───────────────────────────────────────────────────────────┘
```

---

## 🚀 Paso 1: Instalar Claude Code (5 min)

### 1.1 En VS Code

```
1. Abre VS Code (descargar de https://code.visualstudio.com)

2. Presiona Ctrl+Shift+X (abrir Extensions)

3. En la caja de búsqueda, escribe: "Claude Code"

4. Haz click en "Install" (extensión oficial de Anthropic)

5. Reinicia VS Code

6. En la sidebar izquierda, debería aparecer el ícono de Claude Code
```

### 1.2 Primera Vez: Setup

```
1. Click en el ícono de Claude Code

2. Te pedirá login:
   - Usa tu cuenta de Anthropic
   - Si no tienes, crea una en https://console.anthropic.com

3. Selecciona el modelo:
   - Elige "Claude Opus 4.6" (máximo poder, pero más lento)
   - O "Claude Sonnet 4.6" (más rápido, buen balance)

4. Click "Start Claude Code"

5. Te pedirá seleccionar carpeta del proyecto:
   - Abre trading-bot-claude-code/
   - Click "Select Folder"
```

---

## 📝 Paso 2: Crear Estructura Base

### 2.1 Preparar Directorios

En terminal:

```bash
# Navega a directorio
cd /home/tu_usuario/proyectos
mkdir trading-bot-claude-code
cd trading-bot-claude-code

# Crea estructura
mkdir -p {src,scripts,tests,data,results,notebooks}
mkdir -p src/strategies
mkdir -p src/exchange
mkdir -p src/backtester

# Archivos iniciales
touch Cargo.toml
touch src/lib.rs
touch src/main.rs
touch .env
touch README.md
```

### 2.2 Cargo.toml Inicial

En VS Code, abre `Cargo.toml` y pega:

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

[dev-dependencies]
tokio-test = "0.4"
```

### 2.3 src/lib.rs Inicial

```rust
pub mod strategies;
pub mod exchange;
pub mod backtester;

pub use strategies::*;
```

### 2.4 Compilar Base

```bash
cargo build
# Debería decir: Compiling trading-bot-rust v2.1.0 ... Finished
```

---

## 🎯 Paso 3: Ejecutar el PROMPT 1 (Grid Trading)

### 3.1 Abrir Claude Code Chat

```
1. Haz click en el ícono de Claude Code (sidebar izquierda)

2. Verás un panel que dice "Chat with Claude Code"

3. Debajo hay una caja de texto: "Ask Claude Code anything..."

4. Click en esa caja
```

### 3.2 Copiar-Pegar PROMPT 1

```
En Claude Code, COPIA EXACTAMENTE ESTO Y PÉGALO EN LA CAJA:

────────────────────────────────────────────────────────────────

Necesito implementar un Grid Trading Bot para Binance en Rust.

Requisitos:
1. Crear archivo: src/strategies/grid_trading.rs
2. Función principal: fn setup_grid_trading(symbol: &str, lower_bound: f64, upper_bound: f64, grid_lines: usize) -> GridStrategy
3. Debe soportar:
   - Colocar órdenes limit buy en múltiples niveles
   - Cuando una orden buy se ejecuta, coloca automáticamente la correspondiente sell
   - Rastrear P&L por cada grid
   - Configuración dinámica de profit margin (default 0.5%)

4. Parámetros:
   - symbol: "BTCUSDT"
   - lower_bound: 41000.0
   - upper_bound: 43000.0
   - grid_lines: 10
   - profit_margin: 0.5%

5. Output: struct GridStrategy con métodos:
   - place_orders() -> Result<Vec<Order>>
   - on_fill(order_id) -> Result<Order>
   - get_pnl() -> f64
   - cancel_all() -> Result<()>

Por favor genera el código completo con:
- Manejo de errores robusto
- Comentarios explicativos
- Ejemplo de uso al final

────────────────────────────────────────────────────────────────

3. Presiona ENTER o click "Send"

4. Claude Code comenzará a generar:
   - Verás: "Analyzing your request..."
   - Luego: "Generating code..."
   - Finalmente: El código completo se mostrará
```

### 3.3 Qué Hará Claude Code

```
✓ Crear src/strategies/grid_trading.rs con:
  - struct GridStrategy { ... }
  - struct Order { ... }
  - impl GridStrategy {
      fn place_orders() -> Result<...>
      fn on_fill(...) -> Result<...>
      fn get_pnl() -> f64
      fn cancel_all() -> Result<...>
    }
  - Ejemplo de uso comentado

✓ Actualizar src/strategies/mod.rs:
  - pub mod grid_trading;
  - pub use grid_trading::*;

✓ Crear tests/grid_trading_test.rs:
  - Tests básicos de funcionalidad
  - Tests de P&L calculation
```

### 3.4 Verificar Compilación

```bash
cargo build
# Si sale "error", pide a Claude Code:
# "Hay error de compilación en GridStrategy línea X: [copia el error]
#  Corrígelo"

# Si dice "Finished", ¡éxito! ✅
```

---

## 🔄 Paso 4: Ejecutar PROMPTS 2-8 (Secuencialmente)

### 4.1 Pattern para Cada Prompt

```
SIEMPRE SIGUE ESTOS PASOS:

1. COPIA el prompt de la sección correspondiente 
   (PROMPT 2: Spread Trading, PROMPT 3: Event-Driven, etc.)

2. PÉGALO EN Claude Code chat

3. Presiona ENTER

4. Claude Code generará archivos nuevos

5. Después de que termine, ejecuta:
   cargo build
   
   Si hay errores, pide: "Corrige el error: [error message]"

6. Cuando compile, ejecuta:
   cargo test
   
   Para verificar que los tests pasan
```

### 4.2 Orden Recomendado

```
DÍA 1:
  ✓ PROMPT 1: Grid Trading
  cargo build && cargo test
  
  ✓ PROMPT 2: Spread Trading  
  cargo build && cargo test
  
DÍA 2:
  ✓ PROMPT 3: Event-Driven
  ✓ PROMPT 4: Mean Reversion
  cargo build && cargo test
  
DÍA 3:
  ✓ PROMPT 5: DCA Bot
  ✓ PROMPT 6: Correlation Arbitrage
  cargo build && cargo test

DÍA 4:
  ✓ PROMPT 7: Liquidation Hunt
  ✓ PROMPT 8: Pump & Dump
  cargo build && cargo test --all
```

### 4.3 Si Algo Falla

```
Ejemplo: Después de PROMPT 2, ves error:

error: cannot find type `Order` in this scope

SOLUCIÓN:

En Claude Code, pide:

"@src/strategies/spread_trading.rs necesita importar Order de grid_trading.
 Agrega: use crate::strategies::grid_trading::Order;
 al inicio del archivo"

Claude Code hará el arreglo automáticamente.
```

---

## 🏗️ Paso 5: Integración (PROMPT 9)

Después de las 8 estrategias, ejecuta:

### 5.1 PROMPT 9: Marco Base Unificado

```
En Claude Code, pega:

────────────────────────────────────────────────────────────────

Necesito integrar todas las 8 estrategias en un marco unificado.

1. Actualizar src/lib.rs:
   - Importar todos los módulos de estrategias
   - Crear enum StrategyType { GridTrading, SpreadTrading, EventDriven, ... }
   - Crear trait Strategy { execute(), monitor(), exit() }

2. Crear src/main.rs básico:
   - fn main() que permita seleccionar estrategia via CLI
   - Ejemplo: println!("Selecciona estrategia...")

3. Crear src/backtester/engine.rs:
   - struct Backtester { ... }
   - impl Backtester {
       fn new() -> Self
       fn add_candle() -> Result<()>
       fn run() -> Result<BacktestResult>
     }

────────────────────────────────────────────────────────────────

Después:
cargo build
cargo test
```

---

## 🧪 Paso 6: Testing Completo (PROMPT 10)

### 6.1 PROMPT 10: Suite de Tests

```
En Claude Code:

────────────────────────────────────────────────────────────────

Crea una suite de tests exhaustiva en tests/comprehensive_tests.rs:

1. Tests unitarios para cada estrategia:
   #[test]
   fn test_grid_trading_calculation() { }
   
   #[test]
   fn test_spread_detection() { }

2. Tests de integración:
   #[test]
   fn test_multiple_strategies_parallel() { }

3. Fixtures con datos históricos

4. Helpers para assertions

Genera:
- Mínimo 50 tests totales
- Coverage > 80% del código
- Tests que deben pasar antes de live

────────────────────────────────────────────────────────────────

Después:
cargo test --all
# Deberías ver: test result: ok. XX passed
```

---

## 🔌 Paso 7: APIs Reales (PROMPT 11)

Cuando quieras conectar a Binance y Polymarket:

### 7.1 Crear src/exchange/binance.rs

```
En Claude Code:

────────────────────────────────────────────────────────────────

Integra cliente de Binance REST API en Rust.

Requisitos:
1. Crear struct BinanceClient { api_key, api_secret }

2. Métodos:
   - async fn get_price(symbol: &str) -> Result<f64>
   - async fn get_candles(symbol, interval, limit) -> Result<Vec<Candle>>
   - async fn place_order(...) -> Result<OrderId>
   - async fn get_position(...) -> Result<Position>

3. Error handling:
   - Retry en network errors
   - Rate limiting respect
   - Proper logging

4. Config:
   - Usar testnet primero
   - Variables de entorno para credenciales

────────────────────────────────────────────────────────────────

Después de generar, prueba:

cargo build

(Si hay errores de dependencias, Claude Code te dirá qué agregar a Cargo.toml)
```

---

## 📊 Paso 8: Backtesting y Análisis (PROMPT 12)

### 8.1 PROMPT 12: Backtesting Engine

```
En Claude Code:

────────────────────────────────────────────────────────────────

Crea sistema de backtesting con métricas.

Incluir:
1. Cargar datos históricos desde CSV
2. Ejecutar cada estrategia
3. Calcular métricas: win rate, sharpe, max DD, profit factor
4. Generar reportes CSV

Funciones:
- fn load_csv_candles(path: &str) -> Result<Vec<Candle>>
- fn run_backtest(strategy, candles) -> Result<BacktestResult>
- fn calculate_metrics(trades) -> Metrics
- fn save_results(results, output_path) -> Result<()>

────────────────────────────────────────────────────────────────

Después:
cargo build
cargo test
```

---

## 🎮 Paso 9: Ejecutar Todo Junto

### 9.1 Backtesting Completo

```bash
# Terminal:

# Compilar en release (más rápido)
cargo build --release

# Ejecutar backtests
cargo run --release -- --backtest true --days 90

# Esperar resultados (~5-30 min dependiendo de datos)

# Debería imprimir algo como:
# ================== RESULTS ===================
# Strategy: GridTrading
# Win Rate: 73.5%
# Sharpe Ratio: 1.23
# Max Drawdown: 8.5%
# Total Return: +12.3%
# ================================================
```

### 9.2 Si Quieres Optimizar Parámetros

```
En Claude Code:

"@src/ crea función:
 fn optimize_grid_parameters(symbol) -> OptimalParams { }
 
 Que prueba 100 combinaciones de parámetros
 y retorna los mejores para ese símbolo"

Claude Code creará la función de optimización.

Luego:
cargo run --release -- --optimize grid --symbol BTCUSDT
```

---

## 🌐 Paso 10: Setup Paper Trading (Opcional)

Si quieres paper trading sin dinero real:

### 10.1 Crear Mock Client

```
En Claude Code:

────────────────────────────────────────────────────────────────

Crea un MockBinanceClient para paper trading.

Estructura:
- struct MockBinanceClient { balance, positions }
- Simula órdenes sin conectar a exchange real
- Simula precios usando históricos
- Tracks P&L ficticio
- Idéntica interfaz a BinanceClient real

Métodos:
- fn new(initial_balance) -> Self
- async fn place_order(...) -> Result<OrderId>
- async fn get_position(...) -> Result<Position>
- fn get_pnl(&self) -> f64

────────────────────────────────────────────────────────────────

Después:
cargo build
cargo test mock_binance
```

### 10.2 Ejecutar Paper Trading

```bash
cargo run --release -- --paper-trading true --days 14
```

---

## ⚙️ Troubleshooting Común

### Problema 1: "Error: cannot find type X"

```
CAUSA: Módulo no importado

SOLUCIÓN: 
En Claude Code:
"El tipo GridStrategy no se ve en spread_trading.rs
 Agrega importación al inicio:
 use crate::strategies::GridStrategy;"
```

### Problema 2: "Error: futures trait not implemented"

```
CAUSA: Async mismatch

SOLUCIÓN:
En Claude Code:
"Necesito que la función X sea async.
 Cambia fn a async fn y retorna Result<> en lugar de directo"
```

### Problema 3: "Build error: conflicting versions"

```
CAUSA: Dependencias con versiones incompatibles

SOLUCIÓN:
En Claude Code:
"Hay conflicto de versiones. Actualiza Cargo.toml
 a las versiones compatibles más recientes"

O ejecuta:
cargo update
cargo build
```

### Problema 4: "Test failed: assertion"

```
CAUSA: Test lógicamente incorrecto

SOLUCIÓN:
En Claude Code:
"El test test_grid_pnl falló porque esperaba X pero obtuvo Y.
 @tests/ revisa la lógica y corrígela"
```

---

## 📊 Ejecutar Comandos Útiles

### Build & Test

```bash
# Build básico
cargo build

# Build optimizado
cargo build --release

# Tests rápido
cargo test --lib

# Tests completo
cargo test --all

# Tests con output
cargo test -- --nocapture

# Tests de estrategia específica
cargo test grid_trading
```

### Running

```bash
# Ejecutar programa
cargo run

# Con argumentos
cargo run -- --strategy grid --days 90

# Optimizado
cargo run --release -- --backtest true
```

### Code Quality

```bash
# Format código
cargo fmt

# Lint (busca problemas)
cargo clippy

# Documentation
cargo doc --open
```

---

## 🎓 Ejemplo Completo: Primeros 30 Minutos

### Timeline:

```
00:00 - 05:00: Setup
  • Abre VS Code
  • Instala Claude Code
  • crea estructura de carpetas
  
05:00 - 10:00: Primer Prompt
  • Copia PROMPT 1 (Grid Trading)
  • Pégalo en Claude Code
  • Espera generación
  
10:00 - 15:00: Verificación
  • cargo build (debería pasar)
  • cargo test (debería pasar ✅)
  • Revisa archivos generados
  
15:00 - 25:00: Segundo Prompt
  • Copia PROMPT 2 (Spread Trading)
  • Pégalo en Claude Code
  • cargo build && cargo test
  
25:00 - 30:00: Commit
  • git add .
  • git commit -m "Grid + Spread Trading"
  • git push

¡Listo en 30 minutos! 🚀
```

---

## 🚨 Reglas de Oro

```
1. SIEMPRE ejecuta cargo build después de cada prompt
   → Asegura que compila

2. SIEMPRE ejecuta cargo test después de cada prompt
   → Asegura que no rompes código previo

3. SIEMPRE copia prompts EXACTAMENTE como están
   → Cambios menores pueden causar resultados muy diferentes

4. SI hay error, NO intentes arreglarlo tú
   → Pide a Claude Code que lo haga

5. GUARDA progreso: git commit después de cada prompt
   → Poder volver atrás si algo sale mal

6. USA @references en tus nuevos prompts
   → Ej: "@src/strategies/ necesito..."
   → Le da contexto a Claude Code
```

---

## 📞 Ayuda Rápida

```
Si Claude Code no entiende:

1. Sé más específico:
   MALO: "Crea un grid bot"
   BUENO: "Crea struct GridStrategy con método place_orders()
           que coloca 10 órdenes limit buy entre 41000 y 43000"

2. Dale contexto:
   "@src/strategies/grid_trading.rs necesito agregar método X"

3. Copia ejemplos:
   "Crea función similar a esta pero para spread trading:
    [copia código de grid_trading.rs]"

4. Pide tests directamente:
   "Crea tests para GridStrategy que verifican:
    - P&L calculation
    - Order placement
    - P&L tracking"
```

---

**Consejos Finales:**

✅ **Empieza simple**: Grid Trading primero (más fácil)
✅ **Construye gradualmente**: Una estrategia por día
✅ **Testa todo**: cargo test después de cada prompt
✅ **Documenta**: Agrega comments mientras avanzas
✅ **Celebra**: Cada prompt exitoso = 1 estrategia completa! 🎉

---

**Siguientes pasos después de leer esto:**

1. Abre VS Code
2. Instala Claude Code
3. Crea estructura de carpetas
4. Copia PROMPT 1 en Claude Code
5. ¡Espera que genere tu primer bot! ✨
