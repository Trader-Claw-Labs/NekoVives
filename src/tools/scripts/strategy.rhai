// ============================================================================
// POLYMARKET 4-MINUTE TRADING STRATEGY - RHAI SCRIPT
// ============================================================================
// Estrategia de trading embebida en Rhai para ejecución en agente Rust

// ==================== TIPOS Y ESTRUCTURAS ====================

// Resultado de señal de trading
let SIGNAL_NEUTRAL = 0;
let SIGNAL_BULLISH = 1;
let SIGNAL_BEARISH = -1;

// Estados de posición
let POSITION_NONE = 0;
let POSITION_LONG = 1;
let POSITION_SHORT = -1;

// ==================== CONFIGURACIÓN ====================

let config = #{
    // Core Strategy
    lookback_4: 4,
    lookback_14: 14,
    momentum_threshold: 0.8,      // Aumentado de 0.3%
    rsi_threshold: 30.0,
    atr_multiplier: 1.5,
    
    // Risk Management
    max_position_size: 0.2,        // 20% del capital
    max_loss_percent: 2.0,
    take_profit_percent: 3.0,
    max_positions: 3,
    
    // Confirmations
    min_candles: 20,
    min_volume_threshold: 1.2,     // 20% arriba promedio
    required_confirmations: 3,      // De 4 posibles
    
    // Time Management
    max_hold_bars: 5
};

// Estado global del bot
let bot_state = #{
    positions: [],
    trades_log: [],
    current_bar: 0,
    total_pnl: 0.0,
    win_count: 0,
    loss_count: 0
};

// ==================== INDICADORES TÉCNICOS ====================

// Calcula Simple Moving Average
fn calculate_sma(closes, period) {
    if closes.len() < period {
        return 0.0;
    }
    
    let mut sum = 0.0;
    let start = closes.len() - period;
    
    for i in start..closes.len() {
        sum += closes[i];
    }
    
    sum / period
}

// Calcula Average True Range
fn calculate_atr(highs, lows, closes, period) {
    if closes.len() < period {
        return 0.0;
    }
    
    let mut tr_values = [];
    let start = closes.len() - period;
    
    for i in start..closes.len() {
        let high = highs[i];
        let low = lows[i];
        let prev_close = if i > 0 { closes[i-1] } else { closes[i] };
        
        let tr1 = high - low;
        let tr2 = (high - prev_close).abs();
        let tr3 = (low - prev_close).abs();
        
        let tr = if tr1 > tr2 {
            if tr1 > tr3 { tr1 } else { tr3 }
        } else {
            if tr2 > tr3 { tr2 } else { tr3 }
        };
        
        tr_values.push(tr);
    }
    
    let mut atr_sum = 0.0;
    for tr in tr_values {
        atr_sum += tr;
    }
    
    atr_sum / period
}

// Calcula Relative Strength Index (RSI)
fn calculate_rsi(closes, period) {
    if closes.len() < period + 1 {
        return 50.0;  // Neutral si no hay datos suficientes
    }
    
    let mut gains = 0.0;
    let mut losses = 0.0;
    let start = closes.len() - period;
    
    for i in (start + 1)..closes.len() {
        let change = closes[i] - closes[i-1];
        
        if change > 0.0 {
            gains += change;
        } else {
            losses -= change;  // losses es negativo
        }
    }
    
    let avg_gain = gains / period;
    let avg_loss = losses / period;
    
    if avg_loss == 0.0 {
        return 100.0;
    }
    
    let rs = avg_gain / avg_loss;
    100.0 - (100.0 / (1.0 + rs))
}

// Calcula Momentum (% cambio)
fn calculate_momentum(closes, period) {
    if closes.len() < period {
        return 0.0;
    }
    
    let current = closes[closes.len() - 1];
    let previous = closes[closes.len() - period - 1];
    
    ((current - previous) / previous) * 100.0
}

// Calcula volumen promedio
fn calculate_avg_volume(volumes, period) {
    if volumes.len() < period {
        return 0.0;
    }
    
    let mut sum = 0.0;
    let start = volumes.len() - period;
    
    for i in start..volumes.len() {
        sum += volumes[i];
    }
    
    sum / period
}

// ==================== GENERACIÓN DE SEÑALES ====================

