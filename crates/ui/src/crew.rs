//! The crew as the window shows it: each agent with what it said and did,
//! the questions waiting for the person, the messages on the board, and
//! what the person writes.

use iq_agent::{Event, PERSON, Pattern};
use iq_core::{Effort, TokenCount, Usd};

/// What the person asks of the crew, for the program to carry out.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Start an agent on a task.
    Start {
        /// The name asked for it.
        name: String,
        /// Its model, one of the provider's.
        model: String,
        /// The task.
        task: String,
        /// Its part in the crew, when given one.
        role: Option<String>,
        /// How its team works.
        pattern: Pattern,
        /// The second model, for a pattern that takes one: the reviewer's,
        /// a second voice's.
        partner: Option<String>,
        /// The models the person named in the task: the first is the one
        /// it starts on.
        named: Vec<String>,
        /// How hard it thinks, when the person chose; else as the
        /// settings say.
        effort: Option<Effort>,
    },
    /// Have an agent think with this effort from its next call.
    Effort {
        /// The agent.
        agent: String,
        /// Its effort.
        effort: Effort,
    },
    /// Write to an agent.
    Tell {
        /// The agent.
        to: String,
        /// The message.
        text: String,
        /// The models the person named in it, for the agent to use them:
        /// "with opus".
        named: Vec<String>,
    },
    /// Answer a question.
    Answer {
        /// The question.
        id: u64,
        /// Whether it is approved.
        approved: bool,
    },
    /// Let the reviewer answer for the person, or not.
    Autopilot(bool),
    /// Give every command the internet, or only what a rule gives it to
    /// and what the person allows.
    Internet(bool),
    /// Stop an agent's turn.
    Stop {
        /// The agent.
        agent: String,
    },
    /// Sync iq's data with the repository, or not, as asked.
    Sync {
        /// Whether what changed is sent now.
        send: bool,
        /// Whether what changes is sent from now on without asking.
        always: bool,
    },
}

/// One thing in an agent's thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// What it was given: its task, a message that woke it.
    Prompt(String),
    /// What it said.
    Text(String),
    /// A tool it called.
    Tool {
        /// The tool.
        name: String,
        /// Its input, in short.
        input: String,
        /// What it returned.
        output: String,
        /// Whether it succeeded.
        ok: bool,
    },
    /// What the reviewer answered for the person.
    Reviewed {
        /// The action.
        action: String,
        /// Whether it was approved.
        approved: bool,
        /// Why.
        why: String,
    },
    /// What it asked the person, and the answer once given.
    Asked {
        /// The question.
        id: u64,
        /// The action.
        action: String,
        /// Why the gate asked.
        reason: String,
        /// The answer, once given.
        answer: Option<bool>,
    },
    /// A message it wrote or was written.
    Message {
        /// Its writer.
        from: String,
        /// Its reader.
        to: String,
        /// The message.
        text: String,
    },
    /// Something to know about how it works.
    Notice(String),
    /// Its turn ended: its answer, or why it failed.
    Done(Result<String, String>),
}

/// An agent of the crew, as shown.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentView {
    /// Its name.
    pub name: String,
    /// Its model, as it was started on: `anthropic/claude-opus-5-5`.
    pub model: String,
    /// The model that answered its last call; empty before its first.
    pub answered: String,
    /// Who started it.
    pub by: String,
    /// Whether it works.
    pub working: bool,
    /// What it said and did, oldest first.
    pub thread: Vec<Entry>,
    /// What its calls were billed.
    pub billed: Usd,
    /// What each of its calls read, oldest first: the last is how much of
    /// its model's context its conversation fills.
    pub reads: Vec<TokenCount>,
    /// How hard it thinks; `None` leaves it to its model.
    pub effort: Option<Effort>,
    /// Where it works, when it was given a place of its own.
    pub place: Option<std::path::PathBuf>,
}

impl AgentView {
    /// What its last call read: how much of its model's context its
    /// conversation fills.
    #[must_use]
    pub fn context(&self) -> TokenCount {
        self.reads.last().copied().unwrap_or(TokenCount(0))
    }
}

/// A question waiting for the person.
#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    /// Its number.
    pub id: u64,
    /// The agent that asks.
    pub agent: String,
    /// The action.
    pub action: String,
    /// Why the gate asks.
    pub reason: String,
    /// Whether it gives more than the sandbox holds.
    pub widens: bool,
}

/// The crew as the window holds it.
#[derive(Debug, Clone, Default)]
pub struct CrewView {
    agents: Vec<AgentView>,
    questions: Vec<Question>,
    autopilot: bool,
    internet: bool,
    /// The lead's model and the reviewer's, as the program starts them.
    lead: String,
    reviewer: String,
}

