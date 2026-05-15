use k256::ecdsa::SigningKey;
use sha3::{Digest, Keccak256};

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    hasher.finalize().into()
}

fn address_from_signing_key(signing_key: &SigningKey) -> String {
    let verifying_key = signing_key.verifying_key();
    let point = verifying_key.to_encoded_point(false);
    let bytes = point.as_bytes();
    let pub_bytes = &bytes[1..];
    let hash = keccak256(pub_bytes);
    let addr_bytes = &hash[12..];
    format!("0x{}", hex::encode(addr_bytes))
}

fn main() {
    println!("============================================================");
    println!(" Verify Private Key → Wallet Address");
    println!("============================================================\n");

    let pk = prompt("Private Key (0x...)");
    let expected = prompt("Expected Wallet Address (0x...)");

    let clean = pk.strip_prefix("0x").unwrap_or(&pk);
    let bytes = match hex::decode(clean) {
        Ok(b) => b,
        Err(e) => {
            println!("\n✗ Invalid hex: {e}");
            return;
        }
    };

    let signing_key = match SigningKey::from_slice(&bytes) {
        Ok(k) => k,
        Err(e) => {
            println!("\n✗ Invalid private key: {e}");
            return;
        }
    };

    let derived = address_from_signing_key(&signing_key);
    let match_ = derived.to_lowercase() == expected.to_lowercase();

    println!("\n  Expected : {expected}");
    println!("  Derived  : {derived}");
    println!();
    if match_ {
        println!("  ✓ MATCH — this private key belongs to the expected wallet.");
    } else {
        println!("  ✗ MISMATCH — this private key does NOT belong to the expected wallet.");
        println!("    It belongs to: {derived}");
        println!("    If you need credentials for {expected}, use the private key of THAT wallet.");
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
