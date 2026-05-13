// Create NEW Polymarket L2 credentials for an EOA that doesn't have any yet.
// Polymarket's POST /auth/api-key returns "Could not create" if creds already
// exist — in that case use derive_creds instead.
//
// Run:  POLY_PK=... cargo run -p polymarket-trader --example create_creds --release

use polymarket_trader::auth::setup_credentials;

#[tokio::main]
async fn main() {
    let pk = std::env::var("POLY_PK").expect("set POLY_PK env var");
    match setup_credentials(&pk, None).await {
        Ok(c) => {
            println!("api_key={}", c.api_key);
            println!("secret={}", c.secret);
            println!("passphrase={}", c.passphrase);
            println!("wallet_address={}", c.wallet_address);
        }
        Err(e) => {
            eprintln!("setup_credentials failed: {e:#}");
            std::process::exit(1);
        }
    }
}
