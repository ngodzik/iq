//! What the agents do between two of their lines of text: the tools they
//! call, what the person and the reviewer allow or refuse them. An agent's
//! steps make one fold from its last line to its next: while it works, the
//! last of them show as they come; once it writes again, or its turn ends,
//! the fold closes and only its count is said, the steps kept to be
//! unfolded on their page.

use std::collections::{HashMap, VecDeque};

use iq_agent::{Event, PERSON};

/// How many folds are kept, the oldest forgotten first.
const KEPT: usize = 500;

/// How many of an agent's latest steps show while it works, unless the
/// settings say otherwise.
pub const SHOWN: usize = 5;

/// One step of an agent's work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A tool it called.
    Tool {
        /// The tool.
        name: String,
        /// Its input.
        input: String,
        /// What it returned.
        output: String,
        /// Whether it succeeded.
        ok: bool,
    },
    /// What the person answered to what it asked.
    Answered {
        /// The action.
        action: String,
        /// Whether it was allowed.
        approved: bool,
    },
    /// What the reviewer answered for the person.
    Reviewed {
        /// The action.
        action: String,
        /// Whether it was allowed.
        approved: bool,
        /// Why.
        why: String,
    },
}

impl Step {
    /// Whether it went wrong: a tool that failed, an action refused.
    #[must_use]
    pub fn failed(&self) -> bool {
        match self {
            Step::Tool { ok, .. } => !ok,
            Step::Answered { approved, .. } | Step::Reviewed { approved, .. } => !approved,
        }
    }

    /// What it is called in a count of steps: its tool, or who answered.
    #[must_use]
    pub fn kind(&self) -> &str {
        match self {
            Step::Tool { name, .. } => name,
            Step::Answered { .. } => "asked you",
            Step::Reviewed { .. } => "reviewed",
        }
    }
}

/// An agent's steps from one of its lines of text to its next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fold {
    /// Its number, from 1, as its count names it.
    pub id: usize,
    /// The agent.
    pub agent: String,
    /// Its steps, in order.
    pub steps: Vec<Step>,
    /// Whether the agent went on to something else.
    pub closed: bool,
}

impl Fold {
    /// Its steps counted by kind, the most frequent first, then in the
    /// order they first came.
    #[must_use]
    pub fn kinds(&self) -> Vec<(String, usize)> {
        let mut counted: Vec<(String, usize)> = Vec::new();
        for step in &self.steps {
            match counted.iter_mut().find(|(kind, _)| kind == step.kind()) {
                Some((_, count)) => *count += 1,
                None => counted.push((step.kind().to_owned(), 1)),
            }
        }
        // A stable sort keeps the order they came in among equals.
        counted.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        counted
    }

    /// How many went wrong.
    #[must_use]
    pub fn failed(&self) -> usize {
        self.steps.iter().filter(|step| step.failed()).count()
    }
}

/// The agents' steps, fold by fold.
#[derive(Debug, Clone, Default)]
pub struct Steps {
    folds: VecDeque<Fold>,
    /// The number of the last fold made.
    last: usize,
    /// Each agent's fold being made, by its number.
    open: HashMap<String, usize>,
    /// What each question waiting asks, to keep it once answered.
    asked: HashMap<u64, (String, String)>,
}

impl Steps {
    /// Takes what the crew just did: a step goes in its agent's fold, and
    /// anything else the agent does closes it.
    pub fn event(&mut self, event: &Event) {
        match event {
            Event::Tool {
                agent,
                name,
                input,
                output,
                ok,
            } => self.step(
                agent,
                Step::Tool {
                    name: name.clone(),
                    input: input.clone(),
                    output: output.clone(),
                    ok: *ok,
                },
            ),
            Event::Asked {
                id, agent, action, ..
            } => {
                self.asked.insert(*id, (agent.clone(), action.clone()));
            }
            Event::Answered { id, approved } => {
                if let Some((agent, action)) = self.asked.remove(id) {
                    self.step(
                        &agent,
                        Step::Answered {
                            action,
                            approved: *approved,
                        },
                    );
                }
            }
            Event::Reviewed {
                agent,
                action,
                approved,
                why,
                ..
            } => self.step(
                agent,
                Step::Reviewed {
                    action: action.clone(),
                    approved: *approved,
                    why: why.clone(),
                },
            ),
            Event::Working { agent, .. }
            | Event::TextStart { agent }
            | Event::Done { agent, .. }
            | Event::Notice { agent, .. } => self.close(agent),
            Event::Placed {
                agent,
                said: Some(_),
                ..
            } => self.close(agent),
            Event::Posted { from, to, .. } => {
                // What the person writes to an agent comes after what it
                // did before.
                let agent = if from == PERSON { to } else { from };
                self.close(agent);
            }
            Event::Renamed { from, to } => {
                for fold in &mut self.folds {
                    if fold.agent == *from {
                        fold.agent.clone_from(to);
                    }
                }
                if let Some(open) = self.open.remove(from) {
                    self.open.insert(to.clone(), open);
                }
                for (agent, _) in self.asked.values_mut() {
                    if agent == from {
                        agent.clone_from(to);
                    }
                }
            }
            Event::Joined { .. }
            | Event::Effort { .. }
            | Event::Text { .. }
            | Event::Spent { .. }
            | Event::Placed { said: None, .. } => {}
        }
    }

