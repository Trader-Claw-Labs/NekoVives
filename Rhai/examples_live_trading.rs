// example_live_trading.rs
// Ejemplo completo de trading en vivo contra Polymarket
// Ejecutar con: cargo run --example live_trading --release

use std::error::Error;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

// Para incluir estos módulos, añadir al Cargo.toml o crear como ejemplo

/// Estructura de configuración de trading en vivo
#[derive(Debug, Clone)]
pub struct LiveTradingConfig {
    pub market_ids: Vec<String>,
    pub initial_capital: f64,
    pub max_positions_per_market: usize,
    pub polling_interval_secs: u64,
    pub enable_trading: bool,      // false = paper trading
    pub log_file: String,
}

impl Default for LiveTradingConfig {
    fn default() -> Self {
        LiveTradingConfig {
            market_ids: vec!["BTC-5M".to_string(), "ETH-5M".to_string()],
            initial_capital: 10000.0,
            max_positions_per_market: 3,
            polling_interval_secs: 5,
            enable_trading: false,  // NUNCA habilitar sin testing completo
            log_file: "trading_log.csv".to_string(),
        }
    }
}

/// Agente de trading en vivo
pub struct LiveTradingAgent {
    config: LiveTradingConfig,
    market_data: HashMap<String, MarketSnapshot>,
    trades: Vec<TradeRecord>,
    capital: f64,
    pnl: f64,
}

#[derive(Clone, Debug)]
struct MarketSnapshot {
    market_id: String,
    last_price: f64,
    bid: f64,
    ask: f64,
    volume: f64,
    timestamp: u64,
}

#[derive(Clone, Debug)]
struct TradeRecord {
    market_id: String,
    side: String,        // "buy" o "sell"
    entry_price: f64,
    exit_price: Option<f64>,
    amount: f64,
    pnl_percent: f64,
    entry_time: u64,
    exit_time: Option<u64>,
}

impl LiveTradingAgent {
    pub fn new(config: LiveTradingConfig) -> Self {
        LiveTradingAgent {
            capital: config.initial_capital,
            config,
            market_data: HashMap::new(),
            trades: Vec::new(),
            pnl: 0.0,
        }
    }

    /// Actualizar datos de mercado desde Polymarket
    pub async fn update_market_data(
        &mut self,
        market_id: &str,
        snapshot: MarketSnapshot,
    ) {
        self.market_data.insert(market_id.to_string(), snapshot);
    }

    /// Ejecutar lógica de trading (simplificada)
    pub fn evaluate_and_trade(
        &mut self,
        market_id: &str,
        signal: TradeSignal,
    ) -> Option<TradeOrder> {
        if let Some(snapshot) = self.market_data.get(market_id) {
            let price = snapshot.last_price;
            
            match signal {
                TradeSignal::Buy => {
                    let order = TradeOrder {
                        market_id: market_id.to_string(),
                        side: "buy".to_string(),
                        price,
                        amount: self.calculate_position_size(),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                    };
                    
                    // En paper trading, registrar automáticamente
                    if !self.config.enable_trading {
                        self.record_trade_fill(&order);
                    }
                    
                    return Some(order);
                }
                TradeSignal::Sell => {
                    let order = TradeOrder {
                        market_id: market_id.to_string(),
                        side: "sell".to_string(),
                        price,
                        amount: self.calculate_position_size(),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                    };
                    
                    if !self.config.enable_trading {
                        self.record_trade_fill(&order);
                    }
                    
                    return Some(order);
                }
                TradeSignal::Hold => {
                    return None;
                }
            }
        }
        
        None
    }

    fn calculate_position_size(&self) -> f64 {
        // 5% del capital por posición
        self.capital * 0.05
    }

    fn record_trade_fill(&mut self, order: &TradeOrder) {
        let record = TradeRecord {
            market_id: order.market_id.clone(),
            side: order.side.clone(),
            entry_price: order.price,
            exit_price: None,
            amount: order.amount,
            pnl_percent: 0.0,
            entry_time: order.timestamp,
            exit_time: None,
        };
        
        self.trades.push(record);
    }

    pub fn get_stats(&self) -> TradingStats {
        let total_trades = self.trades.len();
        let winning_trades = self.trades.iter()
            .filter(|t| t.pnl_percent > 0.0)
            .count();
        
        let win_rate = if total_trades > 0 {
            (winning_trades as f64 / total_trades as f64) * 100.0
        } else {
            0.0
        };
        
        TradingStats {
            total_trades,
            winning_trades,
            win_rate,
            total_pnl_percent: self.pnl,
            capital: self.capital,
        }
    }

