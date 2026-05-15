// Polymarket BTC UP/DOWN 5m — Hybrid V2
// ======================================
// Combines the original hybrid signals (mom5, mom1, EMA14/EMA50 trend, RSI, volume)
// with the slug-aligned binary engine (window_open delta, token_price filter, debug logging).
//
// CTX FIELDS:
//   ctx.window_open    — BTC price at window start
//   ctx.window_minutes — resolution window size (e.g. 5)
//   ctx.token_price    — estimated YES token cost ($0.50–$0.97)
//
// ACTIONS: ctx.buy(frac) = bet UP, ctx.sell(frac) = bet DOWN

fn on_candle(ctx) {
    let min_idx     = 50;    // warm-up candles (reduced from 80 for window compatibility)
    let max_token_p = 0.65;  // skip if market has priced in the move too much
    let min_score   = 3.0;   // minimum composite score to bet
    let min_edge    = 0.0;   // minimum EV edge to bet (calibrate after runs)

    if ctx.index < min_idx { return; }
    if ctx.open_positions > 0 { return; }

    let window_open = ctx.window_open;
    if window_open <= 0.0 { return; }

    // Skip when token is too expensive (market already priced in)
    if ctx.token_price > max_token_p { ctx.set("debug_reason", 1.0); return; }
    if ctx.token_price < (1.0 - max_token_p) { ctx.set("debug_reason", 2.0); return; }

    let close  = ctx.close;
    let volume = ctx.volume;
    let idx    = ctx.index;

    // ── 1. Window delta (dominant signal, weight 5-7) ──
    // "Is BTC up or down vs the window open price?"
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

    // ── 2. Micro momentum — 1-candle direction (weight 2) ──
    let mom1_threshold = 0.02;
    if idx >= 1 {
        let close_1ago = ctx.close_at(idx - 1);
        if close_1ago > 0.0 {
            let mom1 = (close - close_1ago) / close_1ago * 100.0;
            if mom1 > mom1_threshold  { score += 2.0; }
            else if mom1 < -mom1_threshold { score -= 2.0; }
        }
    }

    // ── 3. Medium momentum — 5-candle direction (weight 2) ──
    let mom5_threshold = 0.08;
    if idx >= 5 {
        let close_5ago = ctx.close_at(idx - 5);
        if close_5ago > 0.0 {
            let mom5 = (close - close_5ago) / close_5ago * 100.0;
            if mom5 > mom5_threshold  { score += 2.0; }
            else if mom5 < -mom5_threshold { score -= 2.0; }
        }
    }

    // ── 4. EMA14 + EMA50 trend filter (weight 1.5) ──
    // Note: sma() is not supported — using ema(50) as proxy
    let ema14 = ctx.ema(14);
    let ema50 = ctx.ema(50);
    if close > ema14 && ema14 > ema50 {
        score += 1.5;  // strong uptrend
    } else if close < ema14 && ema14 < ema50 {
        score -= 1.5;  // strong downtrend
    } else if close > ema14 {
        score += 0.5;  // weak uptrend
    } else if close < ema14 {
        score -= 0.5;  // weak downtrend
    }

    // ── 5. RSI extremes (weight 1-2) ──
    let rsi = ctx.rsi(14);
    if rsi < 35.0 { score += 2.0; }       // oversold → expect bounce up
    else if rsi > 65.0 { score -= 2.0; }  // overbought → expect reversal down

    // ── 6. Volume surge confirmation (weight 1) ──
    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.9 + volume * 0.1);
    if volume > avg_vol * 1.1 {
        if win_pct > 0.0 { score += 1.0; }
        else if win_pct < 0.0 { score -= 1.0; }
    }

    // ── Debug logging ──
    ctx.set("debug_score", score);
    ctx.set("debug_win_pct", win_pct);
    ctx.set("debug_mom1", if idx >= 1 {
        let c1 = ctx.close_at(idx - 1);
        if c1 > 0.0 { (close - c1) / c1 * 100.0 } else { 0.0 }
    } else { 0.0 });
    ctx.set("debug_mom5", if idx >= 5 {
        let c5 = ctx.close_at(idx - 5);
        if c5 > 0.0 { (close - c5) / c5 * 100.0 } else { 0.0 }
    } else { 0.0 });
    ctx.set("debug_ema14", ema14);
    ctx.set("debug_ema50", ema50);
    ctx.set("debug_rsi", rsi);
    ctx.set("debug_token_price", ctx.token_price);
    ctx.set("debug_min_score", min_score);
    ctx.set("debug_avg_vol", avg_vol);
    ctx.set("debug_volume", volume);

    // ── Edge filter: only bet when expected value is positive ──
    let abs_score = if score >= 0.0 { score } else { -score };
    let est_prob = if abs_score >= 8.0 { 0.85 }
                   else if abs_score >= 7.0 { 0.80 }
                   else if abs_score >= 6.0 { 0.75 }
                   else if abs_score >= 5.0 { 0.70 }
                   else if abs_score >= 4.0 { 0.65 }
                   else { 0.62 };
    let implied_p = if score > 0.0 { ctx.token_price } else { 1.0 - ctx.token_price };
    let edge = est_prob - implied_p;
    ctx.set("debug_est_prob", est_prob);
    ctx.set("debug_implied_p", implied_p);
    ctx.set("debug_edge", edge);
    if edge < min_edge { return; }

    // ── Place bet based on composite score ──
    if score >= min_score {
        ctx.buy(0.25);   // bet YES: BTC ends UP
    } else if score <= -min_score {
        ctx.sell(0.25);  // bet NO: BTC ends DOWN
    }
}
