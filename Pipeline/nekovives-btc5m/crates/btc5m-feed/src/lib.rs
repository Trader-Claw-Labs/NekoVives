//! `btc5m-feed` — dual-WebSocket market state + features + fee-aware probability
//! gate for Polymarket BTC Up/Down 5-minute markets, built for NekoVives.
//!
//! Data plane:
//!   Binance aggTrade+depth ─┐
//!                           ├─► MarketState (watch) ─► features ─► P(Up) ─► gate ─► order
//!   Polymarket RTDS (CL) ───┘                                   (Rhai/model)
//!
//! The probability source is pluggable: a transparent logistic baseline, or a
//! Rhai script that calls into the trained+calibrated model exposed from
//! `market-analyzer`. The gate enforces the same hard limits as `risk-manager`.

pub mod binance;
pub mod chainlink_rtds;
pub mod features;
pub mod prob_engine;
pub mod state;
pub mod types;

pub use features::Features;
pub use prob_engine::{gate, logistic_baseline, Decision, GateParams, LogisticWeights};
pub use state::{MarketState, Snapshot};
pub use types::{MarketWindow, PmBook, Side};

/// A probability source. Implement this for the calibrated model (FFI/ONNX/etc.)
/// or use the built-ins.
pub trait ProbModel: Send + Sync {
    /// Return calibrated P(Up at close) in (0, 1).
    fn p_up(&self, f: &Features) -> f64;
}

/// Built-in logistic baseline as a `ProbModel`.
pub struct Baseline(pub LogisticWeights);
impl ProbModel for Baseline {
    fn p_up(&self, f: &Features) -> f64 {
        logistic_baseline(f, &self.0)
    }
}

#[cfg(feature = "rhai")]
pub mod rhai_api {
    //! Expose features to a Rhai script and read back P(Up). Mirrors how other
    //! NekoVives strategies are scripted. The script receives a `#{}` map of the
    //! feature fields and must return a float in (0, 1).
    use super::Features;
    use rhai::{Dynamic, Engine, Map, Scope, AST};

    pub struct RhaiModel {
        engine: Engine,
        ast: AST,
    }

    impl RhaiModel {
        pub fn from_script(src: &str) -> Result<Self, Box<rhai::EvalAltResult>> {
            let engine = Engine::new();
            let ast = engine.compile(src).map_err(|e| {
                Box::new(rhai::EvalAltResult::ErrorSystemError(e.to_string(), Dynamic::UNIT.into()))
            })?;
            Ok(Self { engine, ast })
        }

        pub fn p_up(&self, f: &Features) -> f64 {
            let mut m = Map::new();
            m.insert("dist_bps".into(), (f.dist_bps).into());
            m.insert("dist_bps_spot".into(), (f.dist_bps_spot).into());
            m.insert("basis_bps".into(), (f.basis_bps).into());
            m.insert("book_imbalance".into(), (f.book_imbalance).into());
            m.insert("book_imbalance_l5".into(), (f.book_imbalance_l5).into());
            m.insert("flow_15s".into(), (f.flow_15s).into());
            m.insert("flow_5s".into(), (f.flow_5s).into());
            m.insert("mom_15s_bps".into(), (f.mom_15s_bps).into());
            m.insert("rv_60s_bps".into(), (f.rv_60s_bps).into());
            m.insert("secs_left".into(), (f.secs_left).into());

            let mut scope = Scope::new();
            scope.push("f", m);
            self.engine
                .eval_ast_with_scope::<f64>(&mut scope, &self.ast)
                .unwrap_or(0.5)
                .clamp(0.0001, 0.9999)
        }
    }

    impl super::ProbModel for RhaiModel {
        fn p_up(&self, f: &Features) -> f64 {
            RhaiModel::p_up(self, f)
        }
    }
}

/// Wire up the data plane and run the decision loop. This is a skeleton:
/// `place_order` and window discovery are stubbed for you to connect to your
/// existing `polymarket-trader` client and `risk-manager`.
pub async fn run<M: ProbModel + 'static>(model: M, gate_params: GateParams) {
    let state = MarketState::new();

    tokio::spawn(binance::run(state.clone()));
    tokio::spawn(chainlink_rtds::run(state.clone()));
    // TODO: spawn your polymarket-trader CLOB book stream -> state.on_pm_up/down
    // TODO: spawn window discovery -> state.set_window / state.set_price_to_beat

    let mut rx = state.rx.clone();
    loop {
        if rx.changed().await.is_err() {
            break;
        }
        let snap = state.snapshot();
        let trades = state.recent_trades(60_000);
        let Some(f) = features::compute(&snap, &trades) else {
            continue;
        };
        let p_up = model.p_up(&f);
        if let Some(decision) = gate(&f, p_up, &snap.pm_up, &snap.pm_down, &gate_params) {
            // place_order(decision).await;  // hand off to polymarket-trader (IOC)
            tracing::info!(?decision, p_up, dist_bps = f.dist_bps, "signal");
        }
    }
}
