//! Shared, lock-light market state.
//!
//! Producers (Binance, Chainlink RTDS, Polymarket CLOB tasks) push updates;
//! consumers (feature/probability/strategy loop) read a consistent snapshot via
//! a `tokio::sync::watch` channel. We keep a short rolling history of trades and
//! Chainlink ticks so the feature layer can compute OFI, momentum and realized
//! vol without re-deriving anything.

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::watch;

use crate::types::{BookTop, MarketWindow, PmBook, Trade};

/// How much trade/tick history to retain (by time). 90s comfortably covers a
/// 60s feature window plus slack.
const HISTORY_MS: i64 = 90_000;

/// Immutable snapshot handed to the strategy loop each tick.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub now_ms: i64,
    /// Latest aggregate spot price from the lead venue (e.g. Binance perp/spot).
    pub spot: f64,
    /// Latest Chainlink reference price (the value the market actually resolves
    /// against). Use THIS, not `spot`, for distance-to-beat near the boundary.
    pub chainlink: f64,
    pub book: BookTop,
    pub pm_up: PmBook,
    pub pm_down: PmBook,
    pub window: Option<MarketWindow>,
}

#[derive(Default)]
struct Inner {
    snap: Snapshot,
    trades: VecDeque<Trade>,
    /// (ts_ms, price) Chainlink reference ticks for momentum/vol on the resolving
    /// series itself.
    chainlink_hist: VecDeque<(i64, f64)>,
}

#[derive(Clone)]
pub struct MarketState {
    inner: Arc<RwLock<Inner>>,
    tx: watch::Sender<u64>,
    pub rx: watch::Receiver<u64>,
}

impl Default for MarketState {
    fn default() -> Self {
        Self::new()
    }
}

impl MarketState {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(0u64);
        Self {
            inner: Arc::new(RwLock::new(Inner::default())),
            tx,
            rx,
        }
    }

    fn bump(&self) {
        // Monotonic counter so consumers can `changed().await` cheaply.
        self.tx.send_modify(|v| *v = v.wrapping_add(1));
    }

    fn prune(dq_t: &mut VecDeque<Trade>, dq_c: &mut VecDeque<(i64, f64)>, now_ms: i64) {
        let cutoff = now_ms - HISTORY_MS;
        while dq_t.front().is_some_and(|t| t.ts_ms < cutoff) {
            dq_t.pop_front();
        }
        while dq_c.front().is_some_and(|c| c.0 < cutoff) {
            dq_c.pop_front();
        }
    }

    pub fn on_trade(&self, t: Trade) {
        {
            let mut g = self.inner.write();
            g.snap.now_ms = t.ts_ms;
            g.snap.spot = t.price;
            g.trades.push_back(t);
            let now = t.ts_ms;
            let (a, b) = (&mut g.trades, &mut g.chainlink_hist);
            Self::prune(a, b, now);
        }
        self.bump();
    }

    pub fn on_book(&self, b: BookTop) {
        {
            let mut g = self.inner.write();
            g.snap.now_ms = g.snap.now_ms.max(b.ts_ms);
            g.snap.book = b;
        }
        self.bump();
    }

    pub fn on_chainlink(&self, ts_ms: i64, price: f64) {
        {
            let mut g = self.inner.write();
            g.snap.now_ms = g.snap.now_ms.max(ts_ms);
            g.snap.chainlink = price;
            g.chainlink_hist.push_back((ts_ms, price));
            let now = g.snap.now_ms;
            let (a, b) = (&mut g.trades, &mut g.chainlink_hist);
            Self::prune(a, b, now);
        }
        self.bump();
    }

    pub fn on_pm_up(&self, pb: PmBook) {
        {
            let mut g = self.inner.write();
            g.snap.pm_up = pb;
        }
        self.bump();
    }

    pub fn on_pm_down(&self, pb: PmBook) {
        {
            let mut g = self.inner.write();
            g.snap.pm_down = pb;
        }
        self.bump();
    }

    /// Set/replace the active window. Call when a new 5m market is discovered.
    pub fn set_window(&self, w: MarketWindow) {
        {
            self.inner.write().snap.window = Some(w);
        }
        self.bump();
    }

    /// Record the captured price-to-beat once the open boundary is observed.
    pub fn set_price_to_beat(&self, price: f64) {
        {
            let mut g = self.inner.write();
            if let Some(w) = g.snap.window.as_mut() {
                w.price_to_beat = Some(price);
            }
        }
        self.bump();
    }

    pub fn snapshot(&self) -> Snapshot {
        self.inner.read().snap.clone()
    }

    /// Trades within the last `ms` milliseconds (newest last).
    pub fn recent_trades(&self, ms: i64) -> Vec<Trade> {
        let g = self.inner.read();
        let cutoff = g.snap.now_ms - ms;
        g.trades
            .iter()
            .filter(|t| t.ts_ms >= cutoff)
            .copied()
            .collect()
    }

    /// Chainlink ticks within the last `ms` milliseconds (newest last).
    pub fn recent_chainlink(&self, ms: i64) -> Vec<(i64, f64)> {
        let g = self.inner.read();
        let cutoff = g.snap.now_ms - ms;
        g.chainlink_hist
            .iter()
            .filter(|c| c.0 >= cutoff)
            .copied()
            .collect()
    }
}
