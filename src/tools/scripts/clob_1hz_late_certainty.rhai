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
// Sizing: 5% of balance per bet, further capped by the runner's live_sizing
// value (cap_per_bet in tick_runner_loop). For the $50 pilot this means
// stake = min(5% × balance, cap_per_bet). Set the runner to Fixed $1-$2 to
// keep ~30-40 trades of margin for statistical validation.
// NOTE: kelly_size_cap does NOT apply to the tick runner — only cap_per_bet
// (derived from live_sizing_mode) bounds the stake here.

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
    // Sizing: 5% of balance. Actual stake = min(5% × balance, cap_per_bet),
    // where cap_per_bet comes from the runner's live_sizing_value. For the
    // $50 pilot set the runner to Fixed $1-$2 to preserve trade count.
    let stake = 0.05;

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
