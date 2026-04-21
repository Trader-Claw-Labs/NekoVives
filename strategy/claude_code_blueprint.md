# 🤖 Claude Code Blueprint: Trading Bot Automático (8 Estrategias)

## 📋 Descripción General

Este documento es una **plantilla completa** para usar **Claude Code** (la extensión que integra IA directamente en tu IDE) para **generar, testear y optimizar** un bot de trading automatizado que soporte:

1. ✅ Polymarket Spread Trading (Arbitrage)
2. ✅ Event-Driven Trading (Análisis de noticias)
3. ✅ Grid Trading (Rango-bound)
4. ✅ Volatility Mean Reversion
5. ✅ DCA Bot (Dollar-Cost Averaging)
6. ✅ Multi-Event Arbitrage
7. ✅ Liquidation Hunting
8. ✅ Pump & Dump Detection

---

## 🚀 Parte 1: Setup de Claude Code

### 1.1 Instalación

```bash
# Para VS Code
# 1. Abre VS Code
# 2. Ve a Extensions (Ctrl+Shift+X)
# 3. Busca "Claude Code"
# 4. Instala la extensión oficial de Anthropic

# Verificar instalación
code --version  # VS Code
# Extension aparecerá en la sidebar izquierda

# Setup Inicial
# 1. Click en el ícono de Claude Code
# 2. Inicia sesión con tu cuenta de Anthropic
# 3. Selecciona tu carpeta de proyecto
```

### 1.2 Configuración Inicial

```bash
# Crear directorio base
mkdir trading-bot-claude-code
cd trading-bot-claude-code

# Crear estructura inicial
mkdir -p {src,scripts,tests,data,results,notebooks}
touch {Cargo.toml,pyproject.toml,.env,.gitignore,README.md}

# Estructura final que Claude Code llenará:
trading-bot-claude-code/
├── src/
│   ├── strategies/
│   │   ├── mod.rs
│   │   ├── spread_trading.rs        ← Claude Code genera
│   │   ├── event_driven.rs          ← Claude Code genera
│   │   ├── grid_trading.rs          ← Claude Code genera
│   │   ├── mean_reversion.rs        ← Claude Code genera
│   │   ├── dca_bot.rs               ← Claude Code genera
│   │   ├── correlation_arb.rs       ← Claude Code genera
│   │   ├── liquidation_hunt.rs      ← Claude Code genera
│   │   └── pump_and_dump.rs         ← Claude Code genera
│   ├── exchange/
│   │   ├── mod.rs
│   │   ├── binance.rs               ← Claude Code integra
│   │   └── polymarket.rs            ← Claude Code integra
│   ├── backtester/
│   │   ├── mod.rs
│   │   ├── engine.rs                ← Claude Code genera
│   │   └── metrics.rs               ← Claude Code genera
│   ├── lib.rs
│   └── main.rs                       ← Claude Code actualiza
├── scripts/
│   ├── spread_trading.rhai           ← Claude Code genera
│   ├── event_driven.rhai             ← Claude Code genera
│   ├── grid_trading.rhai             ← Claude Code genera
│   └── ... (8 scripts en total)
├── tests/
│   ├── backtest_tests.rs             ← Claude Code genera
│   ├── integration_tests.rs          ← Claude Code genera
│   └── strategy_tests.rs             ← Claude Code genera
├── notebooks/
│   └── analysis.ipynb                ← Claude Code genera
├── Cargo.toml                        ← Claude Code actualiza
├── pyproject.toml
└── .env
```

---

## 🎯 Parte 2: Workflow con Claude Code

### 2.1 Abrir Proyecto en Claude Code

```
1. VS Code → File → Open Folder → Selecciona trading-bot-claude-code/
2. Click en ícono Claude Code (sidebar izquierda)
3. Click "Start Claude Code" 
4. Elige un modelo (usa claude-opus-4-6 para máxima potencia)
5. Claude Code abre un panel interactivo
```

