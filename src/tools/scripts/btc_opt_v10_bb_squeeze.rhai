// OPT v10: Momentum + ATR expansion breakout — adapted for Polymarket binary mode
// Note: ctx.bollinger_upper/lower not supported — using ATR bands as proxy
// ATR expansion (bb_expand) now directional only, not counted for both sides
fn on_candle(ctx) {
    let min_idx           = 80;
    let max_token_p       = 0.65;
    let min_edge          = 0.0;
    let min_score         = 2.0;
    let lookback_4        = 6;
    let mom_threshold     = 1.8;
    let mom1_threshold    = 0.6;
    let rsi_oversold      = 30.0;
    let rsi_overbought    = 70.0;
    let min_vol_mult      = 1.2;
    let bb_mult           = 2.0;

    let idx = ctx.index;
    if idx < min_idx { return; }
    if ctx.open_positions > 0 { return; }

    let window_open = ctx.window_open;
    if window_open <= 0.0 { return; }
    if ctx.token_price > max_token_p { ctx.set("debug_reason", 1.0); return; }
    if ctx.token_price < (1.0 - max_token_p) { ctx.set("debug_reason", 2.0); return; }

    let close  = ctx.close;
    let volume = ctx.volume;
    let atr    = ctx.atr(14);
    let ema20  = ctx.ema(20);
    let bb_upper = ema20 + atr * bb_mult;
    let bb_lower = ema20 - atr * bb_mult;
    let bb_width = if bb_lower > 0.0 { (bb_upper - bb_lower) / bb_lower * 100.0 } else { 0.0 };
    let prev_bb_width = ctx.get("prev_bb_width", bb_width);
    ctx.set("prev_bb_width", bb_width);
    let bb_expand = bb_width > prev_bb_width * 1.1;

    let win_pct = (close - window_open) / window_open * 100.0;
    let score = 0.0;
    if win_pct >  0.10 { score +=  7.0; }
    else if win_pct >  0.02 { score +=  5.0; }
    else if win_pct >  0.005 { score +=  3.0; }
    else if win_pct >  0.001 { score +=  1.0; }
    else if win_pct < -0.001 { score -=  1.0; }
    else if win_pct < -0.005 { score -=  3.0; }
    else if win_pct < -0.02  { score -=  5.0; }
    else if win_pct < -0.10  { score -=  7.0; }

    let close_4ago = ctx.close_at(idx - lookback_4);
    let mom4 = if close_4ago > 0.0 { (close - close_4ago) / close_4ago * 100.0 } else { 0.0 };
    if mom4 > mom_threshold       { score += 2.0; }
    else if mom4 < -mom_threshold { score -= 2.0; }

    let close_1ago = ctx.close_at(idx - 1);
    let mom1 = if close_1ago > 0.0 { (close - close_1ago) / close_1ago * 100.0 } else { 0.0 };
    if mom1 > mom1_threshold       { score += 1.0; }
    else if mom1 < -mom1_threshold { score -= 1.0; }

    let ema14 = ctx.ema(14);
    if close > ema14 { score += 0.5; } else { score -= 0.5; }

    let rsi = ctx.rsi(14);
    if rsi < rsi_oversold       { score += 2.0; }
    else if rsi > rsi_overbought { score -= 2.0; }

    // ATR expansion: breakout signal — directional only
    if bb_expand && win_pct > 0.0 { score += 1.0; }
    else if bb_expand && win_pct < 0.0 { score -= 1.0; }

    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.9 + volume * 0.1);
    if volume > avg_vol * min_vol_mult {
        if win_pct > 0.0 { score += 1.0; } else if win_pct < 0.0 { score -= 1.0; }
    }

    ctx.set("debug_score", score); ctx.set("debug_win_pct", win_pct);
    ctx.set("debug_mom4", mom4); ctx.set("debug_mom1", mom1);
    ctx.set("debug_rsi", rsi); ctx.set("debug_ema14", ema14);
    ctx.set("debug_bb_width", bb_width); ctx.set("debug_token_price", ctx.token_price);

    let abs_score = if score >= 0.0 { score } else { -score };
    let est_prob = if abs_score >= 8.0 { 0.85 } else if abs_score >= 7.0 { 0.80 }
                   else if abs_score >= 6.0 { 0.75 } else if abs_score >= 5.0 { 0.70 }
                   else if abs_score >= 4.0 { 0.65 } else { 0.62 };
    let implied_p = if score > 0.0 { ctx.token_price } else { 1.0 - ctx.token_price };
    let edge = est_prob - implied_p;
    ctx.set("debug_est_prob", est_prob); ctx.set("debug_implied_p", implied_p); ctx.set("debug_edge", edge);
    if edge < min_edge { return; }

    if score >= min_score { ctx.buy(0.25); }
    else if score <= -min_score { ctx.sell(0.25); }
}