    pub fn log_trade(&self, order: &TradeOrder) {
        println!(
            "[{}] {} {} @ ${:.2}",
            order.market_id,
            order.side.to_uppercase(),
            order.amount,
            order.price
        );
    }

    pub fn save_trading_log(&self) -> Result<(), Box<dyn Error>> {
        use std::fs::File;
        use std::io::Write;

        let mut file = File::create(&self.config.log_file)?;
        
        // Escribir header
        writeln!(file, "market_id,side,entry_price,exit_price,amount,pnl_percent,entry_time,exit_time")?;
        
        // Escribir trades
        for trade in &self.trades {
            writeln!(
                file,
                "{},{},{:.2},{:.2},{:.4},{:.2},{},{}",
                trade.market_id,
                trade.side,
                trade.entry_price,
                trade.exit_price.unwrap_or(0.0),
                trade.amount,
                trade.pnl_percent,
                trade.entry_time,
                trade.exit_time.unwrap_or(0)
            )?;
        }
        
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TradeSignal {
    Buy,
    Sell,
    Hold,
}

#[derive(Debug, Clone)]
pub struct TradeOrder {
    pub market_id: String,
    pub side: String,
    pub price: f64,
    pub amount: f64,
    pub timestamp: u64,
}

#[derive(Debug)]
pub struct TradingStats {
    pub total_trades: usize,
    pub winning_trades: usize,
    pub win_rate: f64,
    pub total_pnl_percent: f64,
    pub capital: f64,
}

// ==================== MAIN EXAMPLE ====================

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("╔══════════════════════════════════════════════════════╗");
    println!("║  POLYMARKET LIVE TRADING AGENT - RUST + RHAI       ║");
    println!("║  v0.1.0 - Example                                 ║");
    println!("╚══════════════════════════════════════════════════════╝\n");

    // Configuración
    let config = LiveTradingConfig {
        market_ids: vec![
            "BTC-5M".to_string(),
            "ETH-5M".to_string(),
        ],
        initial_capital: 10000.0,
        max_positions_per_market: 3,
        polling_interval_secs: 5,
        enable_trading: false,  // Paper trading
        log_file: "trading_activity.csv".to_string(),
    };

    println!("📋 Configuración:");
    println!("  Markets: {}", config.market_ids.join(", "));
    println!("  Capital: ${:.2}", config.initial_capital);
    println!("  Mode: {}", if config.enable_trading { "LIVE TRADING" } else { "PAPER TRADING" });
    println!("  Interval: {}s\n", config.polling_interval_secs);

    let mut agent = LiveTradingAgent::new(config);

    // Simular datos de mercado
    println!("🔄 Iniciando polling de datos...\n");

    for iteration in 0..10 {
        println!("--- Iteración {} ---", iteration + 1);

        // Simular actualizaciones de datos
        let btc_snapshot = MarketSnapshot {
            market_id: "BTC-5M".to_string(),
            last_price: 40000.0 + (iteration as f64 * 100.0),
            bid: 39990.0,
            ask: 40010.0,
            volume: 1000000.0,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        let eth_snapshot = MarketSnapshot {
            market_id: "ETH-5M".to_string(),
            last_price: 2000.0 + (iteration as f64 * 5.0),
            bid: 1995.0,
            ask: 2005.0,
            volume: 500000.0,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        // Actualizar datos
        agent.update_market_data("BTC-5M", btc_snapshot.clone()).await;
        agent.update_market_data("ETH-5M", eth_snapshot.clone()).await;

        // Mostrar precios actuales
        println!("  BTC: ${:.2}", btc_snapshot.last_price);
        println!("  ETH: ${:.2}", eth_snapshot.last_price);

        // Generar señales (simulado)
        let btc_signal = if iteration % 3 == 0 { TradeSignal::Buy } else { TradeSignal::Hold };
        let eth_signal = if iteration % 4 == 0 { TradeSignal::Buy } else { TradeSignal::Hold };

        // Ejecutar órdenes
        if let Some(order) = agent.evaluate_and_trade("BTC-5M", btc_signal) {
            agent.log_trade(&order);
        }

        if let Some(order) = agent.evaluate_and_trade("ETH-5M", eth_signal) {
            agent.log_trade(&order);
        }

        println!();

        // Esperar antes de siguiente iteración
        if iteration < 9 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    // Estadísticas finales
    println!("✓ Polling completado\n");
    
    let stats = agent.get_stats();
    
    println!("═══════════════════════════════════════════════════════");
    println!("ESTADÍSTICAS FINALES");
    println!("═══════════════════════════════════════════════════════");
    println!("Total Trades:       {}", stats.total_trades);
    println!("Winning Trades:     {}", stats.winning_trades);
    println!("Win Rate:           {:.2}%", stats.win_rate);
    println!("Total P&L:          {:.2}%", stats.total_pnl_percent);
    println!("Capital:            ${:.2}", stats.capital);
    println!("═══════════════════════════════════════════════════════\n");

    // Guardar log
    match agent.save_trading_log() {
        Ok(_) => println!("✓ Log guardado en: {}", agent.config.log_file),
        Err(e) => eprintln!("Error guardando log: {}", e),
    }

    println!("\n🎉 Ejecución completada exitosamente\n");

    Ok(())
}

// ==================== TESTS ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_creation() {
        let config = LiveTradingConfig::default();
        let agent = LiveTradingAgent::new(config.clone());
        
        assert_eq!(agent.capital, config.initial_capital);
        assert_eq!(agent.trades.len(), 0);
    }

    #[test]
    fn test_position_sizing() {
        let config = LiveTradingConfig {
            initial_capital: 10000.0,
            ..Default::default()
        };
        
        let agent = LiveTradingAgent::new(config);
        let size = agent.calculate_position_size();
        
        assert_eq!(size, 500.0);  // 5% de 10000
    }

    #[tokio::test]
    async fn test_market_data_update() {
        let config = LiveTradingConfig::default();
        let mut agent = LiveTradingAgent::new(config);

        let snapshot = MarketSnapshot {
            market_id: "TEST-5M".to_string(),
            last_price: 100.0,
            bid: 99.0,
            ask: 101.0,
            volume: 1000.0,
            timestamp: 123456,
        };

        agent.update_market_data("TEST-5M", snapshot.clone()).await;

        assert!(agent.market_data.contains_key("TEST-5M"));
        assert_eq!(agent.market_data["TEST-5M"].last_price, 100.0);
    }

    #[test]
    fn test_trading_stats() {
        let config = LiveTradingConfig::default();
        let agent = LiveTradingAgent::new(config);

        let stats = agent.get_stats();
        assert_eq!(stats.total_trades, 0);
        assert_eq!(stats.win_rate, 0.0);
    }
}

// ==================== INSTRUCCIONES DE USO ====================

/*
PARA EJECUTAR ESTE EJEMPLO:

1. Crear archivo examples/live_trading.rs en el proyecto Rust

2. Ejecutar:
   cargo run --example live_trading --release

3. Para integrar con Polymarket real:
   - Reemplazar simulación de datos con llamadas a PolymartketClient
   - Usar TradingAgent para generar señales en lugar de simulado
   - Configurar API key en variables de entorno
   - Cambiar enable_trading a true solo después de backtest exitoso

ESTRUCTURA COMPLETA:

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // 1. Crear cliente de Polymarket
    let polymarket = PolymartketClient::new(None, Some(api_key));
    
    // 2. Crear agente Rhai
    let market_config = MarketConfig { /* ... */ };
    let mut trading_agent = TradingAgent::new("strategy.rhai", market_config)?;
    
