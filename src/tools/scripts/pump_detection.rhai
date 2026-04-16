// Pump & Dump Detection v1.0 — Trader Claw (Volume spike scoring + momentum burst entry)
// Detects abnormal volume + price acceleration suggesting a coordinated pump.
// Strategy: score pump confidence (0-5 points), enter on confirmed pump, exit fast.
// LONG only: ride the pump. Exit immediately when momentum exhausts or time expires.
// Also detects dump phase (post-pump collapse) for short entries.
fn on_candle(ctx) {
    // ── Parameters ──────────────────────────────────────────────────────────
    let vol_spike_mult   = 3.0;  // Volume must be 3× rolling average to signal pump
    let price_spike_pct  = 2.5;  // 1-bar price change must be >2.5% to signal pump
    let mom_4bar_pct     = 5.0;  // 4-bar momentum must exceed 5% (pump velocity)
    let bb_period        = 20;
    let bb_mult          = 2.0;
    let max_hold_pump    = 4;    // Pumps are short-lived — exit fast (4 bars max)
    let max_hold_dump    = 6;    // Dump shorts can ride a bit longer
    let take_pump_pct    = 4.0;  // Take profit on pump at +4%
    let stop_pump_pct    = 2.0;  // Tight stop on pump trades: -2%
    let take_dump_pct    = 5.0;  // Short profit target at -5% price
    let stop_dump_pct    = 2.5;  // Stop for dump short
    let min_idx          = 30;   // Warmup

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close  = ctx.close;
    let volume = ctx.volume;
    let atr    = ctx.atr(14);

    // ── Rolling volume average (EMA) ─────────────────────────────────────────
    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.88 + volume * 0.12);

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
            ctx.log("EXIT pump_det: pnl=" + eff_pnl + "% bars=" + bars_held);
        }
        return;
    }

    // ── Compute pump confidence score (0–5 points) ───────────────────────────
    let score = 0;

    // Signal 1: Volume spike
    if volume > avg_vol * vol_spike_mult { score += 1; }

    // Signal 2: 1-bar price change
    let close_1ago = ctx.close_at(idx - 1);
    let mom1 = if close_1ago > 0.0 { (close - close_1ago) / close_1ago * 100.0 } else { 0.0 };
    if mom1 > price_spike_pct { score += 1; }

    // Signal 3: 4-bar momentum (sustained velocity)
    let close_4ago = ctx.close_at(idx - 4);
    let mom4 = if close_4ago > 0.0 { (close - close_4ago) / close_4ago * 100.0 } else { 0.0 };
    if mom4 > mom_4bar_pct { score += 1; }

    // Signal 4: Price broke above BB upper (parabolic extension)
    let bb_upper = ctx.bb_upper(bb_period, bb_mult);
    if close > bb_upper { score += 1; }

    // Signal 5: RSI overbought (momentum exhaustion signal — confirms pump peak)
    let rsi = ctx.rsi(14);
    if rsi > 78.0 { score += 1; }

    // ── Pump trade: enter LONG on early pump (score 3+, not yet at full exhaustion) ──
    if score >= 3 && mom1 > 0.5 && rsi < 85.0 {
        ctx.buy(1.0);
        ctx.set_stop_loss(close - atr * 1.5);
        ctx.set_take_profit(close * (1.0 + take_pump_pct / 100.0));
        ctx.set("is_pump", 1.0);
        ctx.log("PUMP ENTRY score=" + score + " vol_ratio=" + (volume/avg_vol) + " mom1=" + mom1 + "% rsi=" + rsi);
        return;
    }

    // ── Dump trade: short on post-pump collapse ──────────────────────────────
    // Detect: high volume, price falling hard, BB upper was recently breached
    let dump_score = 0;
    if volume > avg_vol * 2.5       { dump_score += 1; }
    if mom1 < -price_spike_pct      { dump_score += 1; }
    if mom4 < -mom_4bar_pct * 0.6  { dump_score += 1; }  // less strict on 4bar for dump
    if rsi > 65.0                   { dump_score += 1; }  // was recently overbought
    if close < ctx.bb_middle(bb_period) && close_1ago > ctx.bb_middle(bb_period) { dump_score += 1; }  // crossed below mid-band

    if dump_score >= 3 && mom1 < -0.5 {
        ctx.sell(1.0);  // open short
        ctx.set_stop_loss(close + atr * 1.5);
        ctx.set_take_profit(close * (1.0 - take_dump_pct / 100.0));
        ctx.set("is_pump", 0.0);
        ctx.log("DUMP SHORT score=" + dump_score + " mom1=" + mom1 + "% rsi=" + rsi);
    }
}
