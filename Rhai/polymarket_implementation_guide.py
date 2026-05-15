# 📖 GUÍA PRÁCTICA: Implementación del Bot en Polymarket

## 🔌 Parte 1: Conectar a Polymarket API

### A. Requisitos Previos

```bash
# Instalar dependencias
pip install requests pandas numpy websockets web3 eth-account

# Para soporte completo
pip install python-binance ccxt backtrader ta-lib
```

### B. Obtener Credenciales

1. **Crear cuenta en Polymarket**
   - Ir a https://polymarket.com
   - Conectar wallet (MetaMask, WalletConnect, etc.)
   - Generar API key en Settings (si disponible)

2. **Configurar Chainlink Feed**
   - Para acceso directo a datos: https://feeds.chain.link/btc-usd
   - Para Polymarket: integrado via Oracle Polygon

### C. Script de Conexión Básica

```python
import requests
import json
from datetime import datetime
from typing import Dict, List

class PolymartketAPI:
    """Cliente para interactuar con Polymarket"""
    
    BASE_URL = "https://api.polymarket.com"
    
    def __init__(self, api_key: str = None):
        self.api_key = api_key
        self.session = requests.Session()
        if api_key:
            self.session.headers.update({"Authorization": f"Bearer {api_key}"})
    
    # ==================== MERCADOS ====================
    
    def get_markets(self, filter_type: str = "crypto") -> List[Dict]:
        """
        Obtiene lista de mercados disponibles
        
        Args:
            filter_type: "crypto", "5m", "15m", etc.
        """
        endpoint = f"{self.BASE_URL}/markets"
        params = {
            "filter": filter_type,
            "limit": 100
        }
        
        try:
            response = self.session.get(endpoint, params=params, timeout=10)
            response.raise_for_status()
            return response.json()
        except requests.RequestException as e:
            print(f"Error obteniendo mercados: {e}")
            return []
    
    def get_market_data(self, market_id: str) -> Dict:
        """Obtiene datos detallados de un mercado específico"""
        endpoint = f"{self.BASE_URL}/markets/{market_id}"
        
        try:
            response = self.session.get(endpoint, timeout=10)
            response.raise_for_status()
            return response.json()
        except requests.RequestException as e:
            print(f"Error obteniendo datos del mercado: {e}")
            return {}
    
    # ==================== PRECIOS ====================
    
    def get_market_price(self, market_id: str) -> float:
        """Obtiene precio actual (bid/ask) de un mercado"""
        data = self.get_market_data(market_id)
        
        if 'last_price' in data:
            return float(data['last_price'])
        elif 'bid' in data and 'ask' in data:
            return (float(data['bid']) + float(data['ask'])) / 2
        
        return None
    
    def get_price_history(self, market_id: str, 
                         resolution: str = "5m", 
                         limit: int = 100) -> List[Dict]:
        """
        Obtiene historial de precios
        
        Args:
            market_id: ID del mercado
            resolution: "5m", "15m", "1h", etc.
            limit: Número de velas a obtener
        """
        endpoint = f"{self.BASE_URL}/markets/{market_id}/price-history"
        params = {
            "resolution": resolution,
            "limit": limit
        }
        
        try:
            response = self.session.get(endpoint, params=params, timeout=10)
            response.raise_for_status()
            return response.json()
        except requests.RequestException as e:
            print(f"Error obteniendo historial: {e}")
            return []
    
    # ==================== TRADING ====================
    
    def place_bet(self, market_id: str, 
                 side: str, 
                 amount: float, 
                 price: float) -> Dict:
        """
        Coloca una apuesta (BUY YES o BUY NO)
        
        Args:
            market_id: ID del mercado
            side: "yes" o "no"
            amount: Cantidad en dólares
            price: Precio al que apostar
        """
        endpoint = f"{self.BASE_URL}/orders"
        
        payload = {
            "market_id": market_id,
            "side": side,
            "amount": amount,
            "price": price,
            "timestamp": datetime.now().isoformat()
        }
        
        try:
            response = self.session.post(
                endpoint, 
                json=payload, 
                timeout=10
            )
            response.raise_for_status()
            return response.json()
        except requests.RequestException as e:
            print(f"Error colocando apuesta: {e}")
            return {}
    
    def get_positions(self) -> List[Dict]:
        """Obtiene todas tus posiciones abiertas"""
        endpoint = f"{self.BASE_URL}/user/positions"
        
        try:
            response = self.session.get(endpoint, timeout=10)
            response.raise_for_status()
            return response.json()
        except requests.RequestException as e:
            print(f"Error obteniendo posiciones: {e}")
            return []
    
    def close_position(self, position_id: str) -> Dict:
        """Cierra una posición existente"""
        endpoint = f"{self.BASE_URL}/positions/{position_id}/close"
        
        try:
            response = self.session.post(endpoint, timeout=10)
            response.raise_for_status()
            return response.json()
        except requests.RequestException as e:
            print(f"Error cerrando posición: {e}")
            return {}
    
    # ==================== ORÁCULO ====================
    
    def get_oracle_data(self, asset: str = "BTC") -> Dict:
        """
        Obtiene datos del oráculo (Chainlink) actual
        Esto es CRÍTICO para predicciones 5-minuto
        """
        # En producción, conectar directamente a Chainlink
        endpoints = {
            "BTC": "https://feeds.chain.link/btc-usd",
            "ETH": "https://feeds.chain.link/eth-usd",
            "SOL": "https://feeds.chain.link/sol-usd"
        }
        
        endpoint = endpoints.get(asset)
        if not endpoint:
            return {}
        
        try:
            response = requests.get(endpoint, timeout=10)
            response.raise_for_status()
            # Parse response según formato Chainlink
            return response.json()
        except requests.RequestException as e:
            print(f"Error obteniendo datos oráculo: {e}")
            return {}

```

