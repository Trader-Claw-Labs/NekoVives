// prob.rhai — example P(Up at close) for the btc5m-feed Rhai hook.
//
// Receives `f` (a map of feature fields, see features.rs) and must return a
// float in (0, 1). This is the seam where you call the calibrated model: in
// production, replace the logistic below with a call into market-analyzer's
// model (e.g. via a registered Rhai fn `model_prob(features)` bound from Rust).
//
// Keep this transparent fallback for paper mode and as a circuit-breaker if the
// model service is unavailable.

let secs_left = f.secs_left;

// Urgency: distance matters more as time runs out (less future variance).
let urgency = 1.0 + 0.6 * max(0.0, ln(300.0 / (secs_left + 5.0)));

// Linear score (weights to be tuned against the cost-aware backtest).
let score =
      0.18 * f.dist_bps * urgency
    + 0.60 * f.flow_15s
    + 0.04 * f.mom_15s_bps
    + 0.40 * f.book_imbalance_l5;

// Distrust the signal when spot has drifted far from the resolving Chainlink
// price (basis risk): shrink the score toward 0 (=> p toward 0.5).
let basis_penalty = 1.0 / (1.0 + 0.05 * abs(f.basis_bps));
let adj = score * basis_penalty;

// Logistic.
let p = 1.0 / (1.0 + exp(-adj));

// Clamp away from the boundaries.
if p < 0.0001 { 0.0001 } else if p > 0.9999 { 0.9999 } else { p }
