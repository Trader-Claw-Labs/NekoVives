//! Known engine `kind` identifiers.
//!
//! Each constant matches what users write in `RunnerConfig.kind`.

/// Legacy Rhai candle-based engine — default when `kind` is not set.
pub const RHAI_CANDLE: &str = "rhai_candle";

/// Binary arb engine (ARB-01 / ARB-02, IA-04).
pub const ARB_BINARY: &str = "arb_binary";

/// Minting + Market Making engine (MINT-01/04, Group 2).
pub const MINTING_MM: &str = "minting_mm";

/// Rotation compounder meta-engine (HYB-05).
pub const ROTATION_COMPOUNDER: &str = "rotation_compounder";

/// Fair-value probability engine (IA-03, TRADE-04).
pub const FAIR_VALUE: &str = "fair_value";

/// Fair-value + Momentum AND-gate engine (HYB-03).
pub const FV_MOMENTUM: &str = "fv_momentum";

/// Arb-hedge overlay engine (HYB-02): synthetic arb + hedge overlay.
pub const ARB_HEDGE: &str = "arb_hedge";

/// Rhai tick-based engine — runs `on_tick(ctx)` at 1 Hz against live Polymarket CLOB.
pub const RHAI_TICK: &str = "rhai_tick";

/// Liquidity-rewards maker engine — keeps a two-sided resting quote alive on a slow
/// reward market, re-posting the side that fills and re-centering on mid drift.
pub const REWARDS_MAKER: &str = "rewards_maker";

/// Autonomous liquidity-rewards orchestrator — scans the reward-market pool every poll,
/// auto-selects the top-N safe markets, quotes both sides on each, closes + drops any
/// market that turns toxic, and rotates capital into fresh markets. No fixed condition_id.
pub const REWARDS_ORCHESTRATOR: &str = "rewards_orchestrator";

/// Returns `true` if the given kind string is a recognised engine kind.
pub fn is_known(kind: &str) -> bool {
    matches!(
        kind,
        RHAI_CANDLE | ARB_BINARY | MINTING_MM | ROTATION_COMPOUNDER | FAIR_VALUE | FV_MOMENTUM | ARB_HEDGE | RHAI_TICK | REWARDS_MAKER | REWARDS_ORCHESTRATOR
    )
}

/// Returns the default engine kind used when `RunnerConfig.kind` is `None`.
pub fn default_kind() -> &'static str {
    RHAI_CANDLE
}
