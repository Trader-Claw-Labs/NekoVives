// Cancel a Polymarket order by ID.
// Run: ORDER_ID=0x... cargo run -p polymarket-trader --example cancel_order --release

use polymarket_trader::auth::PolyCredentials;
use polymarket_trader::orders::ClobClient;
use std::path::PathBuf;
use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let order_id = std::env::var("ORDER_ID").expect("set ORDER_ID env var");
    let creds = read_polymarket_config()?;
    let client = ClobClient::new(creds);
    
    println!("Cancelling order: {order_id}");
    match client.cancel_order(&order_id).await {
        Ok(_) => println!("✓ Cancelled"),
        Err(e) => println!("✗ Cancel failed: {e:#}"),
    }
    Ok(())
}

fn read_polymarket_config() -> Result<PolyCredentials> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let path: PathBuf = [home.as_str(), ".traderclaw", "config.toml"].iter().collect();
    let raw = std::fs::read_to_string(&path)?;
    let mut in_poly = false;
    let mut api_key = String::new();
    let mut secret = String::new();
    let mut passphrase = String::new();
    let mut wallet_address = String::new();
    let mut private_key: Option<String> = None;
    let mut proxy_address: Option<String> = None;
    let mut signature_type: Option<String> = None;
    for line in raw.lines() {
        let t = line.trim();
        if t.starts_with('[') { in_poly = t == "[polymarket]"; continue; }
        if !in_poly { continue; }
        let Some(eq) = t.find('=') else { continue };
        let key = t[..eq].trim();
        let value = t[eq+1..].trim().trim_matches('"').to_string();
        match key {
            "api_key" => api_key = value,
            "secret" => secret = value,
            "passphrase" => passphrase = value,
            "wallet_address" => wallet_address = value,
            "private_key" => private_key = Some(value),
            "proxy_address" => proxy_address = Some(value),
            "signature_type" => signature_type = Some(value),
            _ => {}
        }
    }
    Ok(PolyCredentials { api_key, secret, passphrase, wallet_address, private_key, is_builder: false, proxy_address, signature_type })
}
