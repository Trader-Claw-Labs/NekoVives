// monitor_polymarket.rhai
// Scheduled Job: Polymarket Market Monitor
// Recommended schedule: every 10 minutes (*/10 * * * *)
//
// Monitors specific Polymarket markets. Sends a Telegram alert
// when the YES price moves more than 5% since the last check,
// or when volume/activity thresholds are crossed.
//
// To watch different markets, update WATCH_SLUGS below.
//
// Monitoring Rhai API:
//   ctx.polymarket_market(slug) -> map: {question, yes_price, no_price, volume, end_date}
//   ctx.telegram(msg)           -> sends Telegram message
//   ctx.get(key, default)       -> persistent key-value read
//   ctx.set(key, value)         -> persistent key-value write
//   ctx.log(msg)                -> write to job output log

// ── Configure market slugs to watch ────────────────────────────────
let WATCH_SLUGS = [
    "will-btc-hit-100k-in-2025",
    "will-eth-reach-5000-in-2025",
    "will-donald-trump-finish-his-second-term"
];

let PRICE_ALERT_THRESHOLD = 5.0;   // % YES price movement
let MIN_VOLUME = 1000.0;            // Minimum $ volume to bother alerting

fn on_tick(ctx) {
    ctx.log("Checking " + WATCH_SLUGS.len() + " Polymarket markets...");
    let alerts_sent = 0;

    for slug in WATCH_SLUGS {
        let market = ctx.polymarket_market(slug);

        if market == () {
            ctx.log("Market not found: " + slug);
            continue;
        }

        let yes_price = market.yes_price;  // 0.0 – 1.0 (probability)
        let volume    = market.volume;
        let question  = market.question;
        let key       = "pm_yes_" + slug;
        let prev      = ctx.get(key, -1.0);

        let yes_pct = yes_price * 100.0;

        ctx.log(question + " | YES: " + yes_pct + "% | Vol: $" + volume);

        if prev >= 0.0 && volume >= MIN_VOLUME {
            let change = (yes_price - prev) * 100.0;  // in percentage points

            if change > PRICE_ALERT_THRESHOLD || change < -PRICE_ALERT_THRESHOLD {
                let dir   = if change > 0.0 { "▲" } else { "▼" };
                let emoji = if change > 0.0 { "🟢" } else { "🔴" };

                ctx.telegram(emoji + " Polymarket Alert!\n" +
                    "Market: " + question + "\n" +
                    "YES: " + yes_pct + "% " + dir + " " + change.abs() + " pts\n" +
                    "NO:  " + (100.0 - yes_pct) + "%\n" +
                    "Vol: $" + volume);

                alerts_sent += 1;
            }
        } else if prev < 0.0 {
            ctx.log("Baseline set for: " + question);
        }

        ctx.set(key, yes_price);
    }

    ctx.log("Done. " + alerts_sent + " alert(s) sent.");
}