### 2.2 Prompts Efectivos para Claude Code

Claude Code funciona mejor con **prompts específicos y paso-a-paso**. Aquí están los prompts que debes usar:

---

## 📝 Parte 3: Prompts para Generar Cada Estrategia

### PROMPT 1: Grid Trading (EMPIEZA AQUÍ - Más Fácil)

```
[COPIAR EXACTAMENTE ESTO EN CLAUDE CODE]

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
```

**Qué Claude Code hará:**
- ✅ Genera `src/strategies/grid_trading.rs` completo
- ✅ Crea tests automáticos
- ✅ Actualiza `src/strategies/mod.rs`
- ✅ Genera ejemplos en documentación

---

### PROMPT 2: Spread Trading (Polymarket Arbitrage)

```
[COPIAR EXACTAMENTE ESTO]

Necesito un Spread Trading Bot para Polymarket en Rust.

Objetivo: Detectar y ejecutar arbitrage entre mercados binarios relacionados.

Requisitos:
1. Crear archivo: src/strategies/spread_trading.rs
2. Función: fn detect_spread_opportunity(markets: Vec<MarketPrice>) -> Option<SpreadOpportunity>
3. Debe:
   - Monitorear pares de mercados: BTC_UP vs BTC_DOWN, ETH_UP vs ETH_DOWN
   - Detectar cuando (BTC_UP_price + BTC_DOWN_price) != 100% (+ fees)
   - Calcular profit potencial después de fees (assume 1.5% round-trip)
   - Solo ejecutar si profit > 0.5%

4. Estructura:
   - MarketPrice: { market_id, price, liquidity }
   - SpreadOpportunity: { buy_market, sell_market, spread, profit_pct }

5. Lógica:
   - Si spread > threshold, compra mercado barato
   - Vende mercado caro simultáneamente
   - Cierra posición cuando converge

6. Output methods:
   - detect_spread_opportunity(markets) -> Option<SpreadOpportunity>
   - execute_spread(opp: SpreadOpportunity) -> Result<Trade>
   - monitor_convergence() -> Result<PnL>

Genera código con:
- Market price caching
- Fee calculation accuracy
- Error handling
- Logging detallado
```

**Qué Claude Code hará:**
- ✅ Implementa la lógica de arbitrage
- ✅ Crea helpers para fees y conversiones
- ✅ Genera integration tests con Polymarket API mock
- ✅ Documentación de estrategia incluida

---

### PROMPT 3: Event-Driven Trading

```
[COPIAR EXACTAMENTE ESTO]

Implementa un Event-Driven Trading Bot que:

1. Crea archivo: src/strategies/event_driven.rs

2. Funcionalidad:
   - Monitorea un calendario de eventos económicos (FOMC, CPI, etc.)
   - 30 min antes del evento, verifica si hay setup técnico
   - Post-evento, captura momentum en 5-15 min
   - Cierra posición automáticamente después de ventana

3. Parámetros:
   - pre_event_minutes: 30
   - post_event_window: 15
   - min_momentum_threshold: 0.8%
   - exit_profit_target: 2.5%
   - exit_loss_limit: 1.5%

4. Estructura de datos:
   struct EconomicEvent {
       name: String,
       time: DateTime<Utc>,
       importance: u8,  // 1-3 (impact level)
       market_affected: Vec<String>,  // ["BTC", "ETH", "USD"]
   }

5. Logic:
   - Cargar eventos desde calendar API
   - Pre-event: si momentum bullish, compra BTC_UP
   - Post-event: si volatilidad > 1.5x histórico, aumenta posición
   - Exit automático: 15 min después del evento o profit target

6. Métodos principales:
   - fetch_upcoming_events() -> Vec<EconomicEvent>
   - analyze_pre_event_setup() -> Option<Signal>
   - execute_on_event() -> Result<Position>
   - monitor_and_exit() -> Result<PnL>

Incluye:
- DateTime handling
- Timezone conversions (UTC)
- Calendar data fetching
- Logging de eventos
```

