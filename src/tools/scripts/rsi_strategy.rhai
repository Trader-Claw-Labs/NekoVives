fn on_candle(ctx) {
    let rsi = ctx.rsi(14);
    let position = ctx.position;
    
    // Need enough candles for RSI calculation
    if ctx.index < 14 {
        return;
    }
    
    // Buy when oversold
    if rsi < 30.0 && position == 0.0 {
        ctx.buy(0.25);
    }

    // Sell when overbought
    if rsi > 70.0 && position > 0.0 {
        ctx.sell(1.0);
    }
}
