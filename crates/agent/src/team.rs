//! How a team works on a task: the patterns the person may start one in,
//! and the turns iq hands between their agents, decided here from what each
//! turn ended with; and the teams an agent starts with the roles it gives
//! them, as its skills say, which work as their roles say.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use iq_core::{Effort, ModelId};
use serde_json::Value;

/// One member of a team an agent starts: its part, its model, how hard it
/// thinks, and what it does.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Seat {
    /// Its role, in a word: writer, reviewer.
    pub(crate) role: String,
    /// Its model.
    pub(crate) model: ModelId,
    /// How hard it thinks; as the settings say when `None`.
    pub(crate) effort: Option<Effort>,
    /// What it does, and how it works with the others.
    pub(crate) does: String,
}

/// The seats of a team, as `start_team`'s `members` gives them.
pub(crate) fn seats(members: &Value) -> Result<Vec<Seat>, String> {
    let members = members
        .as_array()
        .filter(|members| !members.is_empty())
        .ok_or("`members` must list the team's members, one at least")?;
    members
        .iter()
        .enumerate()
        .map(|(at, member)| {
            let text = |key: &str| {
                member[key]
                    .as_str()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .ok_or_else(|| format!("member {} has no `{key}`", at + 1))
            };
            let effort = match member["effort"].as_str().map(str::trim) {
                Some(level) if !level.is_empty() => Some(level.parse::<Effort>()?),
                _ => None,
            };
            Ok(Seat {
                role: text("role")?.to_owned(),
                model: ModelId::new(text("model")?).map_err(|e| e.to_string())?,
                effort,
                does: text("does")?.to_owned(),
            })
        })
        .collect()
}

/// What the member at `at` of a team is told of it: who is in it, whom it
/// writes to, and where its answer goes. `names` are the members', in the
/// order of `seats`; the first answers `by`, who started the team.
pub(crate) fn roster(names: &[String], seats: &[Seat], at: usize, by: &str) -> String {
    let members: Vec<String> = names
        .iter()
        .zip(seats)
        .map(|(name, seat)| format!("- {name}, {}: {}", seat.role, seat.does))
        .collect();
    let answer = if at == 0 {
        format!(
            "You work on it now. Your answer, once the work is done, goes to {by}, who started \
             the team."
        )
    } else {
        format!(
            "You wait until a teammate writes to you. Your answer at the end of each turn goes \
             to {}.",
            names[0]
        )
    };
    format!(
        "You are listed as {}, in a team of {}:\n{}\nWrite to a teammate with send_message. {answer}",
        names[at],
        names.len(),
        members.join("\n")
    )
}

/// The most rounds a writer and a reviewer go before the person decides.
pub(crate) const PAIR_ROUNDS: usize = 3;

/// How a team works on a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pattern {
    /// One agent.
    #[default]
    Solo,
    /// A lead that splits the task among agents it starts, each with its
    /// part, and puts their work together.
    Lead,
    /// A writer, and a reviewer that reads each of its changes until it
    /// approves, a few rounds at most.
    Pair,
    /// Several agents answer the same task alone, then each reads the
    /// others' answers and revises its own, and the first sums them up.
    Debate,
}

impl Pattern {
    /// Every pattern, in the order offered.
    pub const ALL: [Pattern; 4] = [Pattern::Solo, Pattern::Lead, Pattern::Pair, Pattern::Debate];

    /// Its name, as the command line and the window write it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Pattern::Solo => "solo",
            Pattern::Lead => "lead",
            Pattern::Pair => "pair",
            Pattern::Debate => "debate",
        }
    }

    /// What it does, in a line.
    #[must_use]
    pub fn what(self) -> &'static str {
        match self {
            Pattern::Solo => "one agent",
            Pattern::Lead => "a lead that splits the work among agents it starts",
            Pattern::Pair => "a writer, and a reviewer until it approves",
            Pattern::Debate => "agents answer alone, then read each other and revise",
        }
    }

    /// Whether it needs a second model: the reviewer's, or a second
    /// voice's.
    #[must_use]
    pub fn takes_partner(self) -> bool {
        matches!(self, Pattern::Pair | Pattern::Debate)
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Pattern {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let wanted = text.trim().to_ascii_lowercase();
        Pattern::ALL
            .into_iter()
            .find(|p| p.name() == wanted)
            .ok_or_else(|| {
                let names: Vec<&str> = Pattern::ALL.iter().map(|p| p.name()).collect();
                format!("no pattern `{text}`; the patterns are {}", names.join(", "))
            })
    }
}

/// The lead's part, the cheaper models it may give its agents named.
pub(crate) fn lead_role(cheaper: &[String]) -> String {
    let workers = if cheaper.is_empty() {
        "a cheap model of the provider".to_owned()
    } else {
        cheaper.join(", ")
    };
    format!(
        "lead. Split the task among agents you start with start_agent, each with a role and the \
         files it owns, so that no two edit the same file; put workers on cheaper models when the \
         work allows ({workers}). Give each all it needs, wait for their answers, put the work \
         together and check the whole with the project's tools. Do little of the work yourself"
    )
}

/// The writer's part, in a pair.
pub(crate) const WRITER_ROLE: &str = "writer. A reviewer reads each of your changes once your turn ends, and its remarks come back \
     to you: take them, or say why not";

/// The reviewer's part, in a pair.
pub(crate) const REVIEWER_ROLE: &str = "reviewer. You read, you never change a file. Answer with APPROVE or CHANGES first, then, for \
     CHANGES, what to change and why, in a short list";

