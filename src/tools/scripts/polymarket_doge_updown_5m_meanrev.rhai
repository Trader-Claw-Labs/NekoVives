// Polymarket DOGE UP/DOWN 5-min — Mean-Reversion + Wide Thresholds
// ==================================================================
// DOGE is the outlier of the tracked set:
//   - σ5m = 0.200%  (60% higher than BTC's 0.129%)
//   - |r5m| p99 = 0.65%  (fattest tails)
//   - ATR(14) = 0.101%  (nearly 2× BTC)
//   - autocorr lag-1 = -0.013  (the only negative — slight mean reversion)
//   - Hurst proxy ≈ 0.483  (mean-rev tilt)
//   - Accumulation share = 52.8%  (more volume on green candles → regime asymmetry)
//   - US/Asia vol = 0.93×  (Asia-stronger, unlike BTC's 1.28× US-dominance)
//
// Implications:
//   1. win_pct thresholds 2× the BTC defaults (0.04 instead of 0.02)
//   2. Mean-reversion add-on: at decision minute, if mom1 spike is opposite
//      direction to the current P4-implied probability, fade the spike.
//   3. Keep the universal winners (setups 4, 8) at full stake.
//   4. Remove setup 7 (worst on DOGE: -$24 / 47 bets / 13% WR).

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

    // ═══ §1 Mid-band drift fade (DOGE universal winners) ═══
    if has_drift && p4 >= 0.32 && p4 <= 0.82 {

        // Setup 1: P4 0.50-0.68, drift <= -0.05 → buy YES (53% WR)
        if p4 >= 0.50 && p4 <= 0.68 && dft <= -0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 1.0);
            ctx.buy(0.7);
            return;
        }

        // Setup 4: P4 0.32-0.50, drift >= 0.05 → sell NO  (66% WR / +$8.3)
        if p4 >= 0.32 && p4 < 0.50 && dft >= 0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 4.0);
            ctx.sell(1.0);
            return;
        }

        // Setup 6: smaller positive drift on cheap NO  (70% WR / +$2.1)
        if p4 >= 0.32 && p4 < 0.50 && dft >= 0.02 && dft < 0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 6.0);
            ctx.sell(1.0);
            return;
        }

        // Setup 8: P4 0.68-0.82, stable drift → sell NO  (32% WR / +$15.6)
        if p4 > 0.68 && p4 <= 0.82 && dft >= -0.05 && dft <= 0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 8.0);
            ctx.sell(1.0);
            return;
        }
    }

    // ═══ §2 DOGE-only mean-reversion fade on minute-4 spike ═══
    // If price spiked HARD in last 1m and P4 is still mid-range,
    // DOGE's negative autocorrelation suggests reversal in the 5th minute.
    if ctx.index < 14 { return; }

    let window_open = ctx.window_open;
    if window_open <= 0.0 { return; }

    let close = ctx.close;
    let close_1ago = ctx.close_at(ctx.index - 1);
    let mom1 = if close_1ago > 0.0 { (close - close_1ago) / close_1ago * 100.0 } else { 0.0 };
    let win_pct = (close - window_open) / window_open * 100.0;
    let rsi = ctx.rsi(14);

    ctx.set("debug_mom1", mom1);
    ctx.set("debug_win_pct", win_pct);
    ctx.set("debug_rsi", rsi);

    // DOGE moves 2× BTC → use 0.10% (vs 0.05%) as "extreme spike"
    let spike_threshold = 0.10;

    // Up-spike at minute 4 + token still mid-range → fade DOWN
    // (bet that 5th minute reverts)
    if mom1 > spike_threshold && p4 >= 0.35 && p4 <= 0.65 && rsi > 60.0 {
        ctx.set("debug_regime", 3.0);
        ctx.set("debug_setup", 11.0);
        ctx.sell(0.7);
        return;
    }
    // Down-spike at minute 4 + token still mid-range → fade UP
    if mom1 < -spike_threshold && p4 >= 0.35 && p4 <= 0.65 && rsi < 40.0 {
        ctx.set("debug_regime", 3.0);
        ctx.set("debug_setup", 12.0);
        ctx.buy(0.7);
        return;
    }

    // ═══ §3 Extreme zone — keep only YES underdog (setup 9 was +$12 on DOGE) ═══
    if p4 >= 0.05 && p4 <= 0.18 {
        // Use wider win_pct band for DOGE (2× vol)
        if win_pct > -0.04 && rsi < 44.0 {
            ctx.set("debug_regime", 2.0);
            ctx.set("debug_setup", 9.0);
            ctx.buy(0.7);
            return;
        }
    }
}
