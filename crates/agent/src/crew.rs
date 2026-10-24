//! The crew: the agents at work in one workspace, the board they write to
//! each other on, and the one way any of them acts on the machine, through
//! the gate.
//!
//! An agent is a model of the provider, which iq runs in a loop with its
//! tools, or an agent with tools of its own (Claude Code, Codex), which
//! reaches iq back through its program: Claude Code's tool calls are judged
//! by the gate, and both find the board among their tools. What the gate
//! asks goes to the person, or, the autopilot being on, to the reviewer,
//! which may answer only what stays within the sandbox.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use iq_core::{
    Agent, ChatModel, ChatRequest, Delegate, DelegateEvent, DelegateRequest, Effort, Message,
    ModelId, Reach, ToolCall, Usd,
};
use iq_gate::{Action, Approver, Gate, Helper, Judged, Pending, Permit, Sandbox, Scope};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::event::{Event, PERSON};
use crate::review;
use crate::team::{self, Pattern, Phase, Team};
use crate::tools::{self, Args};

/// Reports what the crew does, as it does it.
pub type Report = Arc<dyn Fn(Event) + Send + Sync>;

/// Everyone on the board: a message to `all` reaches every agent.
pub const ALL: &str = "all";

/// The steward on the board: it writes to agents that spend too fast, and
/// they answer it.
pub const STEWARD: &str = "steward";

/// How often two agents may wake each other in [`PEER_SPAN`], whichever
/// writes: more, and they talk on without the work moving, on the
/// person's money.
const PEER_WAKES: usize = 4;

/// How often agents may wake each other in [`PEER_SPAN`], all of them.
const CREW_WAKES: usize = 16;

/// The span those wakes are counted over.
const PEER_SPAN: Duration = Duration::from_secs(10 * 60);

/// What a payer is called when it is closed: Claude Code's and Codex's
/// subscriptions by their agents' names, the provider by this.
pub const PROVIDER: &str = "provider";

/// What the crew starts with.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The model that answers for the person when the autopilot is on: a
    /// model of the provider, or an agent such as `claude-code/haiku`.
    pub reviewer: ModelId,
    /// How hard models think; `None` leaves it to them.
    pub effort: Option<Effort>,
    /// The most the provider's key may be billed for the crew's calls,
    /// from its start; `None` for no limit.
    pub budget: Option<Usd>,
    /// How long a question waits for the person before it is refused.
    pub patience: Duration,
    /// The most calls to its model one turn of an agent of iq's makes.
    pub steps: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            reviewer: ModelId::agent(Agent::ClaudeCode),
            effort: None,
            budget: None,
            patience: Duration::from_secs(50 * 60),
            steps: 200,
        }
    }
}

/// How the agents with tools of their own reach the crew: through iq's
/// program, speaking to the bridge.
#[derive(Debug, Clone)]
pub struct Bridge {
    /// iq's program.
    pub program: PathBuf,
    /// The socket the crew listens on.
    pub socket: PathBuf,
    /// A folder of iq's own, out of the commands' reach, for the agents'
    /// settings.
    pub folder: PathBuf,
}

/// The variable that names the bridge's socket for iq's program.
pub const BRIDGE_VAR: &str = "IQ_BRIDGE";

/// The variable that names the agent iq's program speaks for.
pub const AGENT_VAR: &str = "IQ_AGENT";

/// The flag that makes iq's program judge one of Claude Code's tool calls.
pub const HOOK_FLAG: &str = "--iq-hook";

/// The flag that makes iq's program serve the board's tools over MCP.
pub const MCP_FLAG: &str = "--iq-mcp";

/// The flag that makes iq's program run a command the gate judged.
pub const EXEC_FLAG: &str = "--iq-exec";

/// The agents in one workspace. Cheap to clone: every clone is the same
/// crew.
pub struct Crew<M, D> {
    inner: Arc<Inner<M, D>>,
}

impl<M, D> Clone for Crew<M, D> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct Inner<M, D> {
    provider: M,
    agents: D,
    gate: Gate,
    sandbox: Sandbox,
    settings: Settings,
    report: Report,
    autopilot: AtomicBool,
    /// Turns under way, the answers they post included.
    busy: AtomicUsize,
    next_question: AtomicU64,
    questions: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    board: Mutex<Board>,
    /// How many messages the board holds, for those who wait for one.
    posted: watch::Sender<usize>,
    /// What the provider's key was billed so far.
    billed: Mutex<Usd>,
    bridge: Mutex<Option<Bridge>>,
    /// Commands Claude Code was allowed, by the number its call runs.
    permits: Mutex<HashMap<String, Permit>>,
    /// The payers whose limits the person set are reached: no agent starts
    /// on them, and the provider bills no more.
    closed: Mutex<Vec<String>>,
    /// When agents woke each other lately, and which two.
    peer_wakes: Mutex<Vec<(Instant, (String, String))>>,
    /// The teams at work, and where each stands.
    teams: Mutex<Vec<Team>>,
}

#[derive(Default)]
struct Board {
    messages: Vec<Posted>,
    members: BTreeMap<String, Member>,
}

/// A message on the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posted {
    /// Its writer.
    pub from: String,
    /// Its reader: an agent, [`ALL`], or the person.
    pub to: String,
    /// The message.
    pub text: String,
}

struct Member {
    model: ModelId,
    by: String,
    /// What it does in the crew, when it was given a part.
    role: Option<String>,
    task: String,
    /// The first message it has not read.
    read: usize,
    /// Its turn under way, when it works.
    turn: Option<JoinHandle<()>>,
    kind: Kind,
}

#[derive(Clone)]
enum Kind {
    /// One of iq's: its conversation, kept between turns.
    Own { messages: Vec<Message> },
    /// An agent with tools of its own, and its session to go on with.
    Other {
        agent: Agent,
        model: String,
        session: Option<String>,
    },
}

/// An agent of the crew, as listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// Its name.
    pub name: String,
    /// Its model.
    pub model: String,
    /// Who started it.
    pub by: String,
    /// Whether it works.
    pub working: bool,
    /// What it does in the crew, when it was given a part.
    pub role: Option<String>,
}

