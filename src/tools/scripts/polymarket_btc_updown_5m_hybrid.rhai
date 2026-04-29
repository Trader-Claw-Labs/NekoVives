// Hybrid Backtest Proxy — Polymarket BTC UP/DOWN 5m
// ==================================================
// LIVE: usar slug dinámico btc-updown-5m-{window_ts}
// BACKTEST: usar BTCUSDT 1m (market_type=crypto) para emular decisión binaria 5m

fn on_candle(ctx) {
    let min_idx           = 80;
    let lookback_5        = 5;
    let lookback_14       = 14;
    let lookback_50       = 50;

    let mom5_threshold    = 0.08; // % en 5 velas
    let mom1_threshold    = 0.02; // % en 1 vela
    let rsi_oversold      = 35.0;
    let rsi_overbought    = 65.0;
    let min_vol_mult      = 1.1;
    let min_confirmations = 3;
    let max_token_p       = 0.65; // skip if market already priced in
    let min_edge          = 0.0;  // minimum EV edge (calibrate after runs)

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close  = ctx.close;
    let volume = ctx.volume;

    // Skip when token is too expensive (market already priced in)
    if ctx.token_price > max_token_p { ctx.set("debug_reason", 1.0); return; }
    if ctx.token_price < (1.0 - max_token_p) { ctx.set("debug_reason", 2.0); return; }

    // Señales base (proxy 5m)
    let close_5ago = ctx.close_at(idx - lookback_5);
    let mom5 = if close_5ago > 0.0 { (close - close_5ago) / close_5ago * 100.0 } else { 0.0 };

    let close_1ago = ctx.close_at(idx - 1);
    let mom1 = if close_1ago > 0.0 { (close - close_1ago) / close_1ago * 100.0 } else { 0.0 };

    let ema14 = ctx.ema(lookback_14);
    // Note: sma() is not supported — using ema(50) as proxy
    let ema50 = ctx.ema(lookback_50);
    let rsi   = ctx.rsi(lookback_14);

    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.9 + volume * 0.1);
    let vol_ok = volume > avg_vol * min_vol_mult;

    let bull = 0;
    if mom5 > mom5_threshold   { bull += 1; }
    if mom1 > mom1_threshold   { bull += 1; }
    if rsi < rsi_oversold      { bull += 1; }
    if vol_ok && mom5 > 0.0    { bull += 1; }  // volume only counts with direction

    let bear = 0;
    if mom5 < -mom5_threshold  { bear += 1; }
    if mom1 < -mom1_threshold  { bear += 1; }
    if rsi > rsi_overbought    { bear += 1; }
    if vol_ok && mom5 < 0.0    { bear += 1; }  // volume only counts with direction

    // Filtro de tendencia para reducir ruido
    let trend_up   = close > ema14 && close > ema50;
    let trend_down = close < ema14 && close < ema50;

    // Compute score from confirmations for edge filter
    let score = if bull >= min_confirmations && trend_up { bull * 1.0 }
                else if bear >= min_confirmations && trend_down { -(bear * 1.0) }
                else { 0.0 };

    // ── Edge filter: only bet when expected value is positive ──
    let abs_score = if score >= 0.0 { score } else { -score };
    let est_prob = if abs_score >= 4.0 { 0.72 }
                   else if abs_score >= 3.0 { 0.67 }
                   else { 0.62 };
    let implied_p = if score > 0.0 { ctx.token_price } else { 1.0 - ctx.token_price };
    let edge = est_prob - implied_p;
    ctx.set("debug_est_prob", est_prob);
    ctx.set("debug_implied_p", implied_p);
    ctx.set("debug_edge", edge);
    ctx.set("debug_score", score);
    ctx.set("debug_bull", bull * 1.0);
    ctx.set("debug_bear", bear * 1.0);
    ctx.set("debug_rsi", rsi);
    ctx.set("debug_ema14", ema14);
    ctx.set("debug_ema50", ema50);
    ctx.set("debug_avg_vol", avg_vol);
    if edge < min_edge { return; }

    if bull >= min_confirmations && trend_up {
        ctx.buy(0.25);    // YES
    } else if bear >= min_confirmations && trend_down {
        ctx.sell(0.25);   // NO
    }
}
