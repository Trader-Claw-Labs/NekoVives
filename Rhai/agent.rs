// agent.rs - Trading Agent que ejecuta scripts Rhai

use rhai::{Engine, Scope, Dynamic, Array, Map};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use serde_json::{json, Value};
use chrono::Utc;

use crate::{
    CandleData, Position, Trade, StrategyConfig, BotState, 
    TradingStats, Signal, OperationResult, MarketConfig, TradingEvent
};

/// Trading Agent principal
pub struct TradingAgent {
    engine: Engine,
    scope: Scope<'static>,
    config: StrategyConfig,
    state: Arc<Mutex<BotState>>,
    script: String,
    market_config: MarketConfig,
    event_log: Arc<Mutex<Vec<TradingEvent>>>,
}

impl TradingAgent {
    /// Crear nuevo agente
    pub fn new(script_path: &str, market_config: MarketConfig) -> Result<Self, Box<dyn std::error::Error>> {
        // Cargar script
        let script = std::fs::read_to_string(script_path)?;
        
        // Crear engine
        let mut engine = Engine::new();
        
        // Registrar funciones personalizadas de Rust para Rhai
        register_custom_functions(&mut engine);
        
        let scope = Scope::new();
        let config = StrategyConfig::default();
        let state = Arc::new(Mutex::new(BotState::default()));
        
        Ok(TradingAgent {
            engine,
            scope,
            config,
            state,
            script,
            market_config,
            event_log: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Actualizar configuración
    pub fn set_config(&mut self, config: StrategyConfig) -> Result<(), Box<dyn std::error::Error>> {
        self.config = config.clone();
        
        // Actualizar configuración en el script
        let config_map = serde_to_rhai_map(&config)?;
        self.scope.set_or_push("config", config_map);
        
        Ok(())
    }

    /// Procesar una vela
    pub fn on_candle(
        &mut self,
        candle: &CandleData,
        capital: f64,
    ) -> Result<OperationResult, Box<dyn std::error::Error>> {
        // Convertir CandleData a formato Rhai
        let candle_map = candle_data_to_rhai(candle)?;
        
        // Llamar función on_candle del script
        let result = self.engine.call_fn::<()>(
            &mut self.scope,
            &self.engine.compile(&self.script)?,
            "on_candle",
            (candle_map, capital),
        );

        match result {
            Ok(_) => {
                // Log event
                let event = TradingEvent {
                    event_type: "candle_processed".to_string(),
                    timestamp: Utc::now(),
                    market_id: self.market_config.market_id.clone(),
                    details: HashMap::new(),
                };
                self.log_event(event);

                Ok(OperationResult::success("Candle processed successfully", None))
            }
            Err(e) => {
                let event = TradingEvent {
                    event_type: "error".to_string(),
                    timestamp: Utc::now(),
                    market_id: self.market_config.market_id.clone(),
                    details: {
                        let mut map = HashMap::new();
                        map.insert("error".to_string(), e.to_string());
                        map
                    },
                };
                self.log_event(event);
                Err(format!("Error processing candle: {}", e).into())
            }
        }
    }

    /// Obtener estadísticas actual
    pub fn get_stats(&mut self) -> Result<TradingStats, Box<dyn std::error::Error>> {
        // Llamar función get_stats del script
        let stats_result = self.engine.call_fn::<Map>(
            &mut self.scope,
            &self.engine.compile(&self.script)?,
            "get_stats",
            (),
        );

        match stats_result {
            Ok(stats_map) => {
                let total_trades = stats_map.get("total_trades")
                    .and_then(|v| v.as_int())
                    .unwrap_or(0) as u32;
                
                let wins = stats_map.get("wins")
                    .and_then(|v| v.as_int())
                    .unwrap_or(0) as u32;
                
                let losses = stats_map.get("losses")
                    .and_then(|v| v.as_int())
                    .unwrap_or(0) as u32;
                
                let win_rate = if total_trades > 0 {
                    (wins as f64 / total_trades as f64) * 100.0
                } else {
                    0.0
                };
                
                let total_pnl = stats_map.get("total_pnl")
                    .and_then(|v| v.as_float())
                    .unwrap_or(0.0);

                Ok(TradingStats {
                    total_trades,
                    wins,
                    losses,
                    win_rate,
                    total_pnl,
                    open_positions: stats_map.get("open_positions")
                        .and_then(|v| v.as_int())
                        .unwrap_or(0) as usize,
                    profit_factor: 0.0,  // Calcular en script si es necesario
                    max_drawdown: 0.0,   // Calcular en script si es necesario
                })
            }
            Err(e) => Err(format!("Error getting stats: {}", e).into())
        }
    }

    /// Reset del bot
    pub fn reset(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Llamar función reset del script
        self.engine.call_fn::<()>(
            &mut self.scope,
            &self.engine.compile(&self.script)?,
            "reset",
            (),
        )?;

        // Reset state local
        *self.state.lock().unwrap() = BotState::default();

        Ok(())
    }

    /// Generar señal para candle data
    pub fn generate_signal(&mut self, candle: &CandleData) -> Result<Signal, Box<dyn std::error::Error>> {
        let candle_map = candle_data_to_rhai(candle)?;
        
        let signal_value = self.engine.call_fn::<i64>(
            &mut self.scope,
            &self.engine.compile(&self.script)?,
            "generate_signal",
            (candle_map,),
        )?;

        let signal = match signal_value {
            1 => Signal::Bullish,
            -1 => Signal::Bearish,
            _ => Signal::Neutral,
        };

        Ok(signal)
    }

    /// Obtener historial de trades
    pub fn get_trades(&self) -> Vec<Trade> {
        self.state.lock().unwrap().trades_log.clone()
    }

    /// Obtener posiciones abiertas
    pub fn get_positions(&self) -> Vec<Position> {
        self.state.lock().unwrap().positions.clone()
    }

    /// Obtener log de eventos
    pub fn get_events(&self) -> Vec<TradingEvent> {
        self.event_log.lock().unwrap().clone()
    }

    /// Log de evento interno
    fn log_event(&self, event: TradingEvent) {
        if let Ok(mut log) = self.event_log.lock() {
            log.push(event);
        }
    }

    /// Exportar estadísticas a JSON
    pub fn export_stats_json(&mut self) -> Result<String, Box<dyn std::error::Error>> {
        let stats = self.get_stats()?;
        let json = serde_json::to_string_pretty(&stats)?;
        Ok(json)
    }

    /// Guardar estado en archivo
    pub fn save_state(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let state = self.state.lock().unwrap().clone();
        let json = serde_json::to_string_pretty(&state)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Cargar estado desde archivo
    pub fn load_state(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let json = std::fs::read_to_string(path)?;
        let state: BotState = serde_json::from_str(&json)?;
        *self.state.lock().unwrap() = state;
        Ok(())
    }
}

// ==================== FUNCIONES AUXILIARES ====================

/// Convertir CandleData a Map de Rhai
fn candle_data_to_rhai(candle: &CandleData) -> Result<rhai::Map, Box<dyn std::error::Error>> {
    let mut map = rhai::Map::new();
    
    // Convertir vectores a arrays de Rhai
    let opens: Array = candle.opens.iter().map(|&x| Dynamic::from(x)).collect();
    let highs: Array = candle.highs.iter().map(|&x| Dynamic::from(x)).collect();
    let lows: Array = candle.lows.iter().map(|&x| Dynamic::from(x)).collect();
    let closes: Array = candle.closes.iter().map(|&x| Dynamic::from(x)).collect();
    let volumes: Array = candle.volumes.iter().map(|&x| Dynamic::from(x)).collect();
    
    map.insert("opens".into(), Dynamic::from(opens));
    map.insert("highs".into(), Dynamic::from(highs));
    map.insert("lows".into(), Dynamic::from(lows));
    map.insert("closes".into(), Dynamic::from(closes));
    map.insert("volumes".into(), Dynamic::from(volumes));
    map.insert("timestamp".into(), Dynamic::from(candle.timestamp.to_rfc3339()));
    
    Ok(map)
}

/// Convertir StrategyConfig a Map de Rhai
fn serde_to_rhai_map(config: &StrategyConfig) -> Result<rhai::Map, Box<dyn std::error::Error>> {
    let mut map = rhai::Map::new();
    
    map.insert("lookback_4".into(), Dynamic::from(config.lookback_4 as i64));
    map.insert("lookback_14".into(), Dynamic::from(config.lookback_14 as i64));
    map.insert("momentum_threshold".into(), Dynamic::from(config.momentum_threshold));
    map.insert("rsi_threshold".into(), Dynamic::from(config.rsi_threshold));
    map.insert("atr_multiplier".into(), Dynamic::from(config.atr_multiplier));
    map.insert("max_position_size".into(), Dynamic::from(config.max_position_size));
    map.insert("max_loss_percent".into(), Dynamic::from(config.max_loss_percent));
    map.insert("take_profit_percent".into(), Dynamic::from(config.take_profit_percent));
    map.insert("max_positions".into(), Dynamic::from(config.max_positions as i64));
    map.insert("min_candles".into(), Dynamic::from(config.min_candles as i64));
    map.insert("min_volume_threshold".into(), Dynamic::from(config.min_volume_threshold));
    map.insert("required_confirmations".into(), Dynamic::from(config.required_confirmations));
    map.insert("max_hold_bars".into(), Dynamic::from(config.max_hold_bars as i64));
    
    Ok(map)
}

/// Registrar funciones personalizadas de Rust para Rhai
fn register_custom_functions(engine: &mut Engine) {
    // Las funciones de cálculo están definidas en el script Rhai
    // Aquí se podrían añadir funciones Rust adicionales si es necesario
    
    // Ejemplo: función para logging desde Rhai
    engine.register_fn("log", |msg: String| {
        println!("[RHAI] {}", msg);
    });
    
    // Función para obtener timestamp actual
    engine.register_fn("now_timestamp", || {
        Utc::now().to_rfc3339()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_creation() {
        let market_config = MarketConfig {
            market_id: "BTC-5M".to_string(),
            market_name: "Bitcoin 5-Minute".to_string(),
            timeframe: "5m".to_string(),
            asset: "BTC".to_string(),
            initial_capital: 10000.0,
        };

        // Este test requeriría el archivo strategy.rhai
        // En un proyecto real, asegurate de tener el path correcto
        // let result = TradingAgent::new("strategy.rhai", market_config);
        // assert!(result.is_ok());
    }
}
