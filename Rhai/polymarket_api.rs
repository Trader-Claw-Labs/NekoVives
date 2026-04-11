// polymarket_api.rs - Integración con Polymarket API
// Añadir este módulo al proyecto para trading en vivo

use reqwest::Client;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::error::Error;
use std::collections::HashMap;

/// Estructura de respuesta de mercado de Polymarket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Market {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub last_price: Option<f64>,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub volume: f64,
    pub resolved: bool,
    pub winner: Option<String>,
    pub timeframe: String,  // "5m", "15m", etc.
}

/// Estructura de vela histórica
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandleHistory {
    pub timestamp: u64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Cliente de Polymarket API
pub struct PolymartketClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

impl PolymartketClient {
    /// Crear nuevo cliente
    pub fn new(base_url: Option<String>, api_key: Option<String>) -> Self {
        let base_url = base_url.unwrap_or_else(|| "https://api.polymarket.com".to_string());
        
        PolymartketClient {
            client: Client::new(),
            base_url,
            api_key,
        }
    }

    /// Obtener lista de mercados disponibles
    pub async fn get_markets(&self, filter: Option<&str>) -> Result<Vec<Market>, Box<dyn Error>> {
        let mut url = format!("{}/markets", self.base_url);
        
        if let Some(f) = filter {
            url.push_str(&format!("?filter={}", f));
        }

        let response = self.client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        let markets = response.json::<Vec<Market>>().await?;
        Ok(markets)
    }

    /// Obtener datos de mercado específico
    pub async fn get_market(&self, market_id: &str) -> Result<Market, Box<dyn Error>> {
        let url = format!("{}/markets/{}", self.base_url, market_id);

        let response = self.client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        let market = response.json::<Market>().await?;
        Ok(market)
    }

    /// Obtener historial de precios (velas)
    pub async fn get_price_history(
        &self,
        market_id: &str,
        resolution: &str,
        limit: Option<u32>,
    ) -> Result<Vec<CandleHistory>, Box<dyn Error>> {
        let mut url = format!(
            "{}/markets/{}/price-history?resolution={}",
            self.base_url, market_id, resolution
        );

        if let Some(l) = limit {
            url.push_str(&format!("&limit={}", l));
        }

        let response = self.client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        let candles = response.json::<Vec<CandleHistory>>().await?;
        Ok(candles)
    }

    /// Colocar una apuesta (requiere autenticación)
    pub async fn place_bet(
        &self,
        market_id: &str,
        side: &str,      // "yes" o "no"
        amount: f64,
        price: f64,
    ) -> Result<OrderResponse, Box<dyn Error>> {
        if self.api_key.is_none() {
            return Err("API key required for trading".into());
        }

        let url = format!("{}/orders", self.base_url);

        let payload = serde_json::json!({
            "market_id": market_id,
            "side": side,
            "amount": amount,
            "price": price,
            "timestamp": Utc::now().to_rfc3339()
        });

        let response = self.client
            .post(&url)
            .bearer_auth(self.api_key.as_ref().unwrap())
            .json(&payload)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        let order = response.json::<OrderResponse>().await?;
        Ok(order)
    }

    /// Obtener posiciones del usuario
    pub async fn get_positions(&self) -> Result<Vec<Position>, Box<dyn Error>> {
        if self.api_key.is_none() {
            return Err("API key required".into());
        }

        let url = format!("{}/user/positions", self.base_url);

        let response = self.client
            .get(&url)
            .bearer_auth(self.api_key.as_ref().unwrap())
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        let positions = response.json::<Vec<Position>>().await?;
        Ok(positions)
    }

    /// Cerrar posición
    pub async fn close_position(&self, position_id: &str) -> Result<bool, Box<dyn Error>> {
        if self.api_key.is_none() {
            return Err("API key required".into());
        }

        let url = format!("{}/positions/{}/close", self.base_url, position_id);

        let response = self.client
            .post(&url)
            .bearer_auth(self.api_key.as_ref().unwrap())
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        Ok(response.status().is_success())
    }

    /// Obtener datos del oráculo (Chainlink)
    pub async fn get_oracle_price(&self, asset: &str) -> Result<OraclePrice, Box<dyn Error>> {
        let chainlink_url = match asset {
            "BTC" => "https://feeds.chain.link/btc-usd",
            "ETH" => "https://feeds.chain.link/eth-usd",
            "SOL" => "https://feeds.chain.link/sol-usd",
            _ => return Err(format!("Asset {} not supported", asset).into()),
        };

        let response = self.client
            .get(chainlink_url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        let oracle = response.json::<OraclePrice>().await?;
        Ok(oracle)
    }
}

/// Respuesta de orden colocada
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderResponse {
    pub order_id: String,
    pub status: String,
    pub amount: f64,
    pub price: f64,
    pub created_at: DateTime<Utc>,
}

/// Posición abierta en Polymarket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub position_id: String,
    pub market_id: String,
    pub side: String,
    pub quantity: f64,
    pub entry_price: f64,
    pub current_price: f64,
    pub pnl: f64,
    pub created_at: DateTime<Utc>,
}

/// Precio del oráculo Chainlink
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OraclePrice {
    pub price: f64,
    pub timestamp: u64,
    pub heartbeat: u64,
}

// ==================== STREAM DE DATOS EN VIVO ====================

use tokio::sync::mpsc;
use futures_util::stream::StreamExt;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Tipo de evento del stream
#[derive(Debug, Clone)]
pub enum StreamEvent {
    PriceUpdate {
        market_id: String,
        price: f64,
        timestamp: u64,
    },
    VolumeUpdate {
        market_id: String,
        volume: f64,
    },
    OrderFilled {
        order_id: String,
        market_id: String,
        amount: f64,
    },
    Error(String),
}

/// Market Data Stream - Conecta a WebSocket de Polymarket
pub struct MarketDataStream {
    ws_url: String,
    tx: mpsc::Sender<StreamEvent>,
}

impl MarketDataStream {
    pub fn new(ws_url: String, tx: mpsc::Sender<StreamEvent>) -> Self {
        MarketDataStream { ws_url, tx }
    }

    /// Conectar y comenzar a streamear datos
    pub async fn start(&self, market_ids: Vec<String>) -> Result<(), Box<dyn Error>> {
        // Construir URL con múltiples mercados
        let markets = market_ids.join(",");
        let url = format!("{}?markets={}", self.ws_url, markets);

        // Conectar a WebSocket
        let (ws_stream, _) = connect_async(&url).await?;
        println!("✓ Conectado a WebSocket: {}", url);

        let (mut write, mut read) = ws_stream.split();

        // Loop de lectura
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    // Parsear mensaje JSON
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                        // Procesar evento
                        let event = self.parse_event(&data);
                        let _ = self.tx.send(event).await;
                    }
                }
                Ok(Message::Close(_)) => {
                    println!("WebSocket cerrado por servidor");
                    break;
                }
                Err(e) => {
                    let event = StreamEvent::Error(format!("WebSocket error: {}", e));
                    let _ = self.tx.send(event).await;
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn parse_event(&self, data: &serde_json::Value) -> StreamEvent {
        match data.get("type").and_then(|v| v.as_str()) {
            Some("price_update") => {
                let market_id = data.get("market_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                
                let price = data.get("price")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                
                let timestamp = data.get("timestamp")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                StreamEvent::PriceUpdate { market_id, price, timestamp }
            }
            Some("volume_update") => {
                let market_id = data.get("market_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                
                let volume = data.get("volume")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);

                StreamEvent::VolumeUpdate { market_id, volume }
            }
            _ => {
                StreamEvent::Error("Unknown event type".to_string())
            }
        }
    }
}

// ==================== EJEMPLO DE USO ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_markets() {
        let client = PolymartketClient::new(None, None);
        
        // Este test requiere conexión a Polymarket
        // Comentar en ambiente offline
        
        // match client.get_markets(Some("5m")).await {
        //     Ok(markets) => {
        //         assert!(!markets.is_empty());
        //         println!("✓ {} mercados encontrados", markets.len());
        //     }
        //     Err(e) => {
        //         eprintln!("Error: {}", e);
        //     }
        // }
    }

    #[tokio::test]
    async fn test_oracle_price() {
        let client = PolymartketClient::new(None, None);
        
        // Obtener precio actual de BTC
        // match client.get_oracle_price("BTC").await {
        //     Ok(price) => {
        //         println!("BTC Price: ${:.2}", price.price);
        //     }
        //     Err(e) => {
        //         eprintln!("Error: {}", e);
        //     }
        // }
    }
}

// ==================== FUNCIONES AUXILIARES ====================

/// Convertir historial Polymarket a CandleData
pub fn candle_history_to_candle_data(
    candles: Vec<CandleHistory>,
) -> crate::CandleData {
    let mut data = crate::CandleData::new();
    
    for candle in candles {
        data.add_candle(
            candle.open,
            candle.high,
            candle.low,
            candle.close,
            candle.volume,
        );
    }
    
    data
}

/// Formatear precio con decimales
pub fn format_price(price: f64) -> String {
    format!("${:.2}", price)
}

/// Obtener múltiples mercados paralelo
pub async fn get_multiple_markets(
    client: &PolymartketClient,
    market_ids: Vec<&str>,
) -> Result<HashMap<String, Market>, Box<dyn Error>> {
    let mut markets = HashMap::new();
    
    for market_id in market_ids {
        match client.get_market(market_id).await {
            Ok(market) => {
                markets.insert(market_id.to_string(), market);
            }
            Err(e) => {
                eprintln!("Error fetching {}: {}", market_id, e);
            }
        }
    }
    
    Ok(markets)
}
