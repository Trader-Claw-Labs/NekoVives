// OPT v3: Señal temprana (lookback=2) — adapted for Polymarket binary mode
// Signals: early 2-candle momentum, mom1, EMA14 trend, RSI, volume (directional)
fn on_candle(ctx) {
    let min_idx           = 60;
    let max_token_p       = 0.65;
    let min_edge          = 0.0;
    let min_score         = 2.0;

    let lookback_early    = 2;
    let lookback_14       = 14;
    let mom_threshold     = 0.6;
    let mom1_threshold    = 0.15;
    let rsi_oversold      = 38.0;
    let rsi_overbought    = 62.0;
    let min_vol_mult      = 1.0;

    let idx = ctx.index;
    if idx < min_idx { return; }
    if ctx.open_positions > 0 { return; }

    let window_open = ctx.window_open;
    if window_open <= 0.0 { return; }

    // Token price filter
    //if ctx.token_price > max_token_p { ctx.set("debug_reason", 1.0); return; }
    //if ctx.token_price < (1.0 - max_token_p) { ctx.set("debug_reason", 2.0); return; }

    let close  = ctx.close;
    let volume = ctx.volume;

    // ── 1. Window delta — dominant signal (weight 5-7) ──
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

    // ── 2. Early momentum — 2-candle lookback (weight 2) ──
    let close_early = ctx.close_at(idx - lookback_early);
    let mom_early = if close_early > 0.0 { (close - close_early) / close_early * 100.0 } else { 0.0 };
    if mom_early > mom_threshold       { score += 2.0; }
    else if mom_early < -mom_threshold { score -= 2.0; }

    // ── 3. Micro momentum (weight 1) ──
    let close_1ago = ctx.close_at(idx - 1);
    let mom1 = if close_1ago > 0.0 { (close - close_1ago) / close_1ago * 100.0 } else { 0.0 };
    if mom1 > mom1_threshold       { score += 1.0; }
    else if mom1 < -mom1_threshold { score -= 1.0; }

    // ── 4. EMA14 trend (weight 0.5) ──
    let ema14 = ctx.ema(lookback_14);
    if close > ema14 { score += 0.5; } else { score -= 0.5; }

    // ── 5. RSI extremes (weight 1-2) ──
    let rsi = ctx.rsi(lookback_14);
    if rsi < rsi_oversold       { score += 2.0; }
    else if rsi > rsi_overbought { score -= 2.0; }

    // ── 6. Volume surge — directional only (weight 1) ──
    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.9 + volume * 0.1);
    if volume > avg_vol * min_vol_mult {
        if win_pct > 0.0 { score += 1.0; }
        else if win_pct < 0.0 { score -= 1.0; }
    }

    // ── Debug logging ──
    ctx.set("debug_score", score);
    ctx.set("debug_win_pct", win_pct);
    ctx.set("debug_mom_early", mom_early);
    ctx.set("debug_mom1", mom1);
    ctx.set("debug_rsi", rsi);
    ctx.set("debug_ema14", ema14);
    ctx.set("debug_token_price", ctx.token_price);
    ctx.set("debug_avg_vol", avg_vol);

    // ── Edge filter ──
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

    if score >= min_score {
        ctx.buy(0.25);
    } else if score <= -min_score {
        ctx.sell(0.25);
    }
}
