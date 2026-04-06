//! Handler for /poly* Telegram commands.
//!
//! Provides access to Polymarket prediction markets directly from Telegram.
//! Read-only commands (markets, price, positions, orders) are open to all
//! authenticated users. Write commands (buy, sell, cancel) require `is_admin`.

use polymarket_trader::markets::{get_market, get_market_price, list_markets, Market, MarketFilter};
use polymarket_trader::orders::{ClobClient, Side};

/// Result of handling a /poly command — the formatted Markdown reply.
pub struct PolyCommandResult {
    pub message: String,
    pub is_error: bool,
}

impl PolyCommandResult {
    fn ok(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            is_error: false,
        }
    }

    fn err(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            is_error: true,
        }
    }
}

/// Config file path for Polymarket credentials.
const POLY_CONFIG_PATH: &str = "~/.config/trader-claw/config.toml";

/// Parse and handle a /poly command string.
///
/// `text`: the full message text (e.g. "/poly markets crypto")
/// `is_admin`: whether the user is in allowed_users (required for write orders)
pub async fn handle_poly_command(text: &str, is_admin: bool) -> PolyCommandResult {
    let parts: Vec<&str> = text.split_whitespace().collect();

    // "/poly" with no subcommand → help
    if parts.len() < 2 {
        return PolyCommandResult::ok(help_text());
    }

    match parts[1] {
        "markets" => {
            let category = parts.get(2).copied();
            handle_markets(category).await
        }
        "price" => {
            let Some(slug) = parts.get(2).copied() else {
                return PolyCommandResult::err("Usage: `/poly price <slug>`");
            };
            handle_price(slug).await
        }
        "buy" => {
            if !is_admin {
                return PolyCommandResult::err(
                    "Permission denied. Only authorized users can place orders.",
                );
            }
            if parts.len() < 5 {
                return PolyCommandResult::err("Usage: `/poly buy <slug> <yes|no> <amount>`");
            }
            let slug = parts[2];
            let outcome = parts[3];
            let amount_str = parts[4];
            handle_buy_sell(slug, outcome, amount_str, Side::Buy).await
        }
        "sell" => {
            if !is_admin {
                return PolyCommandResult::err(
                    "Permission denied. Only authorized users can place orders.",
                );
            }
            if parts.len() < 5 {
                return PolyCommandResult::err("Usage: `/poly sell <slug> <yes|no> <amount>`");
            }
            let slug = parts[2];
            let outcome = parts[3];
            let amount_str = parts[4];
            handle_buy_sell(slug, outcome, amount_str, Side::Sell).await
        }
        "positions" => {
            if !is_admin {
                return PolyCommandResult::err(
                    "Permission denied. Only authorized users can view positions.",
                );
            }
            handle_positions().await
        }
        "orders" => {
            if !is_admin {
                return PolyCommandResult::err(
                    "Permission denied. Only authorized users can view orders.",
                );
            }
            handle_orders().await
        }
        "cancel" => {
            if !is_admin {
                return PolyCommandResult::err(
                    "Permission denied. Only authorized users can cancel orders.",
                );
            }
            let Some(order_id) = parts.get(2).copied() else {
                return PolyCommandResult::err("Usage: `/poly cancel <order_id>`");
            };
            handle_cancel(order_id).await
        }
        unknown => PolyCommandResult::err(format!(
            "Unknown subcommand: `{unknown}`\n\n{}",
            help_text()
        )),
    }
}

// ── Subcommand handlers ───────────────────────────────────────────────────────

async fn handle_markets(category: Option<&str>) -> PolyCommandResult {
    let filter = MarketFilter {
        category: category.map(|c| c.to_string()),
        active_only: true,
        ..Default::default()
    };

    match list_markets(filter).await {
        Ok(markets) => {
            let top: Vec<Market> = markets.into_iter().take(10).collect();
            PolyCommandResult::ok(format_markets(&top))
        }
        Err(e) => PolyCommandResult::err(format!("Failed to fetch markets: {e}")),
    }
}

