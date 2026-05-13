/// Risk-adjusted position sizing for copy trades.
#[derive(Debug, Clone)]
pub struct SizingEngine {
    /// Maximum fraction of capital per single trade (safety cap)
    pub max_single_trade_pct: f64,
    /// Liquidity floor: don't exceed this fraction of available depth
    pub liquidity_floor_factor: f64,
}

impl SizingEngine {
    pub fn new() -> Self {
        Self {
            max_single_trade_pct: 0.02,
            liquidity_floor_factor: 0.05,
        }
    }

    /// Compute copy size using the risk-adjusted formula.
    pub fn compute_size(
        &self,
        my_capital: f64,
        leader_position_pct: f64,
        wallet_score: f64,
        my_atr_target: f64,
        their_implied_atr: f64,
        available_liquidity: f64,
    ) -> f64 {
        // Base proportional sizing
        let base = my_capital * leader_position_pct;

        // Confidence scaling
        let confidence = wallet_score / 100.0;

        // Volatility adjustment (only scale down, never up)
        let vol_adj = (my_atr_target / their_implied_atr).min(1.0);

        // Liquidity floor
        let liq_floor = (available_liquidity * self.liquidity_floor_factor).min(base);
        let liquidity_factor = if base > 0.0 {
            liq_floor / base
        } else {
            1.0
        };

        let size = base * confidence * vol_adj * liquidity_factor;

        // Hard cap per trade
        let max_size = my_capital * self.max_single_trade_pct;
        size.min(max_size)
    }

    /// Venue-specific minimum notional floors (gas economics).
    pub fn venue_min_notional(venue: &str) -> f64 {
        match venue {
            "ethereum" => 1000.0,
            "arbitrum" | "base" | "optimism" => 200.0,
            "polygon" | "polymarket" => 100.0,
            "solana" => 50.0,
            "hyperliquid" => 100.0,
            _ => 100.0,
        }
    }
}
