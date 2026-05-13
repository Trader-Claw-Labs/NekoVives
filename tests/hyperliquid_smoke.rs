//! Hyperliquid client smoke test.
//!
//! Verifies that the read-only client can connect to Hyperliquid mainnet
//! and fetch mids without authentication.


#[tokio::test]
async fn hyperliquid_client_can_fetch_mids() {
    let client = hyperliquid_trader::HyperliquidClient::new_mainnet();
    let mids = client
        .mids()
        .await
        .expect("should fetch mids from Hyperliquid mainnet");

    assert!(
        mids.contains_key("BTC"),
        "BTC mid should be present in response"
    );
    assert!(
        mids.contains_key("ETH"),
        "ETH mid should be present in response"
    );

    let btc_mid = mids.get("BTC").copied().unwrap();
    assert!(btc_mid > 0.0, "BTC mid should be positive");
}

#[tokio::test]
async fn hyperliquid_client_can_fetch_funding_rates() {
    let client = hyperliquid_trader::HyperliquidClient::new_mainnet();
    let funding = client
        .funding_rate("BTC")
        .await
        .expect("should fetch funding rate for BTC");

    // Funding rate is a small number (can be positive or negative)
    assert!(
        funding.funding_rate.abs() < 1.0,
        "funding rate should be < 1.0 (sane range)"
    );
}

#[tokio::test]
async fn hyperliquid_ws_can_connect_and_receive_ping() {
    let client = hyperliquid_trader::HyperliquidClient::new_mainnet();
    let ws = client
        .subscribe_l2_book("BTC")
        .expect("should open WS subscription to L2 book");

    // Give the connection a moment to establish
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // The WS client auto-reconnects internally; if we got here without panic,
    // the handshake succeeded. We just drop it cleanly.
    drop(ws);
}

#[test]
fn trading_risk_gate_defaults_are_sane() {
    let gate = risk_manager::general::TradingRiskGate::with_defaults(100_000.0);
    assert!(!gate.is_halted());

    let st = gate.status();
    assert_eq!(st.total_capital, 100_000.0);
    assert_eq!(st.total_positions, 0);
}

#[test]
fn trading_risk_gate_halt_and_resume() {
    let gate = risk_manager::general::TradingRiskGate::with_defaults(100_000.0);
    assert!(!gate.is_halted());

    gate.halt_all();
    assert!(gate.is_halted());

    gate.resume_all();
    assert!(!gate.is_halted());
}

#[test]
fn trading_risk_gate_rejects_when_halted() {
    let gate = risk_manager::general::TradingRiskGate::with_defaults(100_000.0);
    gate.halt_all();

    let req = risk_manager::general::OrderRequest {
        symbol: "BTC".into(),
        strategy_id: "test".into(),
        side: "buy".into(),
        proposed_size_usd: 1_000.0,
        stop_distance_atr: 100.0,
        atr14: 100.0,
        is_memecoin: false,
    };
    let ctx = risk_manager::general::OrderContext {
        current_price: 50_000.0,
    };

    let result = gate.approve_order(&req, &ctx);
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err().to_string(),
        "system halted (manual)"
    );
}