// Cuenta confirmaciones alcistas (0-4)
fn get_bullish_confirmations(candle_data, momentum_4, momentum_1, rsi, current_volume, avg_volume) {
    let mut confirmations = 0;
    
    // 1. Momentum 4-candles positivo
    if momentum_4 > config.momentum_threshold {
        confirmations += 1;
    }
    
    // 2. Momentum 1-candle positivo
    if momentum_1 > 0.2 {
        confirmations += 1;
    }
    
    // 3. RSI en zona de sobreventa
    if rsi < config.rsi_threshold {
        confirmations += 1;
    }
    
    // 4. Volumen confirmado
    if current_volume > (avg_volume * config.min_volume_threshold) {
        confirmations += 1;
    }
    
    confirmations
}

// Cuenta confirmaciones bajistas (0-4)
fn get_bearish_confirmations(candle_data, momentum_4, momentum_1, rsi, current_volume, avg_volume) {
    let mut confirmations = 0;
    
    // 1. Momentum 4-candles negativo
    if momentum_4 < -config.momentum_threshold {
        confirmations += 1;
    }
    
    // 2. Momentum 1-candle negativo
    if momentum_1 < -0.2 {
        confirmations += 1;
    }
    
    // 3. RSI en zona de sobrecompra
    if rsi > (100.0 - config.rsi_threshold) {
        confirmations += 1;
    }
    
    // 4. Volumen confirmado
    if current_volume > (avg_volume * config.min_volume_threshold) {
        confirmations += 1;
    }
    
    confirmations
}

// Genera señal de trading
fn generate_signal(candle_data) {
    // Extraer datos
    let closes = candle_data.closes;
    let highs = candle_data.highs;
    let lows = candle_data.lows;
    let volumes = candle_data.volumes;
    let current_close = closes[closes.len() - 1];
    let current_volume = volumes[volumes.len() - 1];
    
    // Validar datos suficientes
    if closes.len() < config.min_candles {
        return SIGNAL_NEUTRAL;
    }
    
    // Calcular indicadores
    let momentum_4 = calculate_momentum(closes, config.lookback_4);
    let momentum_1 = calculate_momentum(closes, 1);
    let sma_14 = calculate_sma(closes, config.lookback_14);
    let rsi = calculate_rsi(closes, config.lookback_14);
    let avg_volume = calculate_avg_volume(volumes, 20);
    
    // Obtener confirmaciones
    let bullish_count = get_bullish_confirmations(
        candle_data, momentum_4, momentum_1, rsi, current_volume, avg_volume
    );
    
    let bearish_count = get_bearish_confirmations(
        candle_data, momentum_4, momentum_1, rsi, current_volume, avg_volume
    );
    
    // Generar señal
    if bullish_count >= config.required_confirmations && current_close > sma_14 {
        return SIGNAL_BULLISH;
    } else if bearish_count >= config.required_confirmations && current_close < sma_14 {
        return SIGNAL_BEARISH;
    }
    
    SIGNAL_NEUTRAL
}

// ==================== GESTIÓN DE POSICIONES ====================

// Calcula tamaño de posición
fn calculate_position_size(capital, stop_loss_distance) {
    if stop_loss_distance <= 0.0 {
        return 0.0;
    }
    
    let risk_amount = capital * config.max_position_size / 100.0;
    risk_amount / stop_loss_distance
}

// Procesa entrada de posición
fn process_entry(signal, candle_data, capital) {
    if signal == SIGNAL_NEUTRAL {
        return;
    }
    
    // Verificar límite de posiciones
    if bot_state.positions.len() >= config.max_positions {
        return;
    }
    
    let closes = candle_data.closes;
    let current_price = closes[closes.len() - 1];
    let atr = calculate_atr(candle_data.highs, candle_data.lows, closes, 14);
    
    let stop_loss_distance = atr * config.atr_multiplier;
    let position_size = calculate_position_size(capital, stop_loss_distance);
    
    if position_size <= 0.0 {
        return;
    }
    
    let position = #{
        side: if signal == SIGNAL_BULLISH { "long" } else { "short" },
        entry_price: current_price,
        entry_bar: bot_state.current_bar,
        position_size: position_size,
        stop_loss: if signal == SIGNAL_BULLISH { 
            current_price - stop_loss_distance 
        } else { 
            current_price + stop_loss_distance 
        },
        take_profit: if signal == SIGNAL_BULLISH { 
            current_price + (atr * 2.0) 
        } else { 
            current_price - (atr * 2.0) 
        }
    };
    
    bot_state.positions.push(position);
}

