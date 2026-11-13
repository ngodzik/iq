//! The crew as the window shows it: each agent with what it said and did,
//! the questions waiting for the person, the messages on the board, and
//! what the person writes.

use std::collections::HashMap;

use iq_agent::{Event, PERSON, Pattern};
use iq_core::{Effort, Message, TokenCount, Usd};

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
        /// Whether its name was made from its task's first words, for a
        /// model to name it after what its work is about.
        from_words: bool,
        /// The images the person attached to the task, by their paths.
        images: Vec<std::path::PathBuf>,
        /// How its model was chosen, as the audit log and the chat keep
        /// it: named in the task, by `/new` or `/model`, or the settings'.
        how: String,
    },
    /// Have an agent answered by this model from its next turn.
    Model {
        /// The agent.
        agent: String,
        /// Its model, one of the provider's.
        model: String,
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
        /// The images the person attached to it, by their paths.
        images: Vec<std::path::PathBuf>,
    },
    /// Answer a question with choices: a choice, or the person's own words;
    /// none when they gave none.
    Choose {
        /// The question.
        id: u64,
        /// Their answer.
        answer: Option<String>,
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
    /// Stop a command run in the background.
    StopJob {
        /// Its number.
        id: u64,
    },
    /// Stop an agent's turn.
    Stop {
        /// The agent.
        agent: String,
    },
    /// Give an agent another name.
    Rename {
        /// The agent.
        agent: String,
        /// The name the person gives it.
        name: String,
    },
    /// Sync iq's data with the repository, or not, as asked.
    Sync {
        /// Whether what changed is sent now.
        send: bool,
        /// Whether what changes is sent from now on without asking.
        always: bool,
    },
    /// Read what a sync would send, line by line, for the page that shows
    /// it.
    Changes,
    /// Run a plugin's command.
    Plugin {
        /// The plugin.
        plugin: String,
        /// The command, as the plugin named it.
        command: String,
        /// What the person wrote after it, trimmed.
        args: String,
        /// The chat the person writes to, when they write to one.
        chat: Option<String>,
        /// Its place of its own, when it has one.
        folder: Option<std::path::PathBuf>,
    },
    /// Say how each MCP server the settings declare is.
    Mcp,
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
    /// The model its turns answered on from here, as a chat of an earlier
    /// session says.
    Model(String),
    /// Its turn ended: its answer, or why it failed.
    Done(Result<String, String>),
    /// What it asked the person to choose, and what they answered.
    Chose {
        /// The question.
        question: String,
        /// Their answer; none when they gave none.
        answer: Option<String>,
    },
}

/// What a chat's turn spent so far, from when the person wrote to it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TurnView {
    /// Its number, from 1, in this session.
    pub number: u64,
    /// When it began, in seconds since 1970.
    pub since: u64,
    /// What it spent, as far as prices are known.
    pub total: Usd,
    /// What each who spent in it spent, in the order they first did: its
    /// agents, the autopilot's model, the summaries.
    pub by: Vec<TurnSpending>,
    /// What it may spend before the person is asked whether it goes on;
    /// none never.
    pub ask_at: Option<Usd>,
}

/// What a model read this session, and how much of it came from the
/// provider's cache: what tells whether the cache serves it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CacheView {
    /// The model.
    pub model: String,
    /// Its calls.
    pub calls: u64,
    /// The tokens they read.
    pub read: u64,
    /// Of those, what came from the cache, as far as told.
    pub cached: u64,
    /// Whether the provider told what came from the cache at all.
    pub told: bool,
}

/// What one spent in a chat's turn.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TurnSpending {
    /// Who: an agent, `reviewer`, `summaries`.
    pub who: String,
    /// The model it called last.
    pub model: String,
    /// Its calls.
    pub calls: u64,
    /// What they cost.
    pub spent: Usd,
    /// The tokens they read.
    pub read: u64,
    /// Of those, what came from the provider's cache, as far as told.
    pub cached: u64,
}

/// A command an agent left to run in the background, while it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobView {
    /// The agent.
    pub agent: String,
    /// Its number.
    pub id: u64,
    /// The command line.
    pub line: String,
    /// Since when, in seconds since 1970.
    pub since: u64,
    /// The latest lines it wrote, the latest last.
    pub tail: Vec<String>,
}

