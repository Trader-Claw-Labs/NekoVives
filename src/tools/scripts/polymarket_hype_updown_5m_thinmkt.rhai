// Polymarket HYPE UP/DOWN 5-min — Thin-Market / Drift-Only
// =========================================================
// HYPE is NOT on Binance spot, so this script does not consume RSI/ATR/
// window_open from underlying candles — all decisions are made purely from
// the Polymarket P3→P4 drift signal plus P4 zone classification.
//
// Empirical results on 3168 HYPE windows (2026-03-06 → 2026-05-15):
//   Total: 384 bets, 53.4% WR, +$0.294/$ stake (HIGHEST WR of all 6 assets)
//
//   Setup 2 (P4 0.50-0.68, drift >= 0.05 → sell NO):  128 bets, 52% WR, +$38.9
//   Setup 8 (P4 0.68-0.82, stable drift → sell NO):    44 bets, 43% WR, +$40.4
//   Setup 4 (P4 0.32-0.50, drift >= 0.05 → sell NO):   73 bets, 63% WR, +$7.3
//   Setup 7 (P4 0.18-0.32, stable drift → buy YES):    35 bets, 49% WR, +$32.7
//
// Note: setup 7 is BANNED globally on BTC/alts because it loses everywhere
// EXCEPT HYPE where it's +$32 / 49% WR. Hypothesis: HYPE bettors over-discount
// cheap YES candidates due to thin liquidity → systematic mispricing.
//
// Setup 1 (-$2), 3 (-$5), 6 (+$0.6) marginal — kept at half stake for
// liquidity sampling. No §2 extreme fallback (no underlying candles).

fn on_candle(ctx) {
    if ctx.open_positions > 0 { return; }

    let p4  = ctx.token_price;
    let p3  = ctx.token_price_prev;
    let dft = ctx.token_drift;

    if p4 <= 0.0 || p4 >= 1.0 { return; }
    // HYPE: skip if drift unavailable (no fallback possible without candles)
    if p3 <= 0.0 { return; }

    ctx.set("debug_p4", p4);
    ctx.set("debug_p3", p3);
    ctx.set("debug_drift", dft);

    // ═══ Mid-band drift fade — HYPE's full domain ═══

    // Setup 2: P4 0.50-0.68, drift >= 0.05 → sell NO  (128 bets, +$38.9)
    if p4 >= 0.50 && p4 <= 0.68 && dft >= 0.05 {
        ctx.set("debug_setup", 2.0);
        ctx.sell(1.0);
        return;
    }

    // Setup 4: P4 0.32-0.50, drift >= 0.05 → sell NO  (73 bets, 63% WR, +$7)
    if p4 >= 0.32 && p4 < 0.50 && dft >= 0.05 {
        ctx.set("debug_setup", 4.0);
        ctx.sell(1.0);
        return;
    }

    // Setup 6: P4 0.32-0.50, drift 0.02-0.05 → sell NO  (17 bets, 59% WR)
    if p4 >= 0.32 && p4 < 0.50 && dft >= 0.02 && dft < 0.05 {
        ctx.set("debug_setup", 6.0);
        ctx.sell(0.7);
        return;
    }

    // Setup 7 (HYPE-only): P4 0.18-0.32 stable drift → buy YES  (35 bets, 49% WR, +$32.7)
    // This setup loses on every other asset — HYPE-specific edge.
    if p4 >= 0.18 && p4 < 0.32 && dft >= -0.05 && dft <= 0.05 {
        ctx.set("debug_setup", 7.0);
        ctx.buy(1.0);
        return;
    }

    // Setup 8: P4 0.68-0.82, stable drift → sell NO  (44 bets, 43% WR, +$40.4)
    if p4 > 0.68 && p4 <= 0.82 && dft >= -0.05 && dft <= 0.05 {
        ctx.set("debug_setup", 8.0);
        ctx.sell(1.0);
        return;
    }
}
