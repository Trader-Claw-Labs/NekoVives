// ============================================================================
// Polymarket 4-Minute Strategy v2.0 — Trader Claw
// ============================================================================
// Optimized for Polymarket binary prediction markets (4-5 minute timeframe).
// Signal: RSI + 4-candle momentum + volume confirmation (3-of-4 required).
// Risk:   ATR-based stop loss, take profit, and max 5-bar time exit.
//
// Backtest usage:
//   script:       polymarket_4min.rhai
//   market_type:  polymarket  (or crypto for testing on Binance data)
//   interval:     4m  or  5m
//   symbol:       <Polymarket condition_token_id>
//   initial_balance: 1000
//   fee_pct:      1.0   (Polymarket round-trip ~1-2%)
//
// Strategy rationale (from polymarket_4min_improved.md):
//   - Momentum threshold raised to 0.8% to suppress false signals.
//   - 3-of-4 confirmations required: momentum(4), momentum(1), RSI, volume.
//   - SMA/EMA-14 used as trend filter (only trade in the trend direction).
//   - ATR-based dynamic stop loss and take profit.
//   - Max hold of 5 bars prevents being stuck in losing positions.
//
// Win rate target: 60%+ (see analysis in polymarket_4min_improved.md).
// ============================================================================

fn on_candle(ctx) {
    // ── Strategy parameters ──────────────────────────────────────────────────
    let lookback_4    = 4;
    let lookback_14   = 14;
    let mom_threshold = 0.8;   // minimum 4-bar momentum %
    let rsi_oversold  = 30.0;  // RSI < 30 → bullish rebound expected
    let rsi_overbought= 70.0;  // RSI > 70 → bearish correction expected
    let atr_sl_mult   = 1.5;   // stop loss = ATR × 1.5
    let atr_tp_mult   = 2.0;   // take profit = ATR × 2.0
    let max_loss_pct  = 2.0;   // hard exit if PnL ≤ −2 %
    let take_pft_pct  = 3.0;   // hard exit if PnL ≥ +3 %
    let min_vol_mult  = 1.2;   // volume must exceed 1.2× rolling average
    let max_hold      = 5;     // time-based exit after 5 bars
    let min_idx       = 20;    // warmup: need at least 20 bars

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close  = ctx.close;
    let volume = ctx.volume;
    let atr    = ctx.atr(14);

    // ── Exit logic (runs first to close open positions) ──────────────────────
    if ctx.position != 0.0 {
        let ep        = ctx.entry_price;
        let bars_held = idx - ctx.entry_index;
        let pnl_pct   = if ep > 0.0 { (close - ep) / ep * 100.0 } else { 0.0 };

        // Adjust PnL sign for shorts
        let effective_pnl = if ctx.position < 0.0 { -pnl_pct } else { pnl_pct };

        if effective_pnl >= take_pft_pct || effective_pnl <= -max_loss_pct || bars_held >= max_hold {
            ctx.sell(1.0);
        }
        return;
    }

    // ── Technical indicators ─────────────────────────────────────────────────

    // 4-candle momentum
    let close_4ago = ctx.close_at(idx - lookback_4);
    let mom4 = if close_4ago > 0.0 { (close - close_4ago) / close_4ago * 100.0 } else { 0.0 };

    // 1-candle momentum (confirmation)
    let close_1ago = ctx.close_at(idx - 1);
    let mom1 = if close_1ago > 0.0 { (close - close_1ago) / close_1ago * 100.0 } else { 0.0 };

    // EMA-14 for trend direction
    let ema14 = ctx.ema(lookback_14);

    // RSI-14 for overbought/oversold
    let rsi = ctx.rsi(lookback_14);

    // Rolling average volume (EMA approximation via key-value store)
    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.9 + volume * 0.1);
    let vol_ok = volume > avg_vol * min_vol_mult;

    // ATR-based stop distance
    let stop_dist = atr * atr_sl_mult;

    // ── Entry signals ────────────────────────────────────────────────────────

    // Bullish: 3-of-4 confirmations + price above EMA-14
    let bull = 0;
    if mom4 > mom_threshold  { bull += 1; }
    if mom1 > 0.2            { bull += 1; }
    if rsi  < rsi_oversold   { bull += 1; }
    if vol_ok                { bull += 1; }

    // Bearish: 3-of-4 confirmations + price below EMA-14
    let bear = 0;
    if mom4 < -mom_threshold { bear += 1; }
    if mom1 < -0.2           { bear += 1; }
    if rsi  > rsi_overbought { bear += 1; }
    if vol_ok                { bear += 1; }

    // ── Execute entries ──────────────────────────────────────────────────────
    if bull >= 3 && close > ema14 {
        ctx.buy(1.0);
        ctx.set_stop_loss(close - stop_dist);
        ctx.set_take_profit(close + atr * atr_tp_mult);
    } else if bear >= 3 && close < ema14 {
        // On Polymarket binary markets, shorting = selling YES tokens early.
        // For crypto backtests this opens a short position.
        ctx.sell(1.0);
        ctx.set_stop_loss(close + stop_dist);
        ctx.set_take_profit(close - atr * atr_tp_mult);
    }
}
