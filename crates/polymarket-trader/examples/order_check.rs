use base64::Engine as _;
use polymarket_trader::auth::{create_l2_headers_with_strategy, PolyCredentials, SecretDecodeStrategy};
use polymarket_trader::eip712::{
    build_signed_market_order, build_signed_limit_order, OrderSide,
};
use k256::ecdsa::SigningKey;

#[tokio::main]
async fn main() {
    println!("============================================================");
    println!(" Polymarket Order Creation Diagnostic");
    println!("============================================================\n");

    let api_key = prompt("API Key");
    let secret = prompt("Secret");
    let passphrase = prompt("Passphrase");
    let wallet_address = prompt("Wallet Address (0x...)");
    let private_key = prompt("Private Key (0x...)");
    let token_id = prompt_opt("Token ID (opcional, Enter para dummy)").unwrap_or_else(|| "0x0".to_string());

    let creds = PolyCredentials {
        api_key: api_key.clone(),
        secret: secret.clone(),
        passphrase: passphrase.clone(),
        wallet_address: wallet_address.clone(),
        private_key: Some(private_key.clone()),
        is_builder: false,
        proxy_address: None,
    };

    // Verificar que el private_key deriva la wallet_address
    println!("\n────────────────────────────────────────────────────────────");
    println!("Verificación de claves");
    println!("────────────────────────────────────────────────────────────");

    let key_bytes = match hex::decode(private_key.strip_prefix("0x").unwrap_or(&private_key)) {
        Ok(b) => b,
        Err(e) => {
            println!("✗ private_key no es hex válido: {e}");
            return;
        }
    };
    let signing_key = match SigningKey::from_slice(&key_bytes) {
        Ok(k) => k,
        Err(e) => {
            println!("✗ private_key no es válido: {e}");
            return;
        }
    };

    let derived = polymarket_trader::auth::address_from_signing_key(&signing_key);
    println!("Wallet configurada : {wallet_address}");
    println!("Wallet derivada    : {derived}");
    if derived.to_lowercase() != wallet_address.to_lowercase() {
        println!("\n⚠ MISMATCH! El private_key NO corresponde a la wallet_address.");
        println!("  La firma EIP-712 será inválida y Polymarket rechazará la orden con 401.");
        println!("  Debes usar el private_key de la wallet {wallet_address}, o cambiar");
        println!("  wallet_address a {derived}.");
    } else {
        println!("✓ Private key y wallet coinciden.");
    }

    // Crear orden firmada
    println!("\n────────────────────────────────────────────────────────────");
    println!("Construyendo orden de mercado (FOK)");
    println!("────────────────────────────────────────────────────────────");

    let signed = match build_signed_market_order(
        &signing_key,
        &wallet_address,
        &token_id,
        OrderSide::Buy,
        1.0,      // amount USDC
        0.99,     // worst price
        None,
    ) {
        Ok(o) => o,
        Err(e) => {
            println!("✗ Error construyendo orden: {e}");
            return;
        }
    };

    // Construir body exactamente como lo hace ClobClient
    let salt_value: serde_json::Value = signed
        .salt
        .parse::<u64>()
        .map(|n| n.into())
        .unwrap_or_else(|_| {
            signed
                .salt
                .parse::<f64>()
                .map(|n| n.into())
                .unwrap_or_else(|_| signed.salt.clone().into())
        });

    let body = serde_json::json!({
        "order": {
            "salt": salt_value,
            "maker": signed.maker,
            "signer": signed.signer,
            "taker": signed.taker,
            "tokenId": signed.token_id,
            "makerAmount": signed.maker_amount,
            "takerAmount": signed.taker_amount,
            "expiration": signed.expiration,
            "nonce": signed.nonce,
            "feeRateBps": signed.fee_rate_bps,
            "side": if signed.side == 0 { "BUY" } else { "SELL" },
            "signatureType": signed.signature_type,
            "signature": signed.signature
        },
        "owner": wallet_address,
        "orderType": "FOK",
        "deferExec": false
    });
    let body_str = serde_json::to_string(&body).unwrap();

    println!("Body JSON:\n{}", serde_json::to_string_pretty(&body).unwrap());

    // Headers L2
    println!("\n────────────────────────────────────────────────────────────");
    println!("Headers L2 (HMAC)");
    println!("────────────────────────────────────────────────────────────");

    let headers = create_l2_headers_with_strategy(
        &creds, "POST", "/order", Some(&body_str), SecretDecodeStrategy::Base64
    );
    for (k, v) in &headers {
        println!("  {k}: {v}");
    }

    // Calcular HMAC manualmente para verificación
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let ts = headers.get("POLY_TIMESTAMP").unwrap();
    let msg = format!("{ts}POST/order{body_str}");
    let secret_bytes = base64::engine::general_purpose::STANDARD
        .decode(secret.replace('-', "+").replace('_', "/"))
        .unwrap_or_else(|_| secret.as_bytes().to_vec());
    let mut mac = Hmac::<Sha256>::new_from_slice(&secret_bytes).unwrap();
    mac.update(msg.as_bytes());
    let sig = base64::engine::general_purpose::STANDARD
        .encode(mac.finalize().into_bytes())
        .replace('+', "-")
        .replace('/', "_");
    println!("\n  HMAC msg : {msg}");
    println!("  Computed : {sig}");
    println!("  Header   : {}", headers.get("POLY_SIGNATURE").unwrap_or(&"?".to_string()));

    // Enviar request
    println!("\n────────────────────────────────────────────────────────────");
    println!("Enviando POST /order");
    println!("────────────────────────────────────────────────────────────");

    let client = reqwest::Client::new();
    let mut req = client
        .post("https://clob.polymarket.com/order")
        .header("Content-Type", "application/json")
        .body(body_str.clone());
    for (k, v) in &headers {
        req = req.header(k.as_str(), v.as_str());
    }

    match req.timeout(std::time::Duration::from_secs(15)).send().await {
        Ok(r) => {
            let status = r.status();
            let text = r.text().await.unwrap_or_default();
            println!("HTTP {status}");
            println!("Body: {text}");
            if status == 401 {
                println!("\n⚠ 401 Unauthorized — causas probables (en orden):");
                println!("  1. Mismatch private_key/wallet_address (firma EIP-712 inválida)");
                println!("  2. api_key/secret/passphrase no pertenecen a wallet_address");
                println!("  3. Credenciales revocadas");
            } else if status == 400 {
                println!("\n✓ Auth pasó (orden inválida por otro motivo)");
            }
        }
        Err(e) => println!("Network error: {e}"),
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

fn prompt_opt(label: &str) -> Option<String> {
    let s = prompt(label);
    if s.is_empty() { None } else { Some(s) }
}
