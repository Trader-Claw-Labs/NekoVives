use k256::ecdsa::SigningKey;
use polymarket_trader::auth::{
    address_from_signing_key, setup_credentials,
};

#[tokio::main]
async fn main() {
    println!("============================================================");
    println!(" Polymarket L1 Auth — Generate API Credentials");
    println!("============================================================\n");

    let pk = prompt("Private Key (0x...)");
    if pk.is_empty() {
        println!("Error: private key is required.");
        std::process::exit(1);
    }

    let clean = pk.strip_prefix("0x").unwrap_or(&pk);
    let key_bytes = match hex::decode(clean) {
        Ok(b) => b,
        Err(e) => {
            println!("✗ Invalid hex: {e}");
            return;
        }
    };

    let signing_key = match SigningKey::from_slice(&key_bytes) {
        Ok(k) => k,
        Err(e) => {
            println!("✗ Invalid private key: {e}");
            return;
        }
    };

    let derived = address_from_signing_key(&signing_key);
    println!("Derived address: {derived}\n");

    println!("Sending L1 auth request to Polymarket...\n");

    match setup_credentials(&pk).await {
        Ok(creds) => {
            println!("✓ L1 auth SUCCESS!");
            println!("");
            println!("  api_key     : {}", creds.api_key);
            println!("  secret      : {}", creds.secret);
            println!("  passphrase  : {}", creds.passphrase);
            println!("  wallet      : {}", creds.wallet_address);
            println!("");
            println!("Copy these values to your config.toml under [polymarket]:");
            println!("");
            println!("[polymarket]");
            println!("api_key      = \"{}\"", creds.api_key);
            println!("secret       = \"{}\"", creds.secret);
            println!("passphrase   = \"{}\"", creds.passphrase);
            println!("wallet_address = \"{}\"", creds.wallet_address);
            println!("private_key  = \"{}\"", pk);
            println!("is_builder   = false");
        }
        Err(e) => {
            println!("✗ L1 auth FAILED: {e:#}");
            println!("");
            println!("Common causes:");
            println!("  1. Wallet not registered with Polymarket (needs deposit + KYC)");
            println!("  2. Wallet has no USDC.e balance on Polygon");
            println!("  3. Rate limiting — wait a minute and retry");
        }
    }

    println!();
}

fn prompt(label: &str) -> String {
    use std::io::Write;
    print!("{label}: ");
    let _ = std::io::stdout().flush();
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf).unwrap();
    buf.trim().to_string()
}
