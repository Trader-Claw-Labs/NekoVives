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

/// Returns `true` if the given kind string is a recognised engine kind.
pub fn is_known(kind: &str) -> bool {
    matches!(
        kind,
        RHAI_CANDLE | ARB_BINARY | MINTING_MM | ROTATION_COMPOUNDER | FAIR_VALUE | FV_MOMENTUM | ARB_HEDGE
    )
}

/// Returns the default engine kind used when `RunnerConfig.kind` is `None`.
pub fn default_kind() -> &'static str {
    RHAI_CANDLE
}
