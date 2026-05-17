// Polymarket SOL UP/DOWN 5-min — Favorite-Fade specialist
// =========================================================
// SOL has the strongest "favorite repricing" edge of any tracked asset:
//   - Setup 10 (P4 0.82-0.95, win_pct < 0.02, RSI > 56 → sell NO):
//       25 bets, 60% WR, +$111.3 P&L (best single setup across all assets)
//   - Setup 2 (P4 0.50-0.68 + drift >= 0.05 → sell NO):
//       118 bets, 53% WR, +$37.3 P&L
//   - Setup 4 (P4 0.32-0.50 + drift >= 0.05 → sell NO):
//       78 bets, 59% WR, ~breakeven (but high WR justifies inclusion)
//   - Setup 8 (P4 0.68-0.82 stable → sell NO): 47 bets, 32% WR, +$20
//
// Removed for SOL: setup 1 (-$17), setup 6 (-$4), setup 7 (-$1), setup 9 (-$3).
//
// Asymmetry: SOL is more bearish-tilted (46.7% of 5m windows close UP vs 50% BTC)
// so "sell NO" setups (= bet DOWN) dominate. Vol is similar to BTC (σ5m=0.115%).

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

    // ═══ §1 Mid-band — SOL's edge is on the NO (downside) side ═══
    if has_drift && p4 >= 0.32 && p4 <= 0.82 {

        // Setup 2: P4 0.50-0.68, drift >= 0.05 → sell NO  (53% WR, +$37)
        if p4 >= 0.50 && p4 <= 0.68 && dft >= 0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 2.0);
            ctx.sell(1.0);
            return;
        }

        // Setup 4: P4 0.32-0.50, drift >= 0.05 → sell NO  (59% WR, breakeven)
        if p4 >= 0.32 && p4 < 0.50 && dft >= 0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 4.0);
            ctx.sell(1.0);
            return;
        }

        // Setup 8: P4 0.68-0.82, stable drift → sell NO  (32% WR, +$20)
        if p4 > 0.68 && p4 <= 0.82 && dft >= -0.05 && dft <= 0.05 {
            ctx.set("debug_regime", 1.0);
            ctx.set("debug_setup", 8.0);
            ctx.sell(1.0);
            return;
        }
    }

    // ═══ §2 Extreme favorite fade — SOL's strongest edge ═══
    if ctx.index < 10 { return; }

    let window_open = ctx.window_open;
    if window_open <= 0.0 { return; }

    let close = ctx.close;
    let win_pct = (close - window_open) / window_open * 100.0;
    let rsi = ctx.rsi(14);

    ctx.set("debug_win_pct", win_pct);
    ctx.set("debug_rsi", rsi);

    // Setup 10 ENHANCED for SOL — extended P4 range and 1.5× stake
    // Empirical 60% WR; the deep favorite repeatedly fails to confirm.
    if p4 >= 0.78 && p4 <= 0.95 {
        if win_pct < 0.02 && rsi > 56.0 {
            ctx.set("debug_regime", 2.0);
            ctx.set("debug_setup", 10.0);
            ctx.sell(1.5);  // oversize: 60% WR with 0.78-0.95 token range is rare
            return;
        }
    }
}
