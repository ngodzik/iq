//! A chat kept between sessions: what an agent is and its conversation as
//! it stands, given to a keeper at each turn's end, and taken back to
//! resume the chat where it stopped when iq starts again.

use std::path::PathBuf;

use iq_core::{Effort, Message};
use serde::{Deserialize, Serialize};

/// An agent and its conversation, as kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Its conversation, the standing instructions first.
    pub messages: Vec<Message>,
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

/// Keeps the chats between sessions.
pub trait Keeper: Send + Sync {
    /// Keeps `chat` as it stands now, in place of what was kept of it.
    fn keep(&self, chat: &Saved);

    /// Forgets what was kept of the chat `name`: it goes on under another.
    fn forget(&self, _name: &str) {}
}
