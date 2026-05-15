// Polymarket BTC UPDOWN 5-min strategy — slug-aligned engine, fires at minute 4
//
// Runs ONCE per market window at the decision candle (last complete 1m before close).
// Based on the composite weighted signal from the Archetapp gist.
//
// CTX FIELDS (slug engine only):
//   ctx.window_open    — BTC price at the start of this 5-min market
//   ctx.window_minutes — resolution window size (e.g. 5)
//   ctx.token_price    — estimated YES token cost ($0.50–$0.97)
//
// ACTIONS: ctx.buy(frac) = bet UP, ctx.sell(frac) = bet DOWN
//
// EDGE FILTER: score → estimated win probability mapping (calibrate with live data):
//   abs_score >= 8 → 85%   abs_score >= 7 → 80%   abs_score >= 6 → 75%
//   abs_score >= 5 → 70%   abs_score >= 4 → 65%   abs_score >= 3 → 62%
//   edge = est_prob - implied_p  (positive = we have an advantage)

fn on_candle(ctx) {
    let min_idx     = 30;    // warm-up candles before betting
    let max_token_p = 0.65;  // max token price — above this risk/reward is too poor
                             // at $0.65: win pays +$0.54, loss = -$1.00 → break-even at 65%
                             // at $0.82: win pays +$0.22, loss = -$1.00 → break-even at 82%
    let min_score   = 3.0;   // minimum composite score to bet
    let min_edge    = 0.0;   // minimum edge over implied probability (increase after calibration)

    if ctx.index < min_idx { return; }
    if ctx.open_positions > 0 { return; }

    let window_open = ctx.window_open;
    if window_open <= 0.0 { return; }

    // Skip when token is too expensive (market already priced in the move)
    //if ctx.token_price > max_token_p { ctx.set("debug_reason", 1.0); return; }
    //if ctx.token_price < (1.0 - max_token_p) { ctx.set("debug_reason", 2.0); return; }

    let close = ctx.close;
    let idx   = ctx.index;

    // 1. Window delta — dominant signal (weight 5-7)
    //    "Is BTC up or down vs the window open price?"
    let win_pct = if window_open > 0.0 { (close - window_open) / window_open * 100.0 } else { 0.0 };

    let score = 0.0;
    if win_pct >  0.10 { score +=  7.0; }
    else if win_pct >  0.02 { score +=  5.0; }
    else if win_pct >  0.005 { score +=  3.0; }
    else if win_pct >  0.001 { score +=  1.0; }
    else if win_pct < -0.001 { score -=  1.0; }
    else if win_pct < -0.005 { score -=  3.0; }
    else if win_pct < -0.02  { score -=  5.0; }
    else if win_pct < -0.10  { score -=  7.0; }

    // 2. Micro momentum — last 2 candles direction (weight 2)
    if idx >= 2 {
        let prev2 = ctx.close_at(idx - 2);
        if prev2 > 0.0 {
            let mom2 = (close - prev2) / prev2 * 100.0;
            if mom2 >  0.003 { score += 2.0; }
            else if mom2 < -0.003 { score -= 2.0; }
        }
    }

    // 3. Acceleration — current candle move vs 2 candles ago (weight 1.5)
    if idx >= 3 {
        let c1 = ctx.close_at(idx - 1);
        let c2 = ctx.close_at(idx - 2);
        let c3 = ctx.close_at(idx - 3);
        if c2 > 0.0 && c3 > 0.0 {
            let recent = close - c1;
            let prior  = c2 - c3;
            if recent > 0.0 && recent > prior { score += 1.5; }
            else if recent < 0.0 && recent < prior { score -= 1.5; }
        }
    }

    // 4. EMA crossover 9/21 (weight 1)
    let ema9  = ctx.ema(9);
    let ema21 = ctx.ema(21);
    if ema9 > ema21 { score += 1.0; } else { score -= 1.0; }

    // 5. RSI extremes (weight 1-2)
    let rsi = ctx.rsi(14);
    if rsi < 25.0 { score += 2.0; }
    else if rsi > 75.0 { score -= 2.0; }

    // 6. Volume surge confirms direction (weight 1)
    let avg_vol = ctx.get("avg_vol", ctx.volume);
    ctx.set("avg_vol", avg_vol * 0.9 + ctx.volume * 0.1);
    if ctx.volume > avg_vol * 1.5 {
        if win_pct > 0.0 { score += 1.0; } else if win_pct < 0.0 { score -= 1.0; }
    }

    // ── Edge filter ───────────────────────────────────────────────────────────
    // Map abs(score) → estimated win probability (initial values, calibrate with data).
    // Collect "debug_edge" logs for a few days before tightening min_edge.
    let abs_score = if score >= 0.0 { score } else { -score };
    let est_prob = if abs_score >= 8.0 { 0.85 }
                   else if abs_score >= 7.0 { 0.80 }
                   else if abs_score >= 6.0 { 0.75 }
                   else if abs_score >= 5.0 { 0.70 }
                   else if abs_score >= 4.0 { 0.65 }
                   else { 0.62 };  // score 3.0 – 3.9

    // Implied probability of the side we'd bet
    let implied_p = if score > 0.0 { ctx.token_price }
                    else { 1.0 - ctx.token_price };
    let edge = est_prob - implied_p;

    // Debug — always log for calibration (adjust min_edge after reviewing data)
    ctx.set("debug_score",      score);
    ctx.set("debug_win_pct",    win_pct);
    ctx.set("debug_ema9",       ema9);
    ctx.set("debug_ema21",      ema21);
    ctx.set("debug_rsi",        rsi);
    ctx.set("debug_token_price", ctx.token_price);
    ctx.set("debug_min_score",  min_score);
    ctx.set("debug_avg_vol",    avg_vol);
    ctx.set("debug_volume",     ctx.volume);
    ctx.set("debug_est_prob",   est_prob);
    ctx.set("debug_implied_p",  implied_p);
    ctx.set("debug_edge",       edge);

    // Skip if no positive edge over the market's implied probability
    if edge < min_edge { return; }

    // Place bet based on composite score
    if score >= min_score {
        ctx.buy(0.25);   // bet YES: BTC ends UP
    } else if score <= -min_score {
        ctx.sell(0.25);  // bet NO: BTC ends DOWN
    }
}