/// A question with choices an agent asks the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// Its number, to answer it with.
    pub id: u64,
    /// The agent that asks.
    pub agent: String,
    /// The question.
    pub question: String,
    /// What it offers.
    pub choices: Vec<String>,
    /// Who raised it, when the agent asks for agents it started: their
    /// names, through whom, and what each spent.
    pub raised: Option<String>,
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
    /// What it was first asked: its task.
    pub task: String,
    /// When it last worked, in seconds since 1970; never yet when none.
    pub worked: Option<u64>,
    /// Whom it waits for, while it waits for the agents it started.
    pub waiting: Option<Waiting>,
    /// Whether it is done with the task another gave it: off the board,
    /// its name free for the next, what it did still read here.
    pub retired: bool,
    /// The tokens past which a call of its model costs more, when told.
    pub rises: Option<u64>,
    /// The most its model reads at once, when told.
    pub most: Option<u64>,
}

/// An agent waiting for the agents it started to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    /// Those it waits for.
    pub on: Vec<String>,
    /// Since when, in seconds since 1970.
    pub since: u64,
}

/// An agent retired in an earlier session, shown again with its chat:
/// what it did, read only.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    /// Its name as it retired.
    pub agent: String,
    /// Its model.
    pub model: String,
    /// Who started it, by the name it has now.
    pub by: String,
    /// What it was asked.
    pub task: String,
    /// When it last worked, in seconds since 1970.
    pub worked: u64,
    /// What it was given, said and did, oldest first.
    pub thread: Vec<Entry>,
}

/// A chat of an earlier session, back on the board as it was kept.
#[derive(Debug, Clone, PartialEq)]
pub struct Back {
    /// Its name now.
    pub agent: String,
    /// What it was first asked.
    pub task: String,
    /// When it last worked, in seconds since 1970.
    pub worked: u64,
    /// When the person last wrote to it, if they did.
    pub written: Option<u64>,
    /// What it was given, said and did, oldest first.
    pub thread: Vec<Entry>,
}

/// An agent's thread as its conversation holds it, for a chat that comes
/// back: what it was given, what it said and the tools it called, with
/// what they returned. iq's own words to it (its conversation's size, a
/// nudge to go on) are left out.
///
/// # Examples
///
/// ```
/// use iq_core::Message;
/// use iq_ui::crew::{Entry, past};
///
/// let kept = [
///     Message::system("You are lead"),
///     Message::user("fix the login"),
///     Message::Assistant {
///         content: Some("Fixed.".into()),
///         tool_calls: Vec::new(),
///         reasoning_signature: None,
///     },
/// ];
/// assert_eq!(
///     past(&kept),
///     [Entry::Prompt("fix the login".into()), Entry::Text("Fixed.".into())]
/// );
/// ```
#[must_use]
pub fn past(messages: &[Message]) -> Vec<Entry> {
    past_with(messages, &[])
}

/// What each tool call of `messages` returned, by the call's id.
fn returned(messages: &[Message]) -> std::collections::HashMap<&str, &str> {
    messages
        .iter()
        .filter_map(|message| match message {
            Message::Tool { call_id, content } => Some((call_id.as_str(), content.as_str())),
            _ => None,
        })
        .collect()
}

/// What `message` puts in the thread, the tools' calls with what they
/// `returned`.
fn entries(message: &Message, returned: &std::collections::HashMap<&str, &str>) -> Vec<Entry> {
    let mut thread = Vec::new();
    match message {
        Message::User(text) if !text.starts_with("(iq: ") && !text.starts_with(URGED) => {
            thread.push(Entry::Prompt(text.clone()));
        }
        Message::Assistant {
            content,
            tool_calls,
            ..
        } => {
            if let Some(text) = content.as_ref().filter(|t| !t.trim().is_empty()) {
                thread.push(Entry::Text(text.clone()));
            }
            for call in tool_calls {
                let output = returned.get(call.id.as_str()).copied().unwrap_or_default();
                let input = iq_agent::tool_input(&call.arguments);
                thread.push(Entry::Tool {
                    name: call.name.clone(),
                    input,
                    output: output.to_owned(),
                    ok: !output.starts_with("Error: "),
                });
            }
        }
        _ => {}
    }
    thread
}

