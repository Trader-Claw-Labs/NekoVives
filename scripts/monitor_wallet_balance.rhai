// monitor_wallet_balance.rhai
// Scheduled Job: Wallet Balance Scanner
// Recommended schedule: every 30 minutes (*/30 * * * *)
//
// Checks all configured wallets. If any balance changes by ±3%
// since the last check, sends a Telegram alert.
//
// Monitoring Rhai API:
//   ctx.wallets()              -> array of {address, chain, label}
//   ctx.wallet_balance(addr)   -> float (native token balance)
//   ctx.telegram(msg)          -> sends Telegram message
//   ctx.get(key, default)      -> persistent key-value read
//   ctx.set(key, value)        -> persistent key-value write
//   ctx.log(msg)               -> write to job output log

fn on_tick(ctx) {
    let wallets = ctx.wallets();

    if wallets.len() == 0 {
        ctx.log("No wallets configured. Add wallets in the Wallets section.");
        return;
    }

    let alerts_sent = 0;

    for wallet in wallets {
        let address = wallet.address;
        let label   = wallet.label ?? address.slice(0, 8) + "...";
        let current = ctx.wallet_balance(address);
        let key     = "bal_" + address;
        let prev    = ctx.get(key, -1.0);

        if prev < 0.0 {
            // First run — store baseline
            ctx.set(key, current);
            ctx.log("Baseline set for " + label + ": " + current);
            continue;
        }

        if prev == 0.0 && current == 0.0 {
            continue;
        }

        let change_pct = if prev > 0.0 {
            (current - prev) / prev * 100.0
        } else {
            0.0
        };

        ctx.log(label + ": " + prev + " -> " + current + " (" + change_pct + "%)");

        if change_pct > 3.0 || change_pct < -3.0 {
            let dir   = if change_pct > 0.0 { "📈 UP" } else { "📉 DOWN" };
            let emoji = if change_pct > 0.0 { "🟢" } else { "🔴" };

            ctx.telegram(emoji + " Wallet Balance Alert!\n" +
                "Wallet: " + label + "\n" +
                "Address: " + address + "\n" +
                "Change: " + change_pct + "% " + dir + "\n" +
                "Previous: " + prev + "\n" +
                "Current: " + current);

            alerts_sent += 1;
        }

        // Update stored balance
        ctx.set(key, current);
    }

    ctx.log("Scan complete. " + alerts_sent + " alert(s) sent.");
}