    // 3. Crear agente live trading
    let live_config = LiveTradingConfig {
        market_ids: vec!["BTC-5M".to_string()],
        enable_trading: false,  // Comenzar con paper trading
        ..Default::default()
    };
    let mut live_agent = LiveTradingAgent::new(live_config);
    
    // 4. Loop principal
    loop {
        // Obtener datos de Polymarket
        for market_id in &live_agent.config.market_ids {
            let candles = polymarket.get_price_history(
                market_id,
                "5m",
                Some(50)
            ).await?;
            
            let candle_data = candle_history_to_candle_data(candles);
            
            // Generar señal con Rhai
            let signal = trading_agent.generate_signal(&candle_data)?;
            
            // Ejecutar trade
            match signal {
                Signal::Bullish => {
                    live_agent.evaluate_and_trade(market_id, TradeSignal::Buy);
                }
                Signal::Bearish => {
                    live_agent.evaluate_and_trade(market_id, TradeSignal::Sell);
                }
                Signal::Neutral => {
                    live_agent.evaluate_and_trade(market_id, TradeSignal::Hold);
                }
            }
        }
        
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
```

CHECKLIST ANTES DE LIVE TRADING:
- [ ] Backtest completado, win rate > 60%
- [ ] Paper trading 100+ trades exitoso
- [ ] Configuración guardada y validada
- [ ] API key asegurada (variables de entorno)
- [ ] Alertas configuradas
- [ ] Capital inicial pequeño ($100-500)
- [ ] Rollback procedure documentado
- [ ] Monitoreo 24/7
*/