/// The same, with what the person was shown beside it (iq's notices, the
/// questions and their answers, what the gate asked and the autopilot
/// answered, the errors), each where it came among the messages.
#[must_use]
pub fn past_with(messages: &[Message], aside: &[iq_agent::Aside]) -> Vec<Entry> {
    let returned = returned(messages);
    let mut thread = Vec::new();
    let mut aside = aside.iter().peekable();
    for at in 0..=messages.len() {
        while let Some(shown) = aside.next_if(|shown| shown.after <= at) {
            thread.push(set_aside(&shown.shown));
        }
        if let Some(message) = messages.get(at) {
            thread.extend(entries(message, &returned));
        }
    }
    thread.extend(aside.map(|shown| set_aside(&shown.shown)));
    thread
}

/// What was set aside, as the thread shows it.
fn set_aside(shown: &iq_agent::Shown) -> Entry {
    use iq_agent::Shown;
    match shown {
        Shown::Notice { text } => Entry::Notice(text.clone()),
        Shown::Model { model } => Entry::Model(model.clone()),
        Shown::Question {
            question, answer, ..
        } => Entry::Chose {
            question: question.clone(),
            answer: answer.clone(),
        },
        Shown::Asked {
            id,
            action,
            reason,
            approved,
        } => Entry::Asked {
            id: *id,
            action: action.clone(),
            reason: reason.clone(),
            answer: *approved,
        },
        Shown::Reviewed {
            action,
            approved,
            why,
        } => Entry::Reviewed {
            action: action.clone(),
            approved: *approved,
            why: why.clone(),
        },
        Shown::Failed {
            model,
            status,
            message,
            ..
        } => Entry::Done(Err(match status {
            Some(status) => format!("{model} answered HTTP {status}: {message}"),
            None => format!("{model} could not be reached: {message}"),
        })),
        Shown::Error { text } => Entry::Done(Err(text.clone())),
    }
}

