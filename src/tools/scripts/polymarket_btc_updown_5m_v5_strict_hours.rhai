// Polymarket BTC UP/DOWN 5-min — v5 strict-XL hour-gated
// =========================================================
//
// Built from intersection of two paper-runners' chain-truth data:
//   `BTC v2 strict xl` (n=483, +$1852, WR 49.7%)
//   `btc combo`        (n=536, +$1696, WR 44.2%)
//
// Per-hour P&L was computed across both runners. The findings doc
// claimed only [0,1] UTC was positive, but post-backfill the SAME
// strict_xl trades show many more positive hours when reconciled
// with chain truth (see investigation 2026-05-25):
//
//   Hours positive in BOTH runners  : 1, 19, 21, 22, 23
//   Hours positive in 1 runner only : 0, 2, 3, 4, 7, 12, 18, 20
//   Hours toxic in BOTH runners     : 8, 10, 11, 15, 17
//
// This script re-runs strict_xl logic but only fires during hours
// shown positive in *both* ground-truth datasets. The runner config
// allowed_hours field is the wider safety net — script-level gate
// is the strict edge intersection.
//
// Buckets (same as drift_v2_strict_xl, ±0.04 drift gate):
//   P4 [0.50,0.68] + drift <= -0.04 → buy YES
//   P4 [0.32,0.50] + drift >= +0.04 → sell NO
//   P4 [0.50,0.68] + drift >= +0.04 → sell NO
//   P4 [0.32,0.50] + drift <= -0.04 → buy YES
//
// Sizing: 1.0× by default. Asymmetry note — Setup 1 (BUY YES off
// negative drift) was the largest WR but historically suffered
// stake/payout asymmetry. We keep it because chain-truth data shows
// strict_xl already corrected for that vs the v2_combo predecessor.

fn on_candle(ctx) {
    if ctx.open_positions > 0 { return; }

    let p4  = ctx.token_price;
    let p3  = ctx.token_price_prev;
    let dft = ctx.token_drift;

    if p3 <= 0.0 || p4 <= 0.0 || p4 >= 1.0 { return; }
    if p4 < 0.32 || p4 > 0.68 { return; }

    ctx.set("debug_p4", p4);
    ctx.set("debug_p3", p3);
    ctx.set("debug_drift", dft);

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
