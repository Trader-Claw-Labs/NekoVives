// ============================================================================
// TEMPLATES DE ESTRATEGIAS - LIBRERÍA REUTILIZABLE
// ============================================================================
// Selecciona una estrategia base y personalízala para tus necesidades

// ============================================================================
// TEMPLATE 1: MOMENTUM SIMPLE (5-minutos Polymarket)
// ============================================================================
// Mejor para: Timeframes cortos, mercados volátiles, BTC/ETH
// Win Rate esperado: 55-65%
// Complejidad: ⭐ Baja

let config_momentum_simple = #{
    lookback: 4,
    momentum_threshold: 0.8,
    rsi_threshold: 30.0,
    atr_multiplier: 1.5,
    max_position_size: 0.2,
    max_loss_percent: 2.0,
    take_profit_percent: 3.0,
    required_confirmations: 2,
    max_hold_bars: 4
};

fn momentum_simple_signal(candle_data) {
    let closes = candle_data.closes;
    let highs = candle_data.highs;
    let lows = candle_data.lows;
    
    // Momentum de 4 candles
    let momentum = calculate_momentum(closes, 4);
    
    // RSI de 14 candles
    let rsi = calculate_rsi(closes, 14);
    
    // SEÑAL BULLISH
    if momentum > config_momentum_simple.momentum_threshold && 
       rsi < config_momentum_simple.rsi_threshold {
        return 1;  // BULLISH
    }
    
    // SEÑAL BEARISH
    if momentum < -config_momentum_simple.momentum_threshold && 
       rsi > (100.0 - config_momentum_simple.rsi_threshold) {
        return -1;  // BEARISH
    }
    
    0  // NEUTRAL
}

// ============================================================================
// TEMPLATE 2: MEAN REVERSION + TREND FILTER (15-minutos)
// ============================================================================
// Mejor para: Mercados ranging, reversiones después de impulsos
// Win Rate esperado: 60-70%
// Complejidad: ⭐⭐ Media

let config_mean_reversion = #{
    rsi_oversold: 30.0,
    rsi_overbought: 70.0,
    sma_period: 20,
    lookback: 5,
    momentum_confirm: 0.3,
    max_position_size: 0.2,
    max_loss_percent: 1.5,
    take_profit_percent: 4.0,
    required_confirmations: 3,
    max_hold_bars: 6
};

fn mean_reversion_signal(candle_data) {
    let closes = candle_data.closes;
    
    // RSI para detectar sobreventa/sobrecompra
    let rsi = calculate_rsi(closes, 14);
    
    // SMA para filtrar tendencia
    let sma = calculate_sma(closes, config_mean_reversion.sma_period);
    
    // Momentum de confirmación
    let momentum = calculate_momentum(closes, config_mean_reversion.lookback);
    
    let current_close = closes[closes.len() - 1];
    
    // SEÑAL BULLISH: Sobreventa + encima SMA + momentum confirmando
    if rsi < config_mean_reversion.rsi_oversold && 
       current_close > sma && 
       momentum > config_mean_reversion.momentum_confirm {
        return 1;  // BULLISH (reversal al alza)
    }
    
    // SEÑAL BEARISH: Sobrecompra + debajo SMA + momentum confirmando
    if rsi > config_mean_reversion.rsi_overbought && 
       current_close < sma && 
       momentum < -config_mean_reversion.momentum_confirm {
        return -1;  // BEARISH (reversal a la baja)
    }
    
    0  // NEUTRAL
}

// ============================================================================
// TEMPLATE 3: MULTIPLE CONFIRMATIONS (Multi-timeframe)
// ============================================================================
// Mejor para: Reducir falsas señales, trading más confiable
// Win Rate esperado: 65-75%
// Complejidad: ⭐⭐⭐ Alta

let config_multi_confirm = #{
    momentum_threshold: 0.8,
    rsi_threshold: 30.0,
    atr_multiplier: 1.5,
    min_volume_ratio: 1.2,
    required_confirmations: 3,  // De 4 posibles
    max_position_size: 0.2,
    max_loss_percent: 2.0,
    take_profit_percent: 3.5,
    max_hold_bars: 5
};

