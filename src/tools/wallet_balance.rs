use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

/// Wallet entry loaded from wallets.json (public fields only — no key material).
#[derive(Debug, Clone)]
struct WalletInfo {
    chain: String,
    address: String,
    label: String,
}

fn load_wallets(config_path: &std::path::Path) -> Vec<WalletInfo> {
    #[derive(Deserialize)]
    struct DiskEntry {
        chain: String,
        address: String,
        label: String,
    }

    let wallets_path = config_path
        .parent()
        .unwrap_or(config_path)
        .join("wallets.json");

    let data = match std::fs::read_to_string(&wallets_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let entries: Vec<DiskEntry> = serde_json::from_str(&data).unwrap_or_default();
    entries
        .into_iter()
        .map(|e| WalletInfo { chain: e.chain, address: e.address, label: e.label })
        .collect()
}

// ── Solana balance helpers ───────────────────────────────────────────

async fn solana_balance(address: &str, rpc: Option<&str>) -> Option<f64> {
    let trader = solana_trader::SolanaTrader::new(rpc);
    trader.get_sol_balance(address).await.ok()
}

async fn solana_token_balances(address: &str, rpc: Option<&str>) -> Vec<solana_trader::TokenBalance> {
    let trader = solana_trader::SolanaTrader::new(rpc);
    trader.get_token_balances(address).await.unwrap_or_default()
}

// ── EVM multi-chain balance helpers ─────────────────────────────────

/// Static chain metadata: (id, display_name, native_symbol, default_rpc, block_explorer_tx_url_prefix)
struct ChainInfo {
    id: &'static str,
    name: &'static str,
    symbol: &'static str,
    default_rpc: &'static str,
    explorer: &'static str, // base URL for address pages
}

const EVM_CHAINS: &[ChainInfo] = &[
    ChainInfo { id: "ethereum", name: "Ethereum",  symbol: "ETH",   default_rpc: "https://cloudflare-eth.com",                   explorer: "https://etherscan.io/address/" },
    ChainInfo { id: "arbitrum", name: "Arbitrum",  symbol: "ETH",   default_rpc: "https://arb1.arbitrum.io/rpc",                  explorer: "https://arbiscan.io/address/" },
    ChainInfo { id: "optimism", name: "Optimism",  symbol: "ETH",   default_rpc: "https://mainnet.optimism.io",                   explorer: "https://optimistic.etherscan.io/address/" },
    ChainInfo { id: "base",     name: "Base",      symbol: "ETH",   default_rpc: "https://mainnet.base.org",                      explorer: "https://basescan.org/address/" },
    ChainInfo { id: "bnb",      name: "BNB Chain", symbol: "BNB",   default_rpc: "https://bsc-dataseed.bnbchain.org",             explorer: "https://bscscan.com/address/" },
    ChainInfo { id: "polygon",  name: "Polygon",   symbol: "MATIC", default_rpc: "https://polygon-rpc.com",                       explorer: "https://polygonscan.com/address/" },
    ChainInfo { id: "unichain", name: "Unichain",  symbol: "ETH",   default_rpc: "https://mainnet.unichain.org",                  explorer: "https://uniscan.xyz/address/" },
    ChainInfo { id: "etc",      name: "ETC",       symbol: "ETC",   default_rpc: "https://etc.etcdesktop.com",                    explorer: "https://blockscout.com/etc/mainnet/address/" },
];

async fn evm_balance_on_rpc(address: &str, rpc_url: &str) -> Option<f64> {
    #[derive(Deserialize)]
    struct EthResult { result: Option<String> }

    let resp: EthResult = reqwest::Client::new()
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "eth_getBalance",
            "params": [address, "latest"]
        }))
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    let hex = resp.result?;
    let wei = u128::from_str_radix(hex.trim_start_matches("0x"), 16).ok()?;
    Some(wei as f64 / 1e18)
}

async fn evm_balance(address: &str) -> Option<f64> {
    evm_balance_on_rpc(address, "https://cloudflare-eth.com").await
}

