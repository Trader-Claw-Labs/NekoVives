//! Standalone diagnostic: place a real $5 order on Polymarket and dump
//! every step so we can isolate `order_version_mismatch` failures.
//!
//! Run with:
//!   cargo run -p polymarket-trader --example place_real_order
//!
//! It reads `~/.traderclaw/config.toml` directly (line-based, no toml crate)
//! and uses the values from the [polymarket] section.

use anyhow::{Context, Result};
use polymarket_trader::auth::PolyCredentials;
use polymarket_trader::orders::{ClobClient, Side};
use std::path::PathBuf;

/// Token id from the user's last failing log line.
/// Replace via env `TOKEN_ID=...` to test a different token.
const DEFAULT_TOKEN_ID: &str =
    "771622903390086193591732771642437374443755254772286304800168790498547751742";

/// Default $5 BUY at $0.50 limit (small enough to be effectively market).
const DEFAULT_USD: f64 = 5.0;
const DEFAULT_PRICE: f64 = 0.50;

#[tokio::main]
async fn main() -> Result<()> {
    println!("============================================================");
    println!(" Polymarket — real $5 order diagnostic");
    println!("============================================================\n");

    let creds = read_polymarket_config()?;
    print_creds_summary(&creds);

    // Derive what addresses the SDK would compute from the private key,
    // so we can tell whether `proxy_address` is a Polymarket Proxy (EIP-1167)
    // or a Gnosis Safe — they collide differently and the CLOB cares.
    if let Some(pk_hex) = creds.private_key.as_deref() {
        use polymarket_client_sdk_v2::POLYGON;
        use polymarket_client_sdk_v2::auth::{LocalSigner, Signer};
        use polymarket_client_sdk_v2::{derive_proxy_wallet, derive_safe_wallet};
        use std::str::FromStr;

        let signer = LocalSigner::from_str(pk_hex).expect("parse pk");
        let signer_addr = signer.address();
        let derived_proxy = derive_proxy_wallet(signer_addr, POLYGON);
        let derived_safe  = derive_safe_wallet(signer_addr, POLYGON);
        println!("\n── Derivations from private_key ─────────────────────────");
        println!("signer (EOA)       : {signer_addr}");
        println!("derived Proxy      : {:?}", derived_proxy);
        println!("derived Gnosis Safe: {:?}", derived_safe);
        if let Some(cfg_proxy) = creds.proxy_address.as_deref()
            .and_then(|s| s.parse::<polymarket_client_sdk_v2::types::Address>().ok())
        {
            let is_proxy = derived_proxy.map_or(false, |d| d == cfg_proxy);
            let is_safe  = derived_safe.map_or(false, |d| d == cfg_proxy);
            println!("config proxy_address matches: proxy={is_proxy}  safe={is_safe}");
            if is_safe {
                println!("\n⚠  Your wallet is a Gnosis Safe — set signature_type=\"gnosis_safe\" (NOT \"proxy\")");
            } else if is_proxy {
                println!("\n✓ Wallet is a Polymarket Proxy — signature_type=\"proxy\" is correct");
            } else {
                println!("\n⚠  proxy_address matches NEITHER derivation. Either it's wrong, or");
                println!("   the signer/private_key doesn't own this wallet.");
            }
        }
        println!();
    }

    let token_id =
        std::env::var("TOKEN_ID").unwrap_or_else(|_| DEFAULT_TOKEN_ID.to_string());
    let usd: f64 = std::env::var("USD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_USD);
    let price: f64 = std::env::var("PRICE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PRICE);

    println!("\nToken id : {token_id}");
    println!("USD      : ${usd}");
    println!("Price    : {price}\n");

    // ── Step 1: query neg-risk for this token ──────────────────────────────
    println!("────────────────────────────────────────────────────────────");
    println!("Step 1: GET /neg-risk?token_id=…");
    println!("────────────────────────────────────────────────────────────");
    let http = reqwest::Client::new();
    let neg_risk_url = format!("https://clob.polymarket.com/neg-risk?token_id={token_id}");
    match http.get(&neg_risk_url).send().await {
        Ok(r) => {
            let st = r.status();
            let body = r.text().await.unwrap_or_default();
            println!("HTTP {st}");
            println!("Body: {body}\n");
            if body.contains("\"neg_risk\":true") {
                println!("⚠  This token IS a NegRisk market.");
                println!("   The SDK should sign against the NegRisk Exchange contract.");
                println!("   If order_version_mismatch still happens, the SDK contract_config");
                println!("   may be stale. Check `polymarket_client_sdk_v2::contract_config`.\n");
            } else if body.contains("\"neg_risk\":false") {
                println!("✓ Token is a regular CTF Exchange market (not NegRisk).\n");
            }
        }
        Err(e) => println!("✗ neg-risk request failed: {e}\n"),
    }

    // ── Step 2: balance / allowance via authenticated CLOB API ─────────────
    println!("────────────────────────────────────────────────────────────");
    println!("Step 2: authenticated balance + allowance");
    println!("────────────────────────────────────────────────────────────");
    let client = ClobClient::new(creds.clone());
    match client.get_api_balance().await {
        Ok(b) => println!("CLOB API balance (collateral): ${:.2}\n", b),
        Err(e) => println!("✗ balance_allowance failed: {e:#}\n"),
    }

    // ── Step 3: place a real $5 limit BUY ──────────────────────────────────
    println!("────────────────────────────────────────────────────────────");
    println!("Step 3: place limit order  ${usd} @ {price} BUY");
    println!("────────────────────────────────────────────────────────────");
    let size = (usd / price * 100.0).round() / 100.0; // shares, 2dp
    println!("Computed size = {size} shares");

    match client.create_limit_order(&token_id, Side::Buy, price, size).await {
        Ok(resp) => {
            println!("\n✓ ORDER ACCEPTED");
            println!("   order_id : {}", resp.order_id);
            println!("   status   : {}", resp.status);
        }
        Err(e) => {
            println!("\n✗ ORDER REJECTED");
            println!("   error: {e:#}");
            let s = format!("{e:#}");
            if s.contains("order_version_mismatch") {
                println!("\n   → order_version_mismatch means the EIP-712 verifyingContract");
                println!("     used to sign the order does NOT match the on-chain Exchange");
                println!("     that the CLOB expects for this token.");
                println!("     Common causes:");
                println!("       1. SDK signed against CTF Exchange but the market is NegRisk");
                println!("          (or vice-versa). The SDK 0.4.4 fetches /neg-risk per token,");
                println!("          so check Step 1 output.");
                println!("       2. Wrong signature_type in config (proxy vs gnosis_safe).");
                println!("       3. Funder address (proxy) doesn't actually match the on-chain");
                println!("          owner of the CTF tokens you're trying to trade.");
            }
        }
    }

    println!();
    Ok(())
}

fn print_creds_summary(c: &PolyCredentials) {
    let short = |s: &str| -> String {
        if s.len() <= 12 { s.to_string() } else { format!("{}…{}", &s[..8], &s[s.len() - 4..]) }
    };
    println!("api_key        : {}", short(&c.api_key));
    println!("secret         : {} (len {})", short(&c.secret), c.secret.len());
    println!("passphrase     : {} (len {})", short(&c.passphrase), c.passphrase.len());
    println!("wallet_address : {}", c.wallet_address);
    println!(
        "proxy_address  : {}",
        c.proxy_address.as_deref().unwrap_or("(none)")
    );
    println!(
        "signature_type : {}",
        c.signature_type.as_deref().unwrap_or("(auto-detect)")
    );
    println!(
        "private_key    : {}",
        c.private_key.as_deref().map(|s| short(s)).unwrap_or_else(|| "(none)".to_string())
    );
}

/// Read `[polymarket]` section from `~/.traderclaw/config.toml` line-by-line.
/// Avoids the `toml` crate which fails to parse the user's config (encrypted
/// values like `enc2:…`).
fn read_polymarket_config() -> Result<PolyCredentials> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let path: PathBuf = [home.as_str(), ".traderclaw", "config.toml"].iter().collect();
    println!("Reading config: {}", path.display());
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;

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
        if t.starts_with('[') {
            in_poly = t == "[polymarket]";
            continue;
        }
        if !in_poly { continue; }
        let Some(eq) = t.find('=') else { continue };
        let key = t[..eq].trim();
        let value = t[eq + 1..].trim().trim_matches('"').to_string();
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

    if api_key.is_empty() || secret.is_empty() || passphrase.is_empty() || wallet_address.is_empty() {
        anyhow::bail!("missing one of api_key/secret/passphrase/wallet_address in [polymarket]");
    }

    Ok(PolyCredentials {
        api_key,
        secret,
        passphrase,
        wallet_address,
        private_key,
        is_builder: false,
        proxy_address,
        signature_type,
    })
}
