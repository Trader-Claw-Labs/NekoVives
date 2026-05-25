// Polymarket BTC UP/DOWN 5-min — v5 combo YES-only
// =====================================================
//
// Hypothesis built from `btc combo` paper-runner chain-truth data
// (n=536, +$1696 cumulative, WR 44.2%):
//   side YES : n=197  WR=44.7%  P&L=+$1934.59
//   side NO  : n=339  WR=44.0%  P&L=  -$238.27
//
// The NO leg is a net loser even though WR is similar — payout
// asymmetry kills it (NO bets pay <2× while losing 1× of stake).
// Dropping every ctx.sell() should lift expectancy without
// changing edge-detection logic.
//
// Setups KEPT (all BUY YES, from drift_v2_combo):
//   Setup 1  P4 [0.50,0.68] + drift <= -0.05  →  buy YES
//   Setup 3  P4 [0.50,0.68] + drift in [-0.05,-0.02)  →  buy YES
//   Setup 7  P4 [0.18,0.32] + stable drift  →  buy YES
//   Setup 9  P4 [0.05,0.18] + win_pct > -0.02 + RSI < 44 → buy YES
//
// Setups DROPPED (every NO seller):
//   Setup 2 / 4 / 6 / 8 / 10 — all ctx.sell() — collectively -$238
//   in btc combo over the same window.
//
// Asset: BTC ONLY. Apply same caveat as v4_safe — alts have different
// microstructure. Re-derive per-asset before deploying elsewhere.

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

    // §1 Mid-band drift fade — YES-side only
    if has_drift && p4 >= 0.18 && p4 <= 0.68 {

        // Setup 1: P4 [0.50, 0.68] + dft <= -0.05  →  buy YES
        if p4 >= 0.50 && p4 <= 0.68 && dft <= -0.05 {
            ctx.set("debug_setup", 1.0);
            ctx.buy(1.0);
            return;
        }

        // Setup 3: P4 [0.50, 0.68] + dft in [-0.05, -0.02)  →  buy YES
        if p4 >= 0.50 && p4 <= 0.68 && dft <= -0.02 && dft > -0.05 {
            ctx.set("debug_setup", 3.0);
            ctx.buy(1.0);
            return;
        }

        // Setup 7: P4 [0.18, 0.32) + stable drift  →  buy YES
        if p4 >= 0.18 && p4 < 0.32 && dft >= -0.05 && dft <= 0.05 {
            ctx.set("debug_setup", 7.0);
            ctx.buy(1.0);
            return;
        }
    }

    // §2 Setup 9: deep cheap YES — underdog repricing
    if ctx.index < 14 { return; }

    let window_open = ctx.window_open;
    if window_open <= 0.0 { return; }

    let win_pct = (ctx.close - window_open) / window_open * 100.0;
    let rsi = ctx.rsi(14);

    ctx.set("debug_win_pct", win_pct);
    ctx.set("debug_rsi", rsi);

    if p4 >= 0.05 && p4 <= 0.18 {
        if win_pct > -0.02 && rsi < 44.0 {
            ctx.set("debug_setup", 9.0);
            ctx.buy(1.0);
            return;
        }
    }
}
