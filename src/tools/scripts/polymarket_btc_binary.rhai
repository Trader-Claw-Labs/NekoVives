// Polymarket BTC Binary Strategy — ctx-based API, designed for polymarket_binary engine
//
// HOW IT WORKS:
//   ctx.buy(1.0)  = bet YES: BTC will go UP over the next `window_minutes` candles
//   ctx.sell(1.0) = bet NO:  BTC will go DOWN
//   Positions auto-resolve — no manual exit needed.
//
// EXTRA CTX FIELDS (binary engine only):
//   ctx.token_price    — estimated YES token cost (piecewise model, 0.50..0.97)
//   ctx.window_minutes — resolution window in minutes (same as interval you chose)
//
// EDGE MODEL:
//   Break-even win rate = token_price * 100.
//   If token_price = $0.65, you need >65% win rate to profit.
//   The strategy avoids betting when the signal is so strong the market has already
//   priced it in (token_price > max_token_price).

fn on_candle(ctx) {
    // ── Parameters ────────────────────────────────────────────────────────────
    let min_idx         = 50;       // warm-up candles before betting
    let mom4_threshold  = 0.015;    // 4-candle momentum % to count as a signal
    let mom1_threshold  = 0.004;    // 1-candle momentum % to count as a signal
    let rsi_oversold    = 38.0;     // RSI below this = bullish confirmation
    let rsi_overbought  = 62.0;     // RSI above this = bearish confirmation
    let vol_mult        = 1.1;      // volume must be > vol_mult * EMA of volume
    let min_confirms    = 3;        // out of 4 signals must align
    let max_token_price = 0.78;     // skip bet if YES token costs more than this
    //                               (means market has already priced in the move)

    let idx = ctx.index;
    if idx < min_idx { return; }

    // Already in a bet — wait for auto-resolution
    if ctx.position != 0.0 { return; }

    // Skip if the signal is already "obvious" (market priced it in)
    if ctx.token_price > max_token_price { return; }
    // Also skip if YES token is priced below ~35% (NO would cost >65%,
    // meaning NO is the "obvious" side — flip and check)
    if ctx.token_price < (1.0 - max_token_price) { return; }

    let close  = ctx.close;
    let volume = ctx.volume;

    // 4-candle and 1-candle momentum
    let c4     = ctx.close_at(idx - 4);
    let mom4   = if c4 > 0.0 { (close - c4) / c4 * 100.0 } else { 0.0 };
    let c1     = ctx.close_at(idx - 1);
    let mom1   = if c1 > 0.0 { (close - c1) / c1 * 100.0 } else { 0.0 };

    let ema14  = ctx.ema(14);
    let rsi    = ctx.rsi(14);

    // Rolling volume EMA
    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.9 + volume * 0.1);
    let vol_ok  = volume > avg_vol * vol_mult;

    // ── Bullish count ─────────────────────────────────────────────────────────
    let bull = 0;
    if mom4 >  mom4_threshold { bull += 1; }
    if mom1 >  mom1_threshold { bull += 1; }
    if rsi  <  rsi_oversold   { bull += 1; }
    if vol_ok                 { bull += 1; }

    // ── Bearish count ─────────────────────────────────────────────────────────
    let bear = 0;
    if mom4 < -mom4_threshold { bear += 1; }
    if mom1 < -mom1_threshold { bear += 1; }
    if rsi  >  rsi_overbought { bear += 1; }
    if vol_ok                 { bear += 1; }

    // ── Place bet ─────────────────────────────────────────────────────────────
    if bull >= min_confirms && close > ema14 {
        ctx.buy(1.0);
        ctx.log("BET YES | bull=" + bull + " | token=" + ctx.token_price + " | mom4=" + mom4 + "%");
    } else if bear >= min_confirms && close < ema14 {
        ctx.sell(1.0);
        ctx.log("BET NO  | bear=" + bear + " | token=" + ctx.token_price + " | mom4=" + mom4 + "%");
    }
}
