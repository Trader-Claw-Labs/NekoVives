// Event-Driven Strategy v1.0 — Trader Claw (Session-based momentum + volatility breakout)
// Trades the predictable volatility bursts that occur at key market sessions and
// economic event windows. Works without a live event feed by using:
//   1. Candle-index modulo to approximate session timing (e.g., every 96 bars ≈ 8h on 5m data).
//   2. Volatility regime detection (ATR expansion signals event-like conditions).
//   3. Momentum confirmation to trade in the direction of the breakout.
//
// On live 4-min/5-min data the pattern captures: London open, NY open, Asian session close.
// On Polymarket: detects binary market "event windows" (sudden probability jumps).
//
// Tune: session_bars (bars per session on your timeframe), vol_expansion_mult.
fn on_candle(ctx) {
    // ── Parameters ──────────────────────────────────────────────────────────
    // On 5-min data: 12 bars/hr, 96 bars/8hr session window
    // On 4-min data: 15 bars/hr, 90 bars/6hr session window
    // Adjust session_bars to match your interval
    let session_bars      = 72;   // Bars per session cycle (default: ~6h on 5m)
    let pre_window        = 8;    // Bars before session open to start watching
    let post_window       = 15;   // Bars after session open to enter
    let vol_expand_mult   = 1.6;  // ATR must be X× its rolling avg to confirm event
    let mom_threshold     = 1.5;  // 4-bar momentum % for directional entry
    let rsi_period        = 14;
    let bb_period         = 20;
    let bb_mult           = 2.0;
    let atr_sl_mult       = 2.0;
    let atr_tp_mult       = 3.0;
    let max_loss_pct      = 3.5;
    let take_pft_pct      = 7.0;  // Event moves can be large — give room
    let max_hold          = 18;   // Up to 90 min on 5m data
    let min_idx           = 50;

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close  = ctx.close;
    let atr    = ctx.atr(14);

    // ── ATR rolling average (for volatility regime detection) ────────────────
    let avg_atr = ctx.get("avg_atr", atr);
    ctx.set("avg_atr", avg_atr * 0.95 + atr * 0.05);

    // ── Exit logic ───────────────────────────────────────────────────────────
    if ctx.position != 0.0 {
        let ep        = ctx.entry_price;
        let bars_held = idx - ctx.entry_index;
        let raw_pnl   = if ep > 0.0 { (close - ep) / ep * 100.0 } else { 0.0 };
        let eff_pnl   = if ctx.position < 0.0 { -raw_pnl } else { raw_pnl };

        if eff_pnl >= take_pft_pct || eff_pnl <= -max_loss_pct || bars_held >= max_hold {
            ctx.sell(1.0);
            ctx.log("EXIT event_drv: pnl=" + eff_pnl + "% bars=" + bars_held);
        }
        return;
    }

    // ── Session window detection ─────────────────────────────────────────────
    // We're in the event window if bar index is within pre/post range of a session boundary
    let bar_in_session = idx % session_bars;
    let in_event_window = bar_in_session <= post_window
        || bar_in_session >= (session_bars - pre_window);

    if !in_event_window { return; }

    // ── Volatility regime: require ATR expansion ─────────────────────────────
    let vol_expanding = atr > avg_atr * vol_expand_mult;

    // During early data when avg_atr isn't stable, use BB width as alternative
    let bb_width = ctx.bb_width(bb_period, bb_mult);
    let avg_bbw  = ctx.get("avg_bbw", bb_width);
    ctx.set("avg_bbw", avg_bbw * 0.95 + bb_width * 0.05);
    let bb_expanding = bb_width > avg_bbw * 1.4;

    if !vol_expanding && !bb_expanding { return; }

    // ── Directional momentum signals ─────────────────────────────────────────
    let close_4ago = ctx.close_at(idx - 4);
    let mom4 = if close_4ago > 0.0 { (close - close_4ago) / close_4ago * 100.0 } else { 0.0 };
    let close_1ago = ctx.close_at(idx - 1);
    let mom1 = if close_1ago > 0.0 { (close - close_1ago) / close_1ago * 100.0 } else { 0.0 };
    let rsi   = ctx.rsi(rsi_period);
    let ema14 = ctx.ema(14);

    let stop_dist = atr * atr_sl_mult;
    let tp_dist   = atr * atr_tp_mult;

    // ── LONG: upside breakout on event ────────────────────────────────────────
    if mom4 > mom_threshold && mom1 > 0.3 && close > ema14 && rsi > 45.0 && rsi < 75.0 {
        ctx.buy(1.0);
        ctx.set_stop_loss(close - stop_dist);
        ctx.set_take_profit(close + tp_dist);
        ctx.log("EVENT LONG: bar_in_sess=" + bar_in_session + " mom4=" + mom4 + "% atr_ratio=" + (atr/avg_atr));
    }
    // ── SHORT: downside breakout on event ─────────────────────────────────────
    else if mom4 < -mom_threshold && mom1 < -0.3 && close < ema14 && rsi < 55.0 && rsi > 25.0 {
        ctx.sell(1.0);
        ctx.set_stop_loss(close + stop_dist);
        ctx.set_take_profit(close - tp_dist);
        ctx.log("EVENT SHORT: bar_in_sess=" + bar_in_session + " mom4=" + mom4 + "% atr_ratio=" + (atr/avg_atr));
    }
}