---

## 🤖 Parte 2: Bot Live Trading

```python
import asyncio
import websockets
import json
from datetime import datetime
import logging

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


class LiveTradingBot:
    """Bot que ejecuta la estrategia en tiempo real contra Polymarket"""
    
    def __init__(self, config: TradingConfig, api: PolymartketAPI):
        self.config = config
        self.api = api
        self.positions = {}  # Posiciones actuales por market
        self.pnl = {}        # Tracking de P&L
        self.running = False
    
    async def connect_to_market(self, market_id: str):
        """
        Conecta a websocket de Polymarket para datos en tiempo real
        """
        ws_url = f"wss://ws.polymarket.com/markets/{market_id}"
        
        try:
            async with websockets.connect(ws_url) as websocket:
                logger.info(f"✓ Conectado a {market_id}")
                
                while self.running:
                    try:
                        message = await asyncio.wait_for(
                            websocket.recv(), 
                            timeout=30
                        )
                        
                        data = json.loads(message)
                        await self.process_market_update(market_id, data)
                        
                    except asyncio.TimeoutError:
                        logger.warning(f"Timeout esperando datos de {market_id}")
                        continue
                    except Exception as e:
                        logger.error(f"Error procesando datos: {e}")
                        continue
        
        except Exception as e:
            logger.error(f"Error conectando a websocket: {e}")
    
    async def process_market_update(self, market_id: str, data: dict):
        """
        Procesa actualización de mercado y ejecuta lógica de trading
        """
        try:
            price = data.get('mid_price', data.get('last_price'))
            volume = data.get('volume', 0)
            timestamp = datetime.now()
            
            logger.debug(f"[{timestamp}] {market_id}: ${price}")
            
            # Evalúar si entra/sale de posición
            decision = self.evaluate_signal(market_id, price, volume)
            
            if decision == 'BUY':
                await self.execute_buy(market_id, price)
            elif decision == 'SELL':
                await self.execute_sell(market_id, price)
        
        except Exception as e:
            logger.error(f"Error procesando actualización: {e}")
    
    def evaluate_signal(self, market_id: str, price: float, 
                       volume: float) -> str:
        """
        Determina si comprar, vender o hold
        """
        # Aquí integrar la lógica de tu estrategia
        # Por ahora, placeholder
        
        if market_id not in self.positions:
            # Sin posición: evaluar entrada
            if self.should_enter_long(price, volume):
                return 'BUY'
        else:
            # Con posición: evaluar salida
            if self.should_exit(market_id, price):
                return 'SELL'
        
        return 'HOLD'
    
    def should_enter_long(self, price: float, volume: float) -> bool:
        """Lógica de entrada (simplificada)"""
        # Implementar tu lógica aquí
        return False
    
    def should_exit(self, market_id: str, price: float) -> bool:
        """Lógica de salida (simplificada)"""
        # Implementar tu lógica aquí
        return False
    
    async def execute_buy(self, market_id: str, price: float):
        """Ejecuta compra en el mercado"""
        try:
            amount = 100  # $100 por defecto (configurable)
            
            result = self.api.place_bet(
                market_id=market_id,
                side="yes",
                amount=amount,
                price=price
            )
            
            if result:
                self.positions[market_id] = {
                    'side': 'long',
                    'entry_price': price,
                    'amount': amount,
                    'entry_time': datetime.now()
                }
                logger.info(f"✓ BUY {market_id} @ ${price}")
                return True
        except Exception as e:
            logger.error(f"Error en BUY: {e}")
        
        return False
    
    async def execute_sell(self, market_id: str, price: float):
        """Ejecuta venta/cierre de posición"""
        try:
            result = self.api.close_position(market_id)
            
            if result:
                # Calcular P&L
                entry = self.positions[market_id]['entry_price']
                pnl = ((price - entry) / entry) * 100
                
                logger.info(f"✓ SELL {market_id} @ ${price} | PnL: {pnl:.2f}%")
                
                self.pnl[market_id] = pnl
                del self.positions[market_id]
                return True
        except Exception as e:
            logger.error(f"Error en SELL: {e}")
        
        return False
    
    async def start(self):
        """Inicia el bot"""
        self.running = True
        logger.info("🚀 Bot iniciado")
        
        # Encontrar mercados de 5-minutos disponibles
        markets = self.api.get_markets("5m")
        
        # Conectar a múltiples mercados en paralelo
        tasks = [
            self.connect_to_market(market['id']) 
            for market in markets[:3]  # Limitar a 3 mercados inicialmente
        ]
        
        try:
            await asyncio.gather(*tasks)
        except KeyboardInterrupt:
            logger.info("\n⏹️  Bot detenido por usuario")
            self.running = False
    
    def get_stats(self) -> dict:
        """Retorna estadísticas actual del bot"""
        trades = len(self.pnl)
        winning = sum(1 for p in self.pnl.values() if p > 0)
        total_pnl = sum(self.pnl.values())
        
        return {
            'trades': trades,
            'wins': winning,
            'win_rate': (winning / trades * 100) if trades > 0 else 0,
            'total_pnl_percent': total_pnl,
            'open_positions': len(self.positions)
        }

```

