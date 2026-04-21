// ============================================================================
// POLYMARKET Trading Bot - BTC UP/DOWN 5-Minute Strategy v2.1
// Platform: Rust + Rhai
// Markets: BTC_UP, BTC_DOWN (binary prediction markets)
// ============================================================================

fn on_candle(ctx) {
    // ── CONFIGURATION: POLYMARKET (5-MIN BINARY) ──────────────────────────
    
    // Lookback periods
    let lookback_4      = 4;      // 4-bar momentum calculation
    let lookback_14     = 14;     // RSI + EMA period
    let lookback_50     = 50;     // SMA trend filter (useful but not critical)

    // Momentum thresholds (moderate—less noise in binary markets)
    let mom_threshold   = 1.2;    // Min 4-bar momentum % (UP: >1.2%, DOWN: <-1.2%)
    let mom1_threshold  = 0.3;    // Min 1-bar confirmation (UP: >0.3%, DOWN: <-0.3%)

    // RSI settings (standard—cleaner signals in binaries)
    let rsi_oversold    = 30.0;   // Bullish rebound (standard RSI)
    let rsi_overbought  = 70.0;   // Bearish correction (standard RSI)

    // ATR-based risk (tighter than crypto—lower volatility)
    let atr_sl_mult     = 1.5;    // Stop loss = ATR × 1.5
    let atr_tp_mult     = 2.5;    // Take profit = ATR × 2.5

    // Profit/loss targets (account for Polymarket fees 1-2%)
    let max_loss_pct    = 2.5;    // Hard exit if loss ≥ 2.5%
    let take_pft_pct    = 5.0;    // Hard exit if profit ≥ 5.0%

    // Volume confirmation
    let min_vol_mult    = 1.2;    // Vol must exceed 1.2× rolling average
    
    // Time-based exit (binary markets more stable, hold longer)
    let max_hold        = 12;     // Max 12 bars (60 minutes @ 5-min)

    // Warmup period (less than crypto)
    let min_idx         = 80;     // Need 80 bars before trading

    // Confirmation requirement (3-of-4 OK for Polymarket)
    let min_confirmations = 3;    // Require 3+ signals

    // ── PRE-CONDITIONS ────────────────────────────────────────────────────
    
    let idx = ctx.index;
    if idx < min_idx {
        return;  // Skip warmup period
    }

    let close   = ctx.close;
    let volume  = ctx.volume;
    let atr     = ctx.atr(14);

    // ── EXIT LOGIC (executed first, every candle) ─────────────────────────
    // Close positions based on profit target, stop loss, or time limit
    
    if ctx.position != 0.0 {
        let ep          = ctx.entry_price;
        let bars_held   = idx - ctx.entry_index;
        let pnl_pct     = if ep > 0.0 { 
            (close - ep) / ep * 100.0 
        } else { 
            0.0 
        };

        // Adjust P&L sign for short positions (BTC_DOWN)
        let effective_pnl = if ctx.position < 0.0 { -pnl_pct } else { pnl_pct };

        // Exit conditions
        if effective_pnl >= take_pft_pct || 
           effective_pnl <= -max_loss_pct || 
           bars_held >= max_hold {
            
            ctx.sell(1.0);
            ctx.log("EXIT: PnL=" + effective_pnl + "% | Bars=" + bars_held);
        }
        return;  // Don't open new positions while holding
    }

    // ── TECHNICAL INDICATORS ──────────────────────────────────────────────

    // 4-candle momentum
    let close_4ago = ctx.close_at(idx - lookback_4);
    let mom4 = if close_4ago > 0.0 {
        (close - close_4ago) / close_4ago * 100.0
    } else {
        0.0
    };

    // 1-candle momentum (confirmation)
    let close_1ago = ctx.close_at(idx - 1);
    let mom1 = if close_1ago > 0.0 {
        (close - close_1ago) / close_1ago * 100.0
    } else {
        0.0
    };

    // EMA-14 (trend direction)
    let ema14 = ctx.ema(lookback_14);

    // SMA-50 (secondary trend filter)
    let sma50 = ctx.sma(lookback_50);

    // RSI-14 (overbought/oversold indicator)
    let rsi = ctx.rsi(lookback_14);

    // Rolling volume average
    let avg_vol = ctx.get("avg_vol", volume);
    ctx.set("avg_vol", avg_vol * 0.9 + volume * 0.1);
    let vol_ok = volume > avg_vol * min_vol_mult;

    // ATR-based stop distance
    let stop_dist = atr * atr_sl_mult;
    let tp_dist   = atr * atr_tp_mult;

    // ── ENTRY SIGNALS ─────────────────────────────────────────────────────

    // BULLISH (BTC_UP): Count confirmations (need 3+ of 4)
    let bull_count = 0;
    if mom4 > mom_threshold    { bull_count += 1; }  // Signal 1: 4-bar momentum up
    if mom1 > mom1_threshold   { bull_count += 1; }  // Signal 2: 1-bar momentum up
    if rsi < rsi_oversold      { bull_count += 1; }  // Signal 3: RSI oversold
    if vol_ok                  { bull_count += 1; }  // Signal 4: Volume spike

    // BEARISH (BTC_DOWN): Count confirmations (need 3+ of 4)
    let bear_count = 0;
    if mom4 < -mom_threshold   { bear_count += 1; }  // Signal 1: 4-bar momentum down
    if mom1 < -mom1_threshold  { bear_count += 1; }  // Signal 2: 1-bar momentum down
    if rsi > rsi_overbought    { bear_count += 1; }  // Signal 3: RSI overbought
    if vol_ok                  { bear_count += 1; }  // Signal 4: Volume spike

    // ── EXECUTE ENTRIES ───────────────────────────────────────────────────

    // LONG (BTC_UP): 3-of-4 signals + trend filter
    if bull_count >= min_confirmations && close > ema14 {
        ctx.buy(1.0);
        ctx.set_stop_loss(close - stop_dist);
        ctx.set_take_profit(close + tp_dist);
        ctx.log("BUY: bull_count=" + bull_count + " | mom4=" + mom4 + "% | rsi=" + rsi);
    }

    // SHORT (BTC_DOWN): 3-of-4 signals + trend filter
    else if bear_count >= min_confirmations && close < ema14 {
        ctx.sell(1.0);
        ctx.set_stop_loss(close + stop_dist);
        ctx.set_take_profit(close - tp_dist);
        ctx.log("SELL: bear_count=" + bear_count + " | mom4=" + mom4 + "% | rsi=" + rsi);
    }
}

