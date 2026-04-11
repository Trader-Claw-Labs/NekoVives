"""
Polymarket 4-Minute Trading Bot - Python Implementation
Versión mejorada con risk management y múltiples confirmaciones

Requisitos:
    pip install pandas numpy ta-lib backtrader ccxt python-binance
"""

import pandas as pd
import numpy as np
from dataclasses import dataclass
from typing import Optional, List
from enum import Enum


class TradeSignal(Enum):
    BULLISH = 1
    BEARISH = -1
    NEUTRAL = 0


@dataclass
class TradingConfig:
    """Parámetros de la estrategia"""
    # Core Strategy
    lookback_4: int = 4
    lookback_14: int = 14
    momentum_threshold: float = 0.8  # Aumentado de 0.3% a 0.8%
    rsi_threshold: float = 30.0
    atr_multiplier: float = 1.5
    
    # Risk Management
    max_position_size: float = 0.2  # 20% del capital
    max_loss_percent: float = 2.0
    take_profit_percent: float = 3.0
    max_positions: int = 3
    
    # Confirmations
    min_candles: int = 20
    min_volume_threshold: float = 1.2  # 20% arriba de promedio
    required_confirmations: int = 3  # De 4 posibles
    
    # Time Management
    max_hold_bars: int = 5


class PolymartketBot:
    """Bot de trading para Polymarket con múltiples filtros"""
    
    def __init__(self, config: TradingConfig = None):
        self.config = config or TradingConfig()
        self.positions = []
        self.trades_log = []
        self.current_bar = 0
        
    # ==================== TECHNICAL INDICATORS ====================
    
    def calculate_sma(self, data: pd.Series, period: int) -> pd.Series:
        """Simple Moving Average"""
        return data.rolling(window=period).mean()
    
    def calculate_atr(self, df: pd.DataFrame, period: int = 14) -> pd.Series:
        """Average True Range"""
        high = df['high']
        low = df['low']
        close = df['close']
        
        tr1 = high - low
        tr2 = abs(high - close.shift())
        tr3 = abs(low - close.shift())
        
        tr = pd.concat([tr1, tr2, tr3], axis=1).max(axis=1)
        atr = tr.rolling(window=period).mean()
        
        return atr
    
    def calculate_rsi(self, data: pd.Series, period: int = 14) -> pd.Series:
        """Relative Strength Index"""
        delta = data.diff()
        gain = (delta.where(delta > 0, 0)).rolling(window=period).mean()
        loss = (-delta.where(delta < 0, 0)).rolling(window=period).mean()
        
        rs = gain / loss
        rsi = 100 - (100 / (1 + rs))
        
        return rsi
    
    def calculate_momentum(self, data: pd.Series, period: int) -> pd.Series:
        """Momentum: % change over period"""
        return ((data - data.shift(period)) / data.shift(period)) * 100
    
    # ==================== SIGNAL GENERATION ====================
    
    def get_bullish_confirmations(self, 
                                  df: pd.DataFrame, 
                                  idx: int) -> int:
        """Cuenta confirmaciones alcistas (0-4)"""
        confirmations = 0
        row = df.iloc[idx]
        
        # 1. Momentum 4-candles positivo
        if row['momentum_4'] > self.config.momentum_threshold:
            confirmations += 1
        
        # 2. Momentum 1-candle positivo
        if row['momentum_1'] > 0.2:
            confirmations += 1
        
        # 3. RSI en zona de sobreventa
        if row['rsi'] < self.config.rsi_threshold:
            confirmations += 1
        
        # 4. Volumen confirmado
        if row['volume'] > row['avg_volume'] * self.config.min_volume_threshold:
            confirmations += 1
        
        return confirmations
    
    def get_bearish_confirmations(self, 
                                  df: pd.DataFrame, 
                                  idx: int) -> int:
        """Cuenta confirmaciones bajistas (0-4)"""
        confirmations = 0
        row = df.iloc[idx]
        
        # 1. Momentum 4-candles negativo
        if row['momentum_4'] < -self.config.momentum_threshold:
            confirmations += 1
        
        # 2. Momentum 1-candle negativo
        if row['momentum_1'] < -0.2:
            confirmations += 1
        
        # 3. RSI en zona de sobrecompra
        if row['rsi'] > (100 - self.config.rsi_threshold):
            confirmations += 1
        
        # 4. Volumen confirmado
        if row['volume'] > row['avg_volume'] * self.config.min_volume_threshold:
            confirmations += 1
        
        return confirmations
    
    def generate_signals(self, df: pd.DataFrame) -> pd.Series:
        """Genera señales de trading para todo el DataFrame"""
        signals = pd.Series(0, index=df.index, dtype=int)
        
        for idx in range(self.config.min_candles, len(df)):
            # Asegura que tenemos datos suficientes
            if idx < self.config.lookback_14:
                continue
            
            bullish = self.get_bullish_confirmations(df, idx)
            bearish = self.get_bearish_confirmations(df, idx)
            
            # Requiere mínimas confirmaciones
            if bullish >= self.config.required_confirmations:
                # Validar tendencia alcista
                if df.iloc[idx]['close'] > df.iloc[idx]['sma_14']:
                    signals.iloc[idx] = TradeSignal.BULLISH.value
            
            elif bearish >= self.config.required_confirmations:
                # Validar tendencia bajista
                if df.iloc[idx]['close'] < df.iloc[idx]['sma_14']:
                    signals.iloc[idx] = TradeSignal.BEARISH.value
        
        return signals
    
    # ==================== POSITION MANAGEMENT ====================
    
    def calculate_position_size(self, 
                               capital: float, 
                               stop_loss_distance: float) -> float:
        """Calcula tamaño de posición basado en risk management"""
        risk_amount = capital * self.config.max_position_size / 100
        position_size = risk_amount / stop_loss_distance if stop_loss_distance > 0 else 0
        
        return position_size
    
    def backtest(self, df: pd.DataFrame, initial_capital: float = 10000) -> dict:
        """
        Backtest de la estrategia
        
        Args:
            df: DataFrame con OHLCV + indicadores
            initial_capital: Capital inicial
            
        Returns:
            dict con estadísticas de performance
        """
        
        # Preparar datos
        df = self._prepare_data(df)
        df['signal'] = self.generate_signals(df)
        
        # Variables de tracking
        capital = initial_capital
        position = None
        entry_price = 0
        entry_index = 0
        trades = []
        equity_curve = [capital]
        
        # Simulación
        for idx in range(self.config.min_candles, len(df)):
            current_price = df.iloc[idx]['close']
            signal = df.iloc[idx]['signal']
            stop_loss = df.iloc[idx]['stop_loss']
            take_profit = df.iloc[idx]['take_profit']
            
            # Check para cerrar posición existente
            if position is not None:
                bars_held = idx - entry_index
                pnl_percent = ((current_price - entry_price) / entry_price) * 100
                
                # Stop Loss
                if pnl_percent <= -self.config.max_loss_percent:
                    exit_price = current_price * (1 - self.config.max_loss_percent / 100)
                    profit = capital * (pnl_percent / 100)
                    capital += profit
                    trades.append({
                        'entry_index': entry_index,
                        'exit_index': idx,
                        'entry_price': entry_price,
                        'exit_price': exit_price,
                        'side': position,
                        'pnl_percent': pnl_percent,
                        'pnl_absolute': profit,
                        'reason': 'stop_loss'
                    })
                    position = None
                
                # Take Profit
                elif pnl_percent >= self.config.take_profit_percent:
                    exit_price = current_price
                    profit = capital * (pnl_percent / 100)
                    capital += profit
                    trades.append({
                        'entry_index': entry_index,
                        'exit_index': idx,
                        'entry_price': entry_price,
                        'exit_price': exit_price,
                        'side': position,
                        'pnl_percent': pnl_percent,
                        'pnl_absolute': profit,
                        'reason': 'take_profit'
                    })
                    position = None
                
                # Time-based exit
                elif bars_held >= self.config.max_hold_bars:
                    profit = capital * (pnl_percent / 100)
                    capital += profit
                    trades.append({
                        'entry_index': entry_index,
                        'exit_index': idx,
                        'entry_price': entry_price,
                        'exit_price': current_price,
                        'side': position,
                        'pnl_percent': pnl_percent,
                        'pnl_absolute': profit,
                        'reason': 'time_exit'
                    })
                    position = None
            
            # Check para abrir posición nueva
            if position is None and signal != 0:
                entry_price = current_price
                entry_index = idx
                position = 'long' if signal == 1 else 'short'
            
            equity_curve.append(capital)
        
        # Calcular estadísticas
        return self._calculate_stats(trades, equity_curve, initial_capital)
    
    def _prepare_data(self, df: pd.DataFrame) -> pd.DataFrame:
        """Prepara DataFrame con todos los indicadores necesarios"""
        df = df.copy()
        
        # Indicadores
        df['momentum_4'] = self.calculate_momentum(df['close'], self.config.lookback_4)
        df['momentum_1'] = self.calculate_momentum(df['close'], 1)
        df['sma_14'] = self.calculate_sma(df['close'], self.config.lookback_14)
        df['rsi'] = self.calculate_rsi(df['close'], self.config.lookback_14)
        df['atr'] = self.calculate_atr(df, self.config.lookback_14)
        df['avg_volume'] = self.calculate_sma(df['volume'], 20)
        
        # Stop Loss y Take Profit
        df['stop_loss'] = df['close'] - (df['atr'] * self.config.atr_multiplier)
        df['take_profit'] = df['close'] + (df['atr'] * 2)
        
        return df
    
    def _calculate_stats(self, trades: List[dict], 
                        equity_curve: List[float], 
                        initial_capital: float) -> dict:
        """Calcula estadísticas de backtest"""
        
        if not trades:
            return {
                'total_return_percent': 0,
                'total_trades': 0,
                'win_rate': 0,
                'avg_win': 0,
                'avg_loss': 0,
                'profit_factor': 0,
                'max_drawdown': 0
            }
        
        df_trades = pd.DataFrame(trades)
        
        # Estadísticas básicas
        total_return = (equity_curve[-1] - initial_capital) / initial_capital * 100
        winning_trades = len(df_trades[df_trades['pnl_absolute'] > 0])
        total_trades = len(trades)
        win_rate = (winning_trades / total_trades * 100) if total_trades > 0 else 0
        
        # PnL Statistics
        wins = df_trades[df_trades['pnl_absolute'] > 0]['pnl_absolute']
        losses = df_trades[df_trades['pnl_absolute'] < 0]['pnl_absolute']
        
        avg_win = wins.mean() if len(wins) > 0 else 0
        avg_loss = abs(losses.mean()) if len(losses) > 0 else 0
        
        profit_factor = wins.sum() / abs(losses.sum()) if losses.sum() < 0 else 0
        
        # Drawdown
        equity_array = np.array(equity_curve)
        running_max = np.maximum.accumulate(equity_array)
        drawdown = (equity_array - running_max) / running_max
        max_drawdown = abs(drawdown.min()) * 100
        
        return {
            'total_return_percent': round(total_return, 2),
            'total_trades': total_trades,
            'win_rate': round(win_rate, 2),
            'avg_win': round(avg_win, 2),
            'avg_loss': round(avg_loss, 2),
            'profit_factor': round(profit_factor, 2),
            'max_drawdown': round(max_drawdown, 2),
            'trades': df_trades
        }


