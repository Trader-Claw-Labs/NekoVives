// CLOB 1 HZ — Spread Scalper: fade overpriced YES/NO in final 60 s of the window.
//
// Uses on_tick(ctx) API — called every second with live Polymarket CLOB bid/ask.
// Bets NO when YES is mispriced above 0.70 in the last minute; bets YES when
// YES is below 0.30. Skips wide-spread ticks (> 6 ¢) to avoid illiquid entries.
//
// ctx fields available:
//   ctx.ts_ms             — Unix timestamp in milliseconds
//   ctx.yes_bid / yes_ask — YES token best bid / ask (0-1)
//   ctx.yes_mid           — YES mid price
//   ctx.no_bid  / no_ask  — NO token best bid / ask (0-1)
//   ctx.spread_pct        — (yes_ask - yes_bid) × 100  (in cents)
//   ctx.binance_price     — Binance spot price at this tick
//   ctx.window_ts         — UNIX ts of current window open
//   ctx.window_secs_left  — seconds until window resolves
//   ctx.second_in_window  — seconds elapsed in current window (0 = first tick)
//   ctx.balance           — current balance
//   ctx.position          — 0 = flat, 1 = YES, -1 = NO
//   ctx.entry_price       — token price at entry
//   ctx.bet_yes(size)     — buy YES, size = fraction of balance (0-1)
//   ctx.bet_no(size)      — buy NO, size = fraction of balance (0-1)
//   ctx.set("key", val)   — persist a value across ticks
//   ctx.get("key", def)   — read persisted value

fn on_tick(ctx) {
    // Only trade in the final 60 seconds of the window
    if ctx.window_secs_left > 60 || ctx.window_secs_left <= 0 {
        return;
    }

    // Skip if spread > 6 cents — too wide, fill quality uncertain
    if ctx.spread_pct > 6.0 {
        return;
    }

    // Already in a position — hold until resolution
    if ctx.position != 0 {
        return;
    }

    let stake = 0.05;  // 5 % of balance per bet

    // YES overpriced above 0.70 → bet NO (fade the favourite)
    if ctx.yes_mid > 0.70 && ctx.no_ask < 0.35 {
        ctx.bet_no(stake);
        return;
    }

    // NO overpriced (YES underpriced below 0.30) → bet YES
    if ctx.yes_mid < 0.30 && ctx.yes_ask < 0.35 {
        ctx.bet_yes(stake);
    }
}
