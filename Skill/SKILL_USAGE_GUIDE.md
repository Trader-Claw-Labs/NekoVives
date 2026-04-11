# 🎯 Skill: Generador de Estrategias de Trading Rhai

## ¿Qué es?

Un **sistema completo** para que Claude genere estrategias de trading profesionales en **Rhai Script** que se ejecutan en agentes **Rust**.

**Incluye:**
- ✅ SKILL.md (600+ líneas) - Documentación completa
- ✅ STRATEGY_TEMPLATES.rhai - 7 templates listos para usar
- ✅ strategy_validator.rs - Validador automático
- ✅ Guía paso a paso para crear estrategias

---

## Cómo Activar Este Skill

### Cuando Claude vea una solicitud como:

```
"Crea una estrategia de trading para Polymarket 5-minutos"
"Quiero una estrategia momentum en Rhai"
"Ayúdame a mejorar esta estrategia de trading"
"Genera un script de scalping para crypto"
```

### Entonces Claude debe:

1. **Leer TRADING_STRATEGY_SKILL.md**
2. **Seleccionar template apropiado** de STRATEGY_TEMPLATES.rhai
3. **Personalizar parámetros** según requisitos
4. **Validar con strategy_validator.rs**
5. **Generar script final** documentado y listo

---

## Los 7 Templates Disponibles

| # | Nombre | Mejor Para | Win Rate | Timeframe |
|---|--------|-----------|----------|-----------|
| 1 | **Momentum Simple** | Polymarket 5m | 55-65% | 5m |
| 2 | **Mean Reversion + Trend** | Ranging markets | 60-70% | 15m |
| 3 | **Multiple Confirmations** | Confiabilidad | 65-75% | 15m+ |
| 4 | **MACD + RSI** | Tendencias fuertes | 60-68% | 15m+ |
| 5 | **Volatility Breakout** | Breakouts | 50-60% (pero grandes ganancias) | 1h+ |
| 6 | **Scalping Agresivo** | Scalpers (1-3m) | 55-60% (micro trades) | 1m-3m |
| 7 | **Swing Trading** | Swing trades (4h-1d) | 65-75% (menos operaciones) | 4h-1d |

---

## Flujo de Generación Típico

### Usuario: "Quiero una estrategia para BTC 5-minutos en Polymarket"

#### Paso 1: Entrevistar
```
Claude pregunta:
- ¿Qué estilo prefieres? (momentum, mean reversion, etc.)
- ¿Quieres muchos trades o pocos pero grandes?
- ¿Tolerancia al riesgo? (agresivo, normal, conservador)
- ¿Indicadores específicos?
```

#### Paso 2: Seleccionar Template
```
Para 5m Polymarket → Template 1 (Momentum Simple)
Es el más probado para este timeframe y mercado.
```

#### Paso 3: Personalizar
```
Usuario quiere agresivo:
- momentum_threshold: 0.8 → 0.5
- max_position_size: 0.2 → 0.3
- take_profit_percent: 3.0 → 2.0
```

#### Paso 4: Validar
```
StrategyValidator verifica:
✓ Sintaxis Rhai correcta
✓ Risk management presente
✓ Múltiples confirmaciones
✓ Parámetros en rangos válidos
```

#### Paso 5: Generar Script Final
```rhai
// POLYMARKET 4-MINUTE AGGRESSIVE STRATEGY
// Generada con Skill de Trading Strategies

let config = #{
    momentum_threshold: 0.5,     // Personalizado
    max_position_size: 0.3,      // Personalizado
    // ... más parámetros
};

fn generate_signal(candle_data) {
    // Lógica completa lista para ejecutar
}

// ... más funciones
```

#### Paso 6: Entrega
```
✓ Script validado
✓ Documentación completa
✓ Parámetros justificados
✓ Listo para ejecutar en agente Rust
```

---

## Ejemplo Real: Generando una Estrategia

### Usuario solicita:
> "Crea una estrategia de reversión a la media para ETH en 15-minutos, conservadora, para Polymarket"

### Claude ejecuta el Skill:

**Paso 1:** Lee TRADING_STRATEGY_SKILL.md
- Identifica que es Mean Reversion → Template 2

**Paso 2:** Selecciona Template 2
```rhai
let config_mean_reversion = #{
    rsi_oversold: 30.0,
    // ... parámetros base
};
```

