// ============================================================================
// CRYPTO Trading Bot - Binance 4-Minute Strategy v2.1
// Platform: Rust + Rhai
// Optimized for: BTC, ETH, SOL on Binance
// ============================================================================

fn on_candle(ctx) {
    // ── CONFIGURATION: CRYPTO (BINANCE 4-MIN) ────────────────────────────
    
    // Lookback periods
    let lookback_4      = 4;      // 4-bar momentum calculation
    let lookback_14     = 14;     // RSI + EMA period
    let lookback_50     = 50;     // SMA trend filter (CRITICAL)

    // Momentum thresholds (high to reduce noise)
    let mom_threshold   = 1.8;    // Min 4-bar momentum %
    let mom1_threshold  = 0.5;    // Min 1-bar confirmation

    // RSI settings (loose for high volatility)
    let rsi_oversold    = 25.0;   // Bullish rebound threshold
    let rsi_overbought  = 75.0;   // Bearish correction threshold

    // ATR-based risk management
    let atr_sl_mult     = 2.0;    // Stop loss = ATR × 2.0
    let atr_tp_mult     = 3.0;    // Take profit = ATR × 3.0

    // Profit/loss targets
    let max_loss_pct    = 3.5;    // Hard exit if loss ≥ 3.5%
    let take_pft_pct    = 6.0;    // Hard exit if profit ≥ 6.0%

    // Volume confirmation
    let min_vol_mult    = 1.5;    // Vol must exceed 1.5× rolling average
    
    // Time-based exit
    let max_hold        = 8;      // Max 8 bars (32 minutes @ 4-min)

    // Warmup period
    let min_idx         = 100;    // Need 100+ bars before trading

    // ── PRE-CONDITIONS ────────────────────────────────────────────────────
    
    let idx = ctx.index;
    if idx < min_idx {
        return;  // Skip warmup period
    }

    let close   = ctx.close;
    let volume  = ctx.volume;
    let atr     = ctx.atr(14);

    // ── EXIT LOGIC (executed first, every candle) ─────────────────────────
    // This prevents holding positions through unfavorable conditions
    
    if ctx.position != 0.0 {
        let ep          = ctx.entry_price;
        let bars_held   = idx - ctx.entry_index;
        let pnl_pct     = if ep > 0.0 { 
            (close - ep) / ep * 100.0 
        } else { 
            0.0 
        };

        // Adjust P&L sign for short positions
        let effective_pnl = if ctx.position < 0.0 { -pnl_pct } else { pnl_pct };

        // Exit conditions: profit target, stop loss, or time limit
        if effective_pnl >= take_pft_pct || 
           effective_pnl <= -max_loss_pct || 
           bars_held >= max_hold {
            
            ctx.sell(1.0);
            ctx.log("EXIT: PnL=" + effective_pnl + "% | Bars=" + bars_held);
        }
        return;  // Don't open new positions while holding
    }

    // ── TECHNICAL INDICATORS ──────────────────────────────────────────────

    // 4-candle momentum (short-term velocity)
    let close_4ago = ctx.close_at(idx - lookback_4);
    let mom4 = if close_4ago > 0.0 {
        (close - close_4ago) / close_4ago * 100.0
    } else {
        0.0
    };

    // 1-candle momentum (confirmation signal)
    let close_1ago = ctx.close_at(idx - 1);
    let mom1 = if close_1ago > 0.0 {
        (close - close_1ago) / close_1ago * 100.0
    } else {
        0.0
    };

    // EMA-14 (trend direction filter)
    let ema14 = ctx.ema(lookback_14);

    // SMA-50 (primary trend filter) ← CRITICAL FOR CRYPTO
    let sma50 = ctx.sma(lookback_50);

    // RSI-14 (overbought/oversold indicator)
    let rsi = ctx.rsi(lookback_14);

    // Rolling volume average (exponential moving average)
    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.85 + volume * 0.15);
    let vol_ok = volume > avg_vol * min_vol_mult;

    // ATR-based stop distance (dynamic based on volatility)
    let stop_dist = atr * atr_sl_mult;
    let tp_dist   = atr * atr_tp_mult;

    // ── ENTRY SIGNALS ─────────────────────────────────────────────────────

    // BULLISH: Count confirmations (need exactly 4 of 4)
    let bull_count = 0;
    if mom4 > mom_threshold    { bull_count += 1; }  // Signal 1: 4-bar momentum up
    if mom1 > mom1_threshold   { bull_count += 1; }  // Signal 2: 1-bar momentum up
    if rsi < rsi_oversold      { bull_count += 1; }  // Signal 3: RSI oversold
    if vol_ok                  { bull_count += 1; }  // Signal 4: Volume spike

    // BEARISH: Count confirmations (need exactly 4 of 4)
    let bear_count = 0;
    if mom4 < -mom_threshold   { bear_count += 1; }  // Signal 1: 4-bar momentum down
    if mom1 < -mom1_threshold  { bear_count += 1; }  // Signal 2: 1-bar momentum down
    if rsi > rsi_overbought    { bear_count += 1; }  // Signal 3: RSI overbought
    if vol_ok                  { bear_count += 1; }  // Signal 4: Volume spike

    // ── EXECUTE ENTRIES ───────────────────────────────────────────────────

    // LONG: All 4 signals required + price > EMA-14 + EMA-14 > SMA-50
    // This ensures we only enter confirmed uptrends with strong momentum
    if bull_count == 4 && close > ema14 && ema14 > sma50 {
        ctx.buy(1.0);
        ctx.set_stop_loss(close - stop_dist);
        ctx.set_take_profit(close + tp_dist);
        ctx.log("BUY: mom4=" + mom4 + "% | rsi=" + rsi + " | vol=" + (volume / avg_vol) + "x");
    }

    // SHORT: All 4 signals required + price < EMA-14 + EMA-14 < SMA-50
    // This ensures we only enter confirmed downtrends with strong momentum
    else if bear_count == 4 && close < ema14 && ema14 < sma50 {
        ctx.sell(1.0);
        ctx.set_stop_loss(close + stop_dist);
        ctx.set_take_profit(close - tp_dist);
        ctx.log("SELL: mom4=" + mom4 + "% | rsi=" + rsi + " | vol=" + (volume / avg_vol) + "x");
    }
}

// ============================================================================
// PARAMETER TUNING GUIDE
// ============================================================================
// If experiencing false signals, increase these:
//   • mom_threshold: 1.8 → 2.0 → 2.2 (requires larger price moves)
//   • rsi_oversold: 25 → 20 (more extreme condition)
//
// If missing trades, decrease these:
//   • atr_sl_mult: 2.0 → 1.8 (but watch for whipsaws)
//   • max_hold: 8 → 6 (faster exits create more opportunities)
//
// If taking too many losses:
//   • Increase SMA-50 period to 60 or 70
//   • Require atr_tp_mult 4.0 (wait for bigger moves)
// ============================================================================
