# nekovives-btc5m

Scaffolding for Polymarket BTC Up/Down 5-minute markets in NekoVives.
See **CLAUDE.md** for architecture, NekoVives conventions, Chainlink data sourcing,
the stub wiring, and the validation bar. The edge is latency/execution; the model
only decides when the stale book is mispriced enough to clear the crypto taker fee.

- `crates/btc5m-feed/` — Rust live plane (dual WS, features, fee-aware gate, Rhai hook)
- `ml/` — Python research plane (label vs Chainlink, calibrated training, cost-aware backtest)
