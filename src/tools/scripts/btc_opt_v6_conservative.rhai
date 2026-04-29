// OPT v6: Conservador + ATR bands filter — adapted for Polymarket binary mode
// Note: ctx.bollinger_upper/lower not supported — using ATR bands (ema20 ± atr*2) as proxy
fn on_candle(ctx) {
    let min_idx           = 80;
    let max_token_p       = 0.65;
    let min_edge          = 0.0;
    let min_score         = 3.0;  // conservative: higher bar
    let lookback_4        = 4;
    let lookback_14       = 14;
    let mom_threshold     = 2.0;
    let mom1_threshold    = 0.8;
    let rsi_oversold      = 25.0;
    let rsi_overbought    = 75.0;
    let min_vol_mult      = 1.5;
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

    let ema14 = ctx.ema(lookback_14);
    if close > ema14 { score += 0.5; } else { score -= 0.5; }

    let rsi = ctx.rsi(lookback_14);
    if rsi < rsi_oversold       { score += 2.0; }
    else if rsi > rsi_overbought { score -= 2.0; }

    // ATR band filter: price at extreme → mean reversion signal
    if close < bb_lower { score += 1.5; }
    else if close > bb_upper { score -= 1.5; }

    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.9 + volume * 0.1);
    if volume > avg_vol * min_vol_mult {
        if win_pct > 0.0 { score += 1.0; } else if win_pct < 0.0 { score -= 1.0; }
    }

    ctx.set("debug_score", score); ctx.set("debug_win_pct", win_pct);
    ctx.set("debug_mom4", mom4); ctx.set("debug_mom1", mom1);
    ctx.set("debug_rsi", rsi); ctx.set("debug_ema14", ema14);
    ctx.set("debug_bb_upper", bb_upper); ctx.set("debug_bb_lower", bb_lower);
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
