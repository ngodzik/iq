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
    /// How hard it thinks, when the person chose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    /// Where it works, when it has a place of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<PathBuf>,
    /// The tokens its last call read.
    #[serde(default)]
    pub context: u64,
    /// Its conversation, the standing instructions first.
    pub messages: Vec<Message>,
}

/// Keeps the chats between sessions.
pub trait Keeper: Send + Sync {
    /// Keeps `chat` as it stands now, in place of what was kept of it.
    fn keep(&self, chat: &Saved);
}
