// main.rs - Ejemplo de aplicación del Trading Agent

use std::error::Error;
use trading_agent_lib::{
    CandleData, MarketConfig, StrategyConfig, TradingAgent
};

/// Estructura para simular datos de mercado
struct MarketSimulator {
    candles: Vec<(f64, f64, f64, f64, f64)>,  // open, high, low, close, volume
}

impl MarketSimulator {
    fn new() -> Self {
        // Datos de ejemplo: 100 velas de BTC generadas sintéticamente
        let mut candles = Vec::new();
        let mut price = 40000.0;
        
        for i in 0..100 {
            let change = (i as f64 * 0.001 * (i as f64 % 10.0 - 5.0)).sin() * 50.0;
            let open = price;
            let close = price + change;
            let high = open.max(close) + 30.0;
            let low = open.min(close) - 30.0;
            let volume = 1000.0 + (i as f64 * 10.0) % 5000.0;
            
            candles.push((open, high, low, close, volume));
            price = close;
        }
        
        MarketSimulator { candles }
    }
    
    fn get_candle_range(&self, start: usize, end: usize) -> CandleData {
        let mut candle_data = CandleData::new();
        
        for i in start..end.min(self.candles.len()) {
            let (o, h, l, c, v) = self.candles[i];
            candle_data.add_candle(o, h, l, c, v);
        }
        
        candle_data
    }
}

/// Ejecutar backtest simple
async fn run_backtest() -> Result<(), Box<dyn Error>> {
    println!("🚀 Iniciando Backtest con Polymarket 4-Minute Strategy\n");
    
    // Configurar mercado
    let market_config = MarketConfig {
        market_id: "BTC-5M-POLYMARKET".to_string(),
        market_name: "Bitcoin 5-Minute".to_string(),
        timeframe: "5m".to_string(),
        asset: "BTC".to_string(),
        initial_capital: 10000.0,
    };
    
    // Crear agente
    let mut agent = TradingAgent::new("strategy.rhai", market_config.clone())?;
    
    println!("✓ Agente creado");
    println!("  Market: {}", market_config.market_name);
    println!("  Capital Inicial: ${}\n", market_config.initial_capital);
    
    // Configuración personalizada (opcional)
    let custom_config = StrategyConfig {
        momentum_threshold: 0.8,
        take_profit_percent: 3.0,
        max_loss_percent: 2.0,
        max_position_size: 0.2,
        required_confirmations: 3,
        ..Default::default()
    };
    
    agent.set_config(custom_config)?;
    println!("✓ Configuración actualizada\n");
    
    // Simular datos de mercado
    let simulator = MarketSimulator::new();
    println!("✓ {} velas de mercado generadas\n", simulator.candles.len());
    
    // Procesar velas en chunks (para simular streaming de datos)
    let window_size = 20;  // Ventana deslizante de 20 velas
    let step = 1;         // Avanzar 1 vela por iteración
    
    for i in (0..(simulator.candles.len() - window_size)).step_by(step) {
        let candle_data = simulator.get_candle_range(i, i + window_size);
        
        // Procesar candle
        match agent.on_candle(&candle_data, market_config.initial_capital) {
            Ok(result) => {
                if i % 10 == 0 {
                    println!("📊 Vela {}: Procesada", i);
                }
            }
            Err(e) => {
                eprintln!("❌ Error en vela {}: {}", i, e);
            }
        }
    }
    
    println!("\n✓ Backtest completado\n");
    
    // Mostrar estadísticas finales
    println!("=" .repeat(60));
    println!("ESTADÍSTICAS FINALES");
    println!("=" .repeat(60));
    
    match agent.get_stats() {
        Ok(stats) => {
            println!("Total de Trades:      {}", stats.total_trades);
            println!("Trades Ganadores:     {}", stats.wins);
            println!("Trades Perdedores:    {}", stats.losses);
            println!("Win Rate:             {:.2}%", stats.win_rate);
            println!("P&L Total:            {:.2}%", stats.total_pnl);
            println!("Posiciones Abiertas:  {}", stats.open_positions);
            println!("=" .repeat(60));
            
            // Evaluación
            println!("\n📈 EVALUACIÓN:");
            if stats.win_rate >= 60.0 {
                println!("✅ Win rate bueno (>=60%)");
            } else if stats.win_rate >= 50.0 {
                println!("⚠️  Win rate marginal (50-60%)");
            } else {
                println!("❌ Win rate bajo (<50%)");
            }
        }
        Err(e) => {
            eprintln!("Error obteniendo estadísticas: {}", e);
        }
    }
    
    // Exportar datos
    match agent.export_stats_json() {
        Ok(json) => {
            println!("\n📄 Estadísticas en JSON:\n{}\n", json);
        }
        Err(e) => {
            eprintln!("Error exportando JSON: {}", e);
        }
    }
    
    // Guardar estado
    if let Err(e) = agent.save_state("bot_state.json") {
        eprintln!("Advertencia: No se pudo guardar estado: {}", e);
    } else {
        println!("✓ Estado guardado en bot_state.json");
    }
    
    Ok(())
}

