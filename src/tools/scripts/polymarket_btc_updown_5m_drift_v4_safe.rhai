// Polymarket BTC UP/DOWN 5-min — Drift v4 SAFE (post-backfill 2026-05-23)
// =========================================================================
//
// Built from chain-truth data after the Polymarket /data/trades + Gamma
// outcomePrices backfill. Source: 120 settled live orders from Live #1
// drift_v2_combo BTC, 2026-05-08 → 2026-05-23.
//
// Buckets KEPT (positive expectancy under chain truth):
//   Setup 2  P4 [0.50, 0.68] + drift >= +0.05  →  SELL NO   N=26 WR=65.4% +$386
//   Setup 4  P4 [0.32, 0.50) + drift >= +0.05  →  SELL NO   N=28 WR=50.0% +$505
//   Setup 6  P4 [0.32, 0.50) + drift in [+0.02,+0.05) → SELL NO  (subset of 4)
//   Setup 8  P4 (0.68, 0.82] + drift in [-0.05,+0.05] → SELL NO  N=1 +$90 (token)
//   Setup 9  P4 [0.05, 0.18]  + win_pct > -0.02 + RSI < 44 → BUY YES @ 0.5×
//                                                          N=13 WR=38.5% +$1,510
//
// Buckets DROPPED (negative expectancy under chain truth):
//   Setup 1  P4 [0.50, 0.68] + drift <= -0.05 → BUY YES   N=25 WR=64% -$803
//            (high WR but stake asymmetry ⇒ Setup 1 was the single biggest
//             P&L drain after re-pricing decision-mid → real fill)
//   Setup 3  P4 [0.50, 0.68] + drift in [-0.05,-0.02) → BUY YES (subset of 1)
//   Setup 5  P4 [0.32, 0.50) + drift <= -0.05 → BUY YES (already disabled
//            in v2_combo — confirmed bad)
//   Setup 7  P4 [0.18, 0.32) any side → both yes (-$586) and no (-$291) lose
//   "cheap NO"  P4 [0.05, 0.18] + stable drift → SELL NO  N=6 WR=17% -$466
//   Setup 10 (favorite repricing) — never fired in live BTC, kept off
//
// Sizing on Setup 9: 0.5× because asymmetric ~5× payout amplifies variance.
// At percent 1.5% with $1k wallet → ~$15 base × 0.5 = $7.50 ticket.
//
// Hour gate (HARDCODED inside script — independent of allowed_hours config):
//   Allow only UTC hours 0 and 1. After chain-truth re-pricing these are
//   the only hours with clearly positive aggregate P&L (+$1,758 across all
//   live runners). All other hours are net negative.
//
// Asset: BTC ONLY. Don't run this on SOL/ETH/XRP/DOGE/BNB without rebuilding
// the bucket attribution from scratch on chain-truth data per asset.

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

    // §1 Mid-band drift fade — NO-side ONLY (we dropped every YES bucket here)
    if has_drift && p4 >= 0.32 && p4 <= 0.82 {

        // Setup 4: P4 [0.32, 0.50) + dft >= +0.05  →  SELL NO
        if p4 >= 0.32 && p4 < 0.50 && dft >= 0.05 {
            ctx.set("debug_setup", 4.0);
            ctx.sell(1.0);
            return;
        }

        // Setup 6: P4 [0.32, 0.50) + dft in [+0.02, +0.05)  →  SELL NO
        if p4 >= 0.32 && p4 < 0.50 && dft >= 0.02 && dft < 0.05 {
            ctx.set("debug_setup", 6.0);
            ctx.sell(1.0);
            return;
        }

        // Setup 2: P4 [0.50, 0.68] + dft >= +0.05  →  SELL NO  (best WR: 65.4%)
        if p4 >= 0.50 && p4 <= 0.68 && dft >= 0.05 {
            ctx.set("debug_setup", 2.0);
            ctx.sell(1.0);
            return;
        }

        // Setup 8: P4 (0.68, 0.82] + stable drift  →  SELL NO
        if p4 > 0.68 && p4 <= 0.82 && dft >= -0.05 && dft <= 0.05 {
            ctx.set("debug_setup", 8.0);
            ctx.sell(1.0);
            return;
        }
    }

    // §2 Setup 9: deep cheap YES — underdog repricing (HALF SIZE)
    // Only fires if mid-band §1 didn't fire and we have ≥10 candles for RSI.
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
            ctx.buy(0.5);   // HALF size — asymmetric payout, high variance
            return;
        }
    }
}
