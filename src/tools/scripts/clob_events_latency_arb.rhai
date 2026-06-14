// clob_events — Latency Arb Demo (on_event, sub-second)
// =====================================================
//
// Demonstrates the event-driven engine API (BACKTEST_ENGINE_PLAN Fase C).
// Runs once per market event (book update or trade) at its REAL ms timestamp,
// with the two-sided book reconstructed by the engine.
//
// Thesis (the VPS latency-arb idea, tested at ms resolution): when Binance has
// clearly moved within the window but the Polymarket book still prices the
// "correct" side below a ceiling, take it. With feed/order latency simulated,
// the engine fills against the FUTURE book at t + order_latency_ms — so this
// backtest answers "does the edge survive VPS latency?" honestly.
//
// ctx fields (event-driven):
//   ctx.event_kind   "book" | "trade"        ctx.event_token  "yes"|"no"|"unknown"
//   ctx.event_price  trade price / book ask  ctx.ts_ms        perceived time (incl. feed latency)
//   ctx.yes_bid/ctx.yes_ask/ctx.yes_mid      ctx.no_bid/ctx.no_ask   (live reconstructed book)
//   ctx.binance_price  ctx.window_secs_left  ctx.window_ts    ctx.spread_pct
//   ctx.balance  ctx.position  ctx.entry_price  ctx.window_yes_won
//   ctx.bet_yes(frac) ctx.bet_no(frac)  ctx.set(k,v)  ctx.get(k,def)

fn on_event(ctx) {
    if ctx.position != 0 { return; }
    if ctx.binance_price <= 0.0 { return; }

    // Window-open Binance price: engine-anchored ctx.window_open_price (live-parity;
    // kv persists globally so self-tracking it would freeze on the first window).
    let wopen = ctx.window_open_price;
    if wopen <= 0.0 { return; }

    // Only act in the final 35s, not the last 3s (avoid no-fill), once the
    // direction is clear. React only to fresh book updates.
    if ctx.event_kind != "book" { return; }
    if ctx.window_secs_left > 35 || ctx.window_secs_left <= 3 { return; }
    if ctx.spread_pct > 4.0 { return; }

    let change_pct = (ctx.binance_price - wopen) / wopen * 100.0;
    let stake = 0.05;

    // Clear UP move, YES still cheap → take YES.
    if change_pct > 0.04 && ctx.yes_ask >= 0.05 && ctx.yes_ask < 0.80 {
        ctx.bet_yes(stake);
        return;
    }
    // Clear DOWN move, NO still cheap → take NO.
    if change_pct < -0.04 && ctx.no_ask >= 0.05 && ctx.no_ask < 0.80 {
        ctx.bet_no(stake);
        return;
    }
}