// Procesa salida de posición
fn process_exit(position, current_price) {
    let entry_price = position.entry_price;
    let pnl_percent = ((current_price - entry_price) / entry_price) * 100.0;
    let bars_held = bot_state.current_bar - position.entry_bar;
    
    let should_exit = false;
    let exit_reason = "";
    
    // Check Stop Loss
    if position.side == "long" && current_price <= position.stop_loss {
        return #{
            should_exit: true,
            reason: "stop_loss",
            pnl_percent: pnl_percent,
            exit_price: current_price
        };
    }
    
    if position.side == "short" && current_price >= position.stop_loss {
        return #{
            should_exit: true,
            reason: "stop_loss",
            pnl_percent: pnl_percent,
            exit_price: current_price
        };
    }
    
    // Check Take Profit
    if position.side == "long" && current_price >= position.take_profit {
        return #{
            should_exit: true,
            reason: "take_profit",
            pnl_percent: pnl_percent,
            exit_price: current_price
        };
    }
    
    if position.side == "short" && current_price <= position.take_profit {
        return #{
            should_exit: true,
            reason: "take_profit",
            pnl_percent: pnl_percent,
            exit_price: current_price
        };
    }
    
    // Check Time-based exit
    if bars_held >= config.max_hold_bars {
        return #{
            should_exit: true,
            reason: "time_exit",
            pnl_percent: pnl_percent,
            exit_price: current_price
        };
    }
    
    #{
        should_exit: false,
        reason: "",
        pnl_percent: 0.0,
        exit_price: 0.0
    }
}

// ==================== LÓGICA PRINCIPAL ====================

// Función principal de procesamiento de candle
fn on_candle(candle_data, capital) {
    // Incrementar contador de barras
    bot_state.current_bar += 1;
    
    let current_price = candle_data.closes[candle_data.closes.len() - 1];
    
    // Procesar posiciones existentes
    let mut positions_to_remove = [];
    
    for i in range(0, bot_state.positions.len()) {
        let position = bot_state.positions[i];
        let exit_signal = process_exit(position, current_price);
        
        if exit_signal.should_exit {
            // Registrar trade
            let trade = #{
                entry_price: position.entry_price,
                exit_price: exit_signal.exit_price,
                side: position.side,
                pnl_percent: exit_signal.pnl_percent,
                reason: exit_signal.reason,
                entry_bar: position.entry_bar,
                exit_bar: bot_state.current_bar
            };
            
            bot_state.trades_log.push(trade);
            
            // Actualizar estadísticas
            bot_state.total_pnl += exit_signal.pnl_percent;
            
            if exit_signal.pnl_percent > 0.0 {
                bot_state.win_count += 1;
            } else {
                bot_state.loss_count += 1;
            }
            
            positions_to_remove.push(i);
        }
    }
    
    // Remover posiciones cerradas (en orden inverso para evitar issues de índice)
    for i in range(0, positions_to_remove.len()) {
        let remove_idx = positions_to_remove[positions_to_remove.len() - 1 - i];
        bot_state.positions.remove(remove_idx);
    }
    
    // Generar nueva señal
    let signal = generate_signal(candle_data);
    
    // Procesar entrada
    if signal != SIGNAL_NEUTRAL {
        process_entry(signal, candle_data, capital);
    }
}

// Función para obtener estadísticas
fn get_stats() {
    let total_trades = bot_state.win_count + bot_state.loss_count;
    let win_rate = if total_trades > 0 {
        (bot_state.win_count as float / total_trades as float) * 100.0
    } else {
        0.0
    };
    
    #{
        total_trades: total_trades,
        wins: bot_state.win_count,
        losses: bot_state.loss_count,
        win_rate: win_rate,
        total_pnl: bot_state.total_pnl,
        open_positions: bot_state.positions.len(),
        trades_log: bot_state.trades_log.clone()
    }
}

// Función para reset del bot
fn reset() {
    bot_state.positions.clear();
    bot_state.trades_log.clear();
    bot_state.current_bar = 0;
    bot_state.total_pnl = 0.0;
    bot_state.win_count = 0;
    bot_state.loss_count = 0;
}

// Función para actualizar configuración
fn set_config(new_config) {
    for key in new_config.keys() {
        config[key] = new_config[key];
    }
}

// Exportar funciones principales
export on_candle;
export get_stats;
export reset;
export set_config;
export generate_signal;
export calculate_position_size;
