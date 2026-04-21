// Polymarket BTC UPDOWN 5-min strategy u2014 slug-aligned engine, fires at minute 4
//
// Runs ONCE per market window at the decision candle (last complete 1m before close).
// Based on the composite weighted signal from the Archetapp gist.
//
// CTX FIELDS (slug engine only):
//   ctx.window_open    u2014 BTC price at the start of this 5-min market
//   ctx.window_minutes u2014 resolution window size (e.g. 5)
//   ctx.token_price    u2014 estimated YES token cost ($0.50u2013$0.97)
//
// ACTIONS: ctx.buy(frac) = bet UP, ctx.sell(frac) = bet DOWN

fn on_candle(ctx) {
    let min_idx     = 30;    // warm-up candles before betting
    let max_token_p = 0.82;  // skip if market has priced in the move too much
    let min_score   = 3.0;   // minimum composite score to bet

    if ctx.index < min_idx { return; }
    if ctx.open_positions > 0 { return; }

    let window_open = ctx.window_open;
    if window_open <= 0.0 { return; }

    // Skip when token is too expensive (market already priced in)
    if ctx.token_price > max_token_p { return; }
    if ctx.token_price < (1.0 - max_token_p) { return; }

    let close = ctx.close;
    let idx   = ctx.index;

    // 1. Window delta u2014 dominant signal (weight 5-7)
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

    // 2. Micro momentum u2014 last 2 candles direction (weight 2)
    if idx >= 2 {
        let prev2 = ctx.close_at(idx - 2);
        if prev2 > 0.0 {
            let mom2 = (close - prev2) / prev2 * 100.0;
            if mom2 >  0.003 { score += 2.0; }
            else if mom2 < -0.003 { score -= 2.0; }
        }
    }

    // 3. Acceleration u2014 current candle move vs 2 candles ago (weight 1.5)
    if idx >= 3 {
        let c1 = ctx.close_at(idx - 1);
        let c2 = ctx.close_at(idx - 2);
        let c3 = ctx.close_at(idx - 3);
        if c2 > 0.0 && c3 > 0.0 {
            let recent = close - c1;
            let prior  = c2 - c3;
            if recent > 0.0 && recent > prior { score += 1.5; }   // accelerating up
            else if recent < 0.0 && recent < prior { score -= 1.5; }  // accelerating down
        }
    }

    // 4. EMA crossover 9/21 (weight 1)
    let ema9  = ctx.ema(9);
    let ema21 = ctx.ema(21);
    if ema9 > ema21 { score += 1.0; } else { score -= 1.0; }

    // 5. RSI extremes (weight 1-2)
    let rsi = ctx.rsi(14);
    if rsi < 25.0 { score += 2.0; }       // oversold -> expect bounce up
    else if rsi > 75.0 { score -= 2.0; }  // overbought -> expect reversal down

    // 6. Volume surge (weight 1)
    let avg_vol = ctx.get("avg_vol", ctx.volume);
    ctx.set("avg_vol", avg_vol * 0.9 + ctx.volume * 0.1);
    if ctx.volume > avg_vol * 1.5 {
        // Volume surge confirms current direction
        if win_pct > 0.0 { score += 1.0; } else if win_pct < 0.0 { score -= 1.0; }
    }

    // Place bet based on composite score
    if score >= min_score {
        ctx.buy(0.25);  // bet YES: BTC ends UP
    } else if score <= -min_score {
        ctx.sell(0.25); // bet NO: BTC ends DOWN
    }
}
