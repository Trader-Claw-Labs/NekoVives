// Weather binary strategy — generic threshold market
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
    let warmup = 14;  // need at least 14 days of history
    if ctx.index < warmup { return; }

    let value     = ctx.close;
    let threshold = ctx.threshold;
    if threshold <= 0.0 { return; }  // misconfigured series

    // 1. 7-day EMA vs threshold
    let ema7  = ctx.ema(7);
    let ema14 = ctx.ema(14);

    let mut score = 0.0;

    // EMA trend signal (dominant)
    if ema7 > threshold + 1.0  { score += 3.0; }  // well above threshold
    else if ema7 > threshold   { score += 1.5; }  // just above
    else if ema7 < threshold - 1.0 { score -= 3.0; }  // well below
    else                           { score -= 1.5; }  // just below

    // EMA acceleration (ema7 vs ema14)
    if ema7 > ema14 { score += 1.0; }  // warming trend
    else             { score -= 1.0; }  // cooling trend

    // Current day vs threshold
    let dist = value - threshold;
    if dist > 2.0      { score += 2.0; }
    else if dist > 0.0 { score += 0.5; }
    else if dist < -2.0 { score -= 2.0; }
    else                { score -= 0.5; }

    // Place bet
    if score >= 3.5 {
        ctx.buy(0.4);   // bet YES: value will exceed threshold
    } else if score <= -3.5 {
        ctx.sell(0.4);  // bet NO: value will stay below threshold
    }
}
