//! What the agents do between two of their lines of text: the tools they
//! call, what the person and the reviewer allow or refuse them. An agent's
//! steps make one fold from its last line to its next: while it works, the
//! last of them show as they come; once it writes again, or its turn ends,
//! the fold closes and only its count is said, the steps kept to be
//! unfolded on their page. And what the agents write to each other (the
//! task one is started on, a message, an answer), which the conversation
//! sums up and their page gives whole, numbered with the folds.

use std::collections::{HashMap, HashSet, VecDeque};

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

/// What one agent wrote to another: the task it started it on, a
/// message, an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    /// Its number, counted with the folds'.
    pub id: usize,
    /// Its writer.
    pub from: String,
    /// Its reader: an agent, or all.
    pub to: String,
    /// What it wrote, whole.
    pub text: String,
    /// Whether it is the task the reader was started on.
    pub task: bool,
}

/// A command an agent left to run in the background, once it ended: the
/// conversation says it in a line, its page gives it whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// Its number, counted with the folds'.
    pub id: usize,
    /// The agent.
    pub agent: String,
    /// The job's own number, as its agent names it.
    pub job: u64,
    /// The command line, whole.
    pub line: String,
    /// How it ended.
    pub said: String,
    /// The end of what it wrote.
    pub output: String,
}

