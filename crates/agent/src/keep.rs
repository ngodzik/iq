//! A chat kept between sessions: what an agent is and its conversation as
//! it stands, given to a keeper at each turn's end, and taken back to
//! resume the chat where it stopped when iq starts again.

use std::path::PathBuf;

use iq_core::{Effort, Message, Usd};
use serde::{Deserialize, Serialize};

/// An agent and its conversation, as kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    /// Its name on the board.
    pub name: String,
    /// Its model.
    pub model: String,
    /// Who started it: the person or another agent.
    pub by: String,
    /// Its part in the crew, when it was given one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Its task.
    pub task: String,
    /// What the person asked of it since, the latest last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asked: Vec<String>,
    /// When the person last wrote to it, in seconds since 1970.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written: Option<u64>,
    /// How hard it thinks, when the person chose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    /// Where it works, when it has a place of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<PathBuf>,
    /// The tokens its last call read.
    #[serde(default)]
    pub context: u64,
    /// Each choice of its model, the latest last: the one it was started
    /// on, then each the person gave it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models_chosen: Vec<ModelChoice>,
    /// The calls to its model it made, over all its turns.
    #[serde(default)]
    pub calls: u64,
    /// What those calls cost, as far as their prices are known.
    #[serde(default)]
    pub spent: Usd,
    /// What the person was shown of it beside its conversation, in order:
    /// iq's notices, its questions and their answers, what the gate asked
    /// and the autopilot answered, its errors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aside: Vec<Aside>,
    /// The results of its tool calls set apart to keep its conversation
    /// light, whole, by their ids from 1: what `reread` reads again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set_apart: Vec<String>,
    /// Its conversation, the standing instructions first.
    pub messages: Vec<Message>,
}

/// One thing the person was shown of an agent beside its conversation,
/// with when and in which of its turns, so that its chat can be read
/// back as it happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aside {
    /// When, in seconds since 1970.
    pub at: u64,
    /// The turn of the agent's it came in: 0 before its first.
    pub turn: u64,
    /// How many messages its conversation held then: where it stands
    /// among them, to a step.
    #[serde(default)]
    pub after: usize,
    /// What it was.
    #[serde(flatten)]
    pub shown: Shown,
}

/// What the person was shown beside an agent's conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shown {
    /// Something iq said of how the agent works.
    Notice {
        /// What.
        text: String,
    },
    /// A question the agent asked the person, with its choices, and their
    /// answer once given.
    Question {
        /// The question's number, which its answer names.
        id: u64,
        /// The question.
        question: String,
        /// What it offered.
        choices: Vec<String>,
        /// Whether it was answered, or given up.
        answered: bool,
        /// Their answer, none when they gave none.
        answer: Option<String>,
    },
    /// An action the gate asked the person about, and their answer.
    Asked {
        /// The question's number, which its answer names.
        id: u64,
        /// The action, in words.
        action: String,
        /// Why the gate asked.
        reason: String,
        /// Whether they approved it; none until they answered.
        approved: Option<bool>,
    },
    /// An action the autopilot's model answered for the person.
    Reviewed {
        /// The action, in words.
        action: String,
        /// Whether it approved it.
        approved: bool,
        /// Its reason.
        why: String,
    },
    /// A call to the provider that failed, and the turn with it.
    Failed {
        /// The model called.
        model: String,
        /// The status the provider answered with, none when it was not
        /// reached.
        status: Option<u16>,
        /// What it said, or why it could not be reached.
        message: String,
        /// About how many tokens the call held.
        context: u64,
        /// The calls the agent made before it, and what they cost.
        calls: u64,
        /// What the agent had spent until then.
        spent: Usd,
    },
    /// A turn that failed otherwise: why.
    Error {
        /// Why.
        text: String,
    },
}

/// A choice of the model an agent runs on: which, by whom, how and when,
/// so that how a chat came to run on a model can always be told.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelChoice {
    /// The model chosen.
    pub model: String,
    /// Who chose it: the person, or the agent that started this one.
    pub by: String,
    /// How: the command, the tool or the setting that named it.
    pub how: String,
    /// When, in seconds since 1970.
    pub at: u64,
}

/// Takes off what the answers of `messages` carry of their model's
/// reasoning, as its provider signed it: their text stays. Returns whether
/// any carried it.
pub(crate) fn forget_reasoning(messages: &mut [Message]) -> bool {
    let mut forgot = false;
    for message in messages {
        if let Message::Assistant {
            reasoning_signature,
            ..
        } = message
        {
            forgot |= reasoning_signature.take().is_some();
        }
    }
    forgot
}

/// Keeps the chats between sessions.
pub trait Keeper: Send + Sync {
    /// Keeps `chat` as it stands now, in place of what was kept of it.
    fn keep(&self, chat: &Saved);

    /// Forgets what was kept of the chat `name`: it goes on under another.
    fn forget(&self, _name: &str) {}

    /// Keeps what was kept of the agent `name` apart, as it retires: it
    /// comes back no more, and another may take its name, but what it did
    /// stays readable as long as its chat.
    fn retire(&self, _name: &str) {}
}
