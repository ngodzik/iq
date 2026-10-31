use std::future::Future;

use crate::types::{Busy, ChatRequest, ChatResponse, ModelId, Pricing};

/// Something that can answer a chat request.
///
/// Each provider brings its own error type, so that a caller can tell a
/// network failure from a refusal without parsing strings.
pub trait ChatModel: Send + Sync {
    /// The error this provider reports.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Sends one request and waits for the whole answer.
    fn complete(
        &self,
        request: &ChatRequest,
    ) -> impl Future<Output = Result<ChatResponse, Self::Error>> + Send;

    /// The most `model` can read at once, in tokens, when the provider says.
    /// Used to show how full the context is; `None` shows nothing.
    fn context_window(&self, model: &ModelId) -> impl Future<Output = Option<u64>> + Send {
        let _ = model;
        std::future::ready(None)
    }

    /// What `model` charges, when the provider says: what a call costs when
    /// the provider did not say, and where its price rises, which its
    /// agent is told.
    fn pricing(&self, model: &ModelId) -> impl Future<Output = Option<Pricing>> + Send {
        let _ = model;
        std::future::ready(None)
    }

    /// Whether `error` says the provider is out of capacity for now
    /// rather than refusing the call, which is then worth sending again;
    /// `None`, the default, for an error that would come again.
    fn busy(&self, error: &Self::Error) -> Option<Busy> {
        let _ = error;
        None
    }
}
