use crate::traits::{LeaderActivityStream, LeaderEvent};
use anyhow::Result;
use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;

/// Solana tracker via Yellowstone gRPC (stub — Phase 5).
pub struct SolanaTracker;

impl SolanaTracker {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl LeaderActivityStream for SolanaTracker {
    type Event = LeaderEvent;

    async fn subscribe(
        &self,
        _leaders: Vec<String>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Self::Event>> + Send>>> {
        unimplemented!("SolanaTracker via Yellowstone gRPC — Phase 5")
    }

    async fn unsubscribe(&self, _leader: String) -> Result<()> {
        Ok(())
    }
}
