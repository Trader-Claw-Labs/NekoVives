// Weather binary strategy v1.1 — generic threshold market
//
// Works with any Open-Meteo daily series (temperature, etc.).
// ctx.close         = today's value (e.g. max temperature in °C)
// ctx.value         = alias for ctx.close
// ctx.threshold     = resolution threshold (e.g. 25.0°C)
// ctx.window_open   = previous day's value (for trend)
// ctx.token_price   = estimated YES token cost (flat 0.50 for weather)
//
// ACTIONS: ctx.buy() = bet YES (exceeds threshold), ctx.sell() = bet NO

fn on_candle(ctx) {
    let warmup       = 14;
    let max_token_p  = 0.65;  // skip if market already >65% priced in
    let min_edge     = 0.05;  // require at least 5% edge over implied probability

    if ctx.index < warmup { return; }

    // Token price filter: skip strongly one-sided markets
    if ctx.token_price > max_token_p { return; }
    if ctx.token_price < (1.0 - max_token_p) { return; }

    let value     = ctx.close;
    let threshold = ctx.threshold;
    if threshold <= 0.0 { return; }

    // 7-day EMA vs threshold
    let ema7  = ctx.ema(7);
    let ema14 = ctx.ema(14);

    let score = 0.0;

    // EMA trend signal (dominant)
    if ema7 > threshold + 1.0       { score += 3.0; }
    else if ema7 > threshold        { score += 1.5; }
    else if ema7 < threshold - 1.0  { score -= 3.0; }
    else                            { score -= 1.5; }

    // EMA acceleration (ema7 vs ema14)
    if ema7 > ema14 { score += 1.0; }
    else             { score -= 1.0; }

    // Current day vs threshold
    let dist = value - threshold;
    if dist > 2.0       { score += 2.0; }
    else if dist > 0.0  { score += 0.5; }
    else if dist < -2.0 { score -= 2.0; }
    else                { score -= 0.5; }

    ctx.set("debug_score", score);
    ctx.set("debug_ema7", ema7);
    ctx.set("debug_dist", dist);

    // Edge filter: only bet when expected value is positive
    let abs_score = if score >= 0.0 { score } else { -score };
    let est_prob = if abs_score >= 5.0 { 0.75 }
                   else if abs_score >= 4.0 { 0.70 }
                   else if abs_score >= 3.0 { 0.65 }
                   else { 0.60 };
    let implied_p = if score > 0.0 { ctx.token_price } else { 1.0 - ctx.token_price };
    let edge = est_prob - implied_p;
    ctx.set("debug_edge", edge);
    ctx.set("debug_est_prob", est_prob);
    ctx.set("debug_implied_p", implied_p);
    if edge < min_edge { return; }

    if score >= 3.5 {
        ctx.buy(0.4);
    } else if score <= -3.5 {
        ctx.sell(0.4);
    }
}