impl AgentView {
    /// Its model, as precisely as known: the one that answered last, else
    /// the one it was started on.
    #[must_use]
    pub fn serving(&self) -> &str {
        if self.answered.is_empty() {
            &self.model
        } else {
            &self.answered
        }
    }
}

impl CrewView {
    /// How the program starts the crew: the lead's model, the
    /// reviewer's, whether the reviewer answers for the person, and
    /// whether every command has the internet.
    pub fn started(&mut self, lead: String, reviewer: String, autopilot: bool, internet: bool) {
        self.lead = lead;
        self.reviewer = reviewer;
        self.autopilot = autopilot;
        self.internet = internet;
    }

    /// The lead's model.
    #[must_use]
    pub fn lead(&self) -> &str {
        &self.lead
    }

    /// The reviewer's model.
    #[must_use]
    pub fn reviewer(&self) -> &str {
        &self.reviewer
    }

    /// The agents, in the order they joined.
    #[must_use]
    pub fn agents(&self) -> &[AgentView] {
        &self.agents
    }

    /// The agent named `name`.
    #[must_use]
    pub fn agent(&self, name: &str) -> Option<&AgentView> {
        self.agents.iter().find(|a| a.name == name)
    }

    /// The questions waiting for the person, oldest first.
    #[must_use]
    pub fn questions(&self) -> &[Question] {
        &self.questions
    }

    /// Whether the reviewer answers for the person.
    #[must_use]
    pub fn autopilot(&self) -> bool {
        self.autopilot
    }

    /// Turns the autopilot on or off.
    pub fn toggle_autopilot(&mut self) -> Request {
        self.autopilot = !self.autopilot;
        Request::Autopilot(self.autopilot)
    }

    /// Whether every command has the internet.
    #[must_use]
    pub fn internet(&self) -> bool {
        self.internet
    }

    /// Opens the internet to every command, or closes it to all but what a
    /// rule gives it to and what the person allows.
    pub fn toggle_internet(&mut self) -> Request {
        self.internet = !self.internet;
        Request::Internet(self.internet)
    }

    /// Answers question `id`.
    pub fn answer(&mut self, id: u64, approved: bool) -> Request {
        self.questions.retain(|q| q.id != id);
        Request::Answer { id, approved }
    }

    /// Stops `agent`.
    #[must_use]
    pub fn stop(&self, agent: &str) -> Option<Request> {
        self.agent(agent)
            .filter(|a| a.working)
            .map(|a| Request::Stop {
                agent: a.name.clone(),
            })
    }

