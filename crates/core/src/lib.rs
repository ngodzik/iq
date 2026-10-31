//! Domain types and traits shared by every iq crate.
//!
//! This crate does no I/O and knows nothing about any particular provider.
//! Everything that talks to the network or the filesystem lives in a crate
//! that depends on this one, never the other way round.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod change;
mod error;
mod glob;
mod traits;
mod types;

pub use change::{Change, DiffLine, Hunk};
pub use error::CoreError;
pub use glob::glob_matches;
pub use traits::ChatModel;
pub use types::{
    Busy, CacheTtl, CacheUse, ChatRequest, ChatResponse, Dearer, Effort, Message, ModelId, Pricing,
    TokenCount, ToolCall, ToolSpec, Usage, Usd,
};