/// Ejecutar modo live trading (simulado)
async fn run_live_simulation() -> Result<(), Box<dyn Error>> {
    println!("🔴 Iniciando Live Trading Simulation\n");
    
    let market_config = MarketConfig {
        market_id: "BTC-5M-LIVE".to_string(),
        market_name: "Bitcoin 5-Minute Live".to_string(),
        timeframe: "5m".to_string(),
        asset: "BTC".to_string(),
        initial_capital: 5000.0,
    };
    
    let mut agent = TradingAgent::new("strategy.rhai", market_config.clone())?;
    
    println!("✓ Agente en vivo creado");
    println!("  Capital: ${}\n", market_config.initial_capital);
    
    // Simular stream de datos
    let simulator = MarketSimulator::new();
    
    println!("Procesando datos en tiempo real...\n");
    
    for i in 0..20 {
        // Simular llegada de datos cada segundo
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        let candle_data = simulator.get_candle_range(i.max(20), (i + 20).min(100));
        
        if !candle_data.is_empty() {
            let last_close = candle_data.closes.last().unwrap_or(&0.0);
            
            match agent.on_candle(&candle_data, market_config.initial_capital) {
                Ok(_) => {
                    println!("[{}] Precio: ${:.2}", i, last_close);
                    
                    // Mostrar posiciones cada 5 velas
                    if i % 5 == 0 {
                        let positions = agent.get_positions();
                        if !positions.is_empty() {
                            println!("  📈 {} posición(es) abierta(s)", positions.len());
                        }
                    }
                }
                Err(e) => {
                    eprintln!("❌ Error: {}", e);
                }
            }
        }
    }
    
    // Estadísticas finales
    println!("\n✓ Simulación completada\n");
    
    if let Ok(stats) = agent.get_stats() {
        println!("Resumen: {} trades, {:.2}% win rate", stats.total_trades, stats.win_rate);
    }
    
    Ok(())
}

/// Menu principal
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("\n");
    println!("╔═══════════════════════════════════════════════════════╗");
    println!("║  POLYMARKET TRADING AGENT - RHAI SCRIPT ENGINE      ║");
    println!("║  v0.1.0                                             ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");
    
    // Seleccionar modo
    println!("Selecciona modo de ejecución:");
    println!("1) Backtest (análisis histórico)");
    println!("2) Live Simulation (streaming en tiempo real)");
    println!("3) Salir\n");
    
    // Para este ejemplo, ejecutar ambos
    println!("Ejecutando Backtest...\n");
    
    match run_backtest().await {
        Ok(_) => {
            println!("\n✓ Backtest finalizado exitosamente");
        }
        Err(e) => {
            eprintln!("❌ Error en backtest: {}", e);
        }
    }
    
    println!("\n");
    println!("-".repeat(60));
    println!("\nEjecutando Live Simulation...\n");
    
    match run_live_simulation().await {
        Ok(_) => {
            println!("\n✓ Simulación en vivo finalizada exitosamente");
        }
        Err(e) => {
            eprintln!("❌ Error en simulación: {}", e);
        }
    }
    
    println!("\n");
    println!("=" .repeat(60));
    println!("🎉 Ejecución completada");
    println!("=" .repeat(60));
    println!("\n");
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_market_simulator() {
        let sim = MarketSimulator::new();
        assert_eq!(sim.candles.len(), 100);
    }

    #[tokio::test]
    async fn test_backtest_flow() {
        // Este test requiere el archivo strategy.rhai
        // En ambiente real, ejecutaría el backtest completo
    }
}