**Qué Claude Code hará:**
- ✅ Integra con API de calendario económico
- ✅ Implementa state machine (pre/post/closed)
- ✅ Genera helpers de timing
- ✅ Crea tests con eventos simulados

---

### PROMPT 4: Mean Reversion Trading

```
[COPIAR EXACTAMENTE ESTO]

Crea un Mean Reversion Bot para volatilidad extrema.

1. Archivo: src/strategies/mean_reversion.rs

2. Concepto:
   - Si Bitcoin sube >4% en 5 min → Esto es extremo → Vende (espera reversal)
   - Si Bitcoin cae >4% en 5 min → Esto es extremo → Compra (espera bounce)

3. Parámetros:
   - lookback_minutes: 5
   - volatility_threshold_pct: 4.0
   - mean_reversion_window: 10  // espera 10 min para revertir
   - take_profit: 1.5%
   - stop_loss: 2.0%

4. Indicadores:
   - 5-min price change %
   - 1-hour historical volatility (std dev)
   - Current volatility vs 1-hour average

5. Signal Logic:
   if current_change > (1-hour_volatility * 1.5) {
       signal = MEAN_REVERSION
   }

6. Métodos:
   - detect_volatility_spike() -> Option<Direction>  // UP or DOWN
   - calculate_reversion_target() -> f64
   - execute_counter_trade() -> Result<Position>
   - monitor_for_reversion() -> Result<PnL>

7. Code features:
   - Volatility calculation (rolling std dev)
   - Counter-trend logic with risk management
   - Exit on time, profit, or loss
   - Detailed logging
```

**Qué Claude Code hará:**
- ✅ Implementa cálculo de volatilidad
- ✅ Genera lógica de detección de picos
- ✅ Crea tests de comportamiento
- ✅ Documenta casos de uso

---

### PROMPT 5: DCA Bot (Dollar-Cost Averaging)

```
[COPIAR EXACTAMENTE ESTO]

Implementa DCA Bot automático para Binance.

1. Archivo: src/strategies/dca_bot.rs

2. Funcionalidad:
   - Cada día a hora especificada, compra cantidad fija de BTC/ETH
   - Calcula costo promedio acumulado
   - Cuando precio >= promedio * 1.20, vende todo
   - Reinicia el ciclo

3. Parámetros:
   - daily_amount_usd: 100.0
   - execution_time: "06:00 UTC"
   - target_profit: 1.20  // 20% ganancia
   - symbols: ["BTCUSDT", "ETHUSDT"]

4. Estructura:
   struct DCAPosition {
       symbol: String,
       total_cost_usd: f64,
       total_amount: f64,
       avg_price: f64,
       entry_time: DateTime<Utc>,
       purchases: Vec<Purchase>,  // histórico de compras
   }

   struct Purchase {
       price: f64,
       amount: f64,
       timestamp: DateTime<Utc>,
   }

5. Métodos:
   - buy_daily() -> Result<Purchase>
   - update_average_cost() -> f64
   - check_sell_target() -> Option<SellSignal>
   - sell_all() -> Result<TradeResult>
   - get_statistics() -> DCAStats

6. DCAStats:
   - avg_price
   - current_price
   - current_value
   - unrealized_pnl_pct
   - total_purchases
   - holding_days

Características:
- Cronograma automático
- Persistencia en base de datos
- Reporte diario de estado
- Historial completo de transacciones
```

**Qué Claude Code hará:**
- ✅ Implementa scheduler automático
- ✅ Crea persistencia en SQLite
- ✅ Genera reportes diarios
- ✅ Tests de cálculo de promedio

---

### PROMPT 6: Multi-Event Arbitrage (Crypto + Polymarket)

