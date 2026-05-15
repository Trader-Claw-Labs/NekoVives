// Copy Polymarket Specialist — Mirror mode per-category
// WARNING: This script is DISABLED by default. Only enable after 30+ days
// of Discovery and 30+ days of Consensus validation.
//
// Copy trades from Polymarket specialist wallets (score > 75) matched by category.

fn on_leader_fill(ctx, fill) {
    // Only operate on Polymarket
    if fill.venue != "polymarket" {
        return;
    }

    let score = ctx.leader_score(fill.leader);
    if score < 75.0 {
        return;
    }

    // Category matching: only copy if leader specializes in this market category
    if !ctx.leader_category_match(fill.leader, fill.market_category) {
        return;
    }

    let leader_pct = fill.notional / ctx.leader_capital(fill.leader);
    let my_size = ctx.balance * leader_pct * (score / 100.0) * 0.5;

    // Venue minimum notional (gas economics)
    if my_size < ctx.venue_min_notional(fill.venue) {
        return;
    }

    // Liquidity check — don't be more than 5% of available depth
    let max_size_by_liq = ctx.available_liquidity(fill.venue, fill.symbol) * 0.05;
    let my_size = if my_size > max_size_by_liq { max_size_by_liq } else { my_size };

    if fill.side == "buy" {
        ctx.buy_at(fill.venue, fill.symbol, my_size);
    } else if fill.side == "sell" {
        ctx.sell_at(fill.venue, fill.symbol, my_size);
    }

    // Track for exit mapping
    ctx.set("mirror_" + fill.leader_fill_id, fill);
}