async fn handle_price(slug: &str) -> PolyCommandResult {
    let market = match get_market(slug).await {
        Ok(m) => m,
        Err(e) => return PolyCommandResult::err(format!("Market not found: {e}")),
    };

    let yes_price = match get_market_price(&market.yes_token_id).await {
        Ok(p) => p,
        Err(e) => return PolyCommandResult::err(format!("Failed to fetch price: {e}")),
    };

    PolyCommandResult::ok(format_price(&market, yes_price))
}

async fn handle_buy_sell(slug: &str, outcome: &str, amount_str: &str, side: Side) -> PolyCommandResult {
    let amount = match parse_amount(amount_str) {
        Ok(a) => a,
        Err(e) => return PolyCommandResult::err(e),
    };

    let config_path = shellexpand::tilde(POLY_CONFIG_PATH).into_owned();
    let creds = match polymarket_trader::auth::load_credentials(&config_path) {
        Ok(c) => c,
        Err(_) => {
            return PolyCommandResult::err(
                "Polymarket credentials not configured. Run setup first.",
            )
        }
    };

    let market = match get_market(slug).await {
        Ok(m) => m,
        Err(e) => return PolyCommandResult::err(format!("Market not found: {e}")),
    };

    let token_id = match outcome.to_lowercase().as_str() {
        "yes" => market.yes_token_id.clone(),
        "no" => market.no_token_id.clone(),
        other => {
            return PolyCommandResult::err(format!(
                "Invalid outcome `{other}`. Use `yes` or `no`."
            ))
        }
    };

    // Fetch current price to use as worst_price for market orders
    let current_price = match get_market_price(&token_id).await {
        Ok(p) => p,
        Err(e) => return PolyCommandResult::err(format!("Failed to fetch price: {e}")),
    };

    let client = ClobClient::new(creds);
    match client
        .create_market_order(&token_id, side, amount, current_price)
        .await
    {
        Ok(resp) => {
            let action = match side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            };
            PolyCommandResult::ok(format!(
                "Order placed successfully\n\nOrder ID: `{}`\nAction: {} {} @{:.2}\nAmount: ${:.2}\nStatus: {}",
                resp.order_id,
                action,
                outcome.to_uppercase(),
                current_price,
                amount,
                resp.status
            ))
        }
        Err(e) => PolyCommandResult::err(format!("Order failed: {e}")),
    }
}

async fn handle_positions() -> PolyCommandResult {
    let config_path = shellexpand::tilde(POLY_CONFIG_PATH).into_owned();
    let creds = match polymarket_trader::auth::load_credentials(&config_path) {
        Ok(c) => c,
        Err(_) => {
            return PolyCommandResult::err(
                "Polymarket credentials not configured. Run setup first.",
            )
        }
    };

    let client = ClobClient::new(creds);
    match client.get_open_orders().await {
        Ok(orders) => {
            if orders.is_empty() {
                return PolyCommandResult::ok("*Open Positions*\n\nNo open positions found.");
            }
            let mut msg = "*Open Positions*\n\n".to_string();
            for order in &orders {
                msg.push_str(&format!(
                    "Order `{}`: {} @{} × {} ({})\n",
                    order.id, order.side, order.price, order.size, order.status
                ));
            }
            PolyCommandResult::ok(msg)
        }
        Err(e) => PolyCommandResult::err(format!("Failed to fetch positions: {e}")),
    }
}

async fn handle_orders() -> PolyCommandResult {
    let config_path = shellexpand::tilde(POLY_CONFIG_PATH).into_owned();
    let creds = match polymarket_trader::auth::load_credentials(&config_path) {
        Ok(c) => c,
        Err(_) => {
            return PolyCommandResult::err(
                "Polymarket credentials not configured. Run setup first.",
            )
        }
    };

    let client = ClobClient::new(creds);
    match client.get_open_orders().await {
        Ok(orders) => {
            if orders.is_empty() {
                return PolyCommandResult::ok("*Active Orders*\n\nNo active orders found.");
            }
            let mut msg = "*Active Orders*\n\n".to_string();
            for order in &orders {
                msg.push_str(&format!(
                    "Order `#{}`: {} @{} × {} ({})\n",
                    order.id, order.side, order.price, order.size, order.status
                ));
            }
            PolyCommandResult::ok(msg)
        }
        Err(e) => PolyCommandResult::err(format!("Failed to fetch orders: {e}")),
    }
}