/// One of the steps' page's entries: a fold, a message between agents, a
/// job that ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry<'a> {
    /// An agent's steps between two of its lines.
    Fold(&'a Fold),
    /// What one agent wrote to another.
    Said(&'a Said),
    /// A command run in the background.
    Ran(&'a Ran),
}

impl Entry<'_> {
    /// Its number.
    #[must_use]
    pub fn id(&self) -> usize {
        match self {
            Entry::Fold(fold) => fold.id,
            Entry::Said(said) => said.id,
            Entry::Ran(ran) => ran.id,
        }
    }
}

/// The agents' steps, fold by fold, and what they wrote to each other.
#[derive(Debug, Clone, Default)]
pub struct Steps {
    folds: VecDeque<Fold>,
    /// What agents wrote to each other, the oldest first.
    said: VecDeque<Said>,
    /// The jobs that ended, the oldest first.
    ran: VecDeque<Ran>,
    /// The command line of each job running, by its number.
    jobs: HashMap<u64, String>,
    /// The number of the last fold or message made.
    last: usize,
    /// Who started each agent another started.
    started: HashMap<String, String>,
    /// The agents whose task was taken.
    tasked: HashSet<String>,
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
            Event::Working { agent, prompt } => {
                self.close(agent);
                // Its first is the task it was started on, unless a
                // teammate's message woke it, taken as it was posted.
                if self.tasked.insert(agent.clone())
                    && !prompt.starts_with("Message from ")
                    && let Some(by) = self.started.get(agent).cloned()
                {
                    self.said(&by, agent, prompt, true);
                }
            }
            Event::TextStart { agent }
            | Event::Done { agent, .. }
            | Event::Failed { agent, .. }
            | Event::Retired { agent }
            | Event::Notice { agent, .. } => self.close(agent),
            Event::Placed {
                agent,
                said: Some(_),
                ..
            } => self.close(agent),
            Event::Posted { from, to, text } => {
                // What the person writes to an agent comes after what it
                // did before.
                let agent = if from == PERSON { to } else { from };
                self.close(agent);
                if from != PERSON && to != PERSON && from != to {
                    self.said(from, to, text, false);
                }
            }
            // It waits, for its agents or the person: what it did before
            // is still its fold.
            Event::Waiting { .. }
            | Event::Choose { .. }
            | Event::Chosen { .. }
            | Event::JobOutput { .. } => {}
            Event::JobStarted { id, line, .. } => {
                self.jobs.insert(*id, line.clone());
            }
            Event::JobEnded {
                agent,
                id,
                said,
                output,
            } => {
                let line = self.jobs.remove(id).unwrap_or_default();
                self.last += 1;
                self.ran.push_back(Ran {
                    id: self.last,
                    agent: agent.clone(),
                    job: *id,
                    line,
                    said: said.clone(),
                    output: output.clone(),
                });
                while self.ran.len() > KEPT {
                    self.ran.pop_front();
                }
            }
            Event::Renamed { from, to } => {
                for fold in &mut self.folds {
                    if fold.agent == *from {
                        fold.agent.clone_from(to);
                    }
                }
                for said in &mut self.said {
                    for name in [&mut said.from, &mut said.to] {
                        if name == from {
                            name.clone_from(to);
                        }
                    }
                }
                for ran in &mut self.ran {
                    if ran.agent == *from {
                        ran.agent.clone_from(to);
                    }
                }
                for by in self.started.values_mut() {
                    if by == from {
                        by.clone_from(to);
                    }
                }
                if let Some(by) = self.started.remove(from) {
                    self.started.insert(to.clone(), by);
                }
                if self.tasked.remove(from) {
                    self.tasked.insert(to.clone());
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
            Event::Joined { agent, by, .. } => {
                if by != PERSON {
                    self.started.insert(agent.clone(), by.clone());
                }
            }
            Event::Effort { .. }
            | Event::Model { .. }
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

    /// Keeps what `from` wrote to `to`, under the next number.
    fn said(&mut self, from: &str, to: &str, text: &str, task: bool) {
        self.last += 1;
        self.said.push_back(Said {
            id: self.last,
            from: from.to_owned(),
            to: to.to_owned(),
            text: text.to_owned(),
            task,
        });
        while self.said.len() > KEPT {
            self.said.pop_front();
        }
    }

    fn fold_mut(&mut self, id: usize) -> Option<&mut Fold> {
        // Numbered in order, the messages' numbers between them.
        let at = self.folds.binary_search_by_key(&id, |fold| fold.id).ok()?;
        self.folds.get_mut(at)
    }

    /// Every fold kept, the oldest first.
    pub fn folds(&self) -> impl DoubleEndedIterator<Item = &Fold> {
        self.folds.iter()
    }

    /// The fold numbered `id`, while it is kept.
    #[must_use]
    pub fn fold(&self, id: usize) -> Option<&Fold> {
        let at = self.folds.binary_search_by_key(&id, |fold| fold.id).ok()?;
        self.folds.get(at)
    }

    /// What agents wrote to each other, kept, the oldest first.
    pub fn said_all(&self) -> impl DoubleEndedIterator<Item = &Said> {
        self.said.iter()
    }

    /// The message numbered `id`, while it is kept.
    #[must_use]
    pub fn said_by_id(&self, id: usize) -> Option<&Said> {
        let at = self.said.binary_search_by_key(&id, |said| said.id).ok()?;
        self.said.get(at)
    }

    /// The last message between agents kept: the one the event just taken
    /// made, when it made one.
    #[must_use]
    pub fn latest_said(&self) -> Option<&Said> {
        self.said.back()
    }

    /// The job that ended last: the one the event just taken ended, when
    /// it ended one.
    #[must_use]
    pub fn latest_ran(&self) -> Option<&Ran> {
        self.ran.back()
    }

    /// Every fold, message and job kept, in the order they came.
    #[must_use]
    pub fn entries(&self) -> Vec<Entry<'_>> {
        let mut entries: Vec<Entry<'_>> = self
            .folds
            .iter()
            .map(Entry::Fold)
            .chain(self.said.iter().map(Entry::Said))
            .chain(self.ran.iter().map(Entry::Ran))
            .collect();
        entries.sort_by_key(Entry::id);
        entries
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
    fn what_agents_write_each_other_is_kept_numbered_with_the_folds() {
        let mut steps = Steps::default();
        let joined = |agent: &str, by: &str| Event::Joined {
            agent: agent.into(),
            model: "lab/m".into(),
            by: by.into(),
        };
        let working = |prompt: &str| Event::Working {
            agent: "fix/tests".into(),
            prompt: prompt.into(),
        };
        let posted = |from: &str, to: &str, text: &str| Event::Posted {
            from: from.into(),
            to: to.into(),
            text: text.into(),
        };
        steps.event(&joined("fix", PERSON));
        steps.event(&Event::Working {
            agent: "fix".into(),
            prompt: "fix the build".into(),
        });
        steps.event(&tool("fix", "read_file", true));
        steps.event(&joined("fix/tests", "fix"));
        steps.event(&working("write the tests"));
        steps.event(&tool("fix/tests", "run_command", true));
        steps.event(&posted("fix/tests", "fix", "3 pass"));
        // A wake by a message, and what the person writes, are no more.
        steps.event(&working("Message from fix: and the docs"));
        steps.event(&posted(PERSON, "fix", "go on"));
        let ids: Vec<usize> = steps.entries().iter().map(Entry::id).collect();
        assert_eq!(ids, [1, 2, 3, 4]);
        let task = steps.said_by_id(2).unwrap();
        assert!(task.task);
        assert_eq!(
            (task.from.as_str(), task.to.as_str(), task.text.as_str()),
            ("fix", "fix/tests", "write the tests")
        );
        assert!(!steps.said_by_id(4).unwrap().task);
        assert_eq!(steps.latest_said().unwrap().id, 4);
        assert_eq!(steps.fold(3).unwrap().agent, "fix/tests");
        assert!(steps.fold(3).unwrap().closed);
        // Renamed, its messages follow it.
        steps.event(&Event::Renamed {
            from: "fix".into(),
            to: "fix-build".into(),
        });
        assert_eq!(steps.said_by_id(4).unwrap().to, "fix-build");
        assert_eq!(steps.said_by_id(2).unwrap().from, "fix-build");
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
