//! What one agent writes to another, summed up for the person by a cheap
//! model as the settings give one: who asks or answers what, in a few
//! lines. The summary is the person's alone: the agent it is for reads
//! the message whole, as it was written. It is made on a thread of its
//! own; what the conversation writes after the message waits for it a
//! little, then takes the message's first lines instead.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// What sums up the messages between agents, as the settings choose; it
/// does the I/O and counts what it spends.
pub trait Summarize: Send + Sync {
    /// Sums up `text`, written by `from` to `to`, in a few lines; waits
    /// for it. None when it could not.
    fn summarize(&self, from: &str, to: &str, text: &str) -> Option<String>;
}

/// How long a summary is waited for before the message's first lines are
/// written instead.
const PATIENCE: Duration = Duration::from_secs(20);

/// Where a summary stands.
#[derive(Debug)]
enum State {
    Making(Instant),
    Made(String),
    Failed,
}

/// The summary of one message, made in the background.
#[derive(Debug)]
pub(crate) struct Summary {
    state: Mutex<State>,
}

impl Summary {
    /// The summary of `text`, from `from` to `to`, which `summarize` makes
    /// on a thread of its own.
    pub(crate) fn new(
        from: &str,
        to: &str,
        text: &str,
        summarize: Arc<dyn Summarize>,
    ) -> Arc<Summary> {
        let summary = Arc::new(Summary {
            state: Mutex::new(State::Making(Instant::now())),
        });
        let shared = Arc::clone(&summary);
        let (from, to, text) = (from.to_owned(), to.to_owned(), text.to_owned());
        std::thread::spawn(move || {
            let state = match summarize.summarize(&from, &to, &text) {
                Some(said) if !said.trim().is_empty() => State::Made(said.trim().to_owned()),
                _ => State::Failed,
            };
            *shared.state.lock().unwrap_or_else(PoisonError::into_inner) = state;
        });
        summary
    }

    /// Whether it is still being made, and waited for.
    pub(crate) fn making(&self) -> bool {
        matches!(
            *self.state.lock().unwrap_or_else(PoisonError::into_inner),
            State::Making(since) if since.elapsed() < PATIENCE
        )
    }

    /// The summary, once made.
    pub(crate) fn text(&self) -> Option<String> {
        match &*self.state.lock().unwrap_or_else(PoisonError::into_inner) {
            State::Made(said) => Some(said.clone()),
            State::Making(_) | State::Failed => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Says(Option<&'static str>);

    impl Summarize for Says {
        fn summarize(&self, from: &str, to: &str, text: &str) -> Option<String> {
            assert_eq!((from, to, text), ("fix/tests", "fix", "all of it"));
            self.0.map(str::to_owned)
        }
    }

    fn settled(summary: &Summary) {
        for _ in 0..200 {
            if !summary.making() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the summary was never made");
    }

    #[test]
    fn a_summary_is_made_apart_and_none_when_it_fails() {
        let made = Summary::new(
            "fix/tests",
            "fix",
            "all of it",
            Arc::new(Says(Some(" 3 pass "))),
        );
        settled(&made);
        assert_eq!(made.text().as_deref(), Some("3 pass"));
        let failed = Summary::new("fix/tests", "fix", "all of it", Arc::new(Says(None)));
        settled(&failed);
        assert_eq!(failed.text(), None);
    }
}
