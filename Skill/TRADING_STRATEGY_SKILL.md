# 🎯 SKILL: Generador de Estrategias de Trading Rhai para Crypto/Polymarket

## Descripción

Este skill permite a Claude generar **estrategias de trading profesionales** en **Rhai Script** que se ejecutan en agentes **Rust**. Proporciona:

- ✅ Templates de estrategias probadas
- ✅ Indicadores técnicos reutilizables
- ✅ Validación automática de sintaxis
- ✅ Mejores prácticas de risk management
- ✅ Ejemplos de extensión y customización
- ✅ Integración con Polymarket API

---

## Cuándo Usar Este Skill

**USE ESTE SKILL cuando:**
- El usuario pida crear una estrategia de trading en Rhai
- Necesite generar scripts de estrategia para Polymarket
- Quiera extender o mejorar una estrategia existente
- Requiera indicadores técnicos específicos
- Deba implementar risk management avanzado

**NO use este skill cuando:**
- La solicitud es solo analítica (no necesita código ejecutable)
- Es educación general sobre trading (no necesita script Rhai)
- Pide análisis de sentimiento o predicción pura

---

## Estructuras y Componentes Core

### 1. Anatomía de una Estrategia Rhai

```rhai
// SECCIÓN 1: CONFIGURACIÓN
let config = #{
    lookback_period: 14,
    momentum_threshold: 0.8,
    max_position_size: 0.2,
    // ... más parámetros
};

// SECCIÓN 2: ESTADO
let bot_state = #{
    positions: [],
    trades_log: [],
    current_bar: 0,
    // ... tracking
};

// SECCIÓN 3: INDICADORES
fn calculate_indicator(data, period) {
    // Implementación del indicador
}

// SECCIÓN 4: LÓGICA DE SEÑALES
fn generate_signal(candle_data) {
    // Evalúa condiciones
    // Retorna SIGNAL_BULLISH, SIGNAL_NEUTRAL, SIGNAL_BEARISH
}

// SECCIÓN 5: GESTIÓN DE POSICIONES
fn on_candle(candle_data, capital) {
    // Procesa nuevas velas
    // Maneja entrada/salida
}

// SECCIÓN 6: UTILIDADES
fn get_stats() {
    // Retorna estadísticas
}
```

### 2. Parámetros Configurables

**Obligatorios:**
- `lookback_period` - Período para indicadores (default: 14)
- `momentum_threshold` - Umbral de momentum (default: 0.8)
- `max_position_size` - % del capital por posición (default: 0.2)

**Risk Management:**
- `max_loss_percent` - Stop loss en % (default: 2.0)
- `take_profit_percent` - Take profit en % (default: 3.0)
- `max_positions` - Máximas posiciones simultáneas (default: 3)
- `max_hold_bars` - Máximas barras held (default: 5)

**Confirmaciones:**
- `required_confirmations` - Min confirmaciones requeridas (default: 3 de 4)
- `min_volume_threshold` - Volumen mínimo (default: 1.2x promedio)
- `min_candles` - Candles mínimas antes de tradear (default: 20)

### 3. Tipos de Indicadores Soportados

| Indicador | Función | Parámetro | Uso |
|-----------|---------|-----------|-----|
| **SMA** | Media móvil simple | period | Tendencia |
| **ATR** | Rango verdadero promedio | period | Volatilidad |
| **RSI** | Índice de fuerza relativa | period | Sobreventa/compra |
| **MACD** | Convergencia/divergencia | fast, slow, signal | Momentum |
| **Bollinger** | Bandas de Bollinger | period, std_dev | Volatilidad |
| **Momentum** | Cambio de precio | period | Velocidad |
| **Volumen** | Análisis de volumen | period | Confirmación |

---

## Patrones de Estrategias

### Patrón 1: Mean Reversion (Reversión a la Media)

```rhai
// Compra en sobreventa, vende en sobrecompra
fn generate_signal(candle_data) {
    let rsi = calculate_rsi(closes, 14);
    
    if rsi < 30 {
        return SIGNAL_BULLISH;  // Sobreventa
    } else if rsi > 70 {
        return SIGNAL_BEARISH;  // Sobrecompra
    }
    
    SIGNAL_NEUTRAL
}
```