```
[COPIAR EXACTAMENTE ESTO]

Crea un bot de arbitrage que correlaciona Crypto y Polymarket.

1. Archivo: src/strategies/correlation_arb.rs

2. Concepto:
   - Si Fed decision → Crypto reacciona
   - Si TRUMP_WIN sube en Polymarket → DOGE podría subir
   - Detectar divergencias y explotar

3. Parámetros:
   - correlation_threshold: 0.75  // mínima correlación esperada
   - divergence_threshold: 5.0    // % de desviación antes de arbitrage
   - monitoring_interval: 60      // segundos

4. Pares monitoreados:
   [
       {crypto: "BTCUSDT", polymarket: "BTC_UP", correlation: 0.92},
       {crypto: "ETHUSD", polymarket: "ETH_UP", correlation: 0.88},
       {crypto: "DOGEUSDT", polymarket: "TRUMP_WIN", correlation: 0.65},
   ]

5. Lógica:
   - Calcula precio "fair" en Polymarket basado en BTC real
   - Si Polymarket precio diverge > 5%, es arbitrage
   - Compra barato, vende caro
   - Espera convergencia

6. Métodos:
   - calculate_fair_price(crypto_price, correlation) -> f64
   - detect_divergence(fair: f64, market: f64) -> Option<Divergence>
   - execute_arb(div: Divergence) -> Result<Position>
   - monitor_correlation() -> Result<CorrelationMetrics>

7. Funcionalidades:
   - Histórico de correlaciones
   - Alertas cuando correlación cambia
   - Rebalance automático
   - P&L tracking por par
```

**Qué Claude Code hará:**
- ✅ Implementa correlation calculations
- ✅ Crea lógica de fair pricing
- ✅ Genera helpers de divergencia
- ✅ Tests con datos históricos

---

### PROMPT 7: Liquidation Hunting (Futures)

```
[COPIAR EXACTAMENTE ESTO]

Implementa bot para cazar liquidaciones en Binance Futures.

1. Archivo: src/strategies/liquidation_hunt.rs

2. Concepto:
   - Monitorea posiciones apalancadas de otros traders
   - Cuando hay muchos largos en X precio, si cae → liquidaciones masivas
   - Tu bot compra ANTES de las liquidaciones (sabiendo que revierte)
   - Vende cuando estabiliza

3. Parámetros:
   - min_liquidation_volume: 100000.0  // USD mínimo en liquidaciones
   - leverage_threshold: 10.0           // posiciones > 10x son target
   - detection_window: 5               // min antes de liquidación
   - position_hold_time: 5             // min después de entrar

4. Data Structure:
   struct LiquidationEvent {
       symbol: String,
       liquidated_positions: usize,
       total_volume_usd: f64,
       liquidation_price: f64,
       timestamp: DateTime<Utc>,
   }

5. Métodos:
   - fetch_open_positions() -> Vec<OpenPosition>
   - identify_liquidation_zones() -> Vec<LiquidationZone>
   - predict_cascade() -> Option<LiquidationPrediction>
   - execute_pre_liq_entry() -> Result<Position>
   - exit_on_stabilization() -> Result<TradeResult>

6. Detección:
   - Monitorea 1-5 min precio candlesticks
   - Calcula volatilidad
   - Identifica si hay volumen anómalo (liquidaciones)
   - Score de confianza de liquidación

Características:
- WebSocket para datos en tiempo real
- Predicción probabilística de cascadas
- Escaleo gradual de posición
- Risk management robusto
- Logging detallado de liquidaciones
```

**Qué Claude Code hará:**
- ✅ Implementa WebSocket listener
- ✅ Crea lógica de predicción de cascadas
- ✅ Genera alertas de liquidación
- ✅ Tests con datos de liquidaciones reales

---

### PROMPT 8: Pump & Dump Detection