# ==================== EJEMPLO DE USO ====================

def load_sample_data(ticker: str = 'BTCUSD', timeframe: str = '5m') -> pd.DataFrame:
    """
    Carga datos de ejemplo (en producción, usar tu API de Polymarket)
    
    Para datos reales, puedes usar:
    - Binance API: from binance.client import Client
    - CCXT: import ccxt
    - Polymarket API: requests.get('https://api.polymarket.com/...')
    """
    # Este es un placeholder. En producción carga datos reales
    print(f"Cargando datos de {ticker} en timeframe {timeframe}")
    print("En producción, reemplaza esto con datos reales de tu API")
    
    # Ejemplo con datos dummy
    dates = pd.date_range(start='2024-01-01', periods=1000, freq='5min')
    
    np.random.seed(42)
    price = 40000 + np.cumsum(np.random.randn(1000) * 10)
    
    df = pd.DataFrame({
        'timestamp': dates,
        'open': price + np.random.randn(1000) * 5,
        'high': price + abs(np.random.randn(1000) * 10),
        'low': price - abs(np.random.randn(1000) * 10),
        'close': price,
        'volume': np.random.randint(100, 10000, 1000)
    })
    
    return df


def main():
    """Ejemplo completo de backtesting"""
    
    print("=" * 60)
    print("POLYMARKET 4-MINUTE BOT - BACKTESTING")
    print("=" * 60)
    
    # Cargar datos
    df = load_sample_data('BTCUSD', '5m')
    print(f"\n✓ Datos cargados: {len(df)} velas")
    
    # Configurar bot
    config = TradingConfig(
        momentum_threshold=0.8,
        take_profit_percent=3.0,
        max_loss_percent=2.0,
        max_position_size=0.2,
        required_confirmations=3
    )
    
    bot = PolymartketBot(config)
    print(f"\n✓ Bot configurado:")
    print(f"  - Momentum threshold: {config.momentum_threshold}%")
    print(f"  - Take profit: {config.take_profit_percent}%")
    print(f"  - Stop loss: {config.max_loss_percent}%")
    print(f"  - Confirmaciones requeridas: {config.required_confirmations}/4")
    
    # Ejecutar backtest
    print(f"\n⏳ Ejecutando backtest...")
    stats = bot.backtest(df, initial_capital=10000)
    
    # Mostrar resultados
    print("\n" + "=" * 60)
    print("RESULTADOS")
    print("=" * 60)
    print(f"Retorno Total:      {stats['total_return_percent']}%")
    print(f"Total de Trades:    {stats['total_trades']}")
    print(f"Win Rate:           {stats['win_rate']}%")
    print(f"Ganancia Promedio:  ${stats['avg_win']:.2f}")
    print(f"Pérdida Promedio:   ${stats['avg_loss']:.2f}")
    print(f"Profit Factor:      {stats['profit_factor']}")
    print(f"Max Drawdown:       {stats['max_drawdown']}%")
    print("=" * 60)
    
    # Evaluación
    print("\n📊 EVALUACIÓN:")
    
    if stats['win_rate'] >= 60:
        print("✅ Win rate bueno (>=60%), estrategia potencialmente rentable")
    elif stats['win_rate'] >= 50:
        print("⚠️  Win rate marginal (50-60%), considera ajustar parámetros")
    else:
        print("❌ Win rate bajo (<50%), aumenta momentum_threshold")
    
    if stats['profit_factor'] > 1.5:
        print("✅ Profit factor excelente (>1.5)")
    elif stats['profit_factor'] > 1.0:
        print("⚠️  Profit factor aceptable (1.0-1.5)")
    else:
        print("❌ Profit factor bajo (<1.0), pérdidas mayores que ganancias")
    
    if stats['max_drawdown'] < 10:
        print("✅ Max drawdown bajo (<10%)")
    else:
        print("⚠️  Max drawdown elevado (>10%), considera reducir position size")
    
    return stats


if __name__ == "__main__":
    stats = main()
