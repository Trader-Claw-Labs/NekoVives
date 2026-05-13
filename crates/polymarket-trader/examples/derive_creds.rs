// Derive existing Polymarket L2 credentials for the wallet whose private key
// is set in $POLY_PK env var. Used to recover when config has stale L2 creds
// after switching to a new wallet whose L2 was already created previously.
//
// Run:  POLY_PK=$(python3 -c "import tomllib;print(tomllib.load(open('$HOME/.traderclaw/config.toml','rb'))['polymarket']['private_key'])") cargo run -p polymarket-trader --example derive_creds

use polymarket_trader::auth::derive_api_key;
// is set in $POLY_PK env var.
//
// $POLY_FUNDER (optional) overrides the wallet_address used as POLY_ADDRESS.
// Use this when your Polymarket trading wallet is a Safe/proxy/smart-account
// distinct from the EOA derived from your private key. In that case the API
// key MUST be associated with the trading wallet (funder), otherwise orders
// fail with "the order signer address has to be the address of the API KEY".
//
// Run:  POLY_PK=... POLY_FUNDER=0x... cargo run -p polymarket-trader --example derive_creds

use polymarket_trader::auth::derive_api_key_for;

#[tokio::main]
async fn main() {
    let pk = std::env::var("POLY_PK").expect("set POLY_PK env var to wallet private key hex");
    match derive_api_key(&pk).await {
        Ok(c) => {
            println!("api_key={}", c.api_key);
            println!("secret={}", c.secret);
            println!("passphrase={}", c.passphrase);
            println!("wallet_address={}", c.wallet_address);
        }
        Err(e) => {
            eprintln!("derive_api_key failed: {e:#}");
            std::process::exit(1);
        }
    }
}