```
[COPIAR EXACTAMENTE ESTO]

Crea detector de pump and dump patterns.

1. Archivo: src/strategies/pump_and_dump.rs

2. Concepto:
   - Detecta cuando un mercado sube anormalmente rápido (+30% en 10 min)
   - Entra ASAP en el pump
   - Vende rápido antes que colapse
   - ⚠️ Requiere timing perfecto

3. Parámetros:
   - pump_threshold_pct: 30.0       // min % de aumento rápido
   - detection_window_min: 10       // ventana para detectar pump
   - hold_time_min: 5               // tiempo máximo dentro
   - profit_target: 5.0             // tomar ganancias aquí
   - loss_limit: 3.0                // stop loss aquí

4. Pattern Detection:
   struct PumpPattern {
       price_increase_pct: f64,
       volume_increase_pct: f64,
       time_elapsed_min: usize,
       momentum_score: f64,  // 0.0-1.0
   }

5. Métodos:
   - detect_pump() -> Option<PumpPattern>
   - score_pump_confidence() -> f64  // 0.0-1.0
   - execute_pump_entry() -> Result<Position>
   - quick_exit() -> Result<PnL>
   - calculate_dump_probability() -> f64

6. Lógica:
   - Monitor todos los mercados cada 1 min
   - Si detecta pump, verifica confirmación
   - Entra si confianza > 0.7
   - Vende automáticamente en:
     * Profit target alcanzado
     * Tiempo máximo expirado
     * Dump iniciado (reversal > 2%)

7. Features:
   - Multi-market scanning
   - Confidence scoring
   - Quick entry execution
   - Dump signal detection
   - P&L tracking
```

**Qué Claude Code hará:**
- ✅ Implementa pattern recognition
- ✅ Crea scoring system
- ✅ Genera alerts en tiempo real
- ✅ Tests con pump históricos

---

## 🔧 Parte 4: Prompts para Estructura y Testing

### PROMPT 9: Crear Marco Base + Integración

```
[DESPUÉS DE GENERAR LAS 8 ESTRATEGIAS]

Necesito integrar todas las 8 estrategias en un marco unificado.

1. Actualizar src/lib.rs:
   - Importar todos los módulos de estrategias
   - Crear enum StrategyType { GridTrading, SpreadTrading, EventDriven, ... }
   - Crear trait Strategy { execute(), monitor(), exit() }

2. Crear src/main.rs:
   - CLI que permita seleccionar estrategia
   - Ejemplo: cargo run -- --strategy grid --symbol BTCUSDT --backtest true
   - Soportar múltiples estrategias simultáneamente

3. Crear src/backtester/engine.rs:
   - Motor de backtesting genérico para cualquier estrategia
   - Cargar datos históricos (CSV o API)
   - Ejecutar estrategia on_candle
   - Calcular métricas: win rate, sharpe, max DD

4. Crear tests/:
   - Unit tests para cada estrategia
   - Integration tests (multiples estrategias)
   - Backtest tests (vs datos históricos)
   - Mock tests (sin conectar a APIs reales)

5. Actualizar Cargo.toml:
   - Agregar todas las dependencias necesarias
   - Features: [backtest, live-trading, polymarket, binance]
   - Dev-dependencies para testing

Genera:
- Arquitectura modular y escalable
- Manejo de errores consistente
- Logging a través de todo
- Documentación inline
```

**Qué Claude Code hará:**
- ✅ Crea trait unificado
- ✅ Implementa enum dispatcher
- ✅ Genera esqueleto de main.rs
- ✅ Actualiza Cargo.toml automáticamente

---

### PROMPT 10: Crear Suite de Tests Completa

