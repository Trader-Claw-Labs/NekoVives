// Spread Arbitrage v1.1 — Trader Claw (Statistical spread mean-reversion on single asset)
// Models "spread" as the deviation between a fast EMA and a slow EMA (synthetic spread).
// When the spread is extreme (Z-score > threshold), mean-reversion is expected.
// This captures the same alpha as cross-asset spread arb but works on one price feed.
//
// In a 2-asset deployment (e.g. BTC_UP vs BTC_DOWN on Polymarket):
//   - The "spread" = YES_price + NO_price should always equal ~100¢ (minus fees).
//   - Deviations > fee roundtrip create a risk-free arb opportunity.
//   - This script models that via the EMA spread proxy on a single symbol.
//
// Best for: Polymarket binary markets, BTC/ETH pairs with mean-reverting behavior.
fn on_candle(ctx) {
    // ── Parameters ──────────────────────────────────────────────────────────
    let fast_period     = 8;    // Fast EMA period
    let slow_period     = 34;   // Slow EMA period
    let zscore_period   = 20;   // Z-score lookback for spread normalization
    let entry_z         = 2.0;  // Enter when |Z| > 2.0 standard deviations
    let exit_z          = 0.3;  // Exit when |Z| < 0.3 (spread normalized)
    let rsi_period      = 14;
    let atr_sl_mult     = 1.5;
    let max_loss_pct    = 3.0;
    let take_pft_pct    = 5.0;
    let max_hold        = 20;
    let min_idx         = 50;   // Need warmup for slow EMA + Z-score lookback

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close = ctx.close;
    let atr   = ctx.atr(14);

    // ── Compute synthetic spread (EMA deviation) ─────────────────────────────
    let fast_ema = ctx.ema(fast_period);
    let slow_ema = ctx.ema(slow_period);
    let spread   = fast_ema - slow_ema;

    let spread_mean = ctx.get("spread_mean", spread);
    let spread_var  = ctx.get("spread_var",  0.0);
    let spread_n    = ctx.get("spread_n",    1.0);

    let alpha = 1.0 / zscore_period;
    let new_mean = spread_mean + alpha * (spread - spread_mean);
    let new_var  = (1.0 - alpha) * (spread_var + alpha * (spread - spread_mean) * (spread - spread_mean));

    ctx.set("spread_mean", new_mean);
    ctx.set("spread_var",  new_var);
    ctx.set("spread_n",    (spread_n + 1.0).min(zscore_period));

    let spread_std = new_var.sqrt();
    let zscore = if spread_std > 0.0001 { (spread - new_mean) / spread_std } else { 0.0 };

    // ── Exit logic ───────────────────────────────────────────────────────────
    if ctx.position != 0.0 {
        let ep        = ctx.entry_price;
        let bars_held = idx - ctx.entry_index;
        let raw_pnl   = if ep > 0.0 { (close - ep) / ep * 100.0 } else { 0.0 };
        let eff_pnl   = if ctx.position < 0.0 { -raw_pnl } else { raw_pnl };

        let z_abs = if zscore < 0.0 { -zscore } else { zscore };
        if z_abs < exit_z || eff_pnl >= take_pft_pct || eff_pnl <= -max_loss_pct || bars_held >= max_hold {
            ctx.sell(1.0);
            ctx.set("debug_exit_pnl", eff_pnl);
            ctx.set("debug_exit_z", zscore);
        }
        return;
    }

    // Wait until Z-score is reliable (sufficient history)
    if spread_n < zscore_period * 0.5 { return; }

    let rsi   = ctx.rsi(rsi_period);

    // ── Entry: Long when spread unusually negative ──
    if zscore < -entry_z && rsi < 45.0 {
        ctx.buy(0.25);
        ctx.set_stop_loss(close - atr * atr_sl_mult);
        ctx.set_take_profit(close * (1.0 + take_pft_pct / 100.0));
        ctx.set("debug_entry", 1.0);
        ctx.set("debug_z", zscore);
    }
    // ── Entry: Short when spread unusually positive ──
    else if zscore > entry_z && rsi > 55.0 {
        ctx.sell(1.0);
        ctx.set_stop_loss(close + atr * atr_sl_mult);
        ctx.set_take_profit(close * (1.0 - take_pft_pct / 100.0));
        ctx.set("debug_entry", -1.0);
        ctx.set("debug_z", zscore);
    }
}
