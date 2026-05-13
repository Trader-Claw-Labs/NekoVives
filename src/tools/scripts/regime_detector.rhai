//! Regime Detector — classifies market state every candle.
//!
//! Stores the current regime so other strategies can read it via ctx.regime().
//! In live mode, run this as a cron job every 5 minutes.

fn on_candle(ctx) {
    let regime = ctx.regime();
    ctx.set("regime", regime);
    ctx.log("Regime detected: " + regime);
}
