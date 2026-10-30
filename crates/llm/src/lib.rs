//! Model providers for iq.
//!
//! The provider speaks the OpenAI compatible chat completions protocol,
//! which routers such as Requesty expose for hundreds of models behind one
//! key. Providers for other protocols would be added next to it, each as an
//! implementation of [`iq_core::ChatModel`].

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod openai_compat;

pub use error::LlmError;
pub use openai_compat::{Listed, OpenAiCompatible};
