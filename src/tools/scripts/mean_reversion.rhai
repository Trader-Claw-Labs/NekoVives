// Mean Reversion Strategy v1.0 — Trader Claw (Bollinger Bands + RSI extreme reversal)
// Detects when price has overextended from its statistical mean and bets on a snap-back.
// Signal: price below BB lower (oversold) → long; price above BB upper (overbought) → short.
// Confirmation: RSI extremes + ATR-based position sizing.
// Works on crypto (4m/1h) and Polymarket (5m). Tune bb_period and bb_mult per asset.
fn on_candle(ctx) {
    // ── Parameters ──────────────────────────────────────────────────────────
    let bb_period    = 20;     // Bollinger Band SMA period
    let bb_mult      = 2.0;    // Standard deviations for bands
    let rsi_period   = 14;
    let rsi_low      = 28.0;   // RSI below this → deeply oversold, high reversion probability
    let rsi_high     = 72.0;   // RSI above this → deeply overbought
    let atr_sl_mult  = 1.8;    // Stop loss = ATR × 1.8 (within the band, tight)
    let atr_tp_mult  = 1.2;    // Take profit = ATR × 1.2 (targeting mid-band)
    let max_loss_pct = 3.0;    // Hard stop at -3%
    let take_pft_pct = 4.5;    // Hard target at +4.5%
    let max_hold     = 12;     // Max bars (avoid dead positions)
    let min_idx      = 30;     // Warmup

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close  = ctx.close;
    let atr    = ctx.atr(14);

    // ── Exit logic ───────────────────────────────────────────────────────────
    if ctx.position != 0.0 {
        let ep        = ctx.entry_price;
        let bars_held = idx - ctx.entry_index;
        let raw_pnl   = if ep > 0.0 { (close - ep) / ep * 100.0 } else { 0.0 };
        let eff_pnl   = if ctx.position < 0.0 { -raw_pnl } else { raw_pnl };

        // Also exit when price reverts to the middle band (profit-taking at mean)
        let mid = ctx.bb_middle(bb_period);
        let reverted_to_mean = if ctx.position > 0.0 {
            close >= mid  // long: close when price reached mean
        } else {
            close <= mid  // short: close when price reached mean
        };

        if eff_pnl >= take_pft_pct || eff_pnl <= -max_loss_pct
            || bars_held >= max_hold || reverted_to_mean {
            ctx.sell(1.0);
            ctx.log("EXIT mean_rev: pnl=" + eff_pnl + "% bars=" + bars_held + " reverted=" + reverted_to_mean);
        }
        return;
    }

    // ── Indicators ───────────────────────────────────────────────────────────
    let upper = ctx.bb_upper(bb_period, bb_mult);
    let lower = ctx.bb_lower(bb_period, bb_mult);
    let mid   = ctx.bb_middle(bb_period);
    let width = ctx.bb_width(bb_period, bb_mult);
    let rsi   = ctx.rsi(rsi_period);

    // Only trade when bands have meaningful width (not in a squeeze)
    let avg_width = ctx.get("avg_bb_width", width);
    ctx.set("avg_bb_width", avg_width * 0.95 + width * 0.05);
    if width < avg_width * 0.5 { return; }  // skip BB squeeze (low vol)

    let stop_dist = atr * atr_sl_mult;
    let tp_dist   = atr * atr_tp_mult;

    // ── Entry: long when oversold below lower band ────────────────────────────
    if close < lower && rsi < rsi_low {
        ctx.buy(1.0);
        ctx.set_stop_loss(close - stop_dist);
        ctx.set_take_profit(mid);  // target the mean
        ctx.log("LONG mean_rev: close=" + close + " lower=" + lower + " rsi=" + rsi);
    }
    // ── Entry: short when overbought above upper band ────────────────────────
    else if close > upper && rsi > rsi_high {
        ctx.sell(1.0);
        ctx.set_stop_loss(close + stop_dist);
        ctx.set_take_profit(mid);  // target the mean
        ctx.log("SHORT mean_rev: close=" + close + " upper=" + upper + " rsi=" + rsi);
    }
}
