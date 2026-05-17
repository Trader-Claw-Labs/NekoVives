// Polymarket XRP UP/DOWN 5-min — Low-Vol Drift
// =============================================
// XRP is the lowest-volatility tracked asset:
//   σ5m = 0.105% (vs BTC 0.129%, DOGE 0.200%)
//   |r5m| p99 = 0.36% (smallest tails)
//   ATR(14) = 0.049%
//
// Behavior: prices hug window_open more, so binary outcomes are noisier.
// XRP also has the strongest weekend activity (wknd/wkd 0.66 — highest).
//
// Empirical winners on XRP (3168 windows):
//   - Setup 4 (P4 0.32-0.50 + drift >= 0.05 → sell NO):    67% WR / +$8.6
//   - Setup 1 (P4 0.50-0.68 + drift <= -0.05 → buy YES):   60% WR / +$1.7
//   - Setup 8 (P4 0.68-0.82 stable → sell NO):             31% WR / +$13.8
//   - Setup 10 (P4 0.82-0.95 fav fade):                    22% WR / +$16.1
// Removed: setups 2, 3, 6, 7, 9 (all negative on XRP).
//
// Smaller stakes (0.7×) because lower vol means lower edge per trade.

fn on_candle(ctx) {
    if ctx.open_positions > 0 { return; }

    let p4  = ctx.token_price;
    let p3  = ctx.token_price_prev;
    let dft = ctx.token_drift;

    if p4 <= 0.0 || p4 >= 1.0 { return; }

    ctx.set("debug_p4", p4);
    ctx.set("debug_p3", p3);
    ctx.set("debug_drift", dft);

    let has_drift = p3 > 0.0;

    // ═══ §1 XRP's narrow set of mid-band winners ═══
    if has_drift && p4 >= 0.32 && p4 <= 0.82 {

        // Setup 1: P4 0.50-0.68, drift <= -0.05 → buy YES (XRP 60% WR)
        // The drift fade works on XRP for the YES side (unlike BTC's negative result)
        if p4 >= 0.50 && p4 <= 0.68 && dft <= -0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 1.0);
            ctx.buy(0.7);
            return;
        }

        // Setup 4: P4 0.32-0.50, drift >= 0.05 → sell NO (XRP 67% WR)
        if p4 >= 0.32 && p4 < 0.50 && dft >= 0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 4.0);
            ctx.sell(1.0);
            return;
        }

        // Setup 8: P4 0.68-0.82, stable drift → sell NO  (XRP 31% WR / +$13)
        if p4 > 0.68 && p4 <= 0.82 && dft >= -0.05 && dft <= 0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 8.0);
            ctx.sell(0.7);
            return;
        }
    }

    // ═══ §2 Favorite fade (setup 10) ═══
    if ctx.index < 10 { return; }

    let window_open = ctx.window_open;
    if window_open <= 0.0 { return; }

    let close = ctx.close;
    let win_pct = (close - window_open) / window_open * 100.0;
    let rsi = ctx.rsi(14);

    ctx.set("debug_win_pct", win_pct);
    ctx.set("debug_rsi", rsi);

    if p4 >= 0.82 && p4 <= 0.95 {
        if win_pct < 0.02 && rsi > 56.0 {
            ctx.set("debug_regime", 2.0);
            ctx.set("debug_setup", 10.0);
            ctx.sell(0.7);
            return;
        }
    }
}