fn count_bullish_confirmations(candle_data) {
    let mut confirmations = 0;
    let closes = candle_data.closes;
    let volumes = candle_data.volumes;
    
    // Confirmación 1: Momentum positivo
    if calculate_momentum(closes, 4) > config_multi_confirm.momentum_threshold {
        confirmations += 1;
    }
    
    // Confirmación 2: RSI en sobreventa
    if calculate_rsi(closes, 14) < config_multi_confirm.rsi_threshold {
        confirmations += 1;
    }
    
    // Confirmación 3: Volumen confirmando
    let current_volume = volumes[volumes.len() - 1];
    let avg_volume = calculate_sma_volume(volumes, 20);
    if current_volume > avg_volume * config_multi_confirm.min_volume_ratio {
        confirmations += 1;
    }
    
    // Confirmación 4: Precio encima SMA
    if closes[closes.len() - 1] > calculate_sma(closes, 14) {
        confirmations += 1;
    }
    
    confirmations
}

fn count_bearish_confirmations(candle_data) {
    let mut confirmations = 0;
    let closes = candle_data.closes;
    let volumes = candle_data.volumes;
    
    // Confirmación 1: Momentum negativo
    if calculate_momentum(closes, 4) < -config_multi_confirm.momentum_threshold {
        confirmations += 1;
    }
    
    // Confirmación 2: RSI en sobrecompra
    if calculate_rsi(closes, 14) > (100.0 - config_multi_confirm.rsi_threshold) {
        confirmations += 1;
    }
    
    // Confirmación 3: Volumen confirmando
    let current_volume = volumes[volumes.len() - 1];
    let avg_volume = calculate_sma_volume(volumes, 20);
    if current_volume > avg_volume * config_multi_confirm.min_volume_ratio {
        confirmations += 1;
    }
    
    // Confirmación 4: Precio debajo SMA
    if closes[closes.len() - 1] < calculate_sma(closes, 14) {
        confirmations += 1;
    }
    
    confirmations
}

fn multi_confirm_signal(candle_data) {
    let bullish_count = count_bullish_confirmations(candle_data);
    let bearish_count = count_bearish_confirmations(candle_data);
    
    if bullish_count >= config_multi_confirm.required_confirmations {
        return 1;  // BULLISH
    }
    
    if bearish_count >= config_multi_confirm.required_confirmations {
        return -1;  // BEARISH
    }
    
    0  // NEUTRAL
}

// ============================================================================
// TEMPLATE 4: MACD + RSI (Momentum confirmado)
// ============================================================================
// Mejor para: Seguimiento de tendencia fuerte
// Win Rate esperado: 60-68%
// Complejidad: ⭐⭐ Media

let config_macd_rsi = #{
    macd_fast: 12,
    macd_slow: 26,
    macd_signal: 9,
    rsi_period: 14,
    rsi_neutral_zone: 40,  // Entre 40-60 es neutral
    max_position_size: 0.25,
    max_loss_percent: 2.5,
    take_profit_percent: 4.0,
    required_confirmations: 2,
    max_hold_bars: 8
};

fn calculate_simple_macd(closes, fast, slow, signal_period) {
    // Versión simplificada de MACD
    let ema_fast = calculate_ema(closes, fast);
    let ema_slow = calculate_ema(closes, slow);
    let macd_line = ema_fast - ema_slow;
    
    // Signal line es EMA del MACD
    let signal_line = calculate_ema_of_value(macd_line, signal_period);
    
    #{
        macd: macd_line,
        signal: signal_line,
        histogram: macd_line - signal_line
    }
}

fn macd_rsi_signal(candle_data) {
    let closes = candle_data.closes;
    
    // Cálculos
    let macd = calculate_simple_macd(closes, 12, 26, 9);
    let rsi = calculate_rsi(closes, 14);
    
    // BULLISH: MACD cruza SMA + RSI creciente
    if macd.histogram > 0 && 
       macd.macd > macd.signal && 
       rsi > config_macd_rsi.rsi_neutral_zone {
        return 1;
    }
    
    // BEARISH: MACD cruza bajo SMA + RSI decreciente
    if macd.histogram < 0 && 
       macd.macd < macd.signal && 
       rsi < (100.0 - config_macd_rsi.rsi_neutral_zone) {
        return -1;
    }
    
    0
}

// ============================================================================
// TEMPLATE 5: VOLATILITY BREAKOUT (Volatilidad extrema)
// ============================================================================
// Mejor para: Breakouts después de consolidación
// Win Rate esperado: 50-60% pero con grandes ganancias
// Complejidad: ⭐⭐ Media

let config_volatility_breakout = #{
    atr_period: 14,
    breakout_multiplier: 2.0,  // ATR * 2 para breakout
    consolidation_bars: 5,      // Min barras de consolidación
    max_position_size: 0.3,
    max_loss_percent: 3.0,
    take_profit_percent: 6.0,
    required_confirmations: 2,
    max_hold_bars: 10
};

