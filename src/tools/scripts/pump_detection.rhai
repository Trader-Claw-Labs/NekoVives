// Pump & Dump Detection v1.1 — Trader Claw (Volume spike scoring + momentum burst entry)
// Detects abnormal volume + price acceleration suggesting a coordinated pump.
// Strategy: score pump confidence (0-5 points), enter on confirmed pump, exit fast.
// LONG only: ride the pump. Exit immediately when momentum exhausts or time expires.
// Also detects dump phase (post-pump collapse) for short entries.
fn on_candle(ctx) {
    // ── Parameters ──────────────────────────────────────────────────────────
    let vol_spike_mult   = 3.0;
    let price_spike_pct  = 2.5;
    let mom_4bar_pct     = 5.0;
    let bb_period        = 20;
    let bb_mult          = 2.0;
    let max_hold_pump    = 4;
    let max_hold_dump    = 6;
    let take_pump_pct    = 4.0;
    let stop_pump_pct    = 2.0;
    let take_dump_pct    = 5.0;
    let stop_dump_pct    = 2.5;
    let min_idx          = 30;

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close  = ctx.close;
    let volume = ctx.volume;
    let atr    = ctx.atr(14);

    // ── Rolling volume average (EMA) ─────────────────────────────────────────
    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.88 + volume * 0.12);

    // ATR bands proxy for Bollinger Bands
    let ema20    = ctx.ema(bb_period);
    let bb_upper = ema20 + atr * bb_mult;
    let bb_mid   = ema20;

    // ── Exit logic ───────────────────────────────────────────────────────────
    if ctx.position != 0.0 {
        let ep        = ctx.entry_price;
        let bars_held = idx - ctx.entry_index;
        let raw_pnl   = if ep > 0.0 { (close - ep) / ep * 100.0 } else { 0.0 };
        let eff_pnl   = if ctx.position < 0.0 { -raw_pnl } else { raw_pnl };

        let is_pump_trade = ctx.get("is_pump", 0.0) > 0.0;
        let max_hold = if is_pump_trade { max_hold_pump } else { max_hold_dump };
        let tp_pct   = if is_pump_trade { take_pump_pct } else { take_dump_pct };
        let sl_pct   = if is_pump_trade { stop_pump_pct } else { stop_dump_pct };

        if eff_pnl >= tp_pct || eff_pnl <= -sl_pct || bars_held >= max_hold {
            ctx.sell(1.0);
            ctx.set("is_pump", 0.0);
            ctx.set("debug_exit_pnl", eff_pnl);
        }
        return;
    }

    // ── Compute pump confidence score (0–5 points) ───────────────────────────
    let score = 0;

    if volume > avg_vol * vol_spike_mult { score += 1; }

    let close_1ago = ctx.close_at(idx - 1);
    let mom1 = if close_1ago > 0.0 { (close - close_1ago) / close_1ago * 100.0 } else { 0.0 };
    if mom1 > price_spike_pct { score += 1; }

    let close_4ago = ctx.close_at(idx - 4);
    let mom4 = if close_4ago > 0.0 { (close - close_4ago) / close_4ago * 100.0 } else { 0.0 };
    if mom4 > mom_4bar_pct { score += 1; }

    if close > bb_upper { score += 1; }

    let rsi = ctx.rsi(14);
    if rsi > 78.0 { score += 1; }

    ctx.set("debug_score", score);
    ctx.set("debug_vol_ratio", volume / avg_vol);
    ctx.set("debug_mom1", mom1);

    // ── Pump trade: enter LONG on early pump ──
    if score >= 3 && mom1 > 0.5 && rsi < 85.0 {
        ctx.buy(0.25);
        ctx.set_stop_loss(close - atr * 1.5);
        ctx.set_take_profit(close * (1.0 + take_pump_pct / 100.0));
        ctx.set("is_pump", 1.0);
        return;
    }

    // ── Dump trade: short on post-pump collapse ──────────────────────────────
    let dump_score = 0;
    if volume > avg_vol * 2.5       { dump_score += 1; }
    if mom1 < -price_spike_pct      { dump_score += 1; }
    if mom4 < -mom_4bar_pct * 0.6   { dump_score += 1; }
    if rsi > 65.0                   { dump_score += 1; }
    if close < bb_mid && close_1ago > bb_mid { dump_score += 1; }

    if dump_score >= 3 && mom1 < -0.5 {
        ctx.sell(1.0);
        ctx.set_stop_loss(close + atr * 1.5);
        ctx.set_take_profit(close * (1.0 - take_dump_pct / 100.0));
        ctx.set("is_pump", 0.0);
        ctx.set("debug_dump_score", dump_score);
    }
}
