// Correlation Arbitrage v1.0 — Trader Claw (Lagged momentum + autocorrelation divergence)
// Exploits temporary breakdowns in the asset's own autocorrelation structure.
// When recent returns diverge significantly from their historical autocorrelation pattern,
// mean-reversion is expected — the same principle as cross-asset correlation arb.
//
// Also implements a "pairs proxy": trades the divergence between two rolling return windows
// (short-term vs medium-term momentum), betting on convergence.
//
// Real use case: deploy on BTC_UP and BTC_DOWN simultaneously for true Polymarket arb.
// This script handles the long side — pair it with a short of the other leg.
fn on_candle(ctx) {
    // ── Parameters ──────────────────────────────────────────────────────────
    let short_return_period  = 5;    // Short-term return window (bars)
    let medium_return_period = 20;   // Medium-term return window
    let long_return_period   = 60;   // Long-term baseline
    let divergence_threshold = 3.0;  // % divergence between short and medium return
    let convergence_exit     = 0.5;  // % convergence to trigger exit
    let vol_filter_period    = 14;   // ATR period for volatility filter
    let vol_min_pct          = 0.3;  // Min ATR% of price (filter dead markets)
    let rsi_period           = 14;
    let max_loss_pct         = 4.0;
    let take_pft_pct         = 6.0;
    let max_hold             = 25;
    let min_idx              = 70;   // Need long lookback + warmup

    let idx = ctx.index;
    if idx < min_idx { return; }

    let close = ctx.close;
    let atr   = ctx.atr(vol_filter_period);

    // Volatility filter: skip dead market (no arb opportunity when vol is zero)
    let vol_pct = if close > 0.0 { atr / close * 100.0 } else { 0.0 };
    if vol_pct < vol_min_pct { return; }

    // ── Compute multi-horizon returns ─────────────────────────────────────────
    let close_s = ctx.close_at(idx - short_return_period);
    let close_m = ctx.close_at(idx - medium_return_period);
    let close_l = ctx.close_at(idx - long_return_period);

    let ret_short  = if close_s > 0.0 { (close - close_s) / close_s * 100.0 } else { 0.0 };
    let ret_medium = if close_m > 0.0 { (close - close_m) / close_m * 100.0 } else { 0.0 };
    let ret_long   = if close_l > 0.0 { (close - close_l) / close_l * 100.0 } else { 0.0 };

    // Divergence: how much does short-term momentum deviate from medium-term baseline?
    // Positive divergence = short-term overperforming (might revert)
    let divergence = ret_short - (ret_medium / medium_return_period * short_return_period);

    // Track divergence Z-score via rolling moments
    let div_mean = ctx.get("div_mean", divergence);
    let div_var  = ctx.get("div_var",  0.01);
    let alpha     = 0.05;
    let new_dmean = div_mean + alpha * (divergence - div_mean);
    let new_dvar  = (1.0 - alpha) * (div_var + alpha * (divergence - div_mean) * (divergence - div_mean));
    ctx.set("div_mean", new_dmean);
    ctx.set("div_var",  new_dvar);

    let div_std    = new_dvar.sqrt();
    let div_zscore = if div_std > 0.001 { (divergence - new_dmean) / div_std } else { 0.0 };

    // Store current divergence for exit comparison
    let entry_divergence = ctx.get("entry_div", divergence);

    // ── Exit logic ───────────────────────────────────────────────────────────
    if ctx.position != 0.0 {
        let ep        = ctx.entry_price;
        let bars_held = idx - ctx.entry_index;
        let raw_pnl   = if ep > 0.0 { (close - ep) / ep * 100.0 } else { 0.0 };
        let eff_pnl   = if ctx.position < 0.0 { -raw_pnl } else { raw_pnl };

        // Exit when divergence has converged (mean-reversion completed)
        let div_abs = if divergence < 0.0 { -divergence } else { divergence };
        let converged = div_abs < convergence_exit;

        if eff_pnl >= take_pft_pct || eff_pnl <= -max_loss_pct || bars_held >= max_hold || converged {
            ctx.sell(1.0);
            ctx.set("entry_div", 0.0);
            ctx.log("EXIT corr_arb: pnl=" + eff_pnl + "% zscore=" + div_zscore + " converged=" + converged);
        }
        return;
    }

    let rsi = ctx.rsi(rsi_period);

    // ── Entry signals ─────────────────────────────────────────────────────────
    // LONG: short-term significantly underperformed (div_zscore strongly negative)
    //   → expect mean-reversion upward (convergence of short return to medium)
    if div_zscore < -2.2 && ret_long > -5.0 && rsi < 45.0 {
        ctx.buy(1.0);
        ctx.set_stop_loss(close - atr * 2.0);
        ctx.set_take_profit(close * (1.0 + take_pft_pct / 100.0));
        ctx.set("entry_div", divergence);
        ctx.log("LONG corr_arb: z=" + div_zscore + " div=" + divergence + "% ret_s=" + ret_short + "% ret_m=" + ret_medium + "%");
    }
    // SHORT: short-term significantly outperformed (div_zscore strongly positive)
    //   → expect mean-reversion downward (short return convergence to medium)
    else if div_zscore > 2.2 && ret_long < 5.0 && rsi > 55.0 {
        ctx.sell(1.0);
        ctx.set_stop_loss(close + atr * 2.0);
        ctx.set_take_profit(close * (1.0 - take_pft_pct / 100.0));
        ctx.set("entry_div", divergence);
        ctx.log("SHORT corr_arb: z=" + div_zscore + " div=" + divergence + "% ret_s=" + ret_short + "% ret_m=" + ret_medium + "%");
    }
}
