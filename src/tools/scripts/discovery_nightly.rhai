// Discovery Nightly Job — shadow tracking without real capital
// This script runs as a scheduled cron job to index and score wallets.
// It NEVER executes trades with real capital.

fn on_event(ctx, event) {
    if event.kind != "discovery_run" {
        return;
    }

    let venue = event.venue;
    let lookback_days = event.lookback_days;

    // Score the universe of active wallets
    let scored = ctx.score_universe(venue, lookback_days);

    for candidate in scored {
        if candidate.score < 75.0 {
            continue;
        }
        if ctx.is_in_active_watchlist(candidate.wallet) {
            continue;
        }
        if ctx.is_in_blacklist(candidate.wallet) {
            continue;
        }

        // Add to candidate list and start shadow tracking
        ctx.add_to_candidate_list(candidate.wallet, candidate.score);
        ctx.start_shadow_tracking(candidate.wallet);
    }
}
