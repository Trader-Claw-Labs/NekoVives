// OPT v15: Bollinger mean-reversion SOLO LONGS — compra en BB lower, vende en BB mid/upper
// Note: ctx.bollinger_upper/lower and ctx.sma are not supported — using ATR-based bands
// and ema(20) as equivalents. ATR bands: ema20 ± atr(14) * 2.0 approximates BB ±2σ.
fn on_candle(ctx) {
    let max_hold     = 20;
    let take_pft_pct = 5.0;
    let max_loss_pct = 2.5;
    let atr_sl_mult  = 1.5;
    let bb_mult      = 2.0;  // ATR multiplier approximating BB ±2σ
    let min_idx      = 80;

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close  = ctx.close;
    let atr    = ctx.atr(14);
    let ema20  = ctx.ema(20);
    let rsi    = ctx.rsi(14);

    let bb_upper = ema20 + atr * bb_mult;
    let bb_lower = ema20 - atr * bb_mult;

    if ctx.position > 0.0 {
        let ep        = ctx.entry_price;
        let bars_held = idx - ctx.entry_index;
        let pnl_pct   = if ep > 0.0 { (close - ep) / ep * 100.0 } else { 0.0 };
        let atr_sl    = close < ep - atr * atr_sl_mult;
        if pnl_pct >= take_pft_pct || pnl_pct <= -max_loss_pct || bars_held >= max_hold || atr_sl || close >= ema20 {
            ctx.sell(1.0);
        }
        return;
    }

    if ctx.position != 0.0 { return; }

    // Compra cuando precio toca o cruza la banda inferior con RSI oversold
    if close <= bb_lower && rsi < 35.0 {
        ctx.buy(0.25);
    }
}