    fn step(&mut self, agent: &str, step: Step) {
        let id = match self.open.get(agent) {
            Some(id) => *id,
            None => {
                self.last += 1;
                self.folds.push_back(Fold {
                    id: self.last,
                    agent: agent.to_owned(),
                    steps: Vec::new(),
                    closed: false,
                });
                while self.folds.len() > KEPT {
                    if let Some(gone) = self.folds.pop_front() {
                        self.open.retain(|_, id| *id != gone.id);
                    }
                }
                self.open.insert(agent.to_owned(), self.last);
                self.last
            }
        };
        if let Some(fold) = self.fold_mut(id) {
            fold.steps.push(step);
        }
    }

    /// Closes `agent`'s fold, when it has one being made.
    fn close(&mut self, agent: &str) {
        if let Some(id) = self.open.remove(agent)
            && let Some(fold) = self.fold_mut(id)
        {
            fold.closed = true;
        }
    }

    fn fold_mut(&mut self, id: usize) -> Option<&mut Fold> {
        // Numbered in order, none missing but the oldest forgotten.
        let first = self.folds.front()?.id;
        self.folds.get_mut(id.checked_sub(first)?)
    }

    /// Every fold kept, the oldest first.
    pub fn folds(&self) -> impl DoubleEndedIterator<Item = &Fold> {
        self.folds.iter()
    }

    /// The fold numbered `id`, while it is kept.
    #[must_use]
    pub fn fold(&self, id: usize) -> Option<&Fold> {
        let first = self.folds.front()?.id;
        self.folds.get(id.checked_sub(first)?)
    }

    /// `agent`'s fold being made, while it works.
    #[must_use]
    pub fn open(&self, agent: &str) -> Option<&Fold> {
        self.fold(*self.open.get(agent)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(agent: &str, name: &str, ok: bool) -> Event {
        Event::Tool {
            agent: agent.into(),
            name: name.into(),
            input: "src/main.rs".into(),
            output: String::new(),
            ok,
        }
    }

    #[test]
    fn steps_fold_between_an_agents_lines_and_close_as_it_goes_on() {
        let mut steps = Steps::default();
        steps.event(&tool("lead", "read_file", true));
        steps.event(&tool("helper", "run_command", true));
        steps.event(&tool("lead", "read_file", true));
        steps.event(&Event::Asked {
            id: 7,
            agent: "lead".into(),
            action: "git push".into(),
            reason: "it leaves the machine".into(),
            widens: true,
        });
        steps.event(&Event::Answered {
            id: 7,
            approved: false,
        });
        steps.event(&tool("lead", "edit_file", false));
        let lead = steps.open("lead").unwrap();
        assert_eq!((lead.id, lead.steps.len(), lead.closed), (1, 4, false));
        assert_eq!(
            lead.kinds(),
            [
                ("read_file".to_owned(), 2),
                ("asked you".to_owned(), 1),
                ("edit_file".to_owned(), 1)
            ]
        );
        assert_eq!(lead.failed(), 2);
        // Its text closes it; the next step opens another.
        steps.event(&Event::TextStart {
            agent: "lead".into(),
        });
        assert!(steps.open("lead").is_none());
        assert!(steps.fold(1).unwrap().closed);
        assert_eq!(steps.open("helper").unwrap().id, 2);
        steps.event(&tool("lead", "run_command", true));
        assert_eq!(steps.open("lead").unwrap().id, 3);
        // The person writing to it closes it too, as does its turn's end.
        steps.event(&Event::Posted {
            from: PERSON.into(),
            to: "lead".into(),
            text: "and the tests".into(),
        });
        assert!(steps.open("lead").is_none());
        steps.event(&Event::Done {
            agent: "helper".into(),
            outcome: Ok(String::new()),
        });
        assert!(steps.folds().all(|fold| fold.closed));
        assert_eq!(steps.folds().count(), 3);
    }

    #[test]
    fn the_oldest_folds_are_forgotten_and_numbers_still_find_the_rest() {
        let mut steps = Steps::default();
        for _ in 0..KEPT + 3 {
            steps.event(&tool("lead", "read_file", true));
            steps.event(&Event::TextStart {
                agent: "lead".into(),
            });
        }
        assert_eq!(steps.folds().count(), KEPT);
        assert!(steps.fold(3).is_none());
        assert_eq!(steps.fold(4).unwrap().id, 4);
        assert_eq!(steps.fold(KEPT + 3).unwrap().id, KEPT + 3);
        assert!(steps.fold(KEPT + 4).is_none());
    }
}
