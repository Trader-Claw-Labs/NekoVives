// CLOB 1 HZ — Early Oracle: pre-roll momentum in seconds 15-90
// ========================================================================
//
// Hypothesis: in the first 60-90 seconds of a 5-min window, Binance spot
// has already moved directionally, but the Polymarket YES token is still
// pricing at ~0.50 because market-makers have not fully updated. This
// creates a temporary edge: bet early, before the token catches up.
//
// Entry window: 15-90 seconds after window open.
//   - < 15 s: not enough data, Binance price may still be settling
//   - > 90 s: window too mature; minute-4 strategies already priced in
//
// Gates:
//   - Binance move >= |0.03%| from window_open (statistically significant)
//   - YES mid in [0.40, 0.60] (not yet repriced)
//   - Spread <= 5.0 ¢ (avoid illiquid entries)
//
// Sizing: 8% of balance (moderate — pre-roll has more variance than
// minute-4 entries, so we keep position smaller).

fn on_tick(ctx) {
    if ctx.position != 0 { return; }

    // Track window_open price from first valid Binance tick
    let wopen = ctx.get("window_open", 0.0);
    if wopen <= 0.0 && ctx.binance_price > 0.0 {
        ctx.set("window_open", ctx.binance_price);
        wopen = ctx.binance_price;
    }
    if wopen <= 0.0 { return; }

    // Phase: early window only
    if ctx.second_in_window < 15 || ctx.second_in_window > 90 {
        return;
    }

    // Liquidity gate
    if ctx.spread_pct > 5.0 { return; }

    let change_pct = (ctx.binance_price - wopen) / wopen * 100.0;

    // Minimum directional signal
    if change_pct.abs() < 0.03 { return; }

    // Token must still be in the "undecided" band
    if ctx.yes_mid < 0.40 || ctx.yes_mid > 0.60 { return; }

    let stake = 0.08;

    // BTC moving UP  → YES underpriced  → buy YES
    if change_pct > 0.0 && ctx.yes_ask < 0.58 {
        ctx.bet_yes(stake);
        return;
    }

    // BTC moving DOWN → NO underpriced   → buy NO
    if change_pct < 0.0 && ctx.no_ask < 0.58 {
        ctx.bet_no(stake);
        return;
    }
}