```
[PARA VALIDAR TODO EL BOT]

Crea una suite de tests exhaustiva.

1. tests/unit_tests.rs:
   - Test cada estrategia en aislamiento
   - Verificar: entry conditions, exit conditions, P&L calculations
   - Mock data para cada una

2. tests/integration_tests.rs:
   - Test múltiples estrategias corriendo en paralelo
   - Verificar que no hay conflictos
   - Verificar que cada una mantiene su estado

3. tests/backtest_tests.rs:
   - Cargar 3 meses de datos históricos (BTC, ETH)
   - Ejecutar cada estrategia
   - Verificar que métricas son razonables:
     * Grid Trading: win rate > 70%
     * Mean Reversion: win rate > 55%
     * DCA: positive return si market sube
     * Event-Driven: win rate > 60%

4. tests/stress_tests.rs:
   - Mercado cayendo 50%: ¿sobrevive?
   - Mercado subiendo 50%: ¿cómo performs?
   - Gap abiertos: ¿maneja posiciones?
   - Mensajes de API atrasados: ¿timeout manejo?

5. Estructura:
   #[test]
   fn test_grid_trading_basic() { }
   
   #[test]
   fn test_spread_detection() { }
   
   #[tokio::test]
   async fn test_live_websocket() { }

6. Métricas esperadas en tests:
   - Assert win_rate > minimum_expected
   - Assert sharpe_ratio > threshold
   - Assert max_drawdown < limit
   - Assert execution_time < timeout

Genera:
- Fixtures con datos históricos
- Mock servers para APIs
- Helpers para assertions
- Benchmarking code
```

**Qué Claude Code hará:**
- ✅ Genera todos los tests boilerplate
- ✅ Crea fixtures de datos
- ✅ Implementa test helpers
- ✅ Setup de continuous testing

---

### PROMPT 11: Integración con APIs Reales

```
[CUANDO ESTÉS LISTO PARA LIVE]

Integra conexión real a Binance y Polymarket APIs.

1. Crear src/exchange/binance.rs:
   - REST API client (órdenes, históricos, account)
   - WebSocket client (precio en vivo)
   - Error handling y retries
   - Rate limiting

2. Crear src/exchange/polymarket.rs:
   - REST API client
   - Autenticación (private key)
   - Order placement
   - Position tracking

3. Crear src/config.rs:
   - Cargar API keys desde environment variables
   - Soportar testnet y mainnet
   - Manage credentials seguramente

4. Métodos requeridos:
   - get_price(symbol) -> f64
   - get_candles(symbol, interval, limit) -> Vec<Candle>
   - place_order(symbol, side, quantity, price) -> OrderId
   - get_position(symbol) -> Position
   - get_account_balance() -> Balance

5. Error Handling:
   - Network errors (retry con exponential backoff)
   - Invalid orders (validation antes)
   - Rate limits (respetar límites API)
   - Partial fills (tracking correcto)

6. Logging:
   - Todas las órdenes logeadas
   - Errores con contexto
   - Métricas de API calls

Genera:
- Clientes completamente funcionales
- Test fixtures para mock
- Error types personalizados
- Documentation
```

**Qué Claude Code hará:**
- ✅ Implementa clientes HTTP
- ✅ Setup WebSocket streaming
- ✅ Auth handling
- ✅ Error recovery

---

## 📊 Parte 5: Prompts para Análisis y Optimización

### PROMPT 12: Backtesting y Reportes

```
[DESPUÉS DE INTEGRACIÓN]

Crea sistema de backtesting y reportes.

1. Crear notebooks/analysis.ipynb:
   - Cargar datos históricos
   - Plotear cada estrategia vs mercado
   - Mostrar equity curve
   - Calcular métricas: Sharpe, Sortino, Calmar
   - Comparar estrategias lado a lado

2. Crear resultados/:
   - Generar CSV con cada trade
   - Trade log con: entry, exit, P&L, duration
   - Daily P&L summary
   - Weekly/Monthly P&L
   - Max drawdown analysis

3. Métricas a calcular:
   - Win Rate: % de trades ganadores
   - Avg Win/Loss ratio: profit promedio / loss promedio
   - Profit Factor: total_wins / total_losses
   - Sharpe Ratio: return / volatility
   - Max Drawdown: % caída máxima desde peak
   - Recovery Factor: total_profit / max_drawdown
   - CAR: annualized return
   - Payoff Ratio: avg_win / avg_loss

4. Comparación:
   - Cada estrategia vs buy-and-hold
   - Cada estrategia vs otras
   - Diferentes símbolos vs cada estrategia

5. Optimización:
   - Grid search sobre parámetros
   - Walk-forward analysis
   - Out-of-sample testing

Genera:
- Functions de cálculo de métricas
- Plotting functions
- Report generation
- Data export
```