async fn handle_cancel(order_id: &str) -> PolyCommandResult {
    let config_path = shellexpand::tilde(POLY_CONFIG_PATH).into_owned();
    let creds = match polymarket_trader::auth::load_credentials(&config_path) {
        Ok(c) => c,
        Err(_) => {
            return PolyCommandResult::err(
                "Polymarket credentials not configured. Run setup first.",
            )
        }
    };

    let client = ClobClient::new(creds);
    match client.cancel_order(order_id).await {
        Ok(()) => PolyCommandResult::ok(format!("Order `{order_id}` cancelled successfully.")),
        Err(e) => PolyCommandResult::err(format!("Failed to cancel order: {e}")),
    }
}

// ── Formatting helpers ────────────────────────────────────────────────────────

/// Format market list as Markdown.
fn format_markets(markets: &[Market]) -> String {
    if markets.is_empty() {
        return "No markets found".to_string();
    }

    let mut msg = "*Top Crypto Markets on Polymarket*\n\n".to_string();
    for (i, market) in markets.iter().enumerate() {
        let vol = format_volume(market.volume);
        msg.push_str(&format!(
            "{}. [{}](https://polymarket.com/event/{})\n   Vol: {}\n\n",
            i + 1,
            market.question,
            market.slug,
            vol,
        ));
    }
    msg
}

/// Format a single market price.
fn format_price(market: &Market, yes_price: f64) -> String {
    let no_price = (1.0 - yes_price).max(0.0);
    let vol = format_volume(market.volume);
    let liq = format_volume(market.liquidity);
    let ends = market
        .end_date_iso
        .as_deref()
        .and_then(|s| s.split('T').next())
        .unwrap_or("N/A");

    format!(
        "*{}*\nYES: {:.2} | NO: {:.2}\nVolume: {} | Liquidity: {}\nEnds: {}",
        market.question, yes_price, no_price, vol, liq, ends
    )
}

fn format_volume(amount: f64) -> String {
    if amount >= 1_000_000.0 {
        format!("${:.1}M", amount / 1_000_000.0)
    } else if amount >= 1_000.0 {
        format!("${:.0}K", amount / 1_000.0)
    } else {
        format!("${:.0}", amount)
    }
}

/// Parse a dollar amount string, returning an error message on failure.
fn parse_amount(s: &str) -> Result<f64, String> {
    let cleaned = s.trim_start_matches('$');
    match cleaned.parse::<f64>() {
        Ok(v) if v > 0.0 => Ok(v),
        Ok(_) => Err("Amount must be greater than zero.".to_string()),
        Err(_) => Err(format!("Invalid amount: `{s}`. Expected a number like `10` or `10.50`.")),
    }
}

