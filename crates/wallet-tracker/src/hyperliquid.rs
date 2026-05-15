use crate::traits::{LeaderActivityStream, LeaderEvent};
use anyhow::Result;
use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;

/// Hyperliquid tracker via WebSocket userFills (stub — Phase 6).
pub struct HyperliquidTracker;

impl HyperliquidTracker {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl LeaderActivityStream for HyperliquidTracker {
    type Event = LeaderEvent;

    async fn subscribe(
        &self,
        _leaders: Vec<String>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Self::Event>> + Send>>> {
        unimplemented!("HyperliquidTracker via WS userFills — Phase 6")
    }

    async fn unsubscribe(&self, _leader: String) -> Result<()> {
        Ok(())
    }
}