impl<M, D> Crew<M, D>
where
    M: ChatModel + 'static,
    D: Delegate + 'static,
{
    /// A crew with no agent yet, its models answered by `provider`, its
    /// other agents run by `agents`, acting through `gate`, its commands
    /// run in the gate's sandbox through `helper`.
    pub fn new(
        provider: M,
        agents: D,
        gate: Gate,
        helper: Helper,
        settings: Settings,
        report: Report,
    ) -> Self {
        let sandbox = gate.sandbox(helper);
        Self {
            inner: Arc::new(Inner {
                provider,
                agents,
                gate,
                sandbox,
                settings,
                report,
                autopilot: AtomicBool::new(false),
                busy: AtomicUsize::new(0),
                next_question: AtomicU64::new(1),
                questions: Mutex::new(HashMap::new()),
                board: Mutex::new(Board::default()),
                posted: watch::Sender::new(0),
                billed: Mutex::new(Usd(0.0)),
                bridge: Mutex::new(None),
                permits: Mutex::new(HashMap::new()),
                closed: Mutex::new(Vec::new()),
                peer_wakes: Mutex::new(Vec::new()),
                teams: Mutex::new(Vec::new()),
            }),
        }
    }

    /// The gate the crew acts through.
    pub fn gate(&self) -> &Gate {
        &self.inner.gate
    }

    /// The sandbox its commands run in.
    pub(crate) fn sandbox(&self) -> &Sandbox {
        &self.inner.sandbox
    }

    /// Whether the reviewer answers for the person.
    pub fn autopilot(&self) -> bool {
        self.inner.autopilot.load(Ordering::Relaxed)
    }

    /// Lets the reviewer answer for the person, or not. What widens the
    /// sandbox waits for the person whichever.
    pub fn set_autopilot(&self, on: bool) {
        self.inner.autopilot.store(on, Ordering::Relaxed);
    }

    /// Whether every command has the internet.
    pub fn internet(&self) -> bool {
        self.inner.gate.internet()
    }

    /// Gives every command the internet, Codex's included from its next
    /// turn, or only what a rule gives it to and what the person allows.
    pub fn set_internet(&self, open: bool) {
        self.inner.gate.set_internet(open);
    }

    /// Has the agents with tools of their own reach the crew through
    /// `bridge`, once it listens.
    pub fn reach_through(&self, bridge: Bridge) {
        *lock(&self.inner.bridge) = Some(bridge);
    }

    /// The bridge, taken away: the agents with tools of their own no
    /// longer reach the crew.
    pub(crate) fn take_bridge(&self) -> Option<Bridge> {
        lock(&self.inner.bridge).take()
    }

    /// What the provider's key was billed so far.
    pub fn billed(&self) -> Usd {
        *lock(&self.inner.billed)
    }

    /// The person's answer to question `id`.
    pub fn answer(&self, id: u64, approved: bool) {
        if let Some(reply) = lock(&self.inner.questions).remove(&id) {
            let _ = reply.send(approved);
        }
    }

    /// The agents, in the order of their names.
    pub fn list(&self) -> Vec<Listed> {
        lock(&self.inner.board)
            .members
            .iter()
            .map(|(name, member)| Listed {
                name: name.clone(),
                model: member.model.to_string(),
                by: member.by.clone(),
                working: member.turn.as_ref().is_some_and(|turn| !turn.is_finished()),
                role: member.role.clone(),
            })
            .collect()
    }

    /// Starts a team on the person's `task`, as `pattern` works: one agent
    /// on the first of `models`, a lead that may give its agents the
    /// others, a writer with a reviewer on the second, or as many debaters
    /// as models, two at the least. Returns the first agent's name: `name`,
    /// made unique.
    pub fn launch(
        &self,
        pattern: Pattern,
        name: &str,
        models: &[ModelId],
        task: String,
        role: Option<String>,
    ) -> String {
        let Some(first) = models.first() else {
            return String::new();
        };
        // Held while the team starts, so that a turn ending at once finds
        // its team.
        let mut teams = lock(&self.inner.teams);
        match pattern {
            Pattern::Solo => self.start_as(name, first, task, PERSON, role),
            Pattern::Lead => {
                let cheaper: Vec<String> = models[1..].iter().map(ToString::to_string).collect();
                self.start_as(name, first, task, PERSON, Some(team::lead_role(&cheaper)))
            }
            Pattern::Pair => {
                let writer = self.start_as(
                    name,
                    first,
                    task.clone(),
                    PERSON,
                    Some(team::WRITER_ROLE.to_owned()),
                );
                teams.push(Team::Pair {
                    writer: writer.clone(),
                    reviewer: None,
                    reviewer_model: models
                        .get(1)
                        .cloned()
                        .unwrap_or_else(|| self.inner.settings.reviewer.clone()),
                    task,
                    rounds: 0,
                    over: false,
                });
                writer
            }
            Pattern::Debate => {
                let count = models.len().max(2);
                let members: Vec<String> = (0..count)
                    .map(|at| {
                        self.start_as(
                            name,
                            &models[at % models.len()],
                            task.clone(),
                            PERSON,
                            Some(team::debater_role(count)),
                        )
                    })
                    .collect();
                let first = members[0].clone();
                teams.push(Team::Debate {
                    members,
                    phase: Phase::Alone,
                    answers: BTreeMap::new(),
                });
                first
            }
        }
    }

    /// Hands the work on, when `name`'s turn ended with `outcome` in a
    /// team: the writer's changes to its reviewer, the review back to the
    /// writer or, approved, to the person; a debate to its next phase.
    /// Returns whether the answer was handled, and is not to be sent to
    /// whoever started the agent.
    fn after_turn(&self, name: &str, outcome: &Result<String, String>) -> bool {
        let mut teams = lock(&self.inner.teams);
        for team in teams.iter_mut() {
            match team {
                Team::Pair {
                    writer,
                    reviewer,
                    reviewer_model,
                    task,
                    rounds,
                    over,
                } if !*over => {
                    if name == writer.as_str() {
                        if outcome.is_err() {
                            *over = true;
                            return false;
                        }
                        let prompt = team::review_prompt(writer, task, *rounds);
                        match reviewer {
                            Some(reviewer) => self.wake(reviewer, prompt),
                            None => {
                                let started = self.start_as(
                                    "review",
                                    reviewer_model,
                                    prompt,
                                    writer,
                                    Some(team::REVIEWER_ROLE.to_owned()),
                                );
                                *reviewer = Some(started);
                            }
                        }
                        return false;
                    }
                    if reviewer.as_deref() != Some(name) {
                        continue;
                    }
                    let Ok(review) = outcome else {
                        *over = true;
                        return false;
                    };
                    *rounds += 1;
                    if team::approves(review) {
                        *over = true;
                        let _ = self.post(name, PERSON, review.clone());
                        self.report(Event::Notice {
                            agent: writer.clone(),
                            text: format!("{name} approved it after {rounds} round(s)"),
                        });
                    } else if *rounds >= team::PAIR_ROUNDS {
                        *over = true;
                        let _ = self.post(name, PERSON, review.clone());
                        self.report(Event::Notice {
                            agent: writer.clone(),
                            text: format!(
                                "after {rounds} rounds, {name} still asks for changes: over to you"
                            ),
                        });
                    } else {
                        // Back to the writer, which it wakes.
                        let _ = self.post(name, writer, review.clone());
                    }
                    return true;
                }
                Team::Debate {
                    members,
                    phase,
                    answers,
                } if members.iter().any(|m| m == name) && *phase != Phase::Over => {
                    let said = match outcome {
                        Ok(text) => text.clone(),
                        Err(error) => format!("(it stopped: {error})"),
                    };
                    match *phase {
                        Phase::Alone | Phase::Revising => {
                            answers.insert(name.to_owned(), said);
                            if answers.len() < members.len() {
                                return false;
                            }
                            let all = std::mem::take(answers);
                            if *phase == Phase::Alone {
                                *phase = Phase::Revising;
                                for member in members.iter() {
                                    self.wake(member, team::others_prompt(member, &all));
                                }
                            } else {
                                *phase = Phase::Summing;
                                self.wake(&members[0], team::summing_prompt(&all));
                            }
                        }
                        Phase::Summing if name == members[0] => *phase = Phase::Over,
                        Phase::Summing | Phase::Over => {}
                    }
                    return false;
                }
                _ => {}
            }
        }
        false
    }

    /// Starts an agent on `task`, started `by` the person or another agent.
    /// Returns its name: `name`, made unique.
    pub fn start(&self, name: &str, model: &ModelId, task: String, by: &str) -> String {
        self.start_as(name, model, task, by, None)
    }

    /// Starts an agent as [`Self::start`] does, with a `role` in the crew:
    /// what it does, which its instructions tell it to keep to.
    pub fn start_as(
        &self,
        name: &str,
        model: &ModelId,
        task: String,
        by: &str,
        role: Option<String>,
    ) -> String {
        let mut board = lock(&self.inner.board);
        let base: String = name
            .trim()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let base = if base.is_empty() || base == ALL || base == PERSON {
            "agent".to_owned()
        } else {
            base
        };
        let mut name = base.clone();
        let mut n = 2;
        while board.members.contains_key(&name) {
            name = format!("{base}-{n}");
            n += 1;
        }
        let kind = match model.delegate() {
            Some((agent, model)) => Kind::Other {
                agent,
                model: model.to_owned(),
                session: None,
            },
            None => Kind::Own {
                messages: vec![Message::system(self.instructions(
                    &name,
                    by,
                    role.as_deref(),
                ))],
            },
        };
        let read = board.messages.len();
        board.members.insert(
            name.clone(),
            Member {
                model: model.clone(),
                by: by.to_owned(),
                role,
                task: task.clone(),
                read,
                turn: None,
                kind,
            },
        );
        drop(board);
        self.report(Event::Joined {
            agent: name.clone(),
            model: model.to_string(),
            by: by.to_owned(),
        });
        self.wake(&name, task);
        name
    }

    /// The person writes to `to`: an agent, woken if it rests, or all.
    pub fn tell(&self, to: &str, text: String) -> Result<(), String> {
        self.post(PERSON, to, text)
    }

    /// Stops `agent`'s turn, if it works.
    pub fn stop(&self, agent: &str) {
        let turn = lock(&self.inner.board)
            .members
            .get_mut(agent)
            .and_then(|member| member.turn.take());
        if let Some(turn) = turn {
            turn.abort();
            self.report(Event::Done {
                agent: agent.to_owned(),
                outcome: Err("stopped by the person".into()),
            });
        }
    }

    /// Stops every agent.
    pub fn stop_all(&self) {
        let names: Vec<String> = lock(&self.inner.board).members.keys().cloned().collect();
        for name in names {
            self.stop(&name);
        }
    }

    /// Waits until no agent works, nor any answer is on its way to wake
    /// one.
    pub async fn settled(&self) {
        let mut posted = self.inner.posted.subscribe();
        while self.inner.busy.load(Ordering::SeqCst) > 0 {
            // A turn ends by posting its answer, or soon after a check.
            let _ = tokio::time::timeout(Duration::from_millis(100), posted.changed()).await;
        }
    }

    /// Whether `from` may wake `to` now: two agents a few times in a while,
    /// the crew some more; counted when it may.
    fn peer_may_wake(&self, from: &str, to: &str) -> bool {
        let now = Instant::now();
        let pair = if from < to {
            (from.to_owned(), to.to_owned())
        } else {
            (to.to_owned(), from.to_owned())
        };
        let mut wakes = lock(&self.inner.peer_wakes);
        wakes.retain(|(at, _)| now.duration_since(*at) < PEER_SPAN);
        let ours = wakes.iter().filter(|(_, p)| *p == pair).count();
        if ours >= PEER_WAKES || wakes.len() >= CREW_WAKES {
            return false;
        }
        wakes.push((now, pair));
        true
    }

    /// Posts a message on the board. A message from the person wakes its
    /// reader when it rests, as does the answer of an agent to the one
    /// that started it, and a message from another agent, a few times in a
    /// while; others wait to be read.
    pub(crate) fn post(&self, from: &str, to: &str, text: String) -> Result<(), String> {
        let wakes = {
            let mut board = lock(&self.inner.board);
            if to != ALL && to != PERSON && to != STEWARD && !board.members.contains_key(to) {
                let names: Vec<&str> = board.members.keys().map(String::as_str).collect();
                return Err(format!(
                    "no agent is named {to:?}: the agents are {}",
                    names.join(", ")
                ));
            }
            board.messages.push(Posted {
                from: from.to_owned(),
                to: to.to_owned(),
                text: text.clone(),
            });
            let started_it = board.members.get(from).is_some_and(|m| m.by == to);
            let resting = board
                .members
                .get(to)
                .is_some_and(|m| m.turn.as_ref().is_none_or(JoinHandle::is_finished));
            let peer =
                !started_it && board.members.contains_key(from) && board.members.contains_key(to);
            (resting, from == PERSON || started_it, peer)
        };
        let wakes = match wakes {
            (false, _, _) => false,
            (true, true, _) => true,
            (true, false, true) => {
                let may = self.peer_may_wake(from, to);
                if !may {
                    self.report(Event::Notice {
                        agent: to.to_owned(),
                        text: format!(
                            "{from} wrote while it rested, but they woke each other often lately: \
                             the message waits until you write to it"
                        ),
                    });
                }
                may
            }
            (true, false, false) => false,
        };
        self.inner.posted.send_modify(|count| *count += 1);
        self.report(Event::Posted {
            from: from.to_owned(),
            to: to.to_owned(),
            text: text.clone(),
        });
        if wakes {
            let prompt = self.unread(to).unwrap_or(text);
            self.wake(to, prompt);
        }
        Ok(())
    }

    /// The messages to `agent` it has not read, as one text, marked read.
    fn unread(&self, agent: &str) -> Option<String> {
        let mut board = lock(&self.inner.board);
        let Board { messages, members } = &mut *board;
        let member = members.get_mut(agent)?;
        let new: Vec<&Posted> = messages[member.read..]
            .iter()
            .filter(|m| m.from != agent && (m.to == agent || m.to == ALL))
            .collect();
        member.read = messages.len();
        if new.is_empty() {
            return None;
        }
        Some(
            new.iter()
                .map(|m| format!("Message from {}: {}", m.from, m.text))
                .collect::<Vec<_>>()
                .join("\n\n"),
        )
    }

    /// Waits up to `seconds` for a message to `agent`.
    async fn wait(&self, agent: &str, seconds: u64) -> String {
        let mut posted = self.inner.posted.subscribe();
        let until = tokio::time::Instant::now() + Duration::from_secs(seconds);
        loop {
            posted.borrow_and_update();
            if let Some(news) = self.unread(agent) {
                return news;
            }
            if tokio::time::timeout_at(until, posted.changed())
                .await
                .is_err()
            {
                return format!("No message came in {seconds} seconds.");
            }
        }
    }

    /// Starts a turn of `agent`'s on `prompt`.
    fn wake(&self, agent: &str, prompt: String) {
        let crew = self.clone();
        let name = agent.to_owned();
        let mut board = lock(&self.inner.board);
        let Some(member) = board.members.get_mut(agent) else {
            return;
        };
        if member.turn.as_ref().is_some_and(|turn| !turn.is_finished()) {
            return;
        }
        self.report(Event::Working {
            agent: name.clone(),
            prompt: prompt.clone(),
        });
        self.inner.busy.fetch_add(1, Ordering::SeqCst);
        let busy = Busy(self.clone());
        member.turn = Some(tokio::spawn(async move {
            crew.turn(name, prompt).await;
            drop(busy);
        }));
    }

    /// One turn of an agent's, until it answers. Its state is taken as it
    /// was, and kept again only when the turn ends: a turn stopped halfway
    /// leaves the agent as it was before it.
    async fn turn(self, name: String, prompt: String) {
        let (taken, posted_before) = {
            let board = lock(&self.inner.board);
            let taken = board
                .members
                .get(&name)
                .map(|member| (member.kind.clone(), member.model.clone(), member.by.clone()));
            (taken, board.messages.len())
        };
        let Some((kind, model, by)) = taken else {
            return;
        };
        let (kind, outcome) = match kind {
            Kind::Own { messages } => {
                let (messages, outcome) = self.own_turn(&name, &model, messages, prompt).await;
                (Kind::Own { messages }, outcome)
            }
            Kind::Other {
                agent,
                model: agent_model,
                session,
            } => {
                let (session, outcome) = self
                    .other_turn(&name, agent, &agent_model, session, prompt)
                    .await;
                (
                    Kind::Other {
                        agent,
                        model: agent_model,
                        session,
                    },
                    outcome,
                )
            }
        };
        if let Some(member) = lock(&self.inner.board).members.get_mut(&name) {
            member.kind = kind;
            // It rests from now: what is posted to it wakes it.
            member.turn = None;
        }
        self.report(Event::Done {
            agent: name.clone(),
            outcome: outcome.clone(),
        });
        // Its answer goes to the agent that started it, unless it wrote it
        // there itself during the turn: once is enough.
        let wrote_back = lock(&self.inner.board)
            .messages
            .get(posted_before..)
            .is_some_and(|new| new.iter().any(|m| m.from == name && m.to == by));
        let handled = self.after_turn(&name, &outcome);
        if by != PERSON && !handled && !(wrote_back && outcome.is_ok()) {
            let text = match outcome {
                Ok(text) => text,
                Err(error) => format!("I stopped: {error}"),
            };
            let _ = self.post(&name, &by, text);
        }
        // What the person wrote after its last look at the board.
        if self.person_waits(&name)
            && let Some(news) = self.unread(&name)
        {
            self.wake(&name, news);
        }
    }

    /// Whether the person wrote to `agent` since it last read the board.
    fn person_waits(&self, agent: &str) -> bool {
        let board = lock(&self.inner.board);
        board.members.get(agent).is_some_and(|member| {
            board.messages[member.read..]
                .iter()
                .any(|m| m.from == PERSON && (m.to == agent || m.to == ALL))
        })
    }

    /// A turn of one of iq's agents: its model called with its tools until
    /// it answers without calling one.
    async fn own_turn(
        &self,
        name: &str,
        model: &ModelId,
        mut messages: Vec<Message>,
        prompt: String,
    ) -> (Vec<Message>, Result<String, String>) {
        messages.push(Message::user(prompt));
        let mut tools = tools::work();
        tools.extend(tools::board());
        for _ in 0..self.inner.settings.steps {
            if let Some(news) = self.unread(name) {
                messages.push(Message::user(news));
            }
            if let Err(error) = self.within_budget() {
                return (messages, Err(error));
            }
            let request = ChatRequest {
                model: model.clone(),
                messages: messages.clone(),
                tools: tools.clone(),
                effort: self.inner.settings.effort,
            };
            let response = match self.inner.provider.complete(&request).await {
                Ok(response) => response,
                Err(error) => return (messages, Err(chain(&error))),
            };
            let cost = match response.cost {
                Some(cost) => Some(cost),
                None => self
                    .inner
                    .provider
                    .pricing(model)
                    .await
                    .map(|pricing| pricing.cost(&response.usage)),
            };
            self.spent(name, model.as_str(), response.usage, cost, true);
            messages.push(response.to_message());
            if let Some(text) = response.content.as_ref().filter(|t| !t.trim().is_empty()) {
                self.report(Event::TextStart {
                    agent: name.to_owned(),
                });
                self.report(Event::Text {
                    agent: name.to_owned(),
                    text: text.clone(),
                });
            }
            if response.tool_calls.is_empty() {
                return (messages, Ok(response.content.unwrap_or_default()));
            }
            for call in &response.tool_calls {
                let content = self.call(name, call).await;
                messages.push(Message::Tool {
                    call_id: call.id.clone(),
                    content,
                });
            }
        }
        let steps = self.inner.settings.steps;
        (
            messages,
            Err(format!("stopped after {steps} calls to its model")),
        )
    }

    /// A turn of an agent with tools of its own.
    async fn other_turn(
        &self,
        name: &str,
        agent: Agent,
        model: &str,
        session: Option<String>,
        prompt: String,
    ) -> (Option<String>, Result<String, String>) {
        let reach = lock(&self.inner.bridge).as_ref().map(|bridge| Reach {
            program: bridge.program.clone(),
            hook_args: vec![HOOK_FLAG.to_owned()],
            mcp_args: vec![MCP_FLAG.to_owned()],
            env: vec![
                (BRIDGE_VAR.to_owned(), bridge.socket.display().to_string()),
                (AGENT_VAR.to_owned(), name.to_owned()),
            ],
            folder: bridge.folder.clone(),
        });
        let (by, role) = lock(&self.inner.board)
            .members
            .get(name)
            .map(|member| (member.by.clone(), member.role.clone()))
            .unwrap_or_default();
        let request = DelegateRequest {
            agent,
            read_only: false,
            effort: self.inner.settings.effort,
            model: model.to_owned(),
            prompt,
            instructions: self.instructions_for_other(name, &by, role.as_deref(), reach.is_some()),
            resume: session.clone(),
            fork: false,
            ephemeral: false,
            directory: self.inner.gate.workspace().to_path_buf(),
            reach,
            network: self.inner.gate.internet(),
        };
        let crew = self.clone();
        let who = name.to_owned();
        let shown = if model.is_empty() {
            agent.prefix().to_owned()
        } else {
            format!("{}/{model}", agent.prefix())
        };
        let mut on_event = move |event: DelegateEvent| crew.relay(&who, &shown, event);
        match self.inner.agents.run(&request, &mut on_event).await {
            Ok(reply) => (Some(reply.session), Ok(reply.text)),
            Err(error) => (session, Err(chain(&error))),
        }
    }

    /// Reports what an agent with tools of its own did.
    fn relay(&self, agent: &str, model: &str, event: DelegateEvent) {
        let agent = agent.to_owned();
        match event {
            DelegateEvent::TextStart => self.report(Event::TextStart { agent }),
            DelegateEvent::Text(text) => self.report(Event::Text { agent, text }),
            DelegateEvent::Usage {
                usage,
                cost,
                billed,
                ..
            } => self.spent(&agent, model, usage, cost, billed),
            DelegateEvent::Tool {
                name,
                input,
                output,
            } => {
                let (output, ok) = match output {
                    Ok(output) => (output, true),
                    Err(error) => (error, false),
                };
                self.report(Event::Tool {
                    agent,
                    name,
                    input: tools::summary(&input),
                    output,
                    ok,
                });
            }
            DelegateEvent::Notice(text) => self.report(Event::Notice { agent, text }),
            DelegateEvent::Quota(quota) => self.report(Event::Quota(quota)),
            DelegateEvent::Denied {
                name,
                input,
                reason,
            } => {
                let reason = if reason.is_empty() {
                    String::new()
                } else {
                    format!(": {reason}")
                };
                self.report(Event::Notice {
                    agent,
                    text: format!("{name} {} was refused{reason}", tools::summary(&input)),
                });
            }
        }
    }

    /// Runs one of the tools of iq's agents, and gives what it returned or
    /// why it failed, as the model reads it.
    pub(crate) async fn call(&self, agent: &str, call: &ToolCall) -> String {
        let args = Args::parse(&call.arguments);
        let input = serde_json::from_str(&call.arguments)
            .map(|args| tools::summary(&args))
            .unwrap_or_default();
        let result = match args {
            Ok(args) => self.tool(agent, &call.name, &args).await,
            Err(error) => Err(error),
        };
        let (output, ok) = match result {
            Ok(output) => (output, true),
            Err(error) => (format!("Error: {error}"), false),
        };
        self.report(Event::Tool {
            agent: agent.to_owned(),
            name: call.name.clone(),
            input,
            output: output.clone(),
            ok,
        });
        output
    }

    async fn tool(&self, agent: &str, name: &str, args: &Args) -> Result<String, String> {
        let workspace = self.inner.gate.workspace().to_path_buf();
        match name {
            "read_file" => {
                let path = args.text("path")?.into();
                let permit = self.act(agent, Action::Read { path }).await?;
                let bytes = permit.read().map_err(|e| chain(&e))?;
                let text = String::from_utf8(bytes)
                    .map_err(|_| "the file is not text: run a command to look into it")?;
                Ok(tools::numbered(
                    &text,
                    args.number("offset"),
                    args.number("limit"),
                ))
            }
            "write_file" => {
                let path = args.text("path")?.into();
                let content = args.text("content")?;
                let permit = self.act(agent, Action::Write { path }).await?;
                permit.write(content.as_bytes()).map_err(|e| chain(&e))?;
                Ok(format!("Wrote {} bytes.", content.len()))
            }
            "edit_file" => {
                let path = args.text("path")?.into();
                let permit = self.act(agent, Action::Write { path }).await?;
                let bytes = permit.read().map_err(|e| chain(&e))?;
                let text = String::from_utf8(bytes).map_err(|_| "the file is not text")?;
                let edited = tools::replaced(
                    &text,
                    args.text("old_text")?,
                    args.text("new_text")?,
                    args.flag("all"),
                )?;
                permit.write(edited.as_bytes()).map_err(|e| chain(&e))?;
                Ok("Edited.".into())
            }
            "run" => {
                let line = args.text("command")?.to_owned();
                let action = Action::Run {
                    line,
                    folder: workspace,
                    network: args.flag("network"),
                };
                let permit = self.act(agent, action).await?;
                self.run(permit, OWN_ASK).await.map(|(text, _)| text)
            }
            _ => self.board_tool(agent, name, args).await,
        }
    }

    /// Runs one of the board's tools, for iq's agents or, through the
    /// bridge, for the others.
    pub(crate) async fn board_tool(
        &self,
        agent: &str,
        name: &str,
        args: &Args,
    ) -> Result<String, String> {
        match name {
            "start_agent" => {
                let model = ModelId::new(args.text("model")?).map_err(|e| e.to_string())?;
                self.open_for(&model)?;
                let role = args
                    .text("role")
                    .ok()
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                    .map(str::to_owned);
                let started = self.start_as(
                    args.text("name")?,
                    &model,
                    args.text("task")?.to_owned(),
                    agent,
                    role,
                );
                Ok(format!(
                    "{started} started on {model}. Its answer will come to you as a message."
                ))
            }
            "send_message" => {
                self.post(agent, args.text("to")?, args.text("text")?.to_owned())?;
                Ok("Sent.".into())
            }
            "wait_for_messages" => {
                let seconds = args
                    .number("seconds")
                    .unwrap_or(tools::WAIT_SECONDS)
                    .min(tools::WAIT_MOST);
                Ok(self.wait(agent, seconds).await)
            }
            "list_agents" => Ok(self
                .list()
                .iter()
                .map(|a| {
                    let state = if a.working { "working" } else { "resting" };
                    let role = a
                        .role
                        .as_deref()
                        .map(|r| format!(", its role: {r}"))
                        .unwrap_or_default();
                    format!(
                        "{} ({}), started by {}, {state}{role}",
                        a.name, a.model, a.by
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")),
            _ => Err(format!("there is no tool named {name:?}")),
        }
    }

    /// Runs the command `permit` allows, and says how it went, and whether
    /// it succeeded. A command that failed without the internet, as one
    /// that needs it does, is told how to ask for it: `ask`.
    pub(crate) async fn run(&self, permit: Permit, ask: &str) -> Result<(String, bool), String> {
        let offline = matches!(permit.action(), Action::Run { network: false, .. });
        let ran = self
            .inner
            .sandbox
            .run(permit)
            .await
            .map_err(|e| chain(&e))?;
        let mut out = ran.output;
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        for path in &ran.restored {
            out.push_str(&format!(
                "[iq put back {}: code written there would run outside the sandbox]\n",
                path.display()
            ));
        }
        for path in &ran.touched {
            out.push_str(&format!(
                "[{} changed: code there runs later; the person will see it]\n",
                path.display()
            ));
        }
        match (ran.timed_out, ran.status) {
            (true, _) => out.push_str("[stopped: it ran out of time]"),
            (false, Some(0)) => out.push_str("[exit status 0]"),
            (false, Some(status)) => out.push_str(&format!("[exit status {status}]")),
            (false, None) => out.push_str("[ended by a signal]"),
        }
        if offline && ran.status != Some(0) && OFFLINE.iter().any(|sign| out.contains(sign)) {
            out.push_str(&format!(
                "\n[it ran without the internet, localhost only: {ask}]"
            ));
        }
        Ok((out, ran.status == Some(0)))
    }

    /// Judges `action` for `agent`, asking whoever answers for the person
    /// when the gate asks.
    pub(crate) async fn act(&self, agent: &str, action: Action) -> Result<Permit, String> {
        match self.inner.gate.judge(action).map_err(|e| chain(&e))? {
            Judged::Allowed(permit) => Ok(permit),
            Judged::Denied { reason } => Err(format!("refused, {reason}")),
            Judged::Asked { reason, pending } => self.ask(agent, reason, pending).await,
        }
    }

    /// Asks the reviewer, the autopilot being on and the action staying
    /// within the sandbox, else the person.
    async fn ask(&self, agent: &str, reason: String, pending: Pending) -> Result<Permit, String> {
        let action = describe(pending.action());
        // The reviewer answers what stays within the sandbox, and what only
        // reaches the internet when the person lets it; never more.
        let reviewable = match pending.scope() {
            Scope::Sandbox => true,
            Scope::Internet => self.inner.gate.reviewer_internet(),
            Scope::Beyond => false,
        };
        if self.autopilot() && reviewable {
            match self.review(agent, &action, &reason).await {
                Ok((approved, why)) => {
                    self.report(Event::Reviewed {
                        agent: agent.to_owned(),
                        action: action.clone(),
                        reason: reason.clone(),
                        approved,
                        why: why.clone(),
                    });
                    return if approved {
                        self.inner
                            .gate
                            .approve(pending, Approver::Reviewer)
                            .map_err(|e| chain(&e))
                    } else {
                        Err(format!("the reviewer refused it: {why}"))
                    };
                }
                Err(error) => self.report(Event::Notice {
                    agent: agent.to_owned(),
                    text: format!("the reviewer could not answer, so the person is asked: {error}"),
                }),
            }
        }
        let id = self.inner.next_question.fetch_add(1, Ordering::Relaxed);
        let (reply, answer) = oneshot::channel();
        lock(&self.inner.questions).insert(id, reply);
        self.report(Event::Asked {
            id,
            agent: agent.to_owned(),
            action,
            reason: reason.clone(),
            widens: pending.widens(),
        });
        let answer = tokio::time::timeout(self.inner.settings.patience, answer).await;
        lock(&self.inner.questions).remove(&id);
        let approved = matches!(answer, Ok(Ok(true)));
        self.report(Event::Answered { id, approved });
        match answer {
            Ok(Ok(true)) => self
                .inner
                .gate
                .approve(pending, Approver::Person)
                .map_err(|e| chain(&e)),
            Ok(Ok(false)) => Err(format!("the person refused it ({reason})")),
            _ => Err(format!("nobody answered in time ({reason})")),
        }
    }

    /// The reviewer's verdict on an action of `agent`'s.
    async fn review(
        &self,
        agent: &str,
        action: &str,
        reason: &str,
    ) -> Result<(bool, String), String> {
        let task = lock(&self.inner.board)
            .members
            .get(agent)
            .map(|member| member.task.clone())
            .unwrap_or_default();
        let prompt = review::prompt(&task, action, reason);
        let model = &self.inner.settings.reviewer;
        let answer = match model.delegate() {
            Some((other, its_model)) => {
                let request = DelegateRequest {
                    agent: other,
                    read_only: true,
                    effort: Some(Effort::Low),
                    model: its_model.to_owned(),
                    prompt,
                    instructions: review::INSTRUCTIONS.to_owned(),
                    resume: None,
                    fork: false,
                    ephemeral: true,
                    directory: self.inner.gate.workspace().to_path_buf(),
                    reach: None,
                    network: false,
                };
                let crew = self.clone();
                let shown = model.to_string();
                let mut on_event = move |event: DelegateEvent| {
                    if let DelegateEvent::Usage {
                        usage,
                        cost,
                        billed,
                        ..
                    } = event
                    {
                        crew.spent("reviewer", &shown, usage, cost, billed);
                    }
                };
                self.inner
                    .agents
                    .run(&request, &mut on_event)
                    .await
                    .map(|reply| reply.text)
                    .map_err(|e| chain(&e))?
            }
            None => {
                self.within_budget()?;
                let request = ChatRequest {
                    model: model.clone(),
                    messages: vec![Message::system(review::INSTRUCTIONS), Message::user(prompt)],
                    tools: Vec::new(),
                    effort: Some(Effort::Low),
                };
                let response = self
                    .inner
                    .provider
                    .complete(&request)
                    .await
                    .map_err(|e| chain(&e))?;
                let cost = match response.cost {
                    Some(cost) => Some(cost),
                    None => self
                        .inner
                        .provider
                        .pricing(model)
                        .await
                        .map(|pricing| pricing.cost(&response.usage)),
                };
                self.spent("reviewer", model.as_str(), response.usage, cost, true);
                response.content.unwrap_or_default()
            }
        };
        Ok(review::verdict(&answer))
    }

    /// Answers one of Claude Code's tool calls, as its hook gives it: the
    /// hook's output, empty to leave the call to Claude Code's own mode.
    pub(crate) async fn hook(&self, agent: &str, input: &serde_json::Value) -> String {
        let workspace = self.inner.gate.workspace().to_path_buf();
        let call = crate::hook::read(input, &workspace);
        // What others wrote to it since it last looked rides on the answer:
        // between two of its calls is when Claude Code can read it.
        let news = self.unread(agent);
        let news = news.as_deref();
        match call {
            crate::hook::Call::Defer => crate::hook::defer(news),
            crate::hook::Call::Refuse(reason) => crate::hook::deny(&reason, news),
            crate::hook::Call::Judge { action, command } => match self.act(agent, action).await {
                Ok(permit) => match command {
                    Some(input) => {
                        let program = lock(&self.inner.bridge)
                            .as_ref()
                            .map(|bridge| bridge.program.clone());
                        let Some(program) = program else {
                            return crate::hook::deny("iq's bridge is closed", news);
                        };
                        let id = crate::bridge::token();
                        lock(&self.inner.permits).insert(id.clone(), permit);
                        crate::hook::allow(
                            Some(crate::hook::through(input, &program, &[EXEC_FLAG, &id])),
                            news,
                        )
                    }
                    None => crate::hook::allow(None, news),
                },
                Err(reason) => crate::hook::deny(&reason, news),
            },
        }
    }

    /// Runs the command the gate allowed one of Claude Code's calls, once.
    pub(crate) async fn exec(&self, id: &str) -> Result<(String, bool), String> {
        let permit = lock(&self.inner.permits)
            .remove(id)
            .ok_or("no command waits under that number")?;
        self.run(permit, CLAUDE_ASK).await
    }

    /// Lets the reviewer give a command the internet, the autopilot on, or
    /// leaves that to the person. Publishing and what leaves the workspace
    /// are always the person's.
    pub fn set_autopilot_internet(&self, may: bool) {
        self.inner.gate.set_reviewer_internet(may);
    }

    /// Closes the payers named, whose limits are reached, and opens the
    /// others: `claude-code`, `codex`, or the provider.
    pub fn close_payers(&self, payers: Vec<String>) {
        *lock(&self.inner.closed) = payers;
    }

    /// Writes to `to` as the steward.
    ///
    /// # Errors
    ///
    /// No agent of that name.
    pub fn advise(&self, to: &str, text: String) -> Result<(), String> {
        self.post(STEWARD, to, text)
    }

    /// Refuses a model whose payer is closed, saying which are open.
    fn open_for(&self, model: &ModelId) -> Result<(), String> {
        let closed = lock(&self.inner.closed).clone();
        let payer = model
            .delegate()
            .map_or(PROVIDER, |(agent, _)| agent.prefix());
        if !closed.iter().any(|c| c == payer) {
            return Ok(());
        }
        let open: Vec<&str> = [Agent::ClaudeCode.prefix(), Agent::Codex.prefix(), PROVIDER]
            .into_iter()
            .filter(|p| !closed.iter().any(|c| c == p))
            .collect();
        Err(format!(
            "{payer} has reached the limit the person set; start the agent on another: {}",
            if open.is_empty() {
                "none is open".to_owned()
            } else {
                open.join(", ")
            }
        ))
    }

    /// Fails once the provider's key was billed the budget, or the
    /// provider's limit for the month is reached.
    fn within_budget(&self) -> Result<(), String> {
        if lock(&self.inner.closed).iter().any(|c| c == PROVIDER) {
            return Err(
                "the provider's limit for the month, as the person set it, is reached".to_owned(),
            );
        }
        match self.inner.settings.budget {
            Some(budget) if self.billed() >= budget => Err(format!(
                "the budget of {budget} for the provider is spent; raise it to go on"
            )),
            _ => Ok(()),
        }
    }

    fn spent(
        &self,
        agent: &str,
        model: &str,
        usage: iq_core::Usage,
        cost: Option<Usd>,
        billed: bool,
    ) {
        if billed && let Some(cost) = cost {
            *lock(&self.inner.billed) += cost;
        }
        self.report(Event::Spent {
            agent: agent.to_owned(),
            model: model.to_owned(),
            usage,
            cost,
            billed,
        });
    }

    fn report(&self, event: Event) {
        (self.inner.report)(event);
    }

    /// The standing instructions of one of iq's agents.
    fn instructions(&self, name: &str, by: &str, role: Option<&str>) -> String {
        let workspace = self.inner.gate.workspace().display();
        let started = if by == PERSON {
            "The person gave you your task.".to_owned()
        } else {
            format!("The agent {by} started you and gets your answer.")
        };
        let part = role_line(role);
        format!(
            "You are {name}, an agent of iq, a coding harness, working in {workspace}. {started}{part}\n\n\
             Your tools act on the machine through iq's gate: files are read and written in the \
             workspace, and commands run in a sandbox that writes only the workspace. Commands \
             reach localhost (servers you start, the tests that talk to them); the internet is \
             given to what fetches packages and sources (npm install, cargo fetch, pip install, \
             git fetch), and asked of the person for others with `network: true`, unless the \
             person opened it to every command. What the gate holds waits for the person, or for \
             a reviewer acting for them; when it is refused, you are told why: find another way, \
             or say what you need.\n\n\
             Other agents may work beside you. start_agent hands a task to a new one, \
             send_message writes to one, to all, or to the person, and wait_for_messages waits \
             for an answer. Messages to you also arrive between your calls. The steward watches \
             what the crew spends against the person's limits: when it writes to you, answer it \
             in a line, with send_message to steward, and do as it asks.\n\n\
             Read before you change, check your work with the project's own tools (its build, \
             tests, linters), and be brief. When the task is done, answer with what you did and \
             call no tool."
        )
    }

    /// What an agent with tools of its own is told of the crew.
    fn instructions_for_other(
        &self,
        name: &str,
        by: &str,
        role: Option<&str>,
        reaches: bool,
    ) -> String {
        let part = role_line(role);
        if !reaches {
            return format!("You are {name}, an agent of iq's, started by {by}.{part}");
        }
        format!(
            "You are {name}, an agent of iq's, started by {by}.{part} Your commands run in iq's \
             sandbox, which writes only this folder. They reach localhost; the internet is \
             given to what fetches packages and sources (npm install, cargo fetch, pip install, \
             git fetch), and to ask the person for it for another command, start its line with \
             IQ_NETWORK=1. What iq's gate holds waits for the person. iq's tools (send_message, wait_for_messages, \
             start_agent, list_agents) let you write to the other agents and to the person, and \
             hand tasks to new agents. Messages written to you while you work arrive beside \
             your tools' results; wait for one with wait_for_messages when your work depends on \
             others. The steward watches what the crew spends against the person's limits: when \
             it writes to you, answer it in a line, with send_message to steward, and do as it \
             asks."
        )
    }
}

/// The line of an agent's instructions that gives it its part, if any.
fn role_line(role: Option<&str>) -> String {
    role.map(|role| {
        format!(
            " Your role in the crew: {role}. Keep to it, and hand what lies outside it to the \
             agent whose part it is, or to the one that started you."
        )
    })
    .unwrap_or_default()
}

/// What a command prints when it needed the internet and had none.
const OFFLINE: &[&str] = &[
    "Could not resolve host",
    "Temporary failure in name resolution",
    "Name or service not known",
    "nodename nor servname",
    "getaddrinfo",
    "ENOTFOUND",
    "EAI_AGAIN",
    "Network is unreachable",
    "network is unreachable",
    "failed to lookup address",
    "dns error",
    "Failed to establish a new connection",
    "dial tcp",
    "--offline was specified",
];

/// How one of iq's agents asks for the internet for a command.
const OWN_ASK: &str = "to have it, run it again with \"network\": true, which the person approves, \
                       unless a rule gives it";

/// How Claude Code does.
const CLAUDE_ASK: &str = "to have it, run the same line again starting with IQ_NETWORK=1, \
                          which the person approves, unless a rule gives it";

/// A turn under way: counted until it ends, or is stopped, as its task
/// drops it either way.
struct Busy<M, D>(Crew<M, D>);

impl<M, D> Drop for Busy<M, D> {
    fn drop(&mut self) {
        self.0.inner.busy.fetch_sub(1, Ordering::SeqCst);
    }
}

/// An action in words, as the person reads it.
fn describe(action: &Action) -> String {
    match action {
        Action::Read { path } => format!("read {}", path.display()),
        Action::Write { path } => format!("write {}", path.display()),
        Action::Run {
            line,
            folder,
            network,
        } => {
            let network = if *network { ", with the internet" } else { "" };
            format!("run `{line}` in {}{network}", folder.display())
        }
        Action::Fetch { target } => format!("reach the network for {target}"),
    }
}

/// An error with its causes, as one line.
pub(crate) fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Locks `mutex`, even one a panicking thread left: the state it guards
/// is replaced whole, never left half changed.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