/// Query all EVM chains in parallel and return non-zero balances with explorer links.
pub async fn evm_multichain_balances(
    address: &str,
    custom_rpcs: &crate::config::schema::ChainsRpcConfig,
) -> Vec<(String, f64, String, String)> {
    let address = address.to_string();
    let custom_rpcs = custom_rpcs.clone();

    let futures: Vec<_> = EVM_CHAINS
        .iter()
        .map(|chain| {
            let addr = address.clone();
            let rpc = match chain.id {
                "ethereum" => custom_rpcs.ethereum.clone().unwrap_or_else(|| chain.default_rpc.to_string()),
                "arbitrum" => custom_rpcs.arbitrum.clone().unwrap_or_else(|| chain.default_rpc.to_string()),
                "optimism" => custom_rpcs.optimism.clone().unwrap_or_else(|| chain.default_rpc.to_string()),
                "base"     => custom_rpcs.base.clone().unwrap_or_else(|| chain.default_rpc.to_string()),
                "bnb"      => custom_rpcs.bnb.clone().unwrap_or_else(|| chain.default_rpc.to_string()),
                "polygon"  => custom_rpcs.polygon.clone().unwrap_or_else(|| chain.default_rpc.to_string()),
                "unichain" => custom_rpcs.unichain.clone().unwrap_or_else(|| chain.default_rpc.to_string()),
                "etc"      => custom_rpcs.etc.clone().unwrap_or_else(|| chain.default_rpc.to_string()),
                _          => chain.default_rpc.to_string(),
            };
            let name = chain.name.to_string();
            let symbol = chain.symbol.to_string();
            let explorer = format!("{}{}", chain.explorer, addr);
            async move {
                let balance = evm_balance_on_rpc(&addr, &rpc).await.unwrap_or(0.0);
                (name, balance, symbol, explorer)
            }
        })
        .collect();

    let results = futures_util::future::join_all(futures).await;
    // Return chains with any non-dust balance
    results.into_iter().filter(|(_, bal, _, _)| *bal > 1e-12).collect()
}

// ── Tool ─────────────────────────────────────────────────────────────

pub struct WalletBalanceTool {
    config_path: PathBuf,
    chains_rpc: Arc<crate::config::schema::ChainsRpcConfig>,
}

impl WalletBalanceTool {
    pub fn new(config_path: PathBuf) -> Self {
        Self {
            config_path,
            chains_rpc: Arc::new(crate::config::schema::ChainsRpcConfig::default()),
        }
    }

    pub fn with_chains_rpc(
        config_path: PathBuf,
        chains_rpc: crate::config::schema::ChainsRpcConfig,
    ) -> Self {
        Self { config_path, chains_rpc: Arc::new(chains_rpc) }
    }
}

#[async_trait]
impl Tool for WalletBalanceTool {
    fn name(&self) -> &str {
        "wallet_balance"
    }

