//! What iq keeps of one candidate's run of one exercise: who it was, how it
//! ended, what it spent, what its checks said, and what the judges gave it.

use iq_core::Usd;
use serde::{Deserialize, Serialize};

use crate::candidate::{Candidate, Kind};

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "as", content = "why", rename_all = "snake_case")]
pub enum Ending {
    /// The candidate answered: its work is done, as it says.
    Answered,
    /// It stopped on an error: its model failed, its budget was spent.
    Failed(String),
    /// Its time ran out, and it was stopped.
    OutOfTime,
    /// The person stopped it.
    Stopped,
}

impl Ending {
    /// In a few words.
    #[must_use]
    pub fn said(&self) -> String {
        match self {
            Ending::Answered => "answered".into(),
            Ending::Failed(why) => format!("failed: {why}"),
            Ending::OutOfTime => "out of time".into(),
            Ending::Stopped => "stopped".into(),
        }
    }
}

/// How the hidden checks went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked {
    /// The command run.
    pub line: String,
    /// Whether it ended with status 0.
    pub passed: bool,
    /// The tests that passed and all those run, when the output says.
    pub tally: Option<(u32, u32)>,
    /// The end of what it printed.
    pub output: String,
    /// Why it was not run, when it was not: the gate held it, or the
    /// sandbox could not start it.
    pub refused: Option<String>,
}

/// What a judge gave a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    /// The judge's model.
    pub judge: String,
    /// From 0 to 10.
    pub score: f32,
    /// Why, in its words.
    pub notes: String,
    /// When, in seconds since 1970.
    pub at: u64,
}

/// What a candidate spent on a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Spent {
    /// What the provider billed, in dollars.
    pub billed: Usd,
    /// What its calls under a subscription would cost at list prices,
    /// when they are known: not owed, a measure to compare with.
    pub listed: Usd,
    /// Tokens read and written under a subscription.
    pub subscription_tokens: u64,
    /// Tokens read and written in all.
    pub tokens: u64,
    /// Calls to models.
    pub calls: u32,
}

/// One run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The exercise.
    pub exercise: String,
    /// Its version, and the digest of what it held.
    pub version: u32,
    /// The digest of that version.
    pub digest: String,
    /// The runs asked for together, by when they were asked.
    pub batch: String,
    /// The candidate's letter in its batch, the only name its judges see.
    pub label: String,
    /// Who it was.
    pub candidate: Candidate,
    /// Its kind.
    pub kind: Kind,
    /// The model's name as a grade is kept under, whoever served it: its
    /// letters and digits, without the host, so that a model scores the
    /// same at every provider.
    pub model: String,
    /// Who served the models of iq's loop, as the provider names itself.
    pub provider: Option<String>,
    /// The loop it worked in: `iq`, or the agent's own.
    pub harness: String,
    /// When it started, in seconds since 1970.
    pub started: u64,
    /// How long it worked.
    pub seconds: u64,
    /// How it ended.
    pub ending: Ending,
    /// What it said last.
    pub answer: String,
    /// Its tools' calls, and those of them that failed.
    pub tools: u32,
    /// Those that failed.
    pub tool_errors: u32,
    /// What the gate asked for it, refused since nobody answers in a run.
    pub refused: u32,
    /// What it spent.
    pub spent: Spent,
    /// The files it changed, the lines it added and removed.
    pub changed: (u32, u32, u32),
    /// How its checks went, when the exercise has some.
    pub checked: Option<Checked>,
    /// What the judges gave it.
    #[serde(default)]
    pub verdicts: Vec<Verdict>,
}

impl Record {
    /// Whether its checks passed; a run without checks passes on its
    /// answer alone.
    #[must_use]
    pub fn passed(&self) -> bool {
        match &self.checked {
            Some(checked) => checked.passed,
            None => self.ending == Ending::Answered,
        }
    }

    /// The share of its tests that passed, when the checks said.
    #[must_use]
    pub fn share(&self) -> Option<f32> {
        let (passed, total) = self.checked.as_ref()?.tally?;
        (total > 0).then(|| passed as f32 / total as f32)
    }

    /// Its judges' mean score.
    #[must_use]
    pub fn score(&self) -> Option<f32> {
        if self.verdicts.is_empty() {
            return None;
        }
        Some(self.verdicts.iter().map(|v| v.score).sum::<f32>() / self.verdicts.len() as f32)
    }

    /// Whether `judge` graded it.
    #[must_use]
    pub fn judged_by(&self, judge: &str) -> bool {
        self.verdicts.iter().any(|v| v.judge == judge)
    }
}
