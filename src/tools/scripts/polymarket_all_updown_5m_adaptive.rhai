// Polymarket ALL UP/DOWN 5-min — ATR-Adaptive Universal
// =======================================================
// Single script that adapts to any tracked asset (BTC/ETH/SOL/XRP/DOGE/HYPE)
// by scaling its decision thresholds with the asset's own realised volatility
// (ATR-14 expressed as % of price). Removes the need to maintain per-coin
// constants — the same code self-calibrates.
//
// HOW IT ADAPTS
// -------------
// The §2 extreme-zone fallback uses `win_pct` and `rsi(14)`. The original
// drift_v2_combo hardcoded win_pct band ±0.02 (BTC-tuned). DOGE moves ~2× BTC
// and saturates that band. Solution: scale the band by ATR.
//   band = max(0.02, 1.5 × atr_pct)
// On BTC (ATR ≈ 0.057%) → band ≈ 0.085%. On DOGE (ATR ≈ 0.101%) → band ≈ 0.15%.
//
// SETUP SELECTION (universal positives only)
// -----------------------------------------
// Kept setups (positive ROI on ≥4 of 6 assets):
//   Setup 4  — P4 0.32-0.50 + drift >= 0.05 → sell NO   (universal: 57-67% WR)
//   Setup 8  — P4 0.68-0.82 + stable drift  → sell NO   (universal: +$13 to +$67)
//
// Conditionally kept (with ATR-guard to filter noise):
//   Setup 2  — P4 0.50-0.68 + drift >= 0.05 → sell NO
//                only when atr_pct < 0.08 (skips DOGE / high-vol regimes
//                where mid-band positive-drift fades fail)
//   Setup 10 — P4 0.82-0.95 + win_pct <  band + RSI > 56 → sell NO
//                ATR-scaled win_pct band
//
// REMOVED globally (negative ROI on ≥3 of 6):
//   Setups 1, 3, 5, 6, 7  (over-fit to BTC microstructure or HYPE-only).
//
// DISABLED — Setup 9 (YES side):
//   Setup 9  — P4 0.05-0.18 + win_pct > -band + RSI < 44 → buy YES
//   Dry-run data: 628 bets, 10.8% WR across SOL/ETH/DOGE/BNB → -$1,500+
//   The model mis-estimates YES token prices in extreme zones vs real CLOB ask.
//   This script is now NO-side-only (all bets sell NO tokens).

fn on_candle(ctx) {
    if ctx.open_positions > 0 { return; }

    let p4  = ctx.token_price;
    let p3  = ctx.token_price_prev;
    let dft = ctx.token_drift;

    if p4 <= 0.0 || p4 >= 1.0 { return; }

    let has_drift = p3 > 0.0;

    // ── ATR-based volatility regime ───────────────────────────────────
    // Need at least 14 candles for ATR. If unavailable (e.g. HYPE without
    // underlying candles), atr() returns 0 → treat as low-vol BTC-like default.
    let atr_pct = if ctx.index >= 14 {
        let close = ctx.close;
        let atr = ctx.atr(14);
        if close > 0.0 && atr > 0.0 { atr / close * 100.0 } else { 0.06 }
    } else { 0.06 };

    let band = if atr_pct * 1.5 > 0.02 { atr_pct * 1.5 } else { 0.02 };
    let high_vol = atr_pct > 0.08;

    ctx.set("debug_p4", p4);
    ctx.set("debug_p3", p3);
    ctx.set("debug_drift", dft);
    ctx.set("debug_atr_pct", atr_pct);
    ctx.set("debug_band", band);
    ctx.set("debug_high_vol", if high_vol { 1.0 } else { 0.0 });

    // ═══ §1 Universal mid-band winners ═══
    if has_drift {

        // Setup 4 — universal winner across all 6 assets (57-67% WR)
        if p4 >= 0.32 && p4 < 0.50 && dft >= 0.05 {
            ctx.set("debug_setup", 4.0);
            ctx.sell(1.0);
            return;
        }

        // Setup 8 — universal winner (32-43% WR but YES expensive → NO cheap)
        if p4 > 0.68 && p4 <= 0.82 && dft >= -0.05 && dft <= 0.05 {
            ctx.set("debug_setup", 8.0);
            ctx.sell(1.0);
            return;
        }

        // Setup 2 — gated by low-vol regime (works on BTC/ETH/SOL/XRP/HYPE,
        // fails on DOGE high-vol regimes)
        if !high_vol && p4 >= 0.50 && p4 <= 0.68 && dft >= 0.05 {
            ctx.set("debug_setup", 2.0);
            ctx.sell(0.7);
            return;
        }
    }

    // ═══ §2 Extreme zones with ATR-scaled win_pct band ═══
    if ctx.index < 14 { return; }

    let window_open = ctx.window_open;
    if window_open <= 0.0 { return; }

    let close = ctx.close;
    let win_pct = (close - window_open) / window_open * 100.0;
    let rsi = ctx.rsi(14);

    ctx.set("debug_win_pct", win_pct);
    ctx.set("debug_rsi", rsi);

    // Setup 9 — DISABLED: YES underdog (628 bets, 10.8% WR in dry-run → net loss).
    // Real CLOB ask prices in extreme zones are much higher than model estimates.
    // Uncomment only after collecting 1Hz tick data to re-calibrate the YES threshold.
    //
    // if p4 >= 0.05 && p4 <= 0.18 {
    //     if win_pct > -band && rsi < 44.0 {
    //         ctx.set("debug_setup", 9.0);
    //         ctx.buy(1.0);
    //         return;
    //     }
    // }

    // Setup 10 — deep expensive YES → favorite fade
    if p4 >= 0.82 && p4 <= 0.95 {
        if win_pct < band && rsi > 56.0 {
            ctx.set("debug_setup", 10.0);
            ctx.sell(1.0);
            return;
        }
    }
}