    fn description(&self) -> &str {
        "List all registered wallets and fetch their live on-chain balances \
        (SOL for Solana wallets, ETH for EVM wallets). \
        Use this before executing a trade to confirm the wallet exists and has sufficient funds. \
        Optionally filter by chain or address."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "chain": {
                    "type": "string",
                    "description": "Filter to a specific chain: 'solana', 'evm', or 'ton'. Leave empty for all."
                },
                "address": {
                    "type": "string",
                    "description": "Filter to a specific wallet address. Leave empty for all."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let chain_filter = args.get("chain").and_then(|v| v.as_str()).map(|s| s.to_lowercase());
        let addr_filter = args.get("address").and_then(|v| v.as_str()).map(str::to_lowercase);

        let wallets = load_wallets(&self.config_path);

        if wallets.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: "No wallets registered. Create one on the /wallets page or ask me to create one.".into(),
                error: None,
            });
        }

        let filtered: Vec<&WalletInfo> = wallets
            .iter()
            .filter(|w| {
                let chain_ok = chain_filter.as_deref().map_or(true, |c| w.chain.to_lowercase() == c);
                let addr_ok = addr_filter.as_deref().map_or(true, |a| w.address.to_lowercase().contains(a));
                chain_ok && addr_ok
            })
            .collect();

        if filtered.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: format!(
                    "No wallets match the filter (chain={:?}, address={:?}). Registered chains: {}",
                    chain_filter,
                    addr_filter,
                    wallets.iter().map(|w| w.chain.as_str()).collect::<std::collections::HashSet<_>>()
                        .into_iter().collect::<Vec<_>>().join(", ")
                ),
                error: None,
            });
        }

        let mut lines = vec![
            format!("{:<10} {:<12} {:<46} {:<12}", "LABEL", "CHAIN", "ADDRESS", "BALANCE"),
            "─".repeat(82),
        ];

        for w in &filtered {
            match w.chain.to_lowercase().as_str() {
                "solana" => {
                    let sol_rpc = self.chains_rpc.solana.as_deref();
                    let sol_bal = solana_balance(&w.address, sol_rpc).await
                        .map(|b| format!("{:.6} SOL", b))
                        .unwrap_or_else(|| "RPC error".into());
                    lines.push(format!(
                        "{:<10} {:<12} {:<46} {}",
                        w.label, w.chain, w.address, sol_bal
                    ));
                    // SPL token balances
                    let tokens = solana_token_balances(&w.address, sol_rpc).await;
                    for tok in &tokens {
                        lines.push(format!(
                            "{:<10} {:<12} {:<46} {:.6} {}",
                            "", "spl", tok.mint, tok.amount, tok.symbol
                        ));
                    }
                }
                "evm" => {
                    let multi = evm_multichain_balances(&w.address, &*self.chains_rpc).await;
                    if multi.is_empty() {
                        lines.push(format!(
                            "{:<10} {:<12} {:<46} 0 (no balance on any chain)",
                            w.label, w.chain, w.address
                        ));
                    } else {
                        let mut first = true;
                        for (chain_name, bal, symbol, explorer) in &multi {
                            let label_col = if first { w.label.as_str() } else { "" };
                            lines.push(format!(
                                "{:<10} {:<12} {:.8} {} — {}",
                                label_col, chain_name, bal, symbol, explorer
                            ));
                            first = false;
                        }
                    }
                }
                "ton" => {
                    lines.push(format!(
                        "{:<10} {:<12} {:<46} (TON balance — coming soon)",
                        w.label, w.chain, w.address
                    ));
                }
                _ => {
                    lines.push(format!(
                        "{:<10} {:<12} {:<46} unknown chain",
                        w.label, w.chain, w.address
                    ));
                }
            }
        }

        Ok(ToolResult {
            success: true,
            output: lines.join("\n"),
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn empty_wallets_file() {
        let tmp = TempDir::new().unwrap();
        let tool = WalletBalanceTool::new(tmp.path().join("config.toml"));
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("No wallets"));
    }

    #[tokio::test]
    async fn wallet_list_from_file() {
        let tmp = TempDir::new().unwrap();
        let wallets = serde_json::json!([{
            "chain": "solana",
            "address": "So1anaFakePub1icKeyxxxxxxxxxxxxxxxxxxxxxxxx",
            "label": "main",
            "encrypted_key_b64": "dGVzdA=="
        }]);
        std::fs::write(tmp.path().join("wallets.json"), wallets.to_string()).unwrap();
        let tool = WalletBalanceTool::new(tmp.path().join("config.toml"));
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.success);
        // Balance will be RPC error since fake address, but wallet should appear
        assert!(result.output.contains("main") || result.output.contains("solana"));
    }

    #[test]
    fn chain_filter() {
        let ws = vec![
            WalletInfo { chain: "solana".into(), address: "A".into(), label: "a".into() },
            WalletInfo { chain: "evm".into(), address: "B".into(), label: "b".into() },
        ];
        let filtered: Vec<_> = ws.iter().filter(|w| w.chain == "solana").collect();
        assert_eq!(filtered.len(), 1);
    }
}