---

## 🚦 Parte 3: Risk Management Dashboard

```python
from dataclasses import dataclass
from typing import Dict


@dataclass
class RiskMetrics:
    """Métricas de riesgo en tiempo real"""
    current_drawdown: float
    max_drawdown: float
    daily_pnl: float
    daily_loss_limit: float
    position_count: int
    max_position_count: int
    total_exposure: float
    max_exposure: float
    correlation_check: Dict[str, float]


class RiskManager:
    """Gestiona riesgos y límites de trading"""
    
    def __init__(self, max_daily_loss: float = 500, 
                 max_exposure: float = 5000):
        self.max_daily_loss = max_daily_loss
        self.max_exposure = max_exposure
        self.daily_pnl = 0
        self.peak_equity = 0
        self.current_equity = 0
    
    def check_if_can_trade(self, metrics: RiskMetrics) -> bool:
        """Verifica si se pueden abrir nuevas posiciones"""
        
        checks = [
            metrics.daily_pnl > -self.max_daily_loss,  # Límite pérdida diaria
            metrics.total_exposure < self.max_exposure,  # Exposición máxima
            metrics.position_count < metrics.max_position_count,  # Límite posiciones
            metrics.max_drawdown < 20,  # Drawdown límite
        ]
        
        return all(checks)
    
    def get_position_size(self, 
                         account_balance: float,
                         daily_pnl: float) -> float:
        """Ajusta tamaño de posición dinámicamente"""
        
        # Reducir size si ya hay pérdidas en el día
        if daily_pnl < -200:  # $200 pérdida
            return account_balance * 0.1  # Reduce a 10%
        elif daily_pnl < -100:
            return account_balance * 0.15
        else:
            return account_balance * 0.2  # Normal 20%
    
    def should_stop_trading(self, metrics: RiskMetrics) -> bool:
        """Determina si debe parar trading por riesgo"""
        
        if metrics.daily_pnl < -self.max_daily_loss:
            return True
        
        if metrics.max_drawdown > 25:
            return True
        
        return False

```

