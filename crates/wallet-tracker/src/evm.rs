use crate::traits::{LeaderActivityStream, LeaderEvent};
use anyhow::Result;
use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;

/// EVM tracker via alloy logs subscription (stub — Phase 7).
pub struct EvmTracker;

impl EvmTracker {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl LeaderActivityStream for EvmTracker {
    type Event = LeaderEvent;

    async fn subscribe(
        &self,
        _leaders: Vec<String>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Self::Event>> + Send>>> {
        unimplemented!("EvmTracker via alloy logs — Phase 7")
    }

    async fn unsubscribe(&self, _leader: String) -> Result<()> {
        Ok(())
    }
}
