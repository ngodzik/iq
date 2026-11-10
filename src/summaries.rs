//! What agents write to each other, summed up for the person by the cheap
//! model the settings name `summarizer`: the conversation shows the summary,
//! the agent it is for reads the message whole. Each summary is one call
//! to the provider, its cost counted as any call's.

use iq_agent::{Asked, Report, ask_once};
use iq_core::{Effort, ModelId};
use iq_llm::OpenAiCompatible;

/// The longest message given whole to the summarizer, in characters: past
/// it, its start and its end, which say what is asked and what is answered.
const SUMMED_MOST: usize = 24_000;

/// Who the summaries' costs are reported for.
const SUMMARIZER: &str = "summaries";

/// What the summarizer's model is told.
const SUMMING: &str = "You sum up for a person what one agent of a coding harness wrote to \
    another, so that they follow the work without reading it all. In at most three short lines: \
    what is asked or answered, and what matters in it (a decision, a file, a number, what failed). \
    Write in the message's own language, plainly, with no preamble and no heading. The message \
    follows, after who wrote it to whom.";

/// What the summarizer's model is told to say what a command does.
const LABELLING: &str = "You say in a few words what a shell command line of a coding agent \
    does, for a person who follows its work: at most ten words, verbs first, plainly, no quotes, \
    no full stop, in English. Say what it does (patches a hook and its tests, then runs the \
    checks), never how. The command line follows.";

/// The longest command line given whole to the summarizer to label.
const LABELLED_MOST: usize = 8_000;

/// Sums up the messages between agents with one model of the provider.
pub(crate) struct Summaries {
    provider: OpenAiCompatible,
    model: ModelId,
    handle: tokio::runtime::Handle,
    costs: Report,
}

impl Summaries {
    /// Summaries by `model` of `provider`, asked on `handle`'s runtime,
    /// their costs reported to `costs`.
    pub(crate) fn new(
        provider: OpenAiCompatible,
        model: ModelId,
        handle: tokio::runtime::Handle,
        costs: Report,
    ) -> Self {
        Self {
            provider,
            model,
            handle,
            costs,
        }
    }
}

impl iq_tui::Summarize for Summaries {
    fn summarize(&self, from: &str, to: &str, text: &str) -> Option<String> {
        let asked = format!("From {from} to {to}:\n\n{}", within(text, SUMMED_MOST));
        let answer = self.handle.block_on(ask_once(
            &self.provider,
            &self.model,
            SUMMING,
            asked,
            Asked {
                agent: SUMMARIZER,
                effort: Effort::Low,
                costs: &self.costs,
            },
        ));
        answer.ok().filter(|said| !said.trim().is_empty())
    }

    fn label(&self, line: &str) -> Option<String> {
        let answer = self.handle.block_on(ask_once(
            &self.provider,
            &self.model,
            LABELLING,
            within(line, LABELLED_MOST),
            Asked {
                agent: SUMMARIZER,
                effort: Effort::Low,
                costs: &self.costs,
            },
        ));
        answer.ok().filter(|said| !said.trim().is_empty())
    }
}

/// `text`, or its start and its end when it is longer than `most`
/// characters, said to be cut between them.
fn within(text: &str, most: usize) -> String {
    let count = text.chars().count();
    if count <= most {
        return text.to_owned();
    }
    let half = most / 2;
    let start: String = text.chars().take(half).collect();
    let end: String = text.chars().skip(count - half).collect();
    format!(
        "{start}\n\n[… {} characters left out …]\n\n{end}",
        count - 2 * half
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_message_is_given_by_its_start_and_its_end() {
        assert_eq!(within("short", 10), "short");
        let long = format!("{}{}", "a".repeat(30), "z".repeat(30));
        let given = within(&long, 20);
        assert!(given.starts_with(&"a".repeat(10)), "{given}");
        assert!(given.ends_with(&"z".repeat(10)), "{given}");
        assert!(given.contains("[… 40 characters left out …]"), "{given}");
    }
}
