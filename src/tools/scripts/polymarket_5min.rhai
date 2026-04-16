// Polymarket 5-Minute Binary Strategy v2.1 — Trader Claw (3-of-4 confirmation)
fn on_candle(ctx) {
    let lookback_4    = 4;
    let lookback_14   = 14;
    let lookback_50   = 50;
    let mom_threshold  = 1.2;
    let mom1_threshold = 0.3;
    let rsi_oversold   = 30.0;
    let rsi_overbought = 70.0;
    let atr_sl_mult    = 1.5;
    let atr_tp_mult    = 2.5;
    let max_loss_pct   = 2.5;
    let take_pft_pct   = 5.0;
    let min_vol_mult   = 1.2;
    let max_hold       = 12;
    let min_idx        = 80;
    let min_confirmations = 3;

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
    let rsi   = ctx.rsi(lookback_14);

    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.9 + volume * 0.1);
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

    if bull_count >= min_confirmations && close > ema14 {
        ctx.buy(1.0);
        ctx.set_stop_loss(close - stop_dist);
        ctx.set_take_profit(close + tp_dist);
        ctx.log("BUY: bull=" + bull_count + " | mom4=" + mom4 + "% | rsi=" + rsi);
    } else if bear_count >= min_confirmations && close < ema14 {
        ctx.sell(1.0);
        ctx.set_stop_loss(close + stop_dist);
        ctx.set_take_profit(close - tp_dist);
        ctx.log("SELL: bear=" + bear_count + " | mom4=" + mom4 + "% | rsi=" + rsi);
    }
}
