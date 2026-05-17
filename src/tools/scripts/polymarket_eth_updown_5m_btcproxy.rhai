// Polymarket ETH UP/DOWN 5-min — BTC-Proxy Drift
// ================================================
// ETH-tuned variant of drift_v2_combo. ETH's volatility (σ5m = 0.120%) and
// momentum profile (autocorrelation +0.019) closely track BTC, but ETH
// spends only ~16% of windows in P4 mid-band vs 26% for BTC, so the mid-band
// setups fire less and the extreme-zone fallback contributes more.
//
// Empirical results on 2026-03-06 → 2026-05-15 (3168 windows):
//   - Setup 9 (deep YES underdog) wins big on ETH (26% WR but +$40 on 27 bets)
//   - Setup 8 (P4 0.68-0.82 NO) +$24 — kept
//   - Setup 4 (P4 0.32-0.50 NO drift>=0.05) 63% WR / +$9 — kept
//   - Setup 6 (smaller positive drift) 71% WR / +$3 — kept
//   - Setups 1, 3, 7 all negative on ETH — REMOVED
//   - Setup 2 (mid-band drift fade, sell NO) marginal — KEPT with smaller stake
//   - Setup 10 (favorite fade) 12% WR / +$2 — REMOVED (only +$2 on 25 bets)
//
// ctx.token_price       — P4 (current YES price, decision minute)
// ctx.token_price_prev  — P3 (YES price one minute earlier)
// ctx.token_drift       — P4 - P3
// ctx.window_open       — ETH spot at window open
// ctx.close             — ETH spot at decision minute close
// ctx.rsi(14)           — 14-period RSI on 1m candles

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

    // ═══ §1 Mid-band drift fade (limited to ETH-positive setups) ═══
    if has_drift && p4 >= 0.32 && p4 <= 0.82 {

        // Setup 2-lite: P4 0.50-0.68, strong positive drift → sell NO
        // ETH WR 34% but token cheap; keep at half stake to limit damage
        if p4 >= 0.50 && p4 <= 0.68 && dft >= 0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 2.0);
            ctx.sell(0.50);
            return;
        }

        // Setup 4: P4 0.32-0.50, drift >= 0.05 → sell NO  (ETH 63% WR, +$9)
        if p4 >= 0.32 && p4 < 0.50 && dft >= 0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 4.0);
            ctx.sell(1.0);
            return;
        }

        // Setup 6: P4 0.32-0.50, drift 0.02-0.05 → sell NO  (ETH 71% WR, +$3)
        if p4 >= 0.32 && p4 < 0.50 && dft >= 0.02 && dft < 0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 6.0);
            ctx.sell(1.0);
            return;
        }

        // Setup 8: P4 0.68-0.82, stable drift → sell NO  (ETH 37% WR, +$24)
        if p4 > 0.68 && p4 <= 0.82 && dft >= -0.05 && dft <= 0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 8.0);
            ctx.sell(1.0);
            return;
        }
    }

    // ═══ §2 Extreme zone — deep cheap YES continuation (setup 9) ═══
    // ETH's best setup: 27 bets / +$40 / 26% WR on cheap underdogs
    if ctx.index < 10 { return; }

    let window_open = ctx.window_open;
    if window_open <= 0.0 { return; }

    let close = ctx.close;
    let win_pct = (close - window_open) / window_open * 100.0;
    let rsi = ctx.rsi(14);

    ctx.set("debug_win_pct", win_pct);
    ctx.set("debug_rsi", rsi);

    if p4 >= 0.05 && p4 <= 0.18 {
        if win_pct > -0.02 && rsi < 44.0 {
            ctx.set("debug_regime", 2.0);
            ctx.set("debug_setup", 9.0);
            ctx.buy(1.0);
            return;
        }
    }
}
