mod error;
mod sse;
mod types;
pub mod client;
pub mod providers;

pub use client::{MessageStream, ProviderClient, ProviderConfig};
pub use error::ApiError;
pub use sse::SseParser;
pub use types::{
    ContentBlockDelta, ContentBlockDeltaEvent, ContentBlockStartEvent, ContentBlockStopEvent,
    InputContentBlock, InputMessage, MessageDelta, MessageDeltaEvent, MessageRequest,
    MessageResponse, MessageStartEvent, MessageStopEvent, OutputContentBlock, StreamEvent,
    ToolChoice, ToolDefinition, ToolResultContentBlock, Usage,
};
pub use providers::{detect_provider_kind, resolve_model_alias, ProviderKind};
