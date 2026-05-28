// CLOB 1 HZ — Late Certainty: fade uncertainty in the final 30-45 seconds
// ========================================================================
//
// Hypothesis: when a window is about to close, the Binance spot price has
// already revealed the likely outcome. If the move is clear (|change| > 0.04%)
// but the Polymarket token is still priced away from the ceiling (YES < 0.80
// for UP moves, NO < 0.80 for DOWN moves), there is value in buying the
// "correct" side at a discount. Conversely, if the token is overpriced in
// the wrong direction, fade it.
//
// Entry window: last 35 seconds, excluding final 5 seconds (avoid no-fill).
//
// Gates:
//   - Spread <= 4.0 ¢
//   - Binance move >= |0.04%| from window_open (direction is clear)
//   - Price not at ceiling/floor (room for positive EV)
//
// Setups:
//   A. BTC UP  + YES ask < 0.80  → buy YES (certainty at discount)
//   B. BTC DOWN + NO ask  < 0.80  → buy NO  (certainty at discount)
//   C. YES > 0.75 but BTC DOWN   → buy NO  (fade wrong-side favourite)
//   D. YES < 0.25 but BTC UP     → buy YES (fade wrong-side underdog)
//
// Sizing: 10% of balance (higher confidence = slightly larger stake).

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

    // Phase: final 30-35 seconds, but not the last 5s (avoid no-fill)
    if ctx.window_secs_left > 35 || ctx.window_secs_left <= 5 {
        return;
    }

    // Liquidity gate
    if ctx.spread_pct > 4.0 { return; }

    // Extreme price guard: avoid buying tokens priced near zero (payout explosion)
    if ctx.yes_ask < 0.03 || ctx.no_ask < 0.03 { return; }

    let change_pct = (ctx.binance_price - wopen) / wopen * 100.0;
    // Sizing: 35% of balance (capped at $600 by the backtest engine's
    // max_position_usd setting — actual stake will be min(35% balance, $600))
    let stake = 0.35;

    // ── Setup A: BTC clearly UP, YES not at ceiling ──
    if change_pct > 0.04 && ctx.yes_ask < 0.80 {
        ctx.bet_yes(stake);
        return;
    }

    // ── Setup B: BTC clearly DOWN, NO not at ceiling ──
    if change_pct < -0.04 && ctx.no_ask < 0.80 {
        ctx.bet_no(stake);
        return;
    }

    // ── Setup C: YES overpriced but BTC is DOWN → fade ──
    if ctx.yes_mid > 0.75 && change_pct < -0.02 && ctx.no_ask < 0.30 {
        ctx.bet_no(stake);
        return;
    }

    // ── Setup D: YES underpriced but BTC is UP → fade ──
    if ctx.yes_mid < 0.25 && change_pct > 0.02 && ctx.yes_ask < 0.30 {
        ctx.bet_yes(stake);
        return;
    }
}