/// How iq urges on an agent that answered with nothing.
const URGED: &str = "Your reply held neither text nor a tool call.";

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
    /// The agents retired, the latest last.
    finished: Vec<AgentView>,
    /// Each chat's turn, the one under way or its last, by its chat.
    turns: HashMap<String, TurnView>,
    /// What each model read this session, and from the cache, in the order
    /// they were first called.
    cached: Vec<CacheView>,
    questions: Vec<Question>,
    /// The questions with choices waiting for the person, the oldest first.
    choices: Vec<Choice>,
    /// The commands agents left to run in the background, while they run.
    jobs: Vec<JobView>,
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

    /// What each model read this session, and from the cache.
    #[must_use]
    pub fn cached(&self) -> &[CacheView] {
        &self.cached
    }

    /// The turn of `chat` under way, or its last.
    #[must_use]
    pub fn turn(&self, chat: &str) -> Option<&TurnView> {
        self.turns.get(chat)
    }

    /// What every agent's calls were billed this session, those retired
    /// with the others.
    #[must_use]
    pub fn billed(&self) -> Usd {
        let mut billed = Usd(0.0);
        for agent in self.agents.iter().chain(&self.finished) {
            billed += agent.billed;
        }
        billed
    }

    /// The agents retired, done with the task another gave them, the
    /// latest last.
    #[must_use]
    pub fn finished(&self) -> &[AgentView] {
        &self.finished
    }

    /// The agent named `name`.
    #[must_use]
    pub fn agent(&self, name: &str) -> Option<&AgentView> {
        self.agents.iter().find(|a| a.name == name)
    }

    /// The questions with choices waiting for the person, the oldest
    /// first.
    #[must_use]
    pub fn choices(&self) -> &[Choice] {
        &self.choices
    }

    /// The commands agents left to run in the background, while they run.
    #[must_use]
    pub fn jobs(&self) -> &[JobView] {
        &self.jobs
    }

    /// The person's chat `agent` is part of: itself when the person started
    /// it, else its starter's.
    #[must_use]
    pub fn chat_of(&self, agent: &str) -> Option<String> {
        let mut name = agent;
        for _ in 0..self.agents.len() + 1 {
            let view = self.agent(name)?;
            if view.by == PERSON {
                return Some(view.name.clone());
            }
            name = &view.by;
        }
        None
    }

    /// The chats the person started, in the order they joined: those they
    /// write to.
    pub fn chats(&self) -> impl Iterator<Item = &AgentView> {
        self.agents.iter().filter(|a| a.by == PERSON)
    }

    /// A chat of an earlier session came back: its task, when it last
    /// worked, and its thread as it was kept.
    pub fn back(&mut self, back: Back) {
        if let Some(view) = self.agent_mut(&back.agent) {
            view.task = back.task;
            view.worked = Some(back.worked);
            view.thread = back.thread;
        }
    }

    /// An agent retired in an earlier session, shown with its chat.
    pub fn finished_back(&mut self, finished: Finished) {
        self.finished.push(AgentView {
            name: finished.agent,
            model: finished.model,
            answered: String::new(),
            by: finished.by,
            working: false,
            thread: finished.thread,
            // Billed in an earlier session, not this one's.
            billed: Usd(0.0),
            reads: Vec::new(),
            effort: None,
            place: None,
            task: finished.task,
            worked: Some(finished.worked),
            waiting: None,
            retired: true,
            rises: None,
            most: None,
        });
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
                    task: String::new(),
                    worked: None,
                    waiting: None,
                    retired: false,
                    rises: None,
                    most: None,
                });
            }
            Event::Retired { agent } => {
                if let Some(at) = self.agents.iter().position(|a| a.name == agent) {
                    let mut view = self.agents.remove(at);
                    view.retired = true;
                    view.working = false;
                    view.waiting = None;
                    self.finished.push(view);
                }
            }
            // Its turn's end says it, with the facts.
            Event::Failed { .. } => {}
            // What a file holds now is the conversation's to show.
            Event::Changed { .. } => {}
            Event::Working { agent, prompt } => {
                if let Some(view) = self.agent_mut(&agent) {
                    view.working = true;
                    view.worked = Some(crate::now());
                    if view.task.is_empty() {
                        view.task.clone_from(&prompt);
                    }
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
            // The same turn said again: the person let it go on.
            Event::Turn {
                chat,
                number,
                ask_at,
            } => match self.turns.get_mut(&chat) {
                Some(turn) if turn.number == number => turn.ask_at = ask_at,
                _ => {
                    self.turns.insert(
                        chat,
                        TurnView {
                            number,
                            since: crate::now(),
                            ask_at,
                            ..TurnView::default()
                        },
                    );
                }
            },
            Event::Window { agent, rises, most } => {
                if let Some(view) = self.agent_mut(&agent) {
                    view.rises = rises;
                    view.most = most;
                }
            }
            Event::Spent {
                agent,
                model,
                cost,
                usage,
                cache,
                chat,
            } => {
                let at = match self.cached.iter().position(|seen| seen.model == model) {
                    Some(at) => at,
                    None => {
                        self.cached.push(CacheView {
                            model: model.clone(),
                            ..CacheView::default()
                        });
                        self.cached.len() - 1
                    }
                };
                let seen = &mut self.cached[at];
                seen.calls += 1;
                seen.read += usage.input.0;
                seen.cached += cache.map_or(0, |cache| cache.read.0);
                seen.told |= cache.is_some();
                if let Some(turn) = chat.and_then(|chat| self.turns.get_mut(&chat)) {
                    let at = match turn.by.iter().position(|spent| spent.who == agent) {
                        Some(at) => at,
                        None => {
                            turn.by.push(TurnSpending {
                                who: agent.clone(),
                                ..TurnSpending::default()
                            });
                            turn.by.len() - 1
                        }
                    };
                    let spending = &mut turn.by[at];
                    spending.model.clone_from(&model);
                    spending.calls += 1;
                    spending.read += usage.input.0;
                    spending.cached += cache.map_or(0, |cache| cache.read.0);
                    if let Some(cost) = cost {
                        spending.spent += cost;
                        turn.total += cost;
                    }
                }
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
                    // Its wait, when it waited, ended with this call.
                    view.waiting = None;
                    view.thread.push(Entry::Tool {
                        name,
                        input,
                        output,
                        ok,
                    });
                }
            }
            Event::JobStarted { agent, id, line } => self.jobs.push(JobView {
                agent,
                id,
                line,
                since: crate::now(),
                tail: Vec::new(),
            }),
            Event::JobOutput { id, tail, .. } => {
                if let Some(job) = self.jobs.iter_mut().find(|job| job.id == id) {
                    job.tail = tail;
                }
            }
            Event::JobEnded { id, .. } => self.jobs.retain(|job| job.id != id),
            Event::Choose {
                id,
                agent,
                question,
                choices,
                raised,
            } => self.choices.push(Choice {
                id,
                agent,
                question,
                choices,
                raised,
            }),
            Event::Chosen { id, answer } => {
                if let Some(at) = self.choices.iter().position(|c| c.id == id) {
                    let choice = self.choices.remove(at);
                    if let Some(view) = self.agent_mut(&choice.agent) {
                        view.thread.push(Entry::Chose {
                            question: choice.question,
                            answer,
                        });
                    }
                }
            }
            Event::Waiting { agent, on } => {
                if let Some(view) = self.agent_mut(&agent) {
                    view.waiting = Some(Waiting {
                        on,
                        since: crate::now(),
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
                    view.waiting = None;
                    view.worked = Some(crate::now());
                    view.thread.push(Entry::Done(outcome));
                }
            }
            Event::Renamed { from, to } => {
                for view in &mut self.finished {
                    if view.by == from {
                        view.by.clone_from(&to);
                    }
                }
                for view in &mut self.agents {
                    if view.name == from {
                        view.name.clone_from(&to);
                    }
                    if view.by == from {
                        view.by.clone_from(&to);
                    }
                    for name in view.waiting.iter_mut().flat_map(|w| w.on.iter_mut()) {
                        if *name == from {
                            name.clone_from(&to);
                        }
                    }
                }
                for question in &mut self.questions {
                    if question.agent == from {
                        question.agent.clone_from(&to);
                    }
                }
                for choice in &mut self.choices {
                    if choice.agent == from {
                        choice.agent.clone_from(&to);
                    }
                }
                for job in &mut self.jobs {
                    if job.agent == from {
                        job.agent.clone_from(&to);
                    }
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
            Event::Model { agent, model } => {
                if let Some(view) = self.agent_mut(&agent) {
                    view.model = model;
                    // Its next call says which answered.
                    view.answered.clear();
                }
            }
        }
    }

    fn agent_mut(&mut self, name: &str) -> Option<&mut AgentView> {
        self.agents.iter_mut().find(|a| a.name == name)
    }

    /// A retired agent of that name, the latest, to read what it did once
    /// another took its name.
    #[must_use]
    pub fn retired(&self, name: &str) -> Option<&AgentView> {
        self.finished.iter().rev().find(|a| a.name == name)
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
        crew.event(Event::Turn {
            chat: "lead".into(),
            number: 1,
            ask_at: None,
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
                cache: Some(iq_core::CacheUse {
                    read: TokenCount(1000),
                    written: None,
                    rewrite_extra: None,
                }),
                chat: Some("lead".into()),
            });
        }
        // The reviewer's call for it, counted in its chat's turn.
        crew.event(Event::Spent {
            agent: "reviewer".into(),
            model: "lab/quick".into(),
            usage: iq_core::Usage::default(),
            cost: Some(Usd(0.01)),
            cache: None,
            chat: Some("lead".into()),
        });
        let turn = crew.turn("lead").unwrap();
        assert_eq!(turn.number, 1);
        assert_eq!(turn.total, Usd(1.26));
        let by: Vec<(&str, u64, u64, u64)> = turn
            .by
            .iter()
            .map(|spent| (spent.who.as_str(), spent.calls, spent.read, spent.cached))
            .collect();
        assert_eq!(by, [("lead", 2, 3000, 2000), ("reviewer", 1, 0, 0)]);
        // What each model read, and from the cache, over the session.
        let cached: Vec<(&str, u64, u64, bool)> = crew
            .cached()
            .iter()
            .map(|seen| (seen.model.as_str(), seen.read, seen.cached, seen.told))
            .collect();
        assert_eq!(
            cached,
            [("m", 3000, 2000, true), ("lab/quick", 0, 0, false)]
        );
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

    fn joined(crew: &mut CrewView, agent: &str, by: &str) {
        crew.event(Event::Joined {
            agent: agent.into(),
            model: "m".into(),
            by: by.into(),
        });
    }

    #[test]
    fn the_agents_a_chat_started_follow_its_name() {
        let mut crew = CrewView::default();
        joined(&mut crew, "lead", PERSON);
        joined(&mut crew, "fix", PERSON);
        joined(&mut crew, "review", "lead");
        joined(&mut crew, "helper", "review");
        let chats: Vec<&str> = crew.chats().map(|a| a.name.as_str()).collect();
        assert_eq!(chats, ["lead", "fix"]);
        crew.event(Event::Renamed {
            from: "lead".into(),
            to: "login".into(),
        });
        assert_eq!(crew.agent("review").unwrap().by, "login");
        assert_eq!(crew.chat_of("helper").as_deref(), Some("login"));
        // A chat that comes back brings what it was asked and its thread.
        crew.back(Back {
            agent: "fix".into(),
            task: "fix the build".into(),
            worked: 100,
            written: Some(90),
            thread: vec![Entry::Prompt("fix the build".into())],
        });
        let fix = crew.agent("fix").unwrap();
        assert_eq!(
            (fix.task.as_str(), fix.worked),
            ("fix the build", Some(100))
        );
        assert_eq!(fix.thread.len(), 1);
    }

    #[test]
    fn what_was_set_aside_comes_back_where_it_stood() {
        let kept = [
            Message::system("You are lead"),
            Message::user("deploy"),
            Message::Assistant {
                content: Some("Asking.".into()),
                tool_calls: Vec::new(),
                reasoning_signature: None,
            },
        ];
        let aside = |after: usize, shown: iq_agent::Shown| iq_agent::Aside {
            at: 1,
            turn: 1,
            after,
            shown,
        };
        let thread = past_with(
            &kept,
            &[
                aside(
                    2,
                    iq_agent::Shown::Question {
                        id: 1,
                        question: "Which one?".into(),
                        choices: vec!["a".into(), "b".into()],
                        answered: true,
                        answer: Some("b".into()),
                    },
                ),
                aside(
                    9,
                    iq_agent::Shown::Failed {
                        model: "lab/quill".into(),
                        status: Some(400),
                        message: "Request contains an invalid argument.".into(),
                        context: 190_000,
                        calls: 8,
                        spent: iq_core::Usd(0.78),
                    },
                ),
            ],
        );
        assert_eq!(
            thread,
            [
                Entry::Prompt("deploy".into()),
                Entry::Chose {
                    question: "Which one?".into(),
                    answer: Some("b".into())
                },
                Entry::Text("Asking.".into()),
                Entry::Done(Err(
                    "lab/quill answered HTTP 400: Request contains an invalid argument.".into()
                )),
            ]
        );
    }

    #[test]
    fn a_kept_conversation_becomes_a_thread_without_iq_s_own_words() {
        let kept = [
            Message::system("You are lead"),
            Message::user("fix the login"),
            Message::user("(iq: your conversation is 30k tokens now.)"),
            Message::Assistant {
                content: None,
                tool_calls: vec![iq_core::ToolCall {
                    id: "c1".into(),
                    name: "run".into(),
                    arguments: r#"{"command":"make test"}"#.into(),
                }],
                reasoning_signature: None,
            },
            Message::Tool {
                call_id: "c1".into(),
                content: "Error: the reviewer refused it".into(),
            },
            Message::Assistant {
                content: Some("Blocked.".into()),
                tool_calls: Vec::new(),
                reasoning_signature: None,
            },
        ];
        assert_eq!(
            past(&kept),
            [
                Entry::Prompt("fix the login".into()),
                Entry::Tool {
                    name: "run".into(),
                    input: "make test".into(),
                    output: "Error: the reviewer refused it".into(),
                    ok: false,
                },
                Entry::Text("Blocked.".into()),
            ]
        );
    }
}
