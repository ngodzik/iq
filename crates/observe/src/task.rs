//! One task of iq's agents, as iq's journal keeps it: one line of JSON for
//! each turn an agent worked.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// One task an agent of iq's worked on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// When it started, in seconds since 1970.
    pub at: u64,
    /// The agent.
    pub agent: String,
    /// Its model.
    pub model: String,
    /// The project it worked in, as its memory is kept under.
    #[serde(default)]
    pub project: String,
    /// The start of what it was given.
    pub prompt: String,
    /// The files it wrote or edited, as its tools named them.
    #[serde(default)]
    pub files: Vec<String>,
    /// The commands it ran, their first words.
    #[serde(default)]
    pub commands: Vec<String>,
    /// What the provider billed it, in dollars.
    #[serde(default)]
    pub billed: f64,
    /// The tokens it read and wrote.
    #[serde(default)]
    pub tokens: u64,
    /// How long it worked, in seconds.
    #[serde(default)]
    pub seconds: u64,
    /// Whether it ended with an answer, rather than an error.
    #[serde(default = "answered")]
    pub ok: bool,
    /// Why it ended without an answer, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

fn answered() -> bool {
    true
}

impl Default for Task {
    fn default() -> Self {
        Self {
            at: 0,
            agent: String::new(),
            model: String::new(),
            project: String::new(),
            prompt: String::new(),
            files: Vec::new(),
            commands: Vec::new(),
            billed: 0.0,
            tokens: 0,
            seconds: 0,
            ok: true,
            why: None,
        }
    }
}

impl Task {
    /// Its name among the journal's: when it started, and its agent.
    #[must_use]
    pub fn id(&self) -> String {
        format!("{}-{}", self.at, self.agent)
    }
}

/// The tasks of the journal at `file` since `since`, oldest first.
#[must_use]
pub fn read(file: &Path, since: u64) -> Vec<Task> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Task>(line).ok())
        .filter(|task| task.at >= since)
        .collect()
}
