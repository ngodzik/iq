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

    /// Says what the command line `line` does in a few words, for its
    /// one line in the conversation; waits for it. None when it could
    /// not, or by default: the line's own programs and files say it then.
    fn label(&self, line: &str) -> Option<String> {
        let _ = line;
        None
    }
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
        let (from, to, text) = (from.to_owned(), to.to_owned(), text.to_owned());
        Summary::made(move || summarize.summarize(&from, &to, &text))
    }

    /// The few words that say what the command line `line` does, which
    /// `summarize` gives on a thread of its own.
    pub(crate) fn labelled(line: &str, summarize: Arc<dyn Summarize>) -> Arc<Summary> {
        let line = line.to_owned();
        Summary::made(move || summarize.label(&line))
    }

    /// What `make` gives, made on a thread of its own.
    fn made(make: impl FnOnce() -> Option<String> + Send + 'static) -> Arc<Summary> {
        let summary = Arc::new(Summary {
            state: Mutex::new(State::Making(Instant::now())),
        });
        let shared = Arc::clone(&summary);
        std::thread::spawn(move || {
            let state = match make() {
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
