// DCA Bot v1.0 — Trader Claw (Dollar-Cost Averaging with trend filter + profit target exit)
// Periodically accumulates a position in fixed-size tranches regardless of price direction.
// Buys every N bars when trend is broadly neutral/bullish, then sells when avg cost + target% hit.
// Best suited for: long-horizon crypto backtests (1h, 4h, 1d intervals).
// Tune: dca_interval, tranche_pct, target_profit_pct, max_tranches.
fn on_candle(ctx) {
    // ── Parameters ──────────────────────────────────────────────────────────
    let dca_interval    = 24;   // Buy every 24 bars (e.g. once per day on 1h data)
    let tranche_pct     = 0.20; // Buy 20% of available balance per tranche (max 5 tranches)
    let max_tranches    = 5;    // Maximum position tranches to accumulate
    let target_profit   = 8.0;  // Exit when avg cost gives +8% gain
    let stop_loss_pct   = 15.0; // Exit all if avg cost down > 15% (portfolio protection)
    let trend_sma       = 50;   // Only DCA-buy when close is above this SMA (trend filter)
    let min_idx         = 55;   // Warmup for SMA-50

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close = ctx.close;
    let sma50 = ctx.sma(trend_sma);

    // Track how many tranches we have and average cost
    let tranches  = ctx.get("tranches", 0.0);
    let avg_cost  = ctx.get("avg_cost",  0.0);

    // ── Exit: take profit or stop loss ───────────────────────────────────────
    if ctx.position > 0.0 && avg_cost > 0.0 {
        let pnl_pct = (close - avg_cost) / avg_cost * 100.0;

        if pnl_pct >= target_profit {
            ctx.sell(1.0);
            ctx.set("tranches", 0.0);
            ctx.set("avg_cost", 0.0);
            ctx.set("next_buy", idx + dca_interval);
            ctx.log("DCA PROFIT EXIT: pnl=" + pnl_pct + "% avg_cost=" + avg_cost);
            return;
        }

        if pnl_pct <= -stop_loss_pct {
            ctx.sell(1.0);
            ctx.set("tranches", 0.0);
            ctx.set("avg_cost", 0.0);
            ctx.set("next_buy", idx + dca_interval * 2);  // longer cooldown after stop
            ctx.log("DCA STOP LOSS: pnl=" + pnl_pct + "% avg_cost=" + avg_cost);
            return;
        }
    }

    // ── DCA Buy logic ────────────────────────────────────────────────────────
    let next_buy = ctx.get("next_buy", 0.0);

    // Check if it's time to buy
    if idx < next_buy { return; }

    // Don't add more tranches than the max
    if tranches >= max_tranches { return; }

    // Trend filter: only buy when price is above SMA-50
    // (skip this filter if already in position — we're averaging down within a trend)
    if tranches == 0.0 && close < sma50 { return; }

    // Execute DCA buy (fractional buy of available balance)
    if ctx.balance > 0.0 {
        let t = tranches + 1.0;

        // Compute new average cost before buying
        let buy_qty = (ctx.balance * tranche_pct) / close;
        if ctx.position == 0.0 {
            ctx.set("avg_cost", close);
        } else {
            // Weighted average: existing position at avg_cost + new buy at close
            let new_avg = (avg_cost * ctx.position + close * buy_qty) / (ctx.position + buy_qty);
            ctx.set("avg_cost", new_avg);
        }

        ctx.buy(tranche_pct);
        ctx.set("tranches", t);
        ctx.set("next_buy", idx + dca_interval);
        ctx.log("DCA BUY tranche=" + t + " close=" + close + " avg_cost=" + avg_cost + " sma50=" + sma50);
    }
}