**Qué Claude Code hará:**
- ✅ Implementa cálculos de métricas
- ✅ Genera helpers de análisis
- ✅ Crea templates de reportes
- ✅ Exporta a CSV/JSON

---

## 💡 Parte 6: Workflow Completo (Paso a Paso)

### Day 1-2: Setup Inicial
```
1. Crear proyecto con estructura base
2. PROMPT 1: Generar Grid Trading
3. Verificar que compila: cargo build
4. Ejecutar tests básicos: cargo test
```

### Day 3-4: Agregar Estrategias
```
1. PROMPT 2: Spread Trading
2. PROMPT 3: Event-Driven
3. PROMPT 4: Mean Reversion
4. PROMPT 5: DCA Bot
5. Compilar después de cada prompt: cargo build
```

### Day 5: Más Estrategias
```
1. PROMPT 6: Correlation Arbitrage
2. PROMPT 7: Liquidation Hunt
3. PROMPT 8: Pump & Dump
4. cargo build --release
```

### Day 6-7: Integración
```
1. PROMPT 9: Marco unificado (lib.rs, main.rs)
2. PROMPT 10: Tests completos
3. Ejecutar: cargo test --all
```

### Day 8-10: APIs Reales
```
1. PROMPT 11: Integración Binance + Polymarket
2. Conectar a testnet primero
3. Paper trading (sin dinero real)
```

### Day 11-14: Análisis y Optimización
```
1. PROMPT 12: Backtesting y reportes
2. Ejecutar backtests: cargo run -- --backtest true
3. Generar análisis: python notebooks/analysis.ipynb
4. Revisar resultados
```

---

## 🎯 Parte 7: Ejemplos de Prompts "Pegajosos" para Claude Code

### PROMPT Bonus 1: Auto-Update de Parámetros

```
[SI QUIERES OPTIMIZACIÓN AUTOMÁTICA]

Crea un sistema que auto-optimiza parámetros de estrategias.

1. Crear src/optimizer.rs con:
   - Cargar últimos 30 días de trades
   - Analizar qué parámetros funcionaron mejor
   - Proponer nuevos valores
   - Test en datos out-of-sample
   - Si mejor, actualizar config

2. Algoritmo:
   - Grid search: probar 100 combinaciones de parámetros
   - Backtesting rápido (solo últimos 20 candles)
   - Seleccionar top 5 mejores
   - Cross-validate en datos nuevos
   - Si win rate sigue > 50%, aplicar

3. Ejecutar cada:
   - Cada semana (después de 50+ trades)
   - Antes de cambios de mercado importante
   - Si win rate cae bajo 45%

Genera:
- Grid search implementation
- Parameter validation
- Cross-validation logic
- Config update mechanism
```

---

### PROMPT Bonus 2: Dashboard Integrado

```
[SI QUIERES MONITOREO VISUAL]

Crea un dashboard web en tiempo real.

1. Crear web/:
   - index.html con dashboard
   - main.js para actualizar en tiempo real
   - CSS para styling

2. Dashboard muestra:
   - Current balance y P&L
   - Posiciones abiertas (por estrategia)
   - Recent trades
   - Win rate (últimos 7 días)
   - Métricas de salud

3. Crear src/server.rs:
   - Servidor WebSocket
   - Enviar updates cada segundo
   - Evento-driven (cuando algo cambia)
   - WebSocket server using tokio-tungstenite

4. Setup:
   - cargo run -- --web true
   - Abre http://localhost:3000
   - Ve datos en tiempo real

Genera:
- WebSocket server
- HTML/CSS/JS
- Data serialization
- Auto-refresh logic
```

---

## 🚀 Parte 8: Tips Avanzados para Claude Code

### Tip 1: Usa "Continue" para Iteraciones

