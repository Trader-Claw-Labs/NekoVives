// monitor_eth_price.rhai
// Scheduled Job: ETH/USDC Price Target Alert
// Recommended schedule: every minute (*/1 * * * *)
//
// Monitors ETH/USDC price. Sends a Telegram alert the first time
// ETH crosses above $2,500. Resets the alert flag when price drops
// back below $2,400 so the alert fires again on the next crossing.
//
// Monitoring Rhai API:
//   ctx.price(symbol)  -> float (current spot price)
//   ctx.telegram(msg)  -> sends Telegram message
//   ctx.get(key, def)  -> persistent key-value read
//   ctx.set(key, val)  -> persistent key-value write
//   ctx.log(msg)       -> write to job output log

let SYMBOL        = "ETHUSDC";
let TARGET_PRICE  = 2500.0;   // Alert fires when ETH crosses above this
let RESET_PRICE   = 2400.0;   // Alert flag resets when ETH drops below this

fn on_tick(ctx) {
    let eth_price = ctx.price(SYMBOL);
    let alerted   = ctx.get("eth_above_target", false);

    ctx.log("ETH/USDC: $" + eth_price + " | alerted=" + alerted);

    // ── Rising cross: ETH goes above TARGET ────────────────────────
    if eth_price >= TARGET_PRICE && !alerted {
        ctx.telegram("⚡ ETH Price Alert!\n" +
            "ETH crossed above $" + TARGET_PRICE + "!\n" +
            "Current price: $" + eth_price + "\n" +
            "(Alert will reset if ETH drops below $" + RESET_PRICE + ")");
        ctx.set("eth_above_target", true);
        ctx.log("Alert sent! ETH = $" + eth_price);
        return;
    }

    // ── Falling cross: ETH drops below RESET — clear the flag ──────
    if eth_price < RESET_PRICE && alerted {
        ctx.set("eth_above_target", false);
        ctx.log("Alert flag cleared. ETH below $" + RESET_PRICE);
    }
}