**Cuándo usar:**
- Mercados ranging (sin tendencia)
- Alta volatilidad
- Timeframes cortos (5m, 15m)

**Parámetros recomendados:**
```
rsi_threshold: 30/70
max_hold_bars: 3-5
take_profit_percent: 2-3%
```

---

### Patrón 2: Momentum (Seguimiento de Tendencia)

```rhai
// Compra/vende en dirección del momentum
fn generate_signal(candle_data) {
    let momentum = calculate_momentum(closes, 4);
    let sma = calculate_sma(closes, 14);
    
    if momentum > threshold && close > sma {
        return SIGNAL_BULLISH;
    } else if momentum < -threshold && close < sma {
        return SIGNAL_BEARISH;
    }
    
    SIGNAL_NEUTRAL
}
```

**Cuándo usar:**
- Tendencias claras
- Breakouts
- Mercados con volatilidad media

**Parámetros recomendados:**
```
momentum_threshold: 0.5-1.0
lookback_period: 4-8
max_hold_bars: 5-10
```

---

### Patrón 3: Mean Reversion + Momentum (Híbrido)

```rhai
// Combinación de ambas señales
fn generate_signal(candle_data) {
    let rsi = calculate_rsi(closes, 14);
    let momentum = calculate_momentum(closes, 4);
    let sma = calculate_sma(closes, 14);
    
    // Señal bullish: RSI bajo + momentum positivo + encima SMA
    if rsi < 30 && momentum > 0.2 && close > sma {
        return SIGNAL_BULLISH;
    }
    
    // Señal bearish: RSI alto + momentum negativo + debajo SMA
    if rsi > 70 && momentum < -0.2 && close < sma {
        return SIGNAL_BEARISH;
    }
    
    SIGNAL_NEUTRAL
}
```

**Ventajas:**
- Reducidas falsas señales
- Mejor win rate
- Confirmaciones múltiples

---

## Templates Listos para Usar

### Template 1: Estrategia Básica de 4-Minutos

```rhai
// Polymarket 4-Minute Simple Strategy
let config = #{
    momentum_threshold: 0.8,
    rsi_threshold: 30.0,
    atr_multiplier: 1.5,
    max_position_size: 0.2,
    max_loss_percent: 2.0,
    take_profit_percent: 3.0,
    required_confirmations: 3,
    max_hold_bars: 5
};

// Indicadores
fn calculate_momentum(closes, period) {
    let current = closes[closes.len() - 1];
    let previous = closes[closes.len() - period - 1];
    ((current - previous) / previous) * 100.0
}

fn calculate_rsi(closes, period) {
    // RSI implementation
    let mut gains = 0.0;
    let mut losses = 0.0;
    
    for i in (closes.len() - period)..closes.len() {
        let change = closes[i] - closes[i-1];
        if change > 0.0 { gains += change; }
        else { losses -= change; }
    }
    
    let avg_gain = gains / period;
    let avg_loss = losses / period;
    let rs = avg_gain / avg_loss;
    100.0 - (100.0 / (1.0 + rs))
}

// Señales
fn generate_signal(candle_data) {
    let closes = candle_data.closes;
    let momentum = calculate_momentum(closes, 4);
    let rsi = calculate_rsi(closes, 14);
    
    if momentum > config.momentum_threshold && rsi < config.rsi_threshold {
        return SIGNAL_BULLISH;
    }
    
    if momentum < -config.momentum_threshold && rsi > (100.0 - config.rsi_threshold) {
        return SIGNAL_BEARISH;
    }
    
    SIGNAL_NEUTRAL
}

// Main
fn on_candle(candle_data, capital) {
    // Process entry/exit logic
}
```

---

### Template 2: Estrategia de Múltiples Confirmaciones