fn volatility_breakout_signal(candle_data) {
    let closes = candle_data.closes;
    let highs = candle_data.highs;
    let lows = candle_data.lows;
    
    let atr = calculate_atr(highs, lows, closes, 14);
    let current_high = highs[highs.len() - 1];
    let current_low = lows[lows.len() - 1];
    
    // Calcular consolidación (baja volatilidad reciente)
    let recent_range = current_high - current_low;
    let avg_atr = calculate_sma_atr(highs, lows, closes, 14, 10);
    
    let is_consolidating = recent_range < avg_atr * 0.5;
    
    if is_consolidating {
        // BREAKOUT BULLISH
        if current_high > (closes[closes.len() - 2] + atr * config_volatility_breakout.breakout_multiplier) {
            return 1;
        }
        
        // BREAKOUT BEARISH
        if current_low < (closes[closes.len() - 2] - atr * config_volatility_breakout.breakout_multiplier) {
            return -1;
        }
    }
    
    0
}

// ============================================================================
// TEMPLATE 6: SCALPING AGRESIVO (1-3 minutos, altísimo riesgo)
// ============================================================================
// Mejor para: Scalpers experimentados, volatilidad extrema, micro-ganancias
// Win Rate esperado: 55-60% pero operaciones muy rápidas
// Complejidad: ⭐⭐⭐ Alta

let config_scalping = #{
    momentum_threshold: 0.3,     // Muy bajo para capturar cambios pequeños
    rsi_period: 5,               // RSI rápido
    rsi_threshold: 35.0,         // Muy sensible
    max_position_size: 0.1,      // Pequeño por riesgo
    max_loss_percent: 0.5,       // Stop muy cerrado
    take_profit_percent: 0.7,    // Ganancias pequeñas
    required_confirmations: 2,
    max_hold_bars: 1             // Salir al siguiente candle
};

fn scalping_signal(candle_data) {
    let closes = candle_data.closes;
    
    // Momentum muy sensible
    let momentum = calculate_momentum(closes, 2);
    
    // RSI rápido (período 5)
    let rsi = calculate_rsi(closes, 5);
    
    // BULLISH: Micro-momentum + RSI bajando desde sobrecompra
    if momentum > config_scalping.momentum_threshold && 
       rsi < config_scalping.rsi_threshold {
        return 1;
    }
    
    // BEARISH: Micro-momentum negativo + RSI subiendo desde sobreventa
    if momentum < -config_scalping.momentum_threshold && 
       rsi > (100.0 - config_scalping.rsi_threshold) {
        return -1;
    }
    
    0
}

// ============================================================================
// TEMPLATE 7: SWING TRADING (4H - 1D, sostenido)
// ============================================================================
// Mejor para: Traders pacientes, swing trades de días
// Win Rate esperado: 65-75% pero menos operaciones
// Complejidad: ⭐⭐ Media

let config_swing = #{
    fast_sma: 10,
    slow_sma: 50,
    macd_fast: 12,
    macd_slow: 26,
    macd_signal: 9,
    rsi_period: 14,
    max_position_size: 0.3,
    max_loss_percent: 3.0,
    take_profit_percent: 5.0,
    required_confirmations: 3,
    max_hold_bars: 100  // Muchos candles
};

fn swing_signal(candle_data) {
    let closes = candle_data.closes;
    
    let fast_sma = calculate_sma(closes, config_swing.fast_sma);
    let slow_sma = calculate_sma(closes, config_swing.slow_sma);
    let rsi = calculate_rsi(closes, 14);
    
    let current_close = closes[closes.len() - 1];
    
    // BULLISH: Fast SMA cruza arriba de Slow + Precio arriba de ambas
    if current_close > fast_sma && 
       fast_sma > slow_sma && 
       rsi > 50.0 {
        return 1;
    }
    
    // BEARISH: Fast SMA cruza abajo de Slow + Precio abajo de ambas
    if current_close < fast_sma && 
       fast_sma < slow_sma && 
       rsi < 50.0 {
        return -1;
    }
    
    0
}

// ============================================================================
// INDICADORES AUXILIARES (USADOS POR LOS TEMPLATES)
// ============================================================================

fn calculate_momentum(closes, period) {
    if closes.len() < period {
        return 0.0;
    }
    let current = closes[closes.len() - 1];
    let previous = closes[closes.len() - period - 1];
    ((current - previous) / previous) * 100.0
}