// ============================================================================
// POLYMARKET-SPECIFIC NOTES
// ============================================================================
// 1. Binary Nature: Price oscillates between 0-1 (0% to 100% probability)
//    - Momentum thresholds are naturally lower than crypto (1.2% vs 1.8%)
//    - RSI stays in clear zones more often
//
// 2. Fee Impact: Polymarket charges ~1-2% round-trip fees
//    - take_pft_pct should be 5%+ to overcome fees
//    - max_loss_pct set to 2.5% to exit quickly on wrong signals
//
// 3. Timing: Events matter (elections, sports outcomes, crypto price)
//    - Check event calendar before deploying
//    - Avoid trading last 30 minutes before resolution
//
// 4. Liquidity: Can vary during events
//    - min_vol_mult 1.2 is conservative; can increase to 1.5 in quiet times
//    - Check order book depth before deploying
//
// 5. Position Sizing: Binaries are "all-or-nothing" in nature
//    - 1.0 = 100% of specified position size
//    - Consider 0.5 for high-uncertainty trades
// ============================================================================

// ============================================================================
// PARAMETER ADJUSTMENT GUIDE FOR POLYMARKET
// ============================================================================
// Win Rate too low (< 50%):
//   • Increase min_confirmations: 3 → 4 (require all signals)
//   • Increase mom_threshold: 1.2 → 1.5 (stricter momentum)
//   • Decrease atr_tp_mult: 2.5 → 2.0 (take profits earlier)
//
// Missing good trades:
//   • Decrease min_confirmations: 3 → 2 (more entry opportunities)
//   • Decrease mom_threshold: 1.2 → 0.8 (catch early moves)
//   • Increase max_hold: 12 → 15 (give trades more time)
//
// Too many false signals:
//   • Increase rsi_oversold: 30 → 35 (more extreme RSI)
//   • Increase rsi_overbought: 70 → 65 (more extreme RSI)
//   • Add SMA-50 check: && ema14 > sma50 for longs
// ============================================================================
