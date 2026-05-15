// OPT v8: Triple EMA trend filter — adapted for Polymarket binary mode
fn on_candle(ctx) {
    let min_idx           = 80;
    let max_token_p       = 0.65;
    let min_edge          = 0.0;
    let min_score         = 2.0;
    let lookback_4        = 3;
    let mom_threshold     = 1.0;
    let mom1_threshold    = 0.3;
    let rsi_oversold      = 35.0;
    let rsi_overbought    = 65.0;
    let min_vol_mult      = 1.1;

    let idx = ctx.index;
    if idx < min_idx { return; }
    if ctx.open_positions > 0 { return; }

    let window_open = ctx.window_open;
    if window_open <= 0.0 { return; }
    if ctx.token_price > max_token_p { ctx.set("debug_reason", 1.0); return; }
    if ctx.token_price < (1.0 - max_token_p) { ctx.set("debug_reason", 2.0); return; }

    let close  = ctx.close;
    let volume = ctx.volume;
    let ema8   = ctx.ema(8);
    let ema21  = ctx.ema(21);
    let ema50  = ctx.ema(50);

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

    let rsi = ctx.rsi(14);
    if rsi < rsi_oversold       { score += 2.0; }
    else if rsi > rsi_overbought { score -= 2.0; }

    // Triple EMA alignment (weight 1.5 strong, 0.5 weak)
    if ema8 > ema21 && ema21 > ema50      { score += 1.5; }
    else if ema8 < ema21 && ema21 < ema50 { score -= 1.5; }
    else if ema8 > ema21                  { score += 0.5; }
    else if ema8 < ema21                  { score -= 0.5; }

    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.9 + volume * 0.1);
    if volume > avg_vol * min_vol_mult {
        if win_pct > 0.0 { score += 1.0; } else if win_pct < 0.0 { score -= 1.0; }
    }

    ctx.set("debug_score", score); ctx.set("debug_win_pct", win_pct);
    ctx.set("debug_mom4", mom4); ctx.set("debug_mom1", mom1);
    ctx.set("debug_rsi", rsi); ctx.set("debug_ema8", ema8);
    ctx.set("debug_ema21", ema21); ctx.set("debug_ema50", ema50);
    ctx.set("debug_token_price", ctx.token_price);

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