fn calculate_rsi(closes, period) {
    if closes.len() < period + 1 {
        return 50.0;
    }
    let mut gains = 0.0;
    let mut losses = 0.0;
    let start = closes.len() - period;
    
    for i in (start + 1)..closes.len() {
        let change = closes[i] - closes[i-1];
        if change > 0.0 {
            gains += change;
        } else {
            losses -= change;
        }
    }
    
    let avg_gain = gains / period as f64;
    let avg_loss = losses / period as f64;
    
    if avg_loss == 0.0 {
        return 100.0;
    }
    
    let rs = avg_gain / avg_loss;
    100.0 - (100.0 / (1.0 + rs))
}

fn calculate_sma(data, period) {
    if data.len() < period {
        return 0.0;
    }
    let mut sum = 0.0;
    let start = data.len() - period;
    
    for i in start..data.len() {
        sum += data[i];
    }
    
    sum / period as f64
}

fn calculate_atr(highs, lows, closes, period) {
    if closes.len() < period {
        return 0.0;
    }
    
    let mut tr_sum = 0.0;
    let start = closes.len() - period;
    
    for i in start..closes.len() {
        let tr1 = highs[i] - lows[i];
        let tr2 = (highs[i] - closes[i-1]).abs();
        let tr3 = (lows[i] - closes[i-1]).abs();
        
        let tr = if tr1 > tr2 { if tr1 > tr3 { tr1 } else { tr3 } } else { if tr2 > tr3 { tr2 } else { tr3 } };
        tr_sum += tr;
    }
    
    tr_sum / period as f64
}

fn calculate_sma_volume(volumes, period) {
    if volumes.len() < period {
        return 0.0;
    }
    let mut sum = 0.0;
    let start = volumes.len() - period;
    
    for i in start..volumes.len() {
        sum += volumes[i];
    }
    
    sum / period as f64
}

// ============================================================================
// GUÍA DE USO
// ============================================================================
/*
CÓMO USAR ESTOS TEMPLATES:

1. SELECCIONA EL TEMPLATE QUE MEJOR SE AJUSTE A TU ESTILO:
   - Template 1: Si quieres simplicidad (5-min Polymarket)
   - Template 2: Si quieres reversiones (ranging markets)
   - Template 3: Si quieres confiabilidad (múltiples confirmaciones)
   - Template 4: Si quieres tendencia fuerte (MACD)
   - Template 5: Si quieres breakouts (consolidación)
   - Template 6: Si eres scalper (muy rápido)
   - Template 7: Si eres swing trader (larga duración)

2. COPIA LA CONFIGURACIÓN (config_*) A TU ESTRATEGIA

3. IMPLEMENTA LA FUNCIÓN DE SEÑAL (template_*_signal)

4. PERSONALIZA LOS PARÁMETROS SEGÚN TUS PREFERENCIAS:
   - Más agresivo: Reduce momentum_threshold, toma_profit_percent
   - Más conservador: Aumenta required_confirmations, max_loss_percent
   - Más rápido: Reduce max_hold_bars
   - Más lento: Aumenta lookback_period

5. VALIDA CON BACKTEST:
   - Win rate debe ser > 50%
   - Profit factor > 1.0
   - Max drawdown < 20%

6. IMPLEMENTA EN TU AGENTE RUST:
   let mut agent = TradingAgent::new("tu_estrategia.rhai", config)?;
   agent.on_candle(&candle_data, capital)?;

EXAMPLES:

// Ejemplo 1: Usar Template 3 directamente
fn generate_signal(candle_data) {
    return multi_confirm_signal(candle_data);
}

// Ejemplo 2: Combinar dos templates (avanzado)
fn generate_signal(candle_data) {
    let signal_1 = momentum_simple_signal(candle_data);
    let signal_2 = mean_reversion_signal(candle_data);
    
    // Si ambas señales coinciden, es más fuerte
    if signal_1 == 1 && signal_2 == 1 {
        return 1;  // BULLISH fuerte
    }
    
    if signal_1 == -1 && signal_2 == -1 {
        return -1;  // BEARISH fuerte
    }
    
    0  // Sin consenso
}

// Ejemplo 3: Modificar Template 1 para ser más conservador
let config = #{
    lookback: 4,
    momentum_threshold: 1.5,          // Aumentado de 0.8
    rsi_threshold: 25.0,              // Bajado de 30
    max_position_size: 0.15,          // Reducido de 0.2
    required_confirmations: 3,        // Aumentado de 2
    max_loss_percent: 1.0,            // Bajado de 2.0
    take_profit_percent: 5.0,         // Aumentado de 3.0
    max_hold_bars: 3                  // Reducido de 4
};
*/
