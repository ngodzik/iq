//! The crew as the window shows it: each agent with what it said and did,
//! the questions waiting for the person, the messages on the board, and
//! what the person writes.

use iq_agent::{Event, PERSON};
use iq_core::Usd;

/// What the person asks of the crew, for the program to carry out.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Start the lead agent on a task.
    Start {
        /// The task.
        task: String,
    },
    /// Write to an agent.
    Tell {
        /// The agent.
        to: String,
        /// The message.
        text: String,
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
    /// Its model.
    pub model: String,
    /// Who started it.
    pub by: String,
    /// Whether it works.
    pub working: bool,
    /// What it said and did, oldest first.
    pub thread: Vec<Entry>,
    /// What its calls were billed.
    pub billed: Usd,
    /// What its calls under a subscription would cost at list prices.
    pub estimated: Usd,
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
    selected: usize,
    questions: Vec<Question>,
    autopilot: bool,
    internet: bool,
    input: String,
    /// The lead's model and the reviewer's, as the program starts them.
    lead: String,
    reviewer: String,
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

    /// The agent shown, when there is one.
    #[must_use]
    pub fn selected(&self) -> Option<&AgentView> {
        self.agents.get(self.selected)
    }

    /// Shows the agent at `index`.
    pub fn select(&mut self, index: usize) {
        if index < self.agents.len() {
            self.selected = index;
        }
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

    /// What the person is writing, for the window to edit.
    pub fn input_mut(&mut self) -> &mut String {
        &mut self.input
    }

    /// Sends what the person wrote: the lead's task when no agent works
    /// yet, else a message to the agent shown.
    pub fn send(&mut self) -> Option<Request> {
        let text = self.input.trim().to_owned();
        if text.is_empty() {
            return None;
        }
        self.input.clear();
        Some(match self.selected() {
            None => Request::Start { task: text },
            Some(agent) => Request::Tell {
                to: agent.name.clone(),
                text,
            },
        })
    }

    /// Answers question `id`.
    pub fn answer(&mut self, id: u64, approved: bool) -> Request {
        self.questions.retain(|q| q.id != id);
        Request::Answer { id, approved }
    }

    /// Stops the agent shown.
    pub fn stop(&self) -> Option<Request> {
        self.selected().map(|agent| Request::Stop {
            agent: agent.name.clone(),
        })
    }

    /// What the crew just did.
    pub fn event(&mut self, event: Event) {
        match event {
            Event::Joined { agent, model, by } => {
                if self.agents.is_empty() || by == PERSON {
                    self.selected = self.agents.len();
                }
                self.agents.push(AgentView {
                    name: agent,
                    model,
                    by,
                    working: false,
                    thread: Vec::new(),
                    billed: Usd(0.0),
                    estimated: Usd(0.0),
                });
            }
            Event::Working { agent, prompt } => {
                if let Some(view) = self.agent(&agent) {
                    view.working = true;
                    view.thread.push(Entry::Prompt(prompt));
                }
            }
            Event::TextStart { agent } => {
                if let Some(view) = self.agent(&agent) {
                    view.thread.push(Entry::Text(String::new()));
                }
            }
            Event::Text { agent, text } => {
                if let Some(view) = self.agent(&agent) {
                    match view.thread.last_mut() {
                        Some(Entry::Text(current)) => current.push_str(&text),
                        _ => view.thread.push(Entry::Text(text)),
                    }
                }
            }
            Event::Spent {
                agent,
                cost,
                billed,
                ..
            } => {
                if let (Some(view), Some(cost)) = (self.agent(&agent), cost) {
                    if billed {
                        view.billed += cost;
                    } else {
                        view.estimated += cost;
                    }
                }
            }
            Event::Tool {
                agent,
                name,
                input,
                output,
                ok,
            } => {
                if let Some(view) = self.agent(&agent) {
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
                if let Some(view) = self.agent(&agent) {
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
                if let Some(view) = self.agent(&agent) {
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
                    && let Some(view) = self.agent(&from)
                {
                    view.thread.push(entry.clone());
                }
                if to != from
                    && let Some(view) = self.agent(&to)
                {
                    view.thread.push(entry);
                }
            }
            Event::Done { agent, outcome } => {
                if let Some(view) = self.agent(&agent) {
                    view.working = false;
                    view.thread.push(Entry::Done(outcome));
                }
            }
            Event::Notice { agent, text } => {
                if let Some(view) = self.agent(&agent) {
                    view.thread.push(Entry::Notice(text));
                }
            }
            Event::Quota(_) => {}
        }
    }

    fn agent(&mut self, name: &str) -> Option<&mut AgentView> {
        self.agents.iter_mut().find(|a| a.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_message_starts_the_lead_and_the_next_go_to_the_agent_shown() {
        let mut crew = CrewView::default();
        crew.input_mut().push_str("  fix the build ");
        assert_eq!(
            crew.send(),
            Some(Request::Start {
                task: "fix the build".into()
            })
        );
        assert_eq!(crew.send(), None);
        crew.event(Event::Joined {
            agent: "lead".into(),
            model: "m".into(),
            by: PERSON.into(),
        });
        crew.event(Event::Joined {
            agent: "helper".into(),
            model: "m".into(),
            by: "lead".into(),
        });
        // The lead stays shown: the helper was started by an agent.
        assert_eq!(crew.selected().unwrap().name, "lead");
        crew.input_mut().push_str("and the tests");
        assert_eq!(
            crew.send(),
            Some(Request::Tell {
                to: "lead".into(),
                text: "and the tests".into()
            })
        );
    }

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
        for (cost, billed) in [(0.25, true), (1.0, false)] {
            crew.event(Event::Spent {
                agent: "lead".into(),
                model: "m".into(),
                usage: iq_core::Usage::default(),
                cost: Some(Usd(cost)),
                billed,
            });
        }
        let lead = crew.selected().unwrap();
        assert!(lead.working);
        assert_eq!(lead.thread[1], Entry::Text("Hello".into()));
        assert!(matches!(
            lead.thread[2],
            Entry::Asked {
                answer: Some(true),
                ..
            }
        ));
        assert_eq!((lead.billed, lead.estimated), (Usd(0.25), Usd(1.0)));
        assert_eq!(crew.toggle_autopilot(), Request::Autopilot(true));
        assert_eq!(crew.toggle_internet(), Request::Internet(true));
        assert!(crew.internet());
    }
}
