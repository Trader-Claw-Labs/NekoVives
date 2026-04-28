// Grid Trading v1.1 — Trader Claw (KV-based price ladder simulation)
// Simulates a grid bot: places virtual buy orders at fixed price levels below current price
// and sell orders above. Profits from price oscillation without predicting direction.
//
// Architecture: KV store tracks each grid slot's status (0=empty, price=filled).
// On price drop to a level → virtual buy. On price rise past level + margin → virtual sell.
// Engine's buy()/sell() called proportionally to grid exposure changes for metric tracking.
//
// Best for: range-bound markets (sideways BTC, stablecoins, Polymarket markets near 50%).
// Tune: grid_count, grid_spacing_pct, profit_margin_pct, and initial price range.
fn on_candle(ctx) {
    // ── Parameters ──────────────────────────────────────────────────────────
    let grid_count        = 8;     // Number of grid levels (slots)
    let grid_spacing_pct  = 1.0;   // Distance between grid levels in %
    let profit_margin_pct = 0.8;   // Profit per grid level (net of fees)
    let max_hold_bars     = 200;   // Safety exit if grid stuck too long
    let min_idx           = 10;

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close = ctx.close;

    // ── Initialize grid on first candle ─────────────────────────────────────
    let initialized = ctx.get("grid_init", 0.0);
    if initialized == 0.0 {
        ctx.set("grid_init",    1.0);
        ctx.set("grid_base",    close);
        ctx.set("grid_filled",  0.0);
        ctx.set("grid_pnl",     0.0);
        ctx.set("grid_buys",    0.0);
        ctx.set("grid_sells",   0.0);

        let i = 0;
        while i < grid_count {
            ctx.set("slot_" + i, 0.0);
            i += 1;
        }
        ctx.set("debug_grid_base", close);
        return;
    }

    let base          = ctx.get("grid_base", close);
    let grid_filled   = ctx.get("grid_filled", 0.0);
    let grid_pnl      = ctx.get("grid_pnl", 0.0);

    // ── Compute grid levels ─────────────────────────────────────────────────
    let new_buys  = 0;
    let new_sells = 0;
    let new_filled = grid_filled;
    let new_pnl    = grid_pnl;

    let i = 0;
    while i < grid_count {
        let buy_price  = base * (1.0 - (i + 1) * grid_spacing_pct / 100.0);
        let sell_price = buy_price * (1.0 + profit_margin_pct / 100.0);
        let slot_key   = "slot_" + i;
        let slot_entry = ctx.get(slot_key, 0.0);

        if slot_entry == 0.0 {
            if close <= buy_price * 1.001 {
                ctx.set(slot_key, buy_price);
                new_filled += 1.0;
                new_buys   += 1;
                ctx.set("debug_last_buy_slot", i);
            }
        } else {
            if close >= sell_price * 0.999 {
                let pnl = (sell_price - slot_entry);
                new_pnl += pnl;
                new_filled -= 1.0;
                new_sells  += 1;
                ctx.set(slot_key, 0.0);
                ctx.set("debug_last_sell_pnl", pnl);
            }
        }

        i += 1;
    }

    ctx.set("grid_filled", new_filled);
    ctx.set("grid_pnl",    new_pnl);

    if new_buys > 0 && ctx.balance > 0.0 {
        let fraction = new_buys / (grid_count + 1);
        ctx.buy(fraction);
        ctx.set("grid_buys", ctx.get("grid_buys", 0.0) + new_buys);
    }

    if new_sells > 0 && ctx.position > 0.0 {
        let fraction = new_sells / (grid_count + 1);
        ctx.sell(fraction);
        ctx.set("grid_sells", ctx.get("grid_sells", 0.0) + new_sells);
    }

    // Safety: if all slots filled and price keeps falling, rebase the grid lower
    if new_filled >= grid_count && idx > ctx.get("last_rebase", 0.0) + max_hold_bars {
        ctx.set("grid_base", close);
        let j = 0;
        while j < grid_count {
            ctx.set("slot_" + j, 0.0);
            j += 1;
        }
        ctx.set("grid_filled", 0.0);
        ctx.set("last_rebase", idx);
        ctx.set("debug_rebase_pnl", new_pnl);
    }
}
