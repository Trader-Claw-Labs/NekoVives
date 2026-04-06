use anyhow::{anyhow, Result};
use serde_json::{json, Value};

/// Call eth_call on a contract, returns the hex result string (e.g. "0x...")
pub async fn eth_call(rpc_url: &str, to: &str, data: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "eth_call",
        "params": [
            {"to": to, "data": data},
            "latest"
        ],
        "id": 1
    });

    let resp: Value = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = resp.get("error") {
        return Err(anyhow!("JSON-RPC error: {}", err));
    }

    resp["result"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("eth_call: missing result field in response: {}", resp))
}

/// Estimate gas for a transaction, returns gas units as u64
pub async fn estimate_gas(
    rpc_url: &str,
    from: &str,
    to: &str,
    data: &str,
    value: &str,
) -> Result<u64> {
    let client = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "eth_estimateGas",
        "params": [
            {
                "from": from,
                "to": to,
                "data": data,
                "value": value
            }
        ],
        "id": 1
    });

    let resp: Value = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = resp.get("error") {
        return Err(anyhow!("JSON-RPC error: {}", err));
    }

    let hex_gas = resp["result"]
        .as_str()
        .ok_or_else(|| anyhow!("estimate_gas: missing result field in response: {}", resp))?;

    let gas = u64::from_str_radix(hex_gas.trim_start_matches("0x"), 16)?;
    Ok(gas)
}

/// Send a signed raw transaction, returns the transaction hash
pub async fn send_raw_transaction(rpc_url: &str, raw_tx_hex: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "eth_sendRawTransaction",
        "params": [raw_tx_hex],
        "id": 1
    });

    let resp: Value = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = resp.get("error") {
        return Err(anyhow!("JSON-RPC error: {}", err));
    }

    resp["result"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("send_raw_transaction: missing result field in response: {}", resp))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that all public async functions exist with the expected signatures
    /// by referencing them in a way that triggers type inference without calling them.
    #[test]
    fn test_rpc_functions_compile() {
        // Shadowing as futures (never polled) is enough to verify the signatures compile.
        fn _check() {
            let _ = eth_call("", "", "");
            let _ = estimate_gas("", "", "", "", "");
            let _ = send_raw_transaction("", "");
        }
    }
}