```rhai
// Strategy with 4-point confirmation system
let config = #{
    lookback_4: 4,
    lookback_14: 14,
    momentum_threshold: 0.8,
    rsi_threshold: 30.0,
    atr_multiplier: 1.5,
    required_confirmations: 3,  // De 4 posibles
};

fn get_bullish_confirmations(candle_data) {
    let mut confirmations = 0;
    
    // 1. Momentum check
    if calculate_momentum(candle_data.closes, 4) > config.momentum_threshold {
        confirmations += 1;
    }
    
    // 2. RSI check
    if calculate_rsi(candle_data.closes, 14) < config.rsi_threshold {
        confirmations += 1;
    }
    
    // 3. Volume check
    if candle_data.volumes[candle_data.volumes.len()-1] > avg_volume {
        confirmations += 1;
    }
    
    // 4. Trend check (SMA)
    if candle_data.closes[candle_data.closes.len()-1] > calculate_sma(candle_data.closes, 14) {
        confirmations += 1;
    }
    
    confirmations
}

fn generate_signal(candle_data) {
    let bullish = get_bullish_confirmations(candle_data);
    
    if bullish >= config.required_confirmations {
        return SIGNAL_BULLISH;
    }
    
    // Similar para bearish...
    
    SIGNAL_NEUTRAL
}
```

---

### Template 3: Estrategia con Dynamic Risk Management

```rhai
// Advanced risk management strategy
fn calculate_position_size(capital, volatility, account_drawdown) {
    // Reducir size si hay drawdown
    let drawdown_factor = if account_drawdown > 0.1 { 0.5 } else { 1.0 };
    
    // Aumentar size si volatilidad baja
    let volatility_factor = if volatility < 1.0 { 1.2 } else { 1.0 };
    
    let base_size = capital * 0.2;  // 20% base
    base_size * drawdown_factor * volatility_factor
}

fn calculate_stop_loss(entry_price, volatility, atr) {
    // Dynamic SL basado en volatilidad
    let multiplier = if volatility > 2.0 { 2.0 } else { 1.5 };
    entry_price - (atr * multiplier)
}

fn on_candle(candle_data, capital, account_stats) {
    let volatility = calculate_atr(candle_data.highs, candle_data.lows, candle_data.closes, 14);
    let position_size = calculate_position_size(capital, volatility, account_stats.drawdown);
    let stop_loss = calculate_stop_loss(entry_price, volatility, atr);
    
    // Use dynamic values for trade
}
```

---

## Validación de Estrategias

### Checklist de Validación

Antes de generar una estrategia, verificar:

- ✅ **Sintaxis Rhai válida**
  ```bash
  cargo check  # En proyecto Rust
  ```

- ✅ **Indicadores calculan correctamente**
  - Mínimas 20 velas antes de tradear
  - Parámetros dentro de rangos válidos

- ✅ **Risk management presente**
  - Stop loss definido
  - Take profit definido
  - Position sizing calculado
  - Max loss limit

- ✅ **Señales generan correctamente**
  - Retorna SIGNAL_BULLISH, NEUTRAL, o BEARISH
  - Múltiples confirmaciones (mínimo 2-3)
  - No genera señales contradictorias

- ✅ **Código limpio y documentado**
  - Funciones con propósitos claros
  - Variables nombradas descriptivamente
  - Comentarios en lógica compleja

---

## Patrones Anti (Lo Que NO Hacer)

### ❌ Evitar: Overfitting

```rhai
// MAL: Demasiados parámetros ajustados al histórico
if (rsi > 35 && rsi < 42) && (momentum > 0.75 && momentum < 0.95) {
    // Demasiado específico, no generalizará
}
```

**Solución:**
```rhai
// BIEN: Parámetros simples y robustos
if rsi < 30 && momentum > 0.5 {
    // Generaliza mejor
}
```

### ❌ Evitar: Sin Risk Management

```rhai
// MAL: Sin stop loss
fn on_candle(candle_data) {
    if signal == BULLISH {
        ctx.buy(1.0);  // ¡Sin stop loss!
    }
}
```

**Solución:**
```rhai
// BIEN: Con stop loss y take profit
if signal == BULLISH {
    position.stop_loss = entry_price - atr * 1.5;
    position.take_profit = entry_price + atr * 2.0;
    ctx.buy(position_size);
}
```

### ❌ Evitar: Indicadores Contradictorios

```rhai
// MAL: Señales que se anulan
if momentum > 0 && momentum < -0.5 {  // ¡Imposible!
    // Nunca se ejecuta
}
```

