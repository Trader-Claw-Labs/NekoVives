use polymarket_trader::auth::{
    create_l2_headers_with_strategy, eip712_digest, sign_eip712, setup_credentials,
    address_from_signing_key, PolyCredentials, SecretDecodeStrategy,
};
use std::collections::HashMap;

#[tokio::main]
async fn main() {
    println!("============================================================");
    println!(" Polymarket CLOB Auth Diagnostic");
    println!("============================================================\n");

    // ── Leer credenciales ───────────────────────────────────────────────
    println!("Ingresa tus credenciales actuales (de ~/.config/trader-claw/config.toml):\n");

    let api_key = prompt("API Key");
    let secret = prompt("Secret");
    let passphrase = prompt("Passphrase");
    let wallet_address = prompt("Wallet Address (0x...)");
    let private_key = prompt_opt("Private Key (0x...) para probar L1 auth [opcional]");

    if api_key.is_empty() || secret.is_empty() || passphrase.is_empty() || wallet_address.is_empty() {
        println!("\nError: api_key, secret, passphrase y wallet_address son obligatorios.");
        std::process::exit(1);
    }

    let creds = PolyCredentials {
        api_key: api_key.clone(),
        secret: secret.clone(),
        passphrase: passphrase.clone(),
        wallet_address: wallet_address.clone(),
        private_key: None,
        is_builder: false,
        proxy_address: None,
    };

    // ── Paso 1: Conectividad pública ────────────────────────────────────
    println!("\n────────────────────────────────────────────────────────────");
    println!("Paso 1: Conectividad pública");
    println!("────────────────────────────────────────────────────────────");

    let client = reqwest::Client::builder()
        .user_agent("trader-claw-auth-check/0.1")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("build client");

    match client.get("https://clob.polymarket.com/markets?limit=1").send().await {
        Ok(r) => println!("GET /markets?limit=1 → HTTP {}", r.status()),
        Err(e) => {
            println!("✗ Error de conectividad: {e}");
            return;
        }
    }

    // ── Paso 2: Probar L2 auth con los 3 encodings ─────────────────────
    println!("\n────────────────────────────────────────────────────────────");
    println!("Paso 2: L2 Auth (api_key / secret / passphrase)");
    println!("────────────────────────────────────────────────────────────");
    println!("api_key : {}…{} (len {})", &api_key[..api_key.len().min(8)], &api_key[api_key.len().saturating_sub(4)..], api_key.len());
    println!("secret  : {}…{} (len {})", &secret[..secret.len().min(8)], &secret[secret.len().saturating_sub(4)..], secret.len());
    println!("pass    : {}…{} (len {})", &passphrase[..passphrase.len().min(8)], &passphrase[passphrase.len().saturating_sub(4)..], passphrase.len());
    println!("wallet  : {}\n", wallet_address);

    let strategies: [(&str, SecretDecodeStrategy); 3] = [
        ("Base64", SecretDecodeStrategy::Base64),
        ("Raw", SecretDecodeStrategy::Raw),
        ("Hex", SecretDecodeStrategy::Hex),
    ];

    let mut any_ok = false;

    for (name, strat) in &strategies {
        // GET /auth/api-keys
        let headers = create_l2_headers_with_strategy(&creds, "GET", "/auth/api-keys", None, *strat);
        let (status, body) = probe(&client, "GET", "/auth/api-keys", headers, None).await;

        print!("  [{name}] GET /auth/api-keys → HTTP {status}");

        if status == 401 || status == 403 {
            // Fallback: POST /order dummy
            let dummy = r#"{"order":{"tokenID":"0","price":"0.5","size":"1","side":"BUY","type":"GTC"},"owner":""}"#;
            let h2 = create_l2_headers_with_strategy(&creds, "POST", "/order", Some(dummy), *strat);
            let (s2, b2) = probe(&client, "POST", "/order", h2, Some(dummy)).await;
            print!(" | POST /order → HTTP {s2}");

            if s2 != 401 && s2 != 403 && s2 != 0 {
                println!("\n         ✓ Auth PASSED (order inválido, no auth)");
                println!("         Response: {}", &b2[..b2.len().min(120)]);
                any_ok = true;
                continue;
            }
        }

        if (200..300).contains(&status) {
            println!("\n         ✓ Auth PASSED");
            println!("         Response: {}", &body[..body.len().min(120)]);
            any_ok = true;
        } else if status == 0 {
            println!("\n         ✗ Network error: {body}");
        } else {
            println!("\n         ✗ Auth FAILED — Response: {}", &body[..body.len().min(120)]);
        }
    }

    if !any_ok {
        println!("\n  ⚠ Todos los encodings fallaron con 401/403.");
        println!("  Eso significa que el trío NO pertenece a la wallet {}.", wallet_address);
        println!("  Posibles causas:");
        println!("    1. La wallet_address no coincide con la que generó estas credenciales.");
        println!("    2. Las credenciales fueron revocadas/regeneradas en Polymarket.");
        println!("    3. El 'secret' o 'passphrase' se copiaron mal (¿son idénticos?).");
    }

    // ── Paso 3: L1 auth (regenerar credenciales) ───────────────────────
    if !any_ok {
        if let Some(pk) = private_key.filter(|s| !s.is_empty()) {
            println!("\n────────────────────────────────────────────────────────────");
            println!("Paso 3: L1 Auth (EIP-712) — regenerar credenciales");
            println!("────────────────────────────────────────────────────────────");

            // Verificar que el private_key deriva la wallet_address
            match derive_address(&pk) {
                Ok(derived) => {
                    println!("Private key deriva: {derived}");
                    if derived.to_lowercase() != wallet_address.to_lowercase() {
                        println!("\n  ⚠ MISMATCH: la wallet configurada es {wallet_address},");
                        println!("     pero el private_key deriva {derived}.");
                        println!("     Debes usar la wallet que corresponde al private_key.");
                    }
                }
                Err(e) => println!("  ✗ Error derivando dirección: {e}"),
            }

            match setup_credentials(&pk).await {
                Ok(new_creds) => {
                    println!("\n  ✓ L1 auth SUCCESS!");
                    println!("  api_key     : {}", new_creds.api_key);
                    println!("  secret      : {}", new_creds.secret);
                    println!("  passphrase  : {}", new_creds.passphrase);
                    println!("  wallet      : {}", new_creds.wallet_address);
                    println!("\n  Copia estos valores a tu config y prueba de nuevo.");
                }
                Err(e) => {
                    println!("\n  ✗ L1 auth FAILED: {e:#}");
                    println!("     El private_key no pudo autenticar con Polymarket.");
                    println!("     Verifica que sea la private key REAL de tu wallet (no Builder Code).");
                }
            }
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

fn prompt_opt(label: &str) -> Option<String> {
    let s = prompt(label);
    if s.is_empty() { None } else { Some(s) }
}

async fn probe(
    client: &reqwest::Client,
    method: &str,
    path: &str,
    headers: HashMap<String, String>,
    body: Option<&str>,
) -> (u16, String) {
    let url = format!("https://clob.polymarket.com{path}");
    let mut req = if method == "GET" {
        client.get(&url)
    } else {
        client.post(&url).header("Content-Type", "application/json")
    };
    for (k, v) in &headers {
        req = req.header(k, v);
    }
    if let Some(b) = body {
        req = req.body(b.to_string());
    }
    match req.timeout(std::time::Duration::from_secs(12)).send().await {
        Ok(r) => {
            let s = r.status().as_u16();
            let b = r.text().await.unwrap_or_default();
            (s, b)
        }
        Err(e) => (0, format!("network error: {e}")),
    }
}

fn derive_address(pk_hex: &str) -> anyhow::Result<String> {
    use k256::ecdsa::SigningKey;
    let clean = pk_hex.strip_prefix("0x").unwrap_or(pk_hex);
    let bytes = hex::decode(clean)?;
    let signing_key = SigningKey::from_slice(&bytes)?;
    Ok(address_from_signing_key(&signing_key))
}