**Paso 3:** Personaliza para "conservador"
- Aumenta `required_confirmations`: 3 → 4
- Reduce `max_position_size`: 0.2 → 0.15
- Reduce `max_loss_percent`: 1.5 → 1.0

**Paso 4:** Valida con strategy_validator.rs
```
✓ Sintaxis válida
✓ 5+ funciones encontradas
✓ RSI implementado
✓ Risk management: SL, TP, position sizing
✓ 4 confirmaciones requeridas
```

**Paso 5:** Genera script final documentado
```rhai
// ============================================================
// NOMBRE: ETH Mean Reversion Conservative
// TIMEFRAME: 15 minutos
// ESTILO: Mean Reversion
// DESCRIPCIÓN: Detecta sobreventa/sobrecompra en ETH
// ============================================================

// PARÁMETROS RECOMENDADOS:
// - Win Rate: 65-70%
// - Max Drawdown: 12-15%
// - Mejor en: Mercados ranging

let config = #{
    rsi_oversold: 30.0,
    rsi_overbought: 70.0,
    sma_period: 20,
    required_confirmations: 4,        // Aumentado
    max_position_size: 0.15,          // Reducido
    max_loss_percent: 1.0,            // Reducido
    // ... más parámetros
};

fn mean_reversion_signal(candle_data) {
    // Implementación completa
}

// ... resto del código
```

**Paso 6:** Entrega final
```
✓ Script de 200+ líneas
✓ Documentación completa
✓ Parámetros justificados
✓ Validación pasada
✓ Listo para ejecutar
```

---

## Cómo Extender o Modificar Estrategias

### Agregar un Indicador Nuevo

Si usuario pide: "Añade MACD a la estrategia"

Claude busca en SKILL.md la sección "Extensiones y Modificaciones" y:

1. Implementa `calculate_macd()`
2. Añade a `generate_signal()`
3. Aumenta confirmaciones
4. Valida el script

### Cambiar Parámetros

Si usuario pide: "Hazla más agresiva"

Claude refiere a la tabla de parámetros y:
- Reduce momentum_threshold
- Aumenta max_position_size
- Reduce hold time
- Valida el cambio

### Combinar Templates

Si usuario pide: "Combina momentum con mean reversion"

Claude:
1. Usa Template 1 base (Momentum)
2. Importa lógica de Template 2 (Mean Reversion)
3. Crea confirmaciones combinadas
4. Valida compatibilidad

---

## Parámetros Clave y Cómo Ajustarlos

### Para ser más AGRESIVO:
```
momentum_threshold:      0.8 → 0.3    (Señales más sensibles)
max_position_size:       0.2 → 0.4    (Posiciones más grandes)
required_confirmations:  3 → 2        (Menos confirmaciones)
take_profit_percent:     3.0 → 1.5    (Ganancias rápidas)
max_hold_bars:           5 → 2        (Salida rápida)
```

### Para ser más CONSERVADOR:
```
momentum_threshold:      0.8 → 1.5    (Señales más exigentes)
max_position_size:       0.2 → 0.1    (Posiciones pequeñas)
required_confirmations:  3 → 4        (Más confirmaciones)
max_loss_percent:        2.0 → 1.0    (Stop más cerrado)
max_hold_bars:           5 → 10       (Mantener más tiempo)
```

---

## Validación Automática

Cuando Claude genera una estrategia, usa strategy_validator.rs para verificar:

```
VALIDACIONES CRÍTICAS (Deben pasar):
✓ Sintaxis Rhai correcta
✓ Funciones requeridas presentes
✓ Risk management configurado
✓ Señales generan valores válidos

VALIDACIONES RECOMENDADAS (Warnings):
⚠ Documentación suficiente
⚠ Parámetros en rangos válidos
⚠ Indicadores suficientes (2-3+)
⚠ Complejidad apropiada
```

---

## Checklist de Generación

Claude debe verificar:

- [ ] Entiende requisitos del usuario
- [ ] Selecciona template apropiado
- [ ] Personaliza parámetros
- [ ] Valida con strategy_validator.rs
- [ ] Documenta la estrategia
- [ ] Justifica los parámetros
- [ ] Incluye ejemplos de uso
- [ ] Proporciona métricas esperadas

