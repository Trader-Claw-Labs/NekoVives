// funding_arb.rhai
//
// Funding rate arbitrage between Hyperliquid (1h funding) and Binance perps (8h funding).
// Market-neutral. Long the venue with lower funding, short the venue with higher funding.
//
// Requires:
// - hyperliquid-trader crate enabled in config
// - Binance perps API keys configured in wallet-manager
// - on_event(ctx, event) entry point support in Rhai engine
//
// Tunables. Override in dashboard Strategy Studio without editing this file.
const MIN_APR_DIFF      = 0.10;   // 10% APR minimum after fees to open
const FORCE_CLOSE_DIFF  = 0.02;   // close if APR diff drops below 2%
const MAX_POS_PCT       = 0.15;   // max 15% of capital per pair (split across both legs)
const MAX_OPEN_PAIRS    = 4;      // cap concurrent arbs
const FEE_BUFFER_BPS    = 12.0;   // taker fee + slippage estimate per leg per cycle

const WATCHLIST = [
    "BTC", "ETH", "SOL", "AVAX",
    "ARB", "SUI", "INJ", "OP",
];

// State helpers. Persisted by ctx between calls.
fn k_diff(symbol)  { "open_diff_" + symbol }
fn k_pair(symbol)  { "pair_open_" + symbol }
fn open_pairs(ctx) { ctx.get("open_pairs_count", 0) }

fn record_open(ctx, symbol, diff_apr) {
    ctx.set(k_diff(symbol), diff_apr);
    ctx.set(k_pair(symbol), true);
    ctx.set("open_pairs_count", open_pairs(ctx) + 1);
}

fn record_close(ctx, symbol) {
    ctx.set(k_diff(symbol), 0.0);
    ctx.set(k_pair(symbol), false);
    let n = open_pairs(ctx);
    if n > 0 { ctx.set("open_pairs_count", n - 1); }
}

// Net APR after estimated round-trip fees.
fn net_apr_diff(raw_diff_apr) {
    let fees_apr = (FEE_BUFFER_BPS / 10000.0) * 24.0 * 365.0 / 8.0;
    raw_diff_apr - fees_apr
}

// Main entry point. Called whenever any venue emits a funding cycle event.
fn on_event(ctx, event) {
    if event.kind != "funding_cycle" { return; }

    // Risk gate is upstream — but we still self-throttle.
    if ctx.regime() == "high_vol_chop" { return; }
    if open_pairs(ctx) >= MAX_OPEN_PAIRS && !ctx.get(k_pair(event.symbol), false) {
        return;
    }

    for symbol in WATCHLIST {
        let f_bin_apr = ctx.funding_rate_apr("binance", symbol);
        let f_hl_apr  = ctx.funding_rate_apr("hyperliquid", symbol);
        let raw_diff  = (f_hl_apr - f_bin_apr).abs();
        let net_diff  = net_apr_diff(raw_diff);

        let pos_bin = ctx.position_at("binance", symbol);
        let pos_hl  = ctx.position_at("hyperliquid", symbol);
        let in_pair = pos_bin != 0.0 || pos_hl != 0.0;

        // OPEN
        if !in_pair && net_diff > MIN_APR_DIFF
                    && open_pairs(ctx) < MAX_OPEN_PAIRS {
            let leg_size_usd = ctx.balance * MAX_POS_PCT * 0.5;
            if f_hl_apr > f_bin_apr {
                ctx.short_at("hyperliquid", symbol, leg_size_usd);
                ctx.buy_at("binance", symbol, leg_size_usd);
            } else {
                ctx.short_at("binance", symbol, leg_size_usd);
                ctx.buy_at("hyperliquid", symbol, leg_size_usd);
            }
            record_open(ctx, symbol, net_diff);
            print("opened arb on " + symbol + " net APR " + net_diff);

        // CLOSE
        } else if in_pair && raw_diff < FORCE_CLOSE_DIFF {
            ctx.close_at("binance", symbol);
            ctx.close_at("hyperliquid", symbol);
            record_close(ctx, symbol);
            print("closed arb on " + symbol + " spread collapsed");
        }
    }
}

// Optional: also wire into on_candle for periodic basis monitoring.
// If the spot-perp basis on either venue blows out > 1.5%, we close that pair
// regardless of funding rate (signals counterparty / liquidity stress).
fn on_candle(ctx) {
    if ctx.index < 2 { return; }

    for symbol in WATCHLIST {
        if !ctx.get(k_pair(symbol), false) { continue; }

        let basis_bin = ctx.basis_pct("binance", symbol);
        let basis_hl  = ctx.basis_pct("hyperliquid", symbol);

        if basis_bin.abs() > 0.015 || basis_hl.abs() > 0.015 {
            ctx.close_at("binance", symbol);
            ctx.close_at("hyperliquid", symbol);
            record_close(ctx, symbol);
            print("closed " + symbol + " basis stress " +
                  basis_bin + " / " + basis_hl);
        }
    }
}