    /// What the crew just did.
    pub fn event(&mut self, event: Event) {
        match event {
            Event::Joined { agent, model, by } => {
                self.agents.push(AgentView {
                    name: agent,
                    model,
                    answered: String::new(),
                    by,
                    working: false,
                    thread: Vec::new(),
                    billed: Usd(0.0),
                    reads: Vec::new(),
                    effort: None,
                    place: None,
                });
            }
            Event::Working { agent, prompt } => {
                if let Some(view) = self.agent_mut(&agent) {
                    view.working = true;
                    view.thread.push(Entry::Prompt(prompt));
                }
            }
            Event::TextStart { agent } => {
                if let Some(view) = self.agent_mut(&agent) {
                    view.thread.push(Entry::Text(String::new()));
                }
            }
            Event::Text { agent, text } => {
                if let Some(view) = self.agent_mut(&agent) {
                    match view.thread.last_mut() {
                        Some(Entry::Text(current)) => current.push_str(&text),
                        _ => view.thread.push(Entry::Text(text)),
                    }
                }
            }
            Event::Spent {
                agent,
                model,
                cost,
                usage,
            } => {
                if let Some(view) = self.agent_mut(&agent) {
                    if !model.is_empty() {
                        view.answered = model;
                    }
                    if let Some(cost) = cost {
                        view.billed += cost;
                    }
                    view.reads.push(usage.input);
                }
            }
            Event::Tool {
                agent,
                name,
                input,
                output,
                ok,
            } => {
                if let Some(view) = self.agent_mut(&agent) {
                    view.thread.push(Entry::Tool {
                        name,
                        input,
                        output,
                        ok,
                    });
                }
            }
            Event::Asked {
                id,
                agent,
                action,
                reason,
                widens,
            } => {
                if let Some(view) = self.agent_mut(&agent) {
                    view.thread.push(Entry::Asked {
                        id,
                        action: action.clone(),
                        reason: reason.clone(),
                        answer: None,
                    });
                }
                self.questions.push(Question {
                    id,
                    agent,
                    action,
                    reason,
                    widens,
                });
            }
            Event::Answered { id, approved } => {
                self.questions.retain(|q| q.id != id);
                for view in &mut self.agents {
                    for entry in &mut view.thread {
                        if let Entry::Asked {
                            id: asked, answer, ..
                        } = entry
                            && *asked == id
                        {
                            *answer = Some(approved);
                        }
                    }
                }
            }
            Event::Reviewed {
                agent,
                action,
                approved,
                why,
                ..
            } => {
                if let Some(view) = self.agent_mut(&agent) {
                    view.thread.push(Entry::Reviewed {
                        action,
                        approved,
                        why,
                    });
                }
            }
            Event::Posted { from, to, text } => {
                let entry = Entry::Message {
                    from: from.clone(),
                    to: to.clone(),
                    text,
                };
                // The writer's thread, and the reader's when it is another
                // agent; a message of the person's shows as the prompt it
                // wakes, or in the reader's thread.
                if from != PERSON
                    && let Some(view) = self.agent_mut(&from)
                {
                    view.thread.push(entry.clone());
                }
                if to != from
                    && let Some(view) = self.agent_mut(&to)
                {
                    view.thread.push(entry);
                }
            }
            Event::Done { agent, outcome } => {
                if let Some(view) = self.agent_mut(&agent) {
                    view.working = false;
                    view.thread.push(Entry::Done(outcome));
                }
            }
            Event::Notice { agent, text } => {
                if let Some(view) = self.agent_mut(&agent) {
                    view.thread.push(Entry::Notice(text));
                }
            }
            Event::Placed {
                agent,
                folder,
                said,
            } => {
                if let Some(view) = self.agent_mut(&agent) {
                    view.place = Some(folder);
                    if let Some(said) = said {
                        view.thread.push(Entry::Notice(format!("{agent} {said}")));
                    }
                }
            }
            Event::Effort { agent, effort } => {
                if let Some(view) = self.agent_mut(&agent) {
                    view.effort = effort;
                }
            }
        }
    }

    fn agent_mut(&mut self, name: &str) -> Option<&mut AgentView> {
        self.agents.iter_mut().find(|a| a.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_gathers_text_tools_questions_and_costs() {
        let mut crew = CrewView::default();
        crew.event(Event::Joined {
            agent: "lead".into(),
            model: "m".into(),
            by: PERSON.into(),
        });
        crew.event(Event::Working {
            agent: "lead".into(),
            prompt: "go".into(),
        });
        crew.event(Event::TextStart {
            agent: "lead".into(),
        });
        crew.event(Event::Text {
            agent: "lead".into(),
            text: "Hel".into(),
        });
        crew.event(Event::Text {
            agent: "lead".into(),
            text: "lo".into(),
        });
        crew.event(Event::Asked {
            id: 7,
            agent: "lead".into(),
            action: "run `git push`".into(),
            reason: "it publishes commits".into(),
            widens: true,
        });
        assert_eq!(crew.questions().len(), 1);
        assert_eq!(
            crew.answer(7, true),
            Request::Answer {
                id: 7,
                approved: true
            }
        );
        assert!(crew.questions().is_empty());
        crew.event(Event::Answered {
            id: 7,
            approved: true,
        });
        for (cost, read) in [(0.25, 1200), (1.0, 1800)] {
            crew.event(Event::Spent {
                agent: "lead".into(),
                model: "m".into(),
                usage: iq_core::Usage {
                    input: TokenCount(read),
                    output: TokenCount(300),
                },
                cost: Some(Usd(cost)),
            });
        }
        let lead = crew.agent("lead").unwrap();
        assert!(lead.working);
        assert_eq!(lead.thread[1], Entry::Text("Hello".into()));
        assert!(matches!(
            lead.thread[2],
            Entry::Asked {
                answer: Some(true),
                ..
            }
        ));
        // Its calls add up; its context is what its last call read.
        assert_eq!((lead.billed, lead.context()), (Usd(1.25), TokenCount(1800)));
        assert_eq!(crew.toggle_autopilot(), Request::Autopilot(true));
        assert_eq!(crew.toggle_internet(), Request::Internet(true));
        assert!(crew.internet());
        assert_eq!(
            crew.stop("lead"),
            Some(Request::Stop {
                agent: "lead".into()
            })
        );
        assert_eq!(crew.stop("nobody"), None);
    }
}