**Solución:**
```rhai
// BIEN: Lógica coherente
if momentum > threshold && rsi < 30 {
    // Ambas condiciones pueden ser verdad
}
```

---

## Cómo Generar una Estrategia

### Paso 1: Obtener Requisitos del Usuario

Preguntar:
1. **¿Qué mercados?** (BTC, ETH, altcoins, forex)
2. **¿Qué timeframe?** (5m, 15m, 1h, 4h)
3. **¿Qué estilo?** (momentum, mean reversion, arbitraje)
4. **¿Indicadores preferidos?** (RSI, MACD, Bollinger, etc.)
5. **¿Riesgo?** (agresivo, normal, conservador)
6. **¿Velocidad?** (scalping, day trading, swing)

### Paso 2: Seleccionar Template

```
Estilo → Template Recomendado
───────────────────────────────
Momentum → Template 2 (4-confirmaciones)
Mean Reversion → Template 1 (RSI + Momentum)
Avanzado → Template 3 (Risk dinámico)
Custom → Construir desde cero
```

### Paso 3: Personalizar Parámetros

Usar tabla de parámetros recomendados según estilo.

### Paso 4: Validar

- Verificar sintaxis Rhai
- Validar lógica de señales
- Comprobar risk management

### Paso 5: Documentar

```rhai
// ============================================================
// NOMBRE: Estrategia [Nombre]
// TIMEFRAME: [5m/15m/1h]
// ESTILO: [Momentum/Mean Reversion/Hybrid]
// AUTOR: [Si aplica]
// DESCRIPCIÓN: [Breve descripción]
// ============================================================

// PARÁMETROS RECOMENDADOS:
// - Mercados: BTC, ETH, SOL
// - Timeframe: 5m
// - Win Rate esperado: 60-65%
// - Max drawdown: 15-20%

// CONFIGURACIÓN
let config = #{
    // ...
};

// RESTO DEL CÓDIGO
```

---

## Extensiones y Modificaciones Comunes

### Agregar Indicador Nuevo

```rhai
// 1. Implementar cálculo
fn calculate_macd(closes, fast, slow, signal) {
    let ema_fast = calculate_ema(closes, fast);
    let ema_slow = calculate_ema(closes, slow);
    let macd_line = ema_fast - ema_slow;
    let signal_line = calculate_ema(macd_line, signal);
    #{
        macd: macd_line,
        signal: signal_line,
        histogram: macd_line - signal_line
    }
}

// 2. Usar en estrategia
fn generate_signal(candle_data) {
    let macd = calculate_macd(closes, 12, 26, 9);
    
    if macd.histogram > 0 && macd.macd > macd.signal {
        return SIGNAL_BULLISH;
    }
    
    SIGNAL_NEUTRAL
}
```

### Cambiar Confirmaciones

```rhai
// Aumentar a 4 confirmaciones mínimas
let config = #{
    required_confirmations: 4,  // Fue 3
};

// Añadir nueva confirmación: Volumen en aumento
fn get_bullish_confirmations(candle_data) {
    let mut confirmations = 0;
    
    // Confirmaciones anteriores...
    
    // Nueva confirmación: volumen
    if current_volume > avg_volume * 1.2 {
        confirmations += 1;
    }
    
    confirmations
}
```

### Implementar Trailing Stop

```rhai
fn update_position_with_trailing_stop(position, current_price, atr) {
    // Si precio se mueve a favor, mover stop
    if position.side == "long" {
        let new_stop = current_price - atr * 1.5;
        
        // Solo mover hacia arriba (nunca hacia abajo)
        if new_stop > position.stop_loss {
            position.stop_loss = new_stop;
        }
    }
    
    position
}
```

---

## Integración con Rust Agent

Una estrategia Rhai está diseñada para ejecutarse en un agente Rust:

```rust
// En agent.rs
let mut agent = TradingAgent::new("mi_estrategia.rhai", market_config)?;

// Procesar datos
agent.on_candle(&candle_data, capital)?;

// Obtener resultados
let stats = agent.get_stats()?;
```

