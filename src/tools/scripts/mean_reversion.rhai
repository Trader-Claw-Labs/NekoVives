// Mean Reversion Strategy v1.1 — Trader Claw (ATR bands + RSI extreme reversal)
// Detects when price has overextended from its statistical mean and bets on a snap-back.
// Signal: price below ATR lower band (oversold) → long; price above ATR upper band → short.
// ATR bands (ema ± atr*mult) approximate Bollinger Bands ±2σ.
// Works on crypto (4m/1h) and Polymarket (5m). Tune bb_period and bb_mult per asset.
fn on_candle(ctx) {
    // ── Parameters ──────────────────────────────────────────────────────────
    let bb_period    = 20;
    let bb_mult      = 2.0;
    let rsi_period   = 14;
    let rsi_low      = 28.0;
    let rsi_high     = 72.0;
    let atr_sl_mult  = 1.8;
    let max_loss_pct = 3.0;
    let take_pft_pct = 4.5;
    let max_hold     = 12;
    let min_idx      = 30;

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close  = ctx.close;
    let atr    = ctx.atr(14);
    let ema20  = ctx.ema(bb_period);

    // ATR-band proxies for Bollinger Bands
    let upper = ema20 + atr * bb_mult;
    let lower = ema20 - atr * bb_mult;
    let mid   = ema20;
    let width = if lower > 0.0 { (upper - lower) / lower * 100.0 } else { 0.0 };

    // ── Exit logic ───────────────────────────────────────────────────────────
    if ctx.position != 0.0 {
        let ep        = ctx.entry_price;
        let bars_held = idx - ctx.entry_index;
        let raw_pnl   = if ep > 0.0 { (close - ep) / ep * 100.0 } else { 0.0 };
        let eff_pnl   = if ctx.position < 0.0 { -raw_pnl } else { raw_pnl };

        let reverted_to_mean = if ctx.position > 0.0 {
            close >= mid
        } else {
            close <= mid
        };

        if eff_pnl >= take_pft_pct || eff_pnl <= -max_loss_pct
            || bars_held >= max_hold || reverted_to_mean {
            ctx.sell(1.0);
            ctx.set("debug_exit_pnl", eff_pnl);
            ctx.set("debug_exit_reverted", if reverted_to_mean { 1.0 } else { 0.0 });
        }
        return;
    }

    // ── Indicators ───────────────────────────────────────────────────────────
    let rsi   = ctx.rsi(rsi_period);

    // Only trade when bands have meaningful width (not in a squeeze)
    let avg_width = ctx.get("avg_bb_width", width);
    ctx.set("avg_bb_width", avg_width * 0.95 + width * 0.05);
    if width < avg_width * 0.5 { return; }

    let stop_dist = atr * atr_sl_mult;

    ctx.set("debug_upper", upper);
    ctx.set("debug_lower", lower);
    ctx.set("debug_rsi", rsi);
    ctx.set("debug_width", width);

    // ── Entry: long when oversold below lower band ────────────────────────────
    if close < lower && rsi < rsi_low {
        ctx.buy(0.25);
        ctx.set_stop_loss(close - stop_dist);
        ctx.set_take_profit(mid);
    }
    // ── Entry: short when overbought above upper band ────────────────────────
    else if close > upper && rsi > rsi_high {
        ctx.sell(1.0);
        ctx.set_stop_loss(close + stop_dist);
        ctx.set_take_profit(mid);
    }
}
