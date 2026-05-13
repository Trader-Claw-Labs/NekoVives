//! Strategy 1 — Volatility Breakout (BB-Squeeze + Keltner)
//!
//! Detects a Bollinger Band squeeze inside Keltner Channels,
//! then trades the breakout direction.

const BB_PERIOD = 20;
const BB_MULT = 2.0;
const KC_PERIOD = 20;
const KC_MULT = 1.5;
const ATR_PERIOD = 14;
const RISK_PER_TRADE = 0.02;

fn on_candle(ctx) {
    // Skip if regime is low-vol chop (no directional edge)
    if ctx.regime() == "low_vol_chop" {
        return;
    }

    let close = ctx.close;
    let bb_upper = ctx.bb_upper(BB_PERIOD, BB_MULT);
    let bb_lower = ctx.bb_lower(BB_PERIOD, BB_MULT);
    let bb_middle = ctx.bb_middle(BB_PERIOD);
    let kc_upper = ctx.kc_upper(KC_PERIOD, KC_MULT);
    let kc_lower = ctx.kc_lower(KC_PERIOD, KC_MULT);

    if bb_middle <= 0.0 || kc_upper <= 0.0 || kc_lower <= 0.0 {
        return;
    }

    // Squeeze detection: BB width < KC width (BB inside Keltner)
    let bb_width = ctx.bb_width(BB_PERIOD, BB_MULT);
    let kc_width = (kc_upper - kc_lower) / ((kc_upper + kc_lower) / 2.0) * 100.0;

    let in_squeeze = bb_width < kc_width;

    // Track squeeze state across candles
    let was_squeezed = ctx.get("in_squeeze", 0.0) > 0.5;
    ctx.set("in_squeeze", if in_squeeze { 1.0 } else { 0.0 });

    // Position check
    let pos = ctx.position;

    // Breakout logic: was in squeeze, now broke out
    if was_squeezed && !in_squeeze {
        if close > bb_upper && pos <= 0.0 {
            // Long breakout
            let atr = ctx.atr(ATR_PERIOD);
            let stop_dist = atr * 1.5;
            let size = RISK_PER_TRADE;
            if pos < 0.0 {
                ctx.sell(1.0); // close short first
            }
            ctx.buy(size);
            ctx.set_stop_loss(close - stop_dist);
            ctx.set("trade_side", 1.0);
        } else if close < bb_lower && pos >= 0.0 {
            // Short breakout
            let atr = ctx.atr(ATR_PERIOD);
            let stop_dist = atr * 1.5;
            let size = RISK_PER_TRADE;
            if pos > 0.0 {
                ctx.sell(1.0); // close long first
            }
            ctx.short(size);
            ctx.set_stop_loss(close + stop_dist);
            ctx.set("trade_side", -1.0);
        }
    }

    // Trailing stop / time exit
    if pos != 0.0 {
        let entry = ctx.entry_price;
        let side = ctx.get("trade_side", 0.0);

        // Hard stop via ATR
        let atr = ctx.atr(ATR_PERIOD);
        let stop_dist = atr * 1.5;
        if side > 0.0 && close < entry - stop_dist {
            ctx.sell(1.0);
        }
        if side < 0.0 && close > entry + stop_dist {
            ctx.sell(1.0);
        }
    }
}