La estrategia Rhai y el agente Rust trabajan juntos:
- **Rhai**: Lógica de trading (indicadores, señales)
- **Rust**: Infraestructura (API, persistencia, performance)

---

## Benchmarking de Estrategias

### Métricas Clave

```
Win Rate = Winning Trades / Total Trades
       ↑ Objetivo: > 60%

Profit Factor = Gross Profit / Gross Loss
            ↑ Objetivo: > 1.5

Max Drawdown = Peak Decline / Peak Value
           ↑ Objetivo: < 20%

Expectancy = (Win Rate × Avg Win) - (Loss Rate × Avg Loss)
         ↑ Objetivo: > 0
```

### Cómo Evaluar

```rhai
fn evaluate_strategy(trades) {
    let total_trades = trades.len();
    let wins = trades.filter(|t| t.pnl > 0).len();
    let losses = trades.filter(|t| t.pnl < 0).len();
    
    let win_rate = wins / total_trades;
    let avg_win = trades.filter(|t| t.pnl > 0).map(|t| t.pnl).sum() / wins;
    let avg_loss = trades.filter(|t| t.pnl < 0).map(|t| t.pnl).sum() / losses;
    
    let expectancy = (win_rate * avg_win) - ((1.0 - win_rate) * avg_loss);
    
    #{
        win_rate: win_rate,
        avg_win: avg_win,
        avg_loss: avg_loss,
        expectancy: expectancy,
        is_viable: expectancy > 0.0 && win_rate > 0.5
    }
}
```

---

## Archivos de Referencia

| Archivo | Descripción |
|---------|-------------|
| **strategy.rhai** | Plantilla base completa |
| **strategy_momentum.rhai** | Estrategia de momentum |
| **strategy_mean_reversion.rhai** | Estrategia de reversión |
| **strategy_advanced.rhai** | Estrategia con risk dinámico |
| **indicators.rhai** | Librería de indicadores |

---

## Checklist de Generación

Antes de entregar una estrategia:

- [ ] Sintaxis Rhai válida
- [ ] Indicadores calculan correctamente
- [ ] Risk management presente (SL, TP, position sizing)
- [ ] Múltiples confirmaciones (3+)
- [ ] Documentación clara
- [ ] Parámetros justificados
- [ ] Código limpio y comentado
- [ ] Benchmarks/backtest esperado
- [ ] Ejemplos de uso incluidos
- [ ] Integración con agent.rs validada

---

## Casos de Uso Específicos

### Caso 1: Usuario quiere estrategia de "4-minutos para Polymarket"
→ Usar Template 2 (Momentum) + parámetros agresivos + confirmaciones

### Caso 2: Usuario quiere "mean reversion en BTC"
→ Usar Template 1 + RSI threshold 30/70 + max_hold_bars 3

### Caso 3: Usuario quiere "estrategia personal"
→ Entrevistar requisitos → Combinar templates → Validar

### Caso 4: Usuario quiere "mejorar estrategia existente"
→ Analizar parámetros actuales → Sugerir cambios → Validar mejora

---

## Limitaciones y Consideraciones

⚠️ **Importante:**
- Las estrategias generadas son teóricas
- Requieren backtesting riguroso
- El pasado no garantiza futuro
- Usar con pequeño capital primero
- Never risk más de lo que puedes perder

✅ **Mejor práctica:**
1. Genera estrategia
2. Backtest con datos históricos (3+ meses)
3. Paper trade 100+ operaciones
4. Solo entonces: live con dinero real
5. Monitorea y ajusta continuamente

---

## Recursos Adicionales

- [Rhai Documentation](https://rhai.rs/)
- [Technical Analysis](https://www.investopedia.com/terms/t/technicalanalysis.asp)
- [Trading Systems Design](https://www.amazon.com/Building-Winning-Trading-Systems-TradingBlox/dp/0470389680)
- [Polymarket API](https://docs.polymarket.com/)

---

## Conclusión

Este skill permite generar estrategias profesionales de trading en Rhai:

✅ Basadas en best practices  
✅ Validadas y documentadas  
✅ Listas para ejecutar  
✅ Fáciles de extender  
✅ Integradas con Rust agents  

**¡Ahora puedes crear estrategias de trading profesionales en minutos!**
