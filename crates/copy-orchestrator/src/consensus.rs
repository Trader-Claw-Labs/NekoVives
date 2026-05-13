use std::collections::HashMap;

/// A signal window for consensus detection.
#[derive(Debug, Clone)]
pub struct SignalWindow {
    pub symbol: String,
    pub side: String,
    pub leaders: Vec<String>,
    pub first_seen: chrono::DateTime<chrono::Utc>,
    pub last_seen: chrono::DateTime<chrono::Utc>,
}

/// Accumulates leader fills over time windows to detect consensus.
#[derive(Debug, Default)]
pub struct ConsensusAccumulator {
    /// Key: "symbol|side" -> recent signals
    windows: HashMap<String, Vec<SignalWindow>>,
}

impl ConsensusAccumulator {
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
        }
    }

    /// Record a fill from a leader. Returns true if consensus threshold is met.
    pub fn record(
        &mut self,
        symbol: &str,
        side: &str,
        leader: &str,
        window_secs: i64,
        consensus_n: usize,
    ) -> bool {
        let key = format!("{}|{}", symbol, side);
        let now = chrono::Utc::now();

        // Prune old windows
        let cutoff = now - chrono::Duration::seconds(window_secs);
        self.windows.retain(|_, windows| {
            windows.retain(|w| w.last_seen > cutoff);
            !windows.is_empty()
        });

        let windows = self.windows.entry(key).or_default();

        // Try to add to an existing window
        let mut added = false;
        for window in windows.iter_mut() {
            if window.last_seen > cutoff && !window.leaders.contains(&leader.to_string()) {
                window.leaders.push(leader.to_string());
                window.last_seen = now;
                added = true;
                break;
            }
        }

        // Create new window if not added
        if !added {
            windows.push(SignalWindow {
                symbol: symbol.to_string(),
                side: side.to_string(),
                leaders: vec![leader.to_string()],
                first_seen: now,
                last_seen: now,
            });
        }

        // Check if any window meets consensus threshold
        windows.iter().any(|w| w.leaders.len() >= consensus_n)
    }

    /// Count unique leaders in the active window for a symbol+side.
    pub fn count_leaders(&self, symbol: &str, side: &str, window_secs: i64) -> usize {
        let key = format!("{}|{}", symbol, side);
        let now = chrono::Utc::now();
        let cutoff = now - chrono::Duration::seconds(window_secs);

        self.windows
            .get(&key)
            .map(|windows| {
                windows
                    .iter()
                    .filter(|w| w.last_seen > cutoff)
                    .flat_map(|w| w.leaders.iter())
                    .collect::<std::collections::HashSet<_>>()
                    .len()
            })
            .unwrap_or(0)
    }

    pub fn clear_symbol(&mut self, symbol: &str, side: &str) {
        let key = format!("{}|{}", symbol, side);
        self.windows.remove(&key);
    }
}
