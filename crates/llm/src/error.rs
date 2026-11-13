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

    /// What the provider said, when it refused the call with a status: the
    /// message of its error as JSON gives it (`{"error": {"message": …}}`,
    /// or `{"error": "…"}`), else its answer, cut to 600 characters.
    #[must_use]
    pub fn refusal(&self) -> Option<iq_core::Refusal> {
        let LlmError::Status { status, body, .. } = self else {
            return None;
        };
        let json: Option<serde_json::Value> = serde_json::from_str(body).ok();
        let said = json.as_ref().and_then(|json| {
            let error = json.get("error")?;
            error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .or_else(|| error.as_str())
                .map(str::to_owned)
        });
        let message = said.unwrap_or_else(|| body.trim().to_owned());
        let message = match message.char_indices().nth(SAID_CHARS) {
            Some((at, _)) => format!("{}…", &message[..at]),
            None => message,
        };
        Some(iq_core::Refusal {
            status: *status,
            message,
        })
    }
}

/// The most of a provider's words kept of its refusal.
pub(crate) const SAID_CHARS: usize = 600;
