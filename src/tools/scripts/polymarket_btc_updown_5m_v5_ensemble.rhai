// Polymarket BTC UP/DOWN 5-min — v5 ensemble (strict_xl mid-band + setup 9)
// ===========================================================================
//
// Combines the two highest-confidence buckets surviving chain-truth backfill:
//
//   1. strict_xl mid-band drift fade (±0.04 gate, P4 ∈ [0.32, 0.68])
//      → empirically the engine that produced `BTC v2 strict xl` +$1852
//        over 483 chain-truth-resolved trades.
//
//   2. Setup 9 from drift_v4_safe (deep cheap YES, P4 ∈ [0.05, 0.18])
//      → 13 live BTC trades, 38.5% WR, +$1510 P&L (asymmetric ~5×
//        payout means hit-rate of 38% beats break-even).
//
// Sizing:
//   - mid-band setups: 1.0× (full ticket)
//   - setup 9: 0.5× (asymmetric variance dampener — same as v4_safe)
//
// Mutual exclusion: setup 9 only fires when mid-band gate did not fire.
// One ctx.buy/ctx.sell per window.
//
// Asset: BTC ONLY. Run side-by-side with `BTC v2 strict xl` and
// `Drift V4 Safe BTC` in paper. If after 100+ matched orders the WR
// stays >= 47% and P&L >= 0.5× the strict_xl baseline, promote to live
// at fixed $5 ticket per the post-backfill sizing policy.

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

    // §1 Mid-band strict_xl (±0.04 gate)
    if has_drift && p4 >= 0.32 && p4 <= 0.68 {
        if p4 >= 0.50 && p4 <= 0.68 {
            if dft <= -0.04 {
                ctx.set("debug_setup", 1.0);
                ctx.buy(1.0); return;
            }
            if dft >= 0.04 {
                ctx.set("debug_setup", 2.0);
                ctx.sell(1.0); return;
            }
        }
        if p4 >= 0.32 && p4 < 0.50 {
            if dft >= 0.04 {
                ctx.set("debug_setup", 3.0);
                ctx.sell(1.0); return;
            }
            if dft <= -0.04 {
                ctx.set("debug_setup", 4.0);
                ctx.buy(1.0); return;
            }
        }
    }

    // §2 Setup 9 — deep cheap YES, half size
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
            ctx.buy(0.5);
            return;
        }
    }
}