---

## 📊 Parte 4: Monitoreo y Logging

```python
import csv
from datetime import datetime


class TradeLogger:
    """Registra todos los trades para análisis posterior"""
    
    def __init__(self, filename: str = "trades.csv"):
        self.filename = filename
        self.init_file()
    
    def init_file(self):
        """Inicializa archivo CSV"""
        try:
            with open(self.filename, 'w', newline='') as f:
                writer = csv.DictWriter(f, fieldnames=[
                    'timestamp',
                    'market_id',
                    'side',
                    'entry_price',
                    'exit_price',
                    'amount',
                    'pnl_percent',
                    'pnl_absolute',
                    'duration_bars',
                    'reason'
                ])
                writer.writeheader()
        except Exception as e:
            logger.error(f"Error inicializando log: {e}")
    
    def log_trade(self, trade: dict):
        """Registra un trade ejecutado"""
        try:
            with open(self.filename, 'a', newline='') as f:
                writer = csv.DictWriter(f, fieldnames=[
                    'timestamp',
                    'market_id',
                    'side',
                    'entry_price',
                    'exit_price',
                    'amount',
                    'pnl_percent',
                    'pnl_absolute',
                    'duration_bars',
                    'reason'
                ])
                writer.writerow({
                    'timestamp': datetime.now().isoformat(),
                    **trade
                })
        except Exception as e:
            logger.error(f"Error logging trade: {e}")
    
    def get_daily_summary(self) -> dict:
        """Resumen diario de trading"""
        try:
            df = pd.read_csv(self.filename)
            df['timestamp'] = pd.to_datetime(df['timestamp'])
            df['date'] = df['timestamp'].dt.date
            
            today = datetime.now().date()
            today_trades = df[df['date'] == today]
            
            return {
                'trades': len(today_trades),
                'wins': len(today_trades[today_trades['pnl_percent'] > 0]),
                'losses': len(today_trades[today_trades['pnl_percent'] < 0]),
                'total_pnl': today_trades['pnl_absolute'].sum(),
                'avg_duration': today_trades['duration_bars'].mean()
            }
        except Exception as e:
            logger.error(f"Error obteniendo summary: {e}")
            return {}

```

---

## 🚀 Parte 5: Script de Ejecución

```python
# main.py

import asyncio
import os
from dotenv import load_dotenv

load_dotenv()

# Configuración
API_KEY = os.getenv('POLYMARKET_API_KEY')  # Desde variables de entorno
INITIAL_CAPITAL = 10000

async def main():
    """Ejecuta el bot completo"""
    
    # Inicializar componentes
    config = TradingConfig()
    api = PolymartketAPI(api_key=API_KEY)
    bot = LiveTradingBot(config, api)
    risk_manager = RiskManager(max_daily_loss=500)
    logger = TradeLogger()
    
    print("=" * 60)
    print("POLYMARKET 4-MINUTE TRADING BOT")
    print("=" * 60)
    print(f"Capital Inicial: ${INITIAL_CAPITAL}")
    print(f"Fecha/Hora: {datetime.now()}")
    print("=" * 60)
    
    # Iniciar bot
    try:
        await bot.start()
    
    except KeyboardInterrupt:
        print("\n🛑 Bot detenido")
        
        # Mostrar estadísticas finales
        stats = bot.get_stats()
        print("\n" + "=" * 60)
        print("ESTADÍSTICAS FINALES")
        print("=" * 60)
        print(f"Trades Ejecutados: {stats['trades']}")
        print(f"Trades Ganadores: {stats['wins']}")
        print(f"Win Rate: {stats['win_rate']:.2f}%")
        print(f"P&L Total: {stats['total_pnl_percent']:.2f}%")
        print(f"Posiciones Abiertas: {stats['open_positions']}")
        print("=" * 60)


if __name__ == "__main__":
    asyncio.run(main())

```

