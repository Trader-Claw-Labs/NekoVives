// Check which Polymarket proxy/safe address derives from the configured private key.
// Helps diagnose "maker address not allowed" errors when signature_type is misconfigured.
//
// Run:
//   POLY_PK=$(python3 -c "import tomllib;print(tomllib.load(open('$HOME/.traderclaw/config.toml','rb'))['polymarket']['private_key'])") \
//     cargo run -p polymarket-trader --example check_proxy --release

use polymarket_trader::auth::address_from_signing_key;
use k256::ecdsa::SigningKey;
use polymarket_client_sdk_v2::{derive_proxy_wallet, derive_safe_wallet};

const POLYGON: u64 = 137;

#[tokio::main]
async fn main() {
    let pk_hex = std::env::var("POLY_PK").expect("set POLY_PK env var");
    let key_bytes = hex::decode(pk_hex.strip_prefix("0x").unwrap_or(&pk_hex)).expect("invalid hex");
    let signing_key = SigningKey::from_slice(&key_bytes).expect("invalid pk");
    let eoa_str = address_from_signing_key(&signing_key);
    println!("EOA (from PK):       {eoa_str}");

    let eoa_addr: polymarket_client_sdk_v2::types::Address =
        eoa_str.parse().expect("parse eoa");

    let safe = derive_safe_wallet(eoa_addr, POLYGON);
    let proxy = derive_proxy_wallet(eoa_addr, POLYGON);
    println!("Derived Safe:        {safe:?}");
    println!("Derived EIP-1167 Proxy: {proxy:?}");

    let configured_proxy = std::env::var("POLY_PROXY").unwrap_or_default();
    if !configured_proxy.is_empty() {
        println!("\nConfigured proxy_address: {configured_proxy}");
        let configured = configured_proxy.to_lowercase();
        if let Some(s) = safe {
            if format!("{:#x}", s).to_lowercase() == configured {
                println!("→ MATCHES Gnosis Safe (signature_type=\"gnosis_safe\")");
            }
        }
        if let Some(p) = proxy {
            if format!("{:#x}", p).to_lowercase() == configured {
                println!("→ MATCHES EIP-1167 Proxy (signature_type=\"proxy\")");
            }
        }
    }
}
