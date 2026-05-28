// CLOB 1 HZ — Volatility Regime Switch: sample-then-trade based on early vol
// ========================================================================
//
// Hypothesis: the *optimal* strategy for a 5-min window depends on the
// volatility regime of the first 30 seconds.
//
//   - HIGH volatility (> 3 ¢ range in first 20 s): the window is trending.
//     Follow momentum — the Binance move is likely to persist.
//   - LOW volatility (<= 3 ¢ range): the window is ranging / mean-reverting.
//     Fade any mini-spike > 1.5 ¢ — price snaps back to the mid.
//
// Execution: two-phase per window.
//   Phase 1 (seconds 10-30): sample YES mid range, no trades.
//   Phase 2 (seconds 45-120): apply regime-specific logic.
//
// Gates:
//   - Spread <= 5.0 ¢ in Phase 2
//   - Momentum regime: Binance move >= |0.03%|, token in [0.40, 0.62]
//   - Mean-reversion regime: Binance move >= |0.015%|, token in [0.40, 0.60]
//
// Sizing: 8% of balance (same across regimes; edge size is controlled
// by gate tightness, not stake).

fn on_tick(ctx) {
    if ctx.position != 0 { return; }

    // Guard: Binance price must be valid
    if ctx.binance_price <= 0.0 { return; }

    // Track window_open price
    let wopen = ctx.get("window_open", 0.0);
    if wopen <= 0.0 {
        ctx.set("window_open", ctx.binance_price);
        wopen = ctx.binance_price;
    }
    if wopen <= 0.0 { return; }

    // ── Phase 1: sample volatility (seconds 10-30) ──
    if ctx.second_in_window >= 10 && ctx.second_in_window <= 30 {
        let high = ctx.get("sample_high", ctx.yes_mid);
        let low  = ctx.get("sample_low",  ctx.yes_mid);
        if ctx.yes_mid > high { ctx.set("sample_high", ctx.yes_mid); }
        if ctx.yes_mid < low  { ctx.set("sample_low",  ctx.yes_mid); }
        return;
    }

    // ── Phase 2: trade (seconds 45-120) ──
    if ctx.second_in_window < 45 || ctx.second_in_window > 120 {
        return;
    }

    let high = ctx.get("sample_high", 0.0);
    let low  = ctx.get("sample_low",  0.0);
    if high <= 0.0 || low <= 0.0 { return; }

    let range = high - low;
    let change_pct = (ctx.binance_price - wopen) / wopen * 100.0;

    if ctx.spread_pct > 5.0 { return; }

    // Extreme price guard
    if ctx.yes_ask < 0.03 || ctx.no_ask < 0.03 { return; }

    // Sizing: 25% of balance (capped at $600 by the backtest engine's
    // max_position_usd setting — actual stake will be min(25% balance, $600))
    let stake = 0.25;

    // ═══════════════════════════════════════════════════════════════
    // HIGH-VOLATILITY REGIME → follow momentum
    // ═══════════════════════════════════════════════════════════════
    if range > 0.03 {
        if change_pct > 0.03 && ctx.yes_ask < 0.62 {
            ctx.bet_yes(stake);
            return;
        }
        if change_pct < -0.03 && ctx.no_ask < 0.62 {
            ctx.bet_no(stake);
            return;
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // LOW-VOLATILITY REGIME → fade the mini-move
    // ═══════════════════════════════════════════════════════════════
    if range <= 0.03 {
        // BTC ticked UP but token spiked → fade back to mid
        if change_pct > 0.015 && ctx.yes_mid > 0.52 && ctx.no_ask < 0.50 {
            ctx.bet_no(stake);
            return;
        }
        // BTC ticked DOWN but token dipped → fade back to mid
        if change_pct < -0.015 && ctx.yes_mid < 0.48 && ctx.yes_ask < 0.50 {
            ctx.bet_yes(stake);
            return;
        }
    }
}