---

## Casos de Uso del Skill

### ✅ Usar este Skill cuando:

1. **Usuario quiere crear estrategia**
   > "Crea una estrategia de momentum para Polymarket"

2. **Usuario quiere modificar existente**
   > "Hazla más conservadora"

3. **Usuario quiere entender estrategias**
   > "¿Cuál es la mejor estrategia para BTC 5-minutos?"

4. **Usuario quiere combinar ideas**
   > "Quiero momentum + mean reversion"

5. **Usuario quiere optimizar parámetros**
   > "¿Cuáles son los parámetros óptimos?"

### ❌ NO usar este Skill cuando:

- Solo análisis (no necesita código ejecutable)
- Educación teórica (no necesita Rhai)
- Backtesting de estrategia existente (usar simulator)
- Debugging de código Rust (usar RUST_SETUP_GUIDE)

---

## Integración con Agente Rust

Una estrategia generada con este Skill se ejecuta así:

```rust
// 1. Crear agente
let mut agent = TradingAgent::new("strategy.rhai", market_config)?;

// 2. Procesar datos
agent.on_candle(&candle_data, capital)?;

// 3. Obtener resultados
let stats = agent.get_stats()?;
println!("Win Rate: {:.2}%", stats.win_rate);
```

La estrategia Rhai genera señales, el agente Rust las ejecuta.

---

## Próximos Pasos Después de Generar

1. **Guardar el script**
   ```
   Salva la estrategia como: mi_estrategia.rhai
   ```

2. **Validar localmente**
   ```bash
   cargo check  # En tu proyecto Rust
   ```

3. **Backtest**
   ```bash
   cargo run --release  # Ejecuta el backtest
   ```

4. **Revisar resultados**
   - Win rate debe ser > 50%
   - Profit factor > 1.0
   - Max drawdown < 20%

5. **Paper trade**
   - Simula con datos reales
   - 100+ operaciones
   - Sin dinero real

6. **Live (después de validar)**
   - Capital pequeño
   - Monitoreo 24/7
   - Ajustes según performance

---

## Recursos Incluidos en el Skill

| Archivo | Propósito | Líneas |
|---------|-----------|--------|
| **TRADING_STRATEGY_SKILL.md** | Documentación completa | 600+ |
| **STRATEGY_TEMPLATES.rhai** | 7 templates listos | 500+ |
| **strategy_validator.rs** | Validador automático | 400+ |
| **Esta guía** | Quick reference | - |

---

## Ejemplo: Chat con el Skill Activado

### Usuario:
> "Genera una estrategia de trading para Polymarket. Quiero momentum, agresivo, 5-minutos"

### Claude (con Skill activado):

✅ **Paso 1: Activar Skill**
- Leo TRADING_STRATEGY_SKILL.md
- Identifico: 5m + Momentum + Agresivo = Template 1

✅ **Paso 2: Entrevistar (si es necesario)**
- ¿BTC o altcoin? → Asumo BTC (más líquido)
- ¿Qué tan agresivo? → Muy agresivo = parámetros bajos

✅ **Paso 3: Generar Script**
- Personalizo Template 1
- Reduzco thresholds
- Aumento position size
- Valido con strategy_validator.rs

✅ **Paso 4: Documentar**
```rhai
// ============================================================
// POLYMARKET 5-MINUTE AGGRESSIVE MOMENTUM
// ============================================================

let config = #{
    momentum_threshold: 0.5,      // Agresivo (bajo)
    max_position_size: 0.3,       // Agresivo (alto)
    max_hold_bars: 3,             // Agresivo (corto)
    // ...
};

fn generate_signal(candle_data) {
    // Implementación
}
```

✅ **Paso 5: Entregar**
```
✓ Script validado
✓ 200+ líneas documentadas
✓ Listo para ejecutar
✓ Parámetros agresivos como solicitado
```

---

## ¡Ahora Está Listo!

Este Skill permite que Claude genere estrategias profesionales de trading en **minutos**.

**Características:**
- 7 templates probados
- Validación automática
- Documentación completa
- Parámetros justificados
- Listo para ejecutar

**Próximo paso:** Usa el Skill para generar tu primera estrategia.

¿Qué estrategia quieres crear? 🎯
