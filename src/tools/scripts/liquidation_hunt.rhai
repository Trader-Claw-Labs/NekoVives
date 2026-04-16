// Liquidation Hunting v1.0 — Trader Claw (Volume cascade + reversal detector)
// Identifies forced liquidation events by detecting abnormal volume spikes combined
// with rapid price dislocations. After a cascade, fades the move (counter-trend).
//
// The pattern:
//   1. Price drops sharply → triggers leveraged long stop-losses → volume spike
//   2. Selling exhausts → short-term imbalance → rapid recovery (snap-back)
//   3. Bot enters LONG after detecting the exhaustion signature
//
// Also detects short-squeeze cascades (inverse pattern) for SHORT entries.
// Works on Binance 4m/5m. Volume data required (Polymarket volume is synthetic).
fn on_candle(ctx) {
    // ── Parameters ──────────────────────────────────────────────────────────
    let vol_cascade_mult    = 4.0;   // Volume spike: must be 4× rolling average
    let cascade_drop_pct    = 1.8;   // 1-bar price drop % to signal long liquidation
    let cascade_rise_pct    = 1.8;   // 1-bar price rise % to signal short squeeze
    let recovery_confirm    = 0.4;   // 1-bar recovery % after cascade (entry confirmation)
    let rsi_exhaustion_low  = 22.0;  // RSI after long liq cascade (oversold exhaustion)
    let rsi_exhaustion_high = 78.0;  // RSI after short squeeze (overbought exhaustion)
    let stoch_low           = 15.0;  // Stochastic K < 15 confirms liquidation oversold
    let stoch_high          = 85.0;  // Stochastic K > 85 confirms short squeeze
    let bb_period           = 20;
    let bb_mult             = 2.2;
    let atr_sl_mult         = 1.2;   // Tight stop: cascade entries are high-conviction
    let atr_tp_mult         = 2.5;
    let max_loss_pct        = 2.5;
    let take_pft_pct        = 5.0;
    let max_hold            = 10;    // Snap-backs are fast
    let cooldown_bars       = 5;     // Min bars between trades (avoid whipsaws)
    let min_idx             = 30;

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close  = ctx.close;
    let volume = ctx.volume;
    let atr    = ctx.atr(14);
    let high   = ctx.high;
    let low    = ctx.low;

    // ── Rolling volume average ────────────────────────────────────────────────
    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.87 + volume * 0.13);

    // ── Exit logic ───────────────────────────────────────────────────────────
    if ctx.position != 0.0 {
        let ep        = ctx.entry_price;
        let bars_held = idx - ctx.entry_index;
        let raw_pnl   = if ep > 0.0 { (close - ep) / ep * 100.0 } else { 0.0 };
        let eff_pnl   = if ctx.position < 0.0 { -raw_pnl } else { raw_pnl };

        if eff_pnl >= take_pft_pct || eff_pnl <= -max_loss_pct || bars_held >= max_hold {
            ctx.sell(1.0);
            ctx.log("EXIT liq_hunt: pnl=" + eff_pnl + "% bars=" + bars_held);
        }
        return;
    }

    // ── Cooldown (avoid re-entering immediately after a trade) ───────────────
    let last_trade_bar = ctx.get("last_trade_bar", 0.0);
    if idx - last_trade_bar < cooldown_bars { return; }

    // ── Compute cascade signals ───────────────────────────────────────────────
    let close_1ago = ctx.close_at(idx - 1);
    let close_2ago = if idx > 1 { ctx.close_at(idx - 2) } else { close_1ago };

    let bar_drop  = if close_1ago > 0.0 { (close_1ago - close) / close_1ago * 100.0 } else { 0.0 };
    let bar_rise  = if close_1ago > 0.0 { (close - close_1ago) / close_1ago * 100.0 } else { 0.0 };

    // Check if previous candle had a big drop (potential long liquidation cascade)
    let prev_drop = if close_2ago > 0.0 { (close_2ago - close_1ago) / close_2ago * 100.0 } else { 0.0 };
    let prev_rise = if close_2ago > 0.0 { (close_1ago - close_2ago) / close_2ago * 100.0 } else { 0.0 };

    let vol_spiked = volume > avg_vol * vol_cascade_mult;

    let rsi    = ctx.rsi(14);
    let stoch  = ctx.stoch_k(14);
    let bb_low = ctx.bb_lower(bb_period, bb_mult);
    let bb_up  = ctx.bb_upper(bb_period, bb_mult);

    // ── LONG entry: detect long liquidation cascade + snap-back ──────────────
    // Previous bar: large drop + volume spike (cascade)
    // Current bar: recovery bounce (confirms exhaustion)
    let long_cascade = prev_drop >= cascade_drop_pct && vol_spiked;
    let long_recovery = bar_rise >= recovery_confirm;
    let long_exhausted = rsi < rsi_exhaustion_low && stoch < stoch_low;
    let long_bb_signal = close_1ago < bb_low;  // cascade pushed below lower band

    if long_cascade && (long_recovery || long_exhausted) && long_bb_signal {
        ctx.buy(1.0);
        ctx.set_stop_loss(low - atr * atr_sl_mult);
        ctx.set_take_profit(close + atr * atr_tp_mult);
        ctx.set("last_trade_bar", idx);
        ctx.log("LONG liq_hunt: cascade_drop=" + prev_drop + "% vol_ratio=" + (volume/avg_vol) + " rsi=" + rsi + " stoch=" + stoch);
        return;
    }

    // ── SHORT entry: detect short squeeze cascade + reversal ──────────────────
    // Previous bar: large rise + volume spike (forced short covering)
    // Current bar: price starts fading (squeeze exhausted)
    let short_cascade  = prev_rise >= cascade_rise_pct && vol_spiked;
    let short_reversal = bar_drop >= recovery_confirm;
    let short_exhausted = rsi > rsi_exhaustion_high && stoch > stoch_high;
    let short_bb_signal = close_1ago > bb_up;  // squeeze pushed above upper band

    if short_cascade && (short_reversal || short_exhausted) && short_bb_signal {
        ctx.sell(1.0);  // open short
        ctx.set_stop_loss(high + atr * atr_sl_mult);
        ctx.set_take_profit(close - atr * atr_tp_mult);
        ctx.set("last_trade_bar", idx);
        ctx.log("SHORT liq_hunt: cascade_rise=" + prev_rise + "% vol_ratio=" + (volume/avg_vol) + " rsi=" + rsi + " stoch=" + stoch);
    }
}