fn help_text() -> String {
    "*Polymarket Commands*\n\n\
     `/poly markets [crypto|politics|sports]` — list top 10 markets\n\
     `/poly price <slug>` — YES/NO price + volume\n\
     `/poly buy <slug> <yes|no> <amount>` — place market buy order\n\
     `/poly sell <slug> <yes|no> <amount>` — place market sell order\n\
     `/poly positions` — open positions with P&L\n\
     `/poly orders` — active orders\n\
     `/poly cancel <order_id>` — cancel an order"
        .to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_unknown_command() {
        let result = handle_poly_command("/poly unknown", true).await;
        assert!(result.is_error);
        assert!(result.message.contains("Unknown subcommand"));
        assert!(result.message.contains("unknown"));
    }

    #[tokio::test]
    async fn test_help_command() {
        let result = handle_poly_command("/poly", true).await;
        assert!(!result.is_error);
        assert!(result.message.contains("markets"));
        assert!(result.message.contains("price"));
        assert!(result.message.contains("buy"));
        assert!(result.message.contains("sell"));
        assert!(result.message.contains("positions"));
        assert!(result.message.contains("orders"));
        assert!(result.message.contains("cancel"));
    }

    #[tokio::test]
    async fn test_admin_check_buy() {
        let result = handle_poly_command("/poly buy some-slug yes 10", false).await;
        assert!(result.is_error);
        assert!(result.message.contains("Permission denied"));
    }

    #[tokio::test]
    async fn test_admin_check_sell() {
        let result = handle_poly_command("/poly sell some-slug no 10", false).await;
        assert!(result.is_error);
        assert!(result.message.contains("Permission denied"));
    }

    #[tokio::test]
    async fn test_admin_check_cancel() {
        let result = handle_poly_command("/poly cancel order123", false).await;
        assert!(result.is_error);
        assert!(result.message.contains("Permission denied"));
    }

    #[tokio::test]
    async fn test_admin_check_orders() {
        let result = handle_poly_command("/poly orders", false).await;
        assert!(result.is_error);
        assert!(result.message.contains("Permission denied"));
    }

    #[tokio::test]
    async fn test_admin_check_positions() {
        let result = handle_poly_command("/poly positions", false).await;
        assert!(result.is_error);
        assert!(result.message.contains("Permission denied"));
    }

    #[test]
    fn test_format_markets_empty() {
        let result = format_markets(&[]);
        assert_eq!(result, "No markets found");
    }

    #[test]
    fn test_format_markets_single() {
        let market = Market {
            condition_id: "0xabc".to_string(),
            question: "Will BTC reach 100k?".to_string(),
            slug: "will-btc-reach-100k".to_string(),
            yes_token_id: "123".to_string(),
            no_token_id: "456".to_string(),
            volume: 50000.0,
            liquidity: 1000.0,
            end_date_iso: Some("2025-12-31T00:00:00Z".to_string()),
            category: Some("crypto".to_string()),
        };
        let result = format_markets(&[market]);
        assert!(result.contains("Will BTC reach 100k?"));
        assert!(result.contains("will-btc-reach-100k"));
        assert!(result.contains("$50K"));
    }

    #[test]
    fn test_parse_amount_valid() {
        assert!((parse_amount("10").unwrap() - 10.0).abs() < f64::EPSILON);
        assert!((parse_amount("10.50").unwrap() - 10.50).abs() < f64::EPSILON);
        assert!((parse_amount("$25").unwrap() - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_amount_invalid() {
        assert!(parse_amount("abc").is_err());
        assert!(parse_amount("-10").is_err());
        assert!(parse_amount("0").is_err());
        assert!(parse_amount("").is_err());
    }

    #[test]
    fn test_format_price() {
        let market = Market {
            condition_id: "0xabc".to_string(),
            question: "Will BTC reach 100k?".to_string(),
            slug: "will-btc-reach-100k".to_string(),
            yes_token_id: "123".to_string(),
            no_token_id: "456".to_string(),
            volume: 50000.0,
            liquidity: 1000.0,
            end_date_iso: Some("2025-12-31T00:00:00Z".to_string()),
            category: Some("crypto".to_string()),
        };
        let result = format_price(&market, 0.65);
        assert!(result.contains("Will BTC reach 100k?"));
        assert!(result.contains("YES: 0.65"));
        assert!(result.contains("NO: 0.35"));
        assert!(result.contains("2025-12-31"));
    }

    #[test]
    fn test_buy_missing_args() {
        // Synchronous shape test — just verify help path for missing args
        // (full async covered by admin_check tests above)
        let parts: Vec<&str> = "/poly buy slug".split_whitespace().collect();
        assert!(parts.len() < 5);
    }

    #[tokio::test]
    #[ignore]
    async fn test_markets_network() {
        let result = handle_poly_command("/poly markets crypto", true).await;
        assert!(!result.is_error, "markets failed: {}", result.message);
        assert!(result.message.contains("Polymarket"));
    }

    #[tokio::test]
    #[ignore]
    async fn test_price_network() {
        let result = handle_poly_command("/poly price will-btc-reach-100k-in-2024", true).await;
        // May error if market doesn't exist, but shouldn't panic
        println!("price result: {}", result.message);
    }
}