/// A debater's part, one of `count`.
pub(crate) fn debater_role(count: usize) -> String {
    format!(
        "one of {count} agents who answer the same task alone first. Do not write to the others: \
         their answers come to you once all have answered"
    )
}

/// What the reviewer is asked, round `round`, of `writer`'s work on `task`.
pub(crate) fn review_prompt(writer: &str, task: &str, round: usize) -> String {
    let again = if round == 0 {
        String::new()
    } else {
        " It changed it again after your last review: review the new state the same way.".to_owned()
    };
    format!(
        "Review what {writer} changed for this task: {task}.{again} Read the changes with git diff, \
         and git status for new files; run the project's checks if they only read. Answer with \
         APPROVE or CHANGES first."
    )
}

/// Whether a reviewer's answer approves: its first word, whatever its case
/// and marks.
pub(crate) fn approves(answer: &str) -> bool {
    answer
        .split_whitespace()
        .next()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .eq_ignore_ascii_case("approve")
        })
        .unwrap_or(false)
}

/// What a debater reads once all have answered alone: the others' answers.
pub(crate) fn others_prompt(me: &str, answers: &BTreeMap<String, String>) -> String {
    let others: Vec<String> = answers
        .iter()
        .filter(|(name, _)| name.as_str() != me)
        .map(|(name, answer)| format!("{name} answered:\n{answer}"))
        .collect();
    format!(
        "All have answered alone. {}\n\nSay where they are wrong, or better than you, then give \
         your answer again, revised.",
        others.join("\n\n")
    )
}

/// What the first debater reads to sum up the revised answers.
pub(crate) fn summing_prompt(answers: &BTreeMap<String, String>) -> String {
    let all: Vec<String> = answers
        .iter()
        .map(|(name, answer)| format!("{name}, revised:\n{answer}"))
        .collect();
    format!(
        "{}\n\nSum these up into one answer for the person: what holds, where you still disagree, \
         and what you would do.",
        all.join("\n\n")
    )
}

/// Where a debate is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Each answers alone.
    Alone,
    /// Each revises, having read the others.
    Revising,
    /// The first sums up.
    Summing,
    /// Done.
    Over,
}

/// A team at work.
#[derive(Debug, Clone)]
pub(crate) enum Team {
    /// A writer and its reviewer.
    Pair {
        writer: String,
        reviewer: Option<String>,
        reviewer_model: iq_core::ModelId,
        task: String,
        rounds: usize,
        over: bool,
    },
    /// Agents on one task, then on each other's answers.
    Debate {
        members: Vec<String>,
        phase: Phase,
        answers: BTreeMap<String, String>,
    },
}

impl Team {
    /// The agent `from` is called `to` from now on.
    pub(crate) fn rename(&mut self, from: &str, to: &str) {
        let renamed = |name: &mut String| {
            if name == from {
                *name = to.to_owned();
            }
        };
        match self {
            Team::Pair {
                writer, reviewer, ..
            } => {
                renamed(writer);
                if let Some(reviewer) = reviewer {
                    renamed(reviewer);
                }
            }
            Team::Debate {
                members, answers, ..
            } => {
                members.iter_mut().for_each(renamed);
                if let Some(answer) = answers.remove(from) {
                    answers.insert(to.to_owned(), answer);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_review_approves_by_its_first_word_only() {
        assert!(approves("APPROVE: it does what the task asks."));
        assert!(approves("**Approve** with one remark"));
        assert!(!approves("CHANGES: approve once the test passes"));
        assert!(!approves(""));
        assert_eq!("Pair".parse::<Pattern>().unwrap(), Pattern::Pair);
        assert!("swarm".parse::<Pattern>().is_err());
    }

    #[test]
    fn a_debater_reads_the_others_answers_not_its_own() {
        let answers: BTreeMap<String, String> = [
            ("a".to_owned(), "use a map".to_owned()),
            ("b".to_owned(), "use a list".to_owned()),
        ]
        .into();
        let prompt = others_prompt("a", &answers);
        assert!(prompt.contains("use a list") && !prompt.contains("use a map"));
        assert!(summing_prompt(&answers).contains("use a map"));
    }

    #[test]
    fn a_team_is_read_from_its_members_and_each_told_the_others() {
        let members = serde_json::json!([
            {"role": "writer", "model": "lab/a", "effort": "high", "does": "writes the change"},
            {"role": "reviewer", "model": "lab/b", "does": "reads its diff, two rounds at most"},
        ]);
        let team = seats(&members).unwrap();
        assert_eq!(team[0].effort, Some(Effort::High));
        assert_eq!((team[1].model.as_str(), team[1].effort), ("lab/b", None));
        let names = ["code-writer".to_owned(), "code-reviewer".to_owned()];
        let first = roster(&names, &team, 0, "lead");
        assert!(first.contains("You are listed as code-writer") && first.contains("goes to lead"));
        assert!(first.contains("- code-reviewer, reviewer: reads its diff, two rounds at most"));
        assert!(roster(&names, &team, 1, "lead").contains("goes to code-writer"));
        assert!(seats(&serde_json::json!([])).is_err());
        assert!(
            seats(&serde_json::json!([{"role": "writer", "model": "lab/a"}]))
                .unwrap_err()
                .contains("`does`")
        );
        assert!(
            seats(
                &serde_json::json!([{"role": "w", "model": "lab/a", "does": "x", "effort": "huge"}])
            )
            .is_err()
        );
    }
}
