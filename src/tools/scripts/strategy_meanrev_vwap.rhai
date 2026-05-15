//! Strategy 2 — Mean Reversion VWAP
//!
//! Trades deviations from VWAP in high-vol chop regimes.
//! z-score = (close - vwap) / stddev(20)

const STD_PERIOD = 20;
const RSI_PERIOD = 14;
const Z_ENTRY = 2.0;
const RISK_PER_TRADE = 0.02;

fn on_candle(ctx) {
    // Only trade mean-reversion in high-vol chop
    if ctx.regime() != "high_vol_chop" {
        return;
    }

    let close = ctx.close;
    let vwap = ctx.vwap();
    let stddev = ctx.stddev(STD_PERIOD);
    let rsi = ctx.rsi(RSI_PERIOD);

    if stddev <= 0.0 || vwap <= 0.0 {
        return;
    }

    let z_score = (close - vwap) / stddev;
    let pos = ctx.position;

    // Entry: extreme deviation from VWAP
    if pos == 0.0 {
        if z_score > Z_ENTRY && rsi > 70.0 {
            // Overbought → short (revert to VWAP)
            ctx.short(RISK_PER_TRADE);
            ctx.set("trade_side", -1.0);
            ctx.set_take_profit(vwap);
        } else if z_score < -Z_ENTRY && rsi < 30.0 {
            // Oversold → long (revert to VWAP)
            ctx.buy(RISK_PER_TRADE);
            ctx.set("trade_side", 1.0);
            ctx.set_take_profit(vwap);
        }
    }

    // Exit: reversion to VWAP or regime change
    if pos != 0.0 {
        let side = ctx.get("trade_side", 0.0);
        // Take profit hit
        if side > 0.0 && close >= vwap {
            ctx.sell(1.0);
        }
        if side < 0.0 && close <= vwap {
            ctx.sell(1.0);
        }
        // Stop loss: 2x stddev against position
        let entry = ctx.entry_price;
        if side > 0.0 && close < entry - stddev * 2.0 {
            ctx.sell(1.0);
        }
        if side < 0.0 && close > entry + stddev * 2.0 {
            ctx.sell(1.0);
        }
    }
}
