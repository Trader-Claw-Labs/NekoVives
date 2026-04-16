// CRYPTO 4-Minute Strategy v2.1 — Trader Claw (Binance, 4-of-4 confirmation + SMA-50 trend filter)
fn on_candle(ctx) {
    let lookback_4    = 4;
    let lookback_14   = 14;
    let lookback_50   = 50;
    let mom_threshold  = 1.8;
    let mom1_threshold = 0.5;
    let rsi_oversold   = 25.0;
    let rsi_overbought = 75.0;
    let atr_sl_mult    = 2.0;
    let atr_tp_mult    = 3.0;
    let max_loss_pct   = 3.5;
    let take_pft_pct   = 6.0;
    let min_vol_mult   = 1.5;
    let max_hold       = 8;
    let min_idx        = 100;

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close  = ctx.close;
    let volume = ctx.volume;
    let atr    = ctx.atr(14);

    if ctx.position != 0.0 {
        let ep          = ctx.entry_price;
        let bars_held   = idx - ctx.entry_index;
        let pnl_pct     = if ep > 0.0 { (close - ep) / ep * 100.0 } else { 0.0 };
        let effective_pnl = if ctx.position < 0.0 { -pnl_pct } else { pnl_pct };
        if effective_pnl >= take_pft_pct || effective_pnl <= -max_loss_pct || bars_held >= max_hold {
            ctx.sell(1.0);
            ctx.log("EXIT: PnL=" + effective_pnl + "% | bars=" + bars_held);
        }
        return;
    }

    let close_4ago = ctx.close_at(idx - lookback_4);
    let mom4 = if close_4ago > 0.0 { (close - close_4ago) / close_4ago * 100.0 } else { 0.0 };
    let close_1ago = ctx.close_at(idx - 1);
    let mom1 = if close_1ago > 0.0 { (close - close_1ago) / close_1ago * 100.0 } else { 0.0 };
    let ema14 = ctx.ema(lookback_14);
    let sma50 = ctx.sma(lookback_50);
    let rsi   = ctx.rsi(lookback_14);

    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.85 + volume * 0.15);
    let vol_ok = volume > avg_vol * min_vol_mult;

    let stop_dist = atr * atr_sl_mult;
    let tp_dist   = atr * atr_tp_mult;

    let bull_count = 0;
    if mom4 > mom_threshold  { bull_count += 1; }
    if mom1 > mom1_threshold { bull_count += 1; }
    if rsi < rsi_oversold    { bull_count += 1; }
    if vol_ok                { bull_count += 1; }

    let bear_count = 0;
    if mom4 < -mom_threshold  { bear_count += 1; }
    if mom1 < -mom1_threshold { bear_count += 1; }
    if rsi > rsi_overbought   { bear_count += 1; }
    if vol_ok                 { bear_count += 1; }

    if bull_count == 4 && close > ema14 && ema14 > sma50 {
        ctx.buy(1.0);
        ctx.set_stop_loss(close - stop_dist);
        ctx.set_take_profit(close + tp_dist);
        ctx.log("BUY: mom4=" + mom4 + "% | rsi=" + rsi + " | sma50=" + sma50);
    } else if bear_count == 4 && close < ema14 && ema14 < sma50 {
        ctx.sell(1.0);
        ctx.set_stop_loss(close + stop_dist);
        ctx.set_take_profit(close - tp_dist);
        ctx.log("SELL: mom4=" + mom4 + "% | rsi=" + rsi + " | sma50=" + sma50);
    }
}