---

## ⚙️ Configuración Recomendada

### `.env` file
```
POLYMARKET_API_KEY=your_api_key_here
WALLET_ADDRESS=0x...
CHAINLINK_RPC_URL=https://polygon-rpc.com

# Risk Management
MAX_DAILY_LOSS=500
MAX_POSITION_SIZE=0.2
TAKE_PROFIT_PERCENT=3.0
STOP_LOSS_PERCENT=2.0

# Logging
LOG_LEVEL=INFO
LOG_FILE=bot.log
TRADE_LOG_FILE=trades.csv
```

### Estructura de Carpetas
```
polymarket-bot/
├── main.py
├── config.py
├── trading_bot.py
├── risk_manager.py
├── api_client.py
├── data/
│   ├── trades.csv
│   └── logs/
├── backtest/
│   ├── backtest.py
│   └── data.csv
├── requirements.txt
└── .env
```

---

## 🧪 Testing Antes de Live Trading

```python
# test_bot.py

def test_api_connection():
    """Verifica que la API funciona"""
    api = PolymartketAPI(api_key=API_KEY)
    markets = api.get_markets("5m")
    assert len(markets) > 0, "No se pueden obtener mercados"
    print("✓ API connection OK")

def test_signal_generation():
    """Verifica que la estrategia genera señales"""
    # Cargar datos de prueba
    df = load_sample_data()
    bot = PolymartketBot()
    signals = bot.generate_signals(df)
    assert len(signals) > 0, "No se generan señales"
    print("✓ Signal generation OK")

def test_position_management():
    """Verifica cálculos de posición"""
    capital = 10000
    stop_loss_dist = 100
    bot = PolymartketBot()
    pos_size = bot.calculate_position_size(capital, stop_loss_dist)
    assert pos_size > 0, "Tamaño de posición inválido"
    print("✓ Position management OK")

def test_risk_limits():
    """Verifica límites de riesgo"""
    risk_mgr = RiskManager(max_daily_loss=500)
    assert risk_mgr.should_stop_trading({...}) == False
    print("✓ Risk limits OK")

if __name__ == "__main__":
    test_api_connection()
    test_signal_generation()
    test_position_management()
    test_risk_limits()
    print("\n✅ Todos los tests pasaron")
```

---

## 📝 Checklist Antes de Live Trading

- [ ] API key configurada y testeada
- [ ] Backtest ejecutado con win rate > 60%
- [ ] Risk management configurado
- [ ] Stop loss y take profit ajustados
- [ ] Paper trading (si disponible) exitoso
- [ ] Dinero inicial pequeño ($100-500)
- [ ] Monitoreo configurado
- [ ] Logs activados
- [ ] Wallet tiene fondos suficientes
- [ ] Network testnet validada

---

## 🚨 Troubleshooting Común

| Problema | Solución |
|----------|----------|
| `ConnectionError` | Verifica conexión a internet, proxy, firewall |
| `AuthenticationError` | Valida API key en .env, permisos de cuenta |
| `Timeout` en datos | Aumenta timeout, reduce número de mercados |
| Win rate < 50% | Aumenta momentum_threshold, añade filtros |
| Pérdidas rápidas | Reduce position size, añade pre-filters |
| Drawdown alto | Reduce max_position_size, añade circuit breaker |
