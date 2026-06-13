// clob_events — Late Certainty (on_event port of clob_1hz_late_certainty)
// =========================================================================
//
// Identical logic to clob_1hz_late_certainty.rhai (setups A-D: take the certain
// side in the final 30-35s when Binance has clearly moved but the book lags),
// but driven by the MILLISECOND event stream so it can be backtested at the
// sub-second order latency of a faster VPS — the 1Hz engine can only simulate
// ≥1000ms. Run via market_type = clob_events with Order latency = your VPS's
// real round-trip (e.g. 50/110/150ms).
//
// Window-open anchoring uses a per-window key (ctx.set persists across the whole
// run in on_event, unlike the per-window-reset on_tick ctx).

fn on_event(ctx) {
    if ctx.position != 0 { return; }
    if ctx.binance_price <= 0.0 { return; }
    // React on book updates only (trades don't change our gates).
    if ctx.event_kind != "book" { return; }

    // Anchor window-open Binance price once per window.
    let wkey = "wopen_" + ctx.window_ts;
    let wopen = ctx.get(wkey, 0.0);
    if wopen <= 0.0 {
        ctx.set(wkey, ctx.binance_price);
        wopen = ctx.binance_price;
    }
    if wopen <= 0.0 { return; }

    // Phase: final 30-35s, excluding the last 5s (avoid no-fill).
    if ctx.window_secs_left > 35 || ctx.window_secs_left <= 5 { return; }
    // Liquidity gate.
    if ctx.spread_pct > 4.0 { return; }
    // Extreme-price guard.
    if ctx.yes_ask < 0.03 || ctx.no_ask < 0.03 { return; }

    let change_pct = (ctx.binance_price - wopen) / wopen * 100.0;
    let stake = 0.05;

    // Setup A: BTC clearly UP, YES not at ceiling.
    if change_pct > 0.04 && ctx.yes_ask < 0.80 { ctx.bet_yes(stake); return; }
    // Setup B: BTC clearly DOWN, NO not at ceiling.
    if change_pct < -0.04 && ctx.no_ask < 0.80 { ctx.bet_no(stake); return; }
    // Setup C: YES overpriced but BTC DOWN → fade.
    if ctx.yes_mid > 0.75 && change_pct < -0.02 && ctx.no_ask < 0.30 { ctx.bet_no(stake); return; }
    // Setup D: YES underpriced but BTC UP → fade.
    if ctx.yes_mid < 0.25 && change_pct > 0.02 && ctx.yes_ask < 0.30 { ctx.bet_yes(stake); return; }
}
