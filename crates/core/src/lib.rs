//! Domain types and traits shared by every iq crate.
//!
//! This crate does no I/O and knows nothing about any particular provider.
//! Everything that talks to the network or the filesystem lives in a crate
//! that depends on this one, never the other way round.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod change;
mod error;
mod subscription;
mod traits;
mod types;

pub use change::{Change, DiffLine, Hunk};
pub use error::CoreError;
pub use subscription::{Quota, QuotaWindow, SubscriptionCall};
pub use traits::{ChatModel, Delegate};
pub use types::{
    Agent, CacheTtl, CacheUse, ChatRequest, ChatResponse, ContextUse, DelegateEvent, DelegateReply,
    DelegateRequest, Effort, McpReach, McpServer, Message, ModelId, Pricing, Reach, TokenCount,
    ToolCall, ToolSpec, Usage, Usd,
};
