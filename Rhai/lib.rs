// lib.rs - Librería compartida del trading agent

pub mod agent;

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

pub use agent::TradingAgent;

/// Datos de una vela OHLCV
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandleData {
    pub timestamp: DateTime<Utc>,
    pub opens: Vec<f64>,
    pub highs: Vec<f64>,
    pub lows: Vec<f64>,
    pub closes: Vec<f64>,
    pub volumes: Vec<f64>,
}

impl CandleData {
    pub fn new() -> Self {
        CandleData {
            timestamp: Utc::now(),
            opens: Vec::new(),
            highs: Vec::new(),
            lows: Vec::new(),
            closes: Vec::new(),
            volumes: Vec::new(),
        }
    }

    /// Añade una vela a los datos
    pub fn add_candle(&mut self, open: f64, high: f64, low: f64, close: f64, volume: f64) {
        self.opens.push(open);
        self.highs.push(high);
        self.lows.push(low);
        self.closes.push(close);
        self.volumes.push(volume);
        self.timestamp = Utc::now();
    }

    /// Obtiene la última vela
    pub fn last_candle(&self) -> Option<(f64, f64, f64, f64, f64)> {
        if self.closes.is_empty() {
            return None;
        }
        let idx = self.closes.len() - 1;
        Some((
            self.opens[idx],
            self.highs[idx],
            self.lows[idx],
            self.closes[idx],
            self.volumes[idx],
        ))
    }

    /// Obtiene número de velas
    pub fn len(&self) -> usize {
        self.closes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.closes.is_empty()
    }
}

/// Posición abierta
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: String,
    pub side: String,              // "long" o "short"
    pub entry_price: f64,
    pub entry_bar: u32,
    pub position_size: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub entry_time: DateTime<Utc>,
}

/// Trade completado
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub side: String,
    pub pnl_percent: f64,
    pub pnl_absolute: f64,
    pub reason: String,
    pub entry_bar: u32,
    pub exit_bar: u32,
    pub entry_time: DateTime<Utc>,
    pub exit_time: DateTime<Utc>,
    pub duration_bars: u32,
}

/// Configuración de la estrategia
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    pub lookback_4: usize,
    pub lookback_14: usize,
    pub momentum_threshold: f64,
    pub rsi_threshold: f64,
    pub atr_multiplier: f64,
    pub max_position_size: f64,
    pub max_loss_percent: f64,
    pub take_profit_percent: f64,
    pub max_positions: usize,
    pub min_candles: usize,
    pub min_volume_threshold: f64,
    pub required_confirmations: i64,
    pub max_hold_bars: u32,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        StrategyConfig {
            lookback_4: 4,
            lookback_14: 14,
            momentum_threshold: 0.8,
            rsi_threshold: 30.0,
            atr_multiplier: 1.5,
            max_position_size: 0.2,
            max_loss_percent: 2.0,
            take_profit_percent: 3.0,
            max_positions: 3,
            min_candles: 20,
            min_volume_threshold: 1.2,
            required_confirmations: 3,
            max_hold_bars: 5,
        }
    }
}

/// Estado del bot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotState {
    pub positions: Vec<Position>,
    pub trades_log: Vec<Trade>,
    pub current_bar: u32,
    pub total_pnl: f64,
    pub win_count: u32,
    pub loss_count: u32,
}

impl Default for BotState {
    fn default() -> Self {
        BotState {
            positions: Vec::new(),
            trades_log: Vec::new(),
            current_bar: 0,
            total_pnl: 0.0,
            win_count: 0,
            loss_count: 0,
        }
    }
}

/// Estadísticas del trading
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingStats {
    pub total_trades: u32,
    pub wins: u32,
    pub losses: u32,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub open_positions: usize,
    pub profit_factor: f64,
    pub max_drawdown: f64,
}

/// Señal de trading
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Signal {
    Neutral = 0,
    Bullish = 1,
    Bearish = -1,
}

/// Resultado de una operación
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationResult {
    pub success: bool,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

impl OperationResult {
    pub fn success(message: impl Into<String>, data: Option<serde_json::Value>) -> Self {
        OperationResult {
            success: true,
            message: message.into(),
            data,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        OperationResult {
            success: false,
            message: message.into(),
            data: None,
        }
    }
}

/// Configuración del mercado
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketConfig {
    pub market_id: String,
    pub market_name: String,
    pub timeframe: String,  // "5m", "15m", etc.
    pub asset: String,      // "BTC", "ETH", etc.
    pub initial_capital: f64,
}

/// Evento de trading
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingEvent {
    pub event_type: String,  // "entry", "exit", "error", etc.
    pub timestamp: DateTime<Utc>,
    pub market_id: String,
    pub details: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candle_data() {
        let mut candles = CandleData::new();
        candles.add_candle(100.0, 101.0, 99.0, 100.5, 1000.0);
        
        assert_eq!(candles.len(), 1);
        assert!(!candles.is_empty());
        
        let last = candles.last_candle().unwrap();
        assert_eq!(last.3, 100.5);
    }

    #[test]
    fn test_strategy_config() {
        let config = StrategyConfig::default();
        assert_eq!(config.momentum_threshold, 0.8);
        assert_eq!(config.max_positions, 3);
    }
}
