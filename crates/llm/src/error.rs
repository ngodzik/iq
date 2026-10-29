use std::time::Duration;

use thiserror::Error;

/// What can go wrong between iq and a provider.
#[derive(Debug, Error)]
pub enum LlmError {
    /// The request never got an HTTP answer: DNS, TLS, proxy, timeout.
    #[error("could not reach {url}")]
    Transport {
        /// The endpoint that was called.
        url: String,
        /// The underlying failure, kept whole so that its cause chain is not lost.
        #[source]
        source: reqwest::Error,
    },

    /// The provider answered with an error status.
    #[error("{url} answered HTTP {status}: {body}")]
    Status {
        /// The endpoint that was called.
        url: String,
        /// The HTTP status code.
        status: u16,
        /// The response body, which usually says why.
        body: String,
        /// How long the provider asked to be left before the next call,
        /// when it said: in its `Retry-After`, or in words in the body.
        after: Option<Duration>,
    },

    /// The provider answered, but not in the expected shape.
    #[error("unexpected response from {url}: {reason}")]
    Malformed {
        /// The endpoint that was called.
        url: String,
        /// What was missing or wrong.
        reason: String,
    },

    /// No key was given for the provider, so its models cannot be called.
    #[error("no API key for {url}: set REQUESTY_API_KEY")]
    NoKey {
        /// The endpoint that would have been called.
        url: String,
    },

    /// The provider held the request without answering. Replies are not
    /// streamed, so nothing says whether it is still working: the call
    /// stops, and is not sent again by itself since it may have been billed.
    #[error(
        "{url} gave no answer in {minutes} minutes: the request may have been billed; send it again"
    )]
    TimedOut {
        /// The endpoint that was called.
        url: String,
        /// How long it was waited for.
        minutes: u64,
        /// The underlying failure.
        #[source]
        source: reqwest::Error,
    },

    /// The provider does not list the requested model.
    #[error("model {0} is not listed by the provider")]
    UnknownModel(String),
}

/// The statuses of a provider out of capacity for now, rather than
/// refusing the call: too many requests or tokens for the minute, an
/// error of its own, a gateway that could not reach it, an overload.
const BUSY: [u16; 6] = [429, 500, 502, 503, 504, 529];

impl LlmError {
    /// Whether the provider is out of capacity for now, the call worth
    /// sending again: a status that says so, or a connection that could
    /// not be made, the request never sent. With how long it asked to be
    /// left, when it said.
    #[must_use]
    pub fn busy(&self) -> Option<iq_core::Busy> {
        match self {
            LlmError::Status { status, after, .. } if BUSY.contains(status) => {
                Some(iq_core::Busy { after: *after })
            }
            LlmError::Transport { source, .. } if source.is_connect() => {
                Some(iq_core::Busy { after: None })
            }
            _ => None,
        }
    }
}
