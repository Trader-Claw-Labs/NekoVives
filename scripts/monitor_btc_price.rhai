// monitor_btc_price.rhai
// Scheduled Job: BTC/USDT Price Alert (15-minute window)
// Recommended schedule: every 5 minutes (*/5 * * * *)
//
// Fetches the current BTC/USDT price from Binance.
// Compares against the price captured 15 minutes ago.
// If the move is ±3% or more, fires a Telegram alert.
//
// Monitoring Rhai API:
//   ctx.price(symbol)   -> float (current spot price, e.g. "BTCUSDT")
//   ctx.timestamp()     -> integer (current Unix epoch seconds)
//   ctx.telegram(msg)   -> sends Telegram message
//   ctx.get(key, def)   -> persistent key-value read
//   ctx.set(key, val)   -> persistent key-value write
//   ctx.log(msg)        -> write to job output log

let SYMBOL         = "BTCUSDT";
let ALERT_PCT      = 3.0;   // Trigger alert if move >= this %
let WINDOW_MINUTES = 15;    // Compare current price to N minutes ago

fn on_tick(ctx) {
    let now       = ctx.timestamp();
    let btc_price = ctx.price(SYMBOL);

    ctx.log("BTC/USDT: $" + btc_price);

    let last_price = ctx.get("btc_ref_price", 0.0);
    let last_time  = ctx.get("btc_ref_time",  0);

    if last_price <= 0.0 {
        // First run — set baseline
        ctx.set("btc_ref_price", btc_price);
        ctx.set("btc_ref_time",  now);
        ctx.log("Baseline set: $" + btc_price);
        return;
    }

    let elapsed_mins = (now - last_time) / 60;

    if elapsed_mins >= WINDOW_MINUTES {
        let change_pct = (btc_price - last_price) / last_price * 100.0;

        ctx.log("15-min change: " + change_pct + "%");

        if change_pct > ALERT_PCT || change_pct < -ALERT_PCT {
            let dir   = if change_pct > 0.0 { "🚀 PUMP" } else { "🔴 DUMP" };
            let emoji = if change_pct > 0.0 { "📈" } else { "📉" };

            ctx.telegram(emoji + " BTC Price Alert!\n" +
                "Move: " + change_pct + "% " + dir + "\n" +
                "Now (" + WINDOW_MINUTES + "min ago):\n" +
                "  $" + btc_price + " (was $" + last_price + ")\n" +
                "Timeframe: " + elapsed_mins + " minutes");
        }

        // Slide the window forward
        ctx.set("btc_ref_price", btc_price);
        ctx.set("btc_ref_time",  now);
    } else {
        ctx.log("Window not elapsed yet (" + elapsed_mins + "/" + WINDOW_MINUTES + " min)");
    }
}