```
Primero genera el código base con PROMPT 1.

Luego en la próxima sesión:
"Continúa con lo anterior. Ahora necesito agregar [nueva feature]"

Claude Code mantiene contexto del proyecto anterior.
```

### Tip 2: Aprovecha Test-Driven Development

```
Primero pide tests:
"Quiero tests para una función que [description]"

Luego:
"Ahora genera la implementación que pase estos tests"

Claude Code generará código mejor estructurado.
```

### Tip 3: Uso de Context (muy importante)

```
@src/strategies/grid_trading.rs    ← Referencia archivo específico
@tests/                            ← Referencia carpeta
@crate                             ← Referencia proyecto entero

"@src/strategies/ genera función compatible con GridTrading struct"

Claude Code entiende contexto del proyecto.
```

### Tip 4: Genera Documentación

```
"Genera documentación README para cada estrategia"

Claude Code creará:
- README.md explicando cada estrategia
- Ejemplos de uso
- Parámetros recomendados
- Casos de uso
```

---

## 📋 Checklist Final

Antes de hacer live trading, verifica:

```
✅ SETUP
[ ] Proyecto Rust compila sin errores
[ ] Todas las 8 estrategias están implementadas
[ ] Tests pasan: cargo test --all

✅ BACKTESTING
[ ] Grid Trading: 90 días, win rate > 70%
[ ] Event-Driven: 30 días, win rate > 60%
[ ] Mean Reversion: 90 días, win rate > 55%
[ ] Spread Trading: paper trade 1 semana OK
[ ] DCA Bot: 30 días, positive return

✅ APIS
[ ] Binance testnet: orders funcionan
[ ] Polymarket testnet: price updates OK
[ ] WebSocket: recibe datos en vivo
[ ] Error handling: reintentos funcionan

✅ PAPER TRADING
[ ] 2 semanas simulado: resultados consistentes
[ ] Drawdown aceptable: < 10%
[ ] Operaciones tienen lógica correcta

✅ MONITOREO
[ ] Logging funciona: cargo run -- --log debug
[ ] Dashboard actualiza: http://localhost:3000
[ ] Alertas funcionan: recibe Slack/Email

✅ LIVE TRADING (MINI)
[ ] Capital inicial pequeño: $100-500
[ ] Leverage: 1.0 (sin apalancamiento)
[ ] Max loss: $5 por trade
[ ] Monitoreo 24/7: alguien viendo
```

---

## 🎓 Recursos Adicionales

### Para entender mejor cada estrategia:
- Spread Trading: https://en.wikipedia.org/wiki/Spread_trading
- Mean Reversion: https://www.investopedia.com/terms/m/meanreversion.asp
- Grid Trading: Tutorial de Binance oficial
- Event-Driven: Finance calendars (tradingeconomics.com)

### Documentación oficial:
- Rhai: https://rhai.rs/
- Tokio: https://tokio.rs/
- Binance API: https://binance-docs.github.io/apidocs/
- Polymarket Docs: https://docs.polymarket.com/

### Para optimización:
- https://www.statlearning.com/ (Statistical Learning Theory)
- https://www.investopedia.com/terms/p/probsharpe.asp (Probabilistic Sharpe Ratio)

---

## 📞 Soporte y Next Steps

Si tienes dudas sobre prompts específicos:

1. **Copia exactamente el prompt** de la sección correspondiente
2. **Pégalo en Claude Code**
3. **Espera a que genere el código**
4. **Si hay errores, pide correcciones**:
   ```
   "El código en GridTrading tiene error de compilación en línea X.
    Corrígelo manteniendo la lógica"
   ```

5. **Para next features, usa**:
   ```
   "@src/strategies/grid_trading.rs
    @src/backtester/
    Necesito agregar [nueva funcionalidad]"
   ```

---

**Autor**: Trading Bot Blueprint for Claude Code  
**Última actualización**: 2024-04-14  
**Status**: Production-Ready Template
