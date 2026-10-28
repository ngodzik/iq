//! The crew: the agents at work in one workspace, the board they write to
//! each other on, and the one way any of them acts on the machine, through
//! the gate.
//!
//! An agent is a model of the provider, which iq runs in a loop with its
//! tools, the board's among them. What the gate asks goes to the person,
//! or, the autopilot being on, to the reviewer, which may answer only what
//! stays within the sandbox.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use iq_core::{
    ChatModel, ChatRequest, Effort, Message, ModelId, TokenCount, ToolCall, ToolSpec, Usd,
};
use iq_gate::{Action, Approver, Gate, Helper, Judged, Pending, Permit, Sandbox, Scope};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::event::{Event, PERSON};
use crate::extension::Extension;
use crate::review;
use crate::rules::{self, RulesFile};
use crate::team::{self, Pattern, Phase, Seat, Team};
use crate::tools::{self, Args};

/// Reports what the crew does, as it does it.
pub type Report = Arc<dyn Fn(Event) + Send + Sync>;

/// Everyone on the board: a message to `all` reaches every agent.
pub const ALL: &str = "all";

/// How often two agents may wake each other in [`PEER_SPAN`], whichever
/// writes: more, and they talk on without the work moving, on the
/// person's money.
const PEER_WAKES: usize = 4;

/// How often agents may wake each other in [`PEER_SPAN`], all of them.
const CREW_WAKES: usize = 16;

/// The span those wakes are counted over.
const PEER_SPAN: Duration = Duration::from_secs(10 * 60);

/// How many crews this program made: each numbers its questions apart
/// from the others', so that an answer reaches the crew that asked when
/// one window shows several.
static CREWS: AtomicU64 = AtomicU64::new(0);

/// The files a project's rules for agents are read from, each found: the
/// shared format, then the other one many keep, which often imports it.
const RULES_FILES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// The folder of the project's rules, each a Markdown file, for the whole
/// project or, as its head names `paths`, for those files.
const RULES_FOLDER: &str = ".claude/rules";

/// How long the project's rules for some files are read once for, before
/// they are read again for what changed.
const SCOPED_FRESH: Duration = Duration::from_secs(30);

/// How many tokens an agent's conversation grows between two words to it
/// of its size, unless the settings say.
pub const CONTEXT_EVERY: u64 = 25_000;

/// How many actions of one agent the reviewer refuses in a row before the
/// next is asked of the person, unless the settings say.
pub const REFUSALS: u32 = 3;

/// What the crew starts with.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The model of the provider that answers for the person when the
    /// autopilot is on: none, and the person is asked.
    pub reviewer: Option<ModelId>,
    /// How hard models think; `None` leaves it to them.
    pub effort: Option<Effort>,
    /// How long a question waits for the person before it is refused.
    pub patience: Duration,
    /// The most calls to its model one turn of an agent of iq's makes.
    pub steps: usize,
    /// The person's own rules, read from their files, given to the agents
    /// the person starts, with the project's.
    pub rules: Vec<RulesFile>,
    /// How many tokens an agent's conversation grows between two words to
    /// it of its size, which it compacts as its skills say; 0 for none.
    pub context_every: u64,
    /// How many actions of one agent the reviewer refuses in a row before
    /// the next is asked of the person instead; 0 never.
    pub refusals: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            reviewer: None,
            effort: None,
            patience: Duration::from_secs(50 * 60),
            steps: 200,
            rules: Vec::new(),
            context_every: CONTEXT_EVERY,
            refusals: REFUSALS,
        }
    }
}

/// The agents in one workspace. Cheap to clone: every clone is the same
/// crew.
pub struct Crew<M> {
    inner: Arc<Inner<M>>,
}

impl<M> Clone for Crew<M> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct Inner<M> {
    provider: M,
    gate: Gate,
    sandbox: Sandbox,
    settings: Settings,
    report: Report,
    autopilot: AtomicBool,
    /// How many actions of each agent the reviewer refused in a row.
    refused: Mutex<HashMap<String, u32>>,
    /// Turns under way, the answers they post included.
    busy: AtomicUsize,
    next_question: AtomicU64,
    questions: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    board: Mutex<Board>,
    /// How many messages the board holds, for those who wait for one.
    posted: watch::Sender<usize>,
    /// What the provider's key was billed so far.
    billed: Mutex<Usd>,
    /// When agents woke each other lately, and which two.
    peer_wakes: Mutex<Vec<(Instant, (String, String))>>,
    /// The teams at work, and where each stands.
    teams: Mutex<Vec<Team>>,
    /// The tools other parts of iq offer some agents.
    extensions: Mutex<Vec<Arc<dyn Extension<M>>>>,
    /// The project's rules for some of its files, as last read, and when.
    scoped: Mutex<Option<(Instant, Vec<rules::Scoped>)>>,
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
    /// Its conversation, kept between turns.
    messages: Vec<Message>,
    /// How hard it thinks: as it was started or changed, else as the
    /// settings say.
    effort: Option<Effort>,
    /// The tokens its last call read: its conversation, as it is now.
    context: u64,
    /// Its size when it was last told it.
    told: u64,
    /// The summary it wrote of its conversation, which replaces it once its
    /// turn ends.
    compacted: Option<String>,
    /// The project's rules for some files it was given, by their files.
    rules_given: Vec<String>,
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

impl<M> Crew<M>
where
    M: ChatModel + 'static,
{
    /// A crew with no agent yet, its models answered by `provider`, acting
    /// through `gate`, its commands run in the gate's sandbox through
    /// `helper`.
    pub fn new(
        provider: M,
        gate: Gate,
        helper: Helper,
        settings: Settings,
        report: Report,
    ) -> Self {
        let sandbox = gate.sandbox(helper);
        Self {
            inner: Arc::new(Inner {
                provider,
                gate,
                sandbox,
                settings,
                report,
                autopilot: AtomicBool::new(false),
                refused: Mutex::new(HashMap::new()),
                busy: AtomicUsize::new(0),
                next_question: AtomicU64::new((CREWS.fetch_add(1, Ordering::Relaxed) << 32) | 1),
                questions: Mutex::new(HashMap::new()),
                board: Mutex::new(Board::default()),
                posted: watch::Sender::new(0),
                billed: Mutex::new(Usd(0.0)),
                peer_wakes: Mutex::new(Vec::new()),
                teams: Mutex::new(Vec::new()),
                extensions: Mutex::new(Vec::new()),
                scoped: Mutex::new(None),
            }),
        }
    }

    /// Offers `extension`'s tools to the agents it serves, from their
    /// next turn.
    pub fn extend(&self, extension: Arc<dyn Extension<M>>) {
        lock(&self.inner.extensions).push(extension);
    }

    /// The extensions, taken out of their lock to be called.
    fn extensions(&self) -> Vec<Arc<dyn Extension<M>>> {
        lock(&self.inner.extensions).clone()
    }

    /// The extensions that serve an agent started `by` the person or
    /// another agent: those for leads serve only the person's.
    fn extensions_for(&self, by: &str) -> Vec<Arc<dyn Extension<M>>> {
        self.extensions()
            .into_iter()
            .filter(|extension| by == PERSON || !extension.leads_only())
            .collect()
    }

    /// Who started `agent`: the person, or another agent.
    fn started_by(&self, agent: &str) -> String {
        lock(&self.inner.board)
            .members
            .get(agent)
            .map(|member| member.by.clone())
            .unwrap_or_default()
    }

    /// The tools `agent` finds through iq: the board's, and those
    /// extensions offer it.
    pub fn tools_for(&self, agent: &str) -> Vec<ToolSpec> {
        let mut tools = tools::board();
        for extension in self.extensions_for(&self.started_by(agent)) {
            tools.extend(extension.tools(agent));
        }
        tools
    }

    /// What extensions tell `agent`, started `by`, of their tools, as a
    /// paragraph of its instructions, if any does.
    fn extended(&self, agent: &str, by: &str) -> String {
        self.extensions_for(by)
            .iter()
            .filter_map(|extension| extension.instructions(agent))
            .map(|text| format!("\n\n{text}"))
            .collect()
    }

    /// The gate the crew acts through.
    pub fn gate(&self) -> &Gate {
        &self.inner.gate
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

    /// Gives every command the internet, or only what a rule gives it to
    /// and what the person allows.
    pub fn set_internet(&self, open: bool) {
        self.inner.gate.set_internet(open);
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
    /// as models, two at the least; the first thinking with `effort`, as
    /// the settings say when `None`. Returns the first agent's name:
    /// `name`, made unique.
    pub fn launch(
        &self,
        pattern: Pattern,
        name: &str,
        models: &[ModelId],
        task: String,
        role: Option<String>,
        effort: Option<Effort>,
    ) -> String {
        let Some(first) = models.first() else {
            return String::new();
        };
        // Held while the team starts, so that a turn ending at once finds
        // its team.
        let mut teams = lock(&self.inner.teams);
        match pattern {
            Pattern::Solo => self.start_with(name, first, task, PERSON, role, effort),
            Pattern::Lead => {
                let cheaper: Vec<String> = models[1..].iter().map(ToString::to_string).collect();
                let role = Some(team::lead_role(&cheaper));
                self.start_with(name, first, task, PERSON, role, effort)
            }
            Pattern::Pair => {
                let writer = self.start_with(
                    name,
                    first,
                    task.clone(),
                    PERSON,
                    Some(team::WRITER_ROLE.to_owned()),
                    effort,
                );
                teams.push(Team::Pair {
                    writer: writer.clone(),
                    reviewer: None,
                    reviewer_model: models.get(1).unwrap_or(first).clone(),
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
        self.start_with(name, model, task, by, role, None)
    }

    /// Starts an agent as [`Self::start_as`] does, thinking with `effort`,
    /// as the settings say when `None`.
    fn start_with(
        &self,
        name: &str,
        model: &ModelId,
        task: String,
        by: &str,
        role: Option<String>,
        effort: Option<Effort>,
    ) -> String {
        let name = self.join(name, model, task.clone(), by, role, effort, true);
        self.wake(&name, task);
        name
    }

    /// Puts an agent on the board, its name `name` made unique, and gives
    /// it. One that `works` is about to be woken on `task`; another holds
    /// its task and waits until it is written to.
    #[allow(clippy::too_many_arguments)]
    fn join(
        &self,
        name: &str,
        model: &ModelId,
        task: String,
        by: &str,
        role: Option<String>,
        effort: Option<Effort>,
        works: bool,
    ) -> String {
        let mut board = lock(&self.inner.board);
        let name = free_name(&board.members, &base_name(name), &[]);
        let mut messages = vec![Message::system(self.instructions(
            &name,
            by,
            role.as_deref(),
        ))];
        if !works {
            messages.push(Message::user(task.clone()));
        }
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
                messages,
                effort,
                context: 0,
                told: 0,
                compacted: None,
                rules_given: Vec::new(),
            },
        );
        drop(board);
        self.report(Event::Joined {
            agent: name.clone(),
            model: model.to_string(),
            by: by.to_owned(),
        });
        self.report(Event::Effort {
            agent: name.clone(),
            effort: effort.or(self.inner.settings.effort),
        });
        name
    }

    /// Starts a team, named `team`, on `task` for `by`: one agent for each
    /// seat, told the others. The first works at once and answers `by`;
    /// the others wait until a teammate writes to them, and answer the
    /// first. Says what was started, as the agent that asked reads it.
    fn start_team(&self, by: &str, team: &str, task: &str, seats: &[Seat]) -> String {
        let names: Vec<String> = {
            let board = lock(&self.inner.board);
            let mut names: Vec<String> = Vec::new();
            for seat in seats {
                let base = base_name(&format!("{team}-{}", seat.role));
                names.push(free_name(&board.members, &base, &names));
            }
            names
        };
        let brief = |at: usize| format!("{task}\n\n{}", team::roster(&names, seats, at, by));
        let role = |seat: &Seat| Some(format!("{} in the team {team}: {}", seat.role, seat.does));
        // Those who wait join first, so that the first finds them when it
        // writes to them.
        for (at, seat) in seats.iter().enumerate().skip(1) {
            self.join(
                &names[at],
                &seat.model,
                brief(at),
                &names[0],
                role(seat),
                seat.effort,
                false,
            );
        }
        let first = &seats[0];
        let lead = self.start_with(
            &names[0],
            &first.model,
            brief(0),
            by,
            role(first),
            first.effort,
        );
        let waiting: Vec<String> = names[1..]
            .iter()
            .zip(&seats[1..])
            .map(|(name, seat)| format!("{name} on {}", seat.model))
            .collect();
        let waits = match waiting.len() {
            0 => String::new(),
            1 => format!(" {} waits for a teammate to write to it.", waiting[0]),
            _ => format!(
                " {} wait for a teammate to write to them.",
                waiting.join(", ")
            ),
        };
        format!(
            "The team {team} is started: {lead} works on it now, on {}.{waits} {lead}'s answer \
             will come to you as a message.",
            first.model
        )
    }

    /// Has `agent` think with `effort` from its next call to its model.
    ///
    /// # Errors
    ///
    /// No agent of that name.
    pub fn set_effort(&self, agent: &str, effort: Effort) -> Result<(), String> {
        let mut board = lock(&self.inner.board);
        let member = board
            .members
            .get_mut(agent)
            .ok_or_else(|| format!("no agent is named {agent:?}"))?;
        member.effort = Some(effort);
        drop(board);
        self.report(Event::Effort {
            agent: agent.to_owned(),
            effort: Some(effort),
        });
        Ok(())
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

    /// Ends the crew: every agent stopped, and what their commands left
    /// running stopped with the sandbox.
    pub async fn end(&self) {
        self.stop_all();
        self.inner.sandbox.end().await;
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
            if to != ALL && to != PERSON && !board.members.contains_key(to) {
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
            let taken = board.members.get(&name).map(|member| {
                (
                    member.messages.clone(),
                    member.model.clone(),
                    member.by.clone(),
                )
            });
            (taken, board.messages.len())
        };
        let Some((messages, model, by)) = taken else {
            return;
        };
        let (messages, outcome) = self.own_turn(&name, &model, messages, prompt).await;
        let mut compacted_from = None;
        if let Some(member) = lock(&self.inner.board).members.get_mut(&name) {
            member.messages = match member.compacted.take() {
                // Its request done, its conversation is its summary.
                Some(summary) => {
                    compacted_from = Some(member.context);
                    member.context = 0;
                    member.told = 0;
                    // Its summary may not hold them: given again as it
                    // touches their files.
                    member.rules_given.clear();
                    compacted(&messages, &member.task, &summary, outcome.as_ref().ok())
                }
                None => messages,
            };
            // It rests from now: what is posted to it wakes it.
            member.turn = None;
        }
        if let Some(before) = compacted_from {
            self.report(Event::Notice {
                agent: name.clone(),
                text: format!(
                    "compacted: its calls read {} tokens; from its next turn, its task and its \
                     summary",
                    TokenCount(before)
                ),
            });
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

    /// What `agent` on `model` is told of its conversation's size, once it
    /// grew by the settings' step since it was last told: how much each
    /// call reads, and where its model's price rises, when the provider
    /// says. Counted as told.
    async fn context_note(&self, agent: &str, model: &ModelId) -> Option<String> {
        let step = self.inner.settings.context_every;
        if step == 0 {
            return None;
        }
        let context = {
            let mut board = lock(&self.inner.board);
            let member = board.members.get_mut(agent)?;
            if member.context < member.told + step {
                return None;
            }
            member.told = member.context;
            member.context
        };
        let rises = self
            .inner
            .provider
            .pricing(model)
            .await
            .and_then(|pricing| pricing.rises_at());
        Some(context_said(TokenCount(context), rises))
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
        tools.extend(self.tools_for(name));
        let mut urged = false;
        for _ in 0..self.inner.settings.steps {
            if let Some(news) = self.unread(name) {
                messages.push(Message::user(news));
            }
            if let Some(note) = self.context_note(name, model).await {
                messages.push(Message::user(note));
            }
            // Read at each call, so that a change reaches a turn under way.
            let effort = lock(&self.inner.board)
                .members
                .get(name)
                .and_then(|member| member.effort)
                .or(self.inner.settings.effort);
            let request = ChatRequest {
                model: model.clone(),
                messages: messages.clone(),
                tools: tools.clone(),
                effort,
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
            self.spent(name, model.as_str(), response.usage, cost);
            if let Some(member) = lock(&self.inner.board).members.get_mut(name) {
                member.context = response.usage.input.0;
            }
            let said = response
                .content
                .as_ref()
                .is_some_and(|t| !t.trim().is_empty());
            if !said && response.tool_calls.is_empty() {
                // Nothing written and nothing called is no answer: a model
                // that spent its room thinking, or lost its way. It is
                // urged on once, then the turn fails saying why.
                if urged {
                    return (
                        messages,
                        Err("its model answered twice with nothing: no text, no tool call".into()),
                    );
                }
                urged = true;
                self.report(Event::Notice {
                    agent: name.to_owned(),
                    text: "its model answered with nothing; it is told to go on".into(),
                });
                messages.push(Message::user(
                    "Your reply held neither text nor a tool call. Go on with the task: call \
                     a tool, or give your answer.",
                ));
                continue;
            }
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
                let path: std::path::PathBuf = args.text("path")?.into();
                let permit = self.act(agent, Action::Read { path: path.clone() }).await?;
                let bytes = permit.read().map_err(|e| chain(&e))?;
                let text = String::from_utf8(bytes)
                    .map_err(|_| "the file is not text: run a command to look into it")?;
                Ok(
                    tools::numbered(&text, args.number("offset"), args.number("limit"))
                        + &self.scoped_for(agent, &path),
                )
            }
            "write_file" => {
                let path: std::path::PathBuf = args.text("path")?.into();
                let content = args.text("content")?;
                let permit = self
                    .act(agent, Action::Write { path: path.clone() })
                    .await?;
                permit.write(content.as_bytes()).map_err(|e| chain(&e))?;
                Ok(format!("Wrote {} bytes.", content.len()) + &self.scoped_for(agent, &path))
            }
            "edit_file" => {
                let path: std::path::PathBuf = args.text("path")?.into();
                let permit = self
                    .act(agent, Action::Write { path: path.clone() })
                    .await?;
                let bytes = permit.read().map_err(|e| chain(&e))?;
                let text = String::from_utf8(bytes).map_err(|_| "the file is not text")?;
                let edited = tools::replaced(
                    &text,
                    args.text("old_text")?,
                    args.text("new_text")?,
                    args.flag("all"),
                )?;
                permit.write(edited.as_bytes()).map_err(|e| chain(&e))?;
                // Given once the edit is made: an edit refused would give
                // them to nobody.
                Ok(format!("Edited.{}", self.scoped_for(agent, &path)))
            }
            "run" => {
                let line = args.text("command")?.to_owned();
                let action = Action::Run {
                    line,
                    folder: workspace,
                    network: args.flag("network"),
                    host: args.flag("host"),
                };
                let permit = self.act(agent, action).await?;
                self.run(permit, OWN_ASK).await.map(|(text, _)| text)
            }
            _ => self.board_tool(agent, name, args).await,
        }
    }

    /// Runs one of the board's tools.
    pub(crate) async fn board_tool(
        &self,
        agent: &str,
        name: &str,
        args: &Args,
    ) -> Result<String, String> {
        match name {
            "start_agent" => {
                let model = ModelId::new(args.text("model")?).map_err(|e| e.to_string())?;
                let role = args
                    .text("role")
                    .ok()
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                    .map(str::to_owned);
                let effort = match args.text("effort").map(str::trim) {
                    Ok(level) if !level.is_empty() => Some(level.parse::<Effort>()?),
                    _ => None,
                };
                let started = self.start_with(
                    args.text("name")?,
                    &model,
                    args.text("task")?.to_owned(),
                    agent,
                    role,
                    effort,
                );
                Ok(format!(
                    "{started} started on {model}. Its answer will come to you as a message."
                ))
            }
            "start_team" => {
                let seats = team::seats(&args.value()["members"])?;
                Ok(self.start_team(agent, args.text("name")?, args.text("task")?, &seats))
            }
            "compact" => {
                let summary = args.text("summary")?.trim();
                if summary.is_empty() {
                    return Err("write the summary in `summary`".into());
                }
                if let Some(member) = lock(&self.inner.board).members.get_mut(agent) {
                    member.compacted = Some(summary.to_owned());
                }
                Ok(
                    "Kept: once you answer, your conversation is your task and this summary. \
                    Answer now."
                        .into(),
                )
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
            _ => {
                let extension = self
                    .extensions_for(&self.started_by(agent))
                    .into_iter()
                    .find(|extension| extension.tools(agent).iter().any(|t| t.name == name));
                match extension {
                    Some(extension) => {
                        extension
                            .call(
                                self.clone(),
                                agent.to_owned(),
                                name.to_owned(),
                                args.value().clone(),
                            )
                            .await
                    }
                    None => Err(format!("there is no tool named {name:?}")),
                }
            }
        }
    }

    /// Runs the command `permit` allows, and says how it went, and whether
    /// it succeeded. A command that failed without the internet, as one
    /// that needs it does, is told how to ask for it: `ask`.
    pub(crate) async fn run(&self, permit: Permit, ask: &str) -> Result<(String, bool), String> {
        let offline = matches!(
            permit.action(),
            Action::Run {
                network: false,
                host: false,
                ..
            }
        );
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
            Judged::Denied { reason } => Err(refusal("the gate", &reason, false)),
            Judged::Asked { reason, pending } => self.ask(agent, reason, pending).await,
        }
    }

    /// Asks the reviewer, the autopilot being on and the action one it may
    /// answer, else the person. After as many refusals in a row as the
    /// settings say, an agent's next action goes to the person: the
    /// reviewer and the agent are not getting anywhere.
    async fn ask(&self, agent: &str, reason: String, pending: Pending) -> Result<Permit, String> {
        let action = describe(pending.action());
        // The reviewer answers what stays within the sandbox, what only
        // reaches the internet and what runs on the machine when the
        // person lets it; never more.
        let reviewable = match pending.scope() {
            Scope::Sandbox => true,
            Scope::Internet => self.inner.gate.reviewer_internet(),
            Scope::Host => self.inner.gate.reviewer_host(),
            Scope::Beyond => false,
        };
        let most = self.inner.settings.refusals;
        let refused = lock(&self.inner.refused).get(agent).copied().unwrap_or(0);
        if self.autopilot() && reviewable && most > 0 && refused >= most {
            self.report(Event::Notice {
                agent: agent.to_owned(),
                text: format!(
                    "the reviewer refused {refused} actions of {agent} in a row: the person is \
                     asked this one"
                ),
            });
        } else if self.autopilot() && reviewable {
            match self.review(agent, &action, pending.scope(), &reason).await {
                Ok((approved, why)) => {
                    self.report(Event::Reviewed {
                        agent: agent.to_owned(),
                        action: action.clone(),
                        reason: reason.clone(),
                        approved,
                        why: why.clone(),
                    });
                    let mut refused = lock(&self.inner.refused);
                    if approved {
                        refused.remove(agent);
                    } else {
                        *refused.entry(agent.to_owned()).or_default() += 1;
                    }
                    drop(refused);
                    return if approved {
                        self.inner
                            .gate
                            .approve(pending, Approver::Reviewer)
                            .map_err(|e| chain(&e))
                    } else {
                        Err(refusal("the reviewer", &why, true))
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
        // The person's answer, whichever it is, starts the count over.
        lock(&self.inner.refused).remove(agent);
        match answer {
            Ok(Ok(true)) => self
                .inner
                .gate
                .approve(pending, Approver::Person)
                .map_err(|e| chain(&e)),
            Ok(Ok(false)) => Err(refusal("the person", &reason, true)),
            _ => Err(refusal(
                "the gate",
                &format!("nobody answered in time ({reason})"),
                true,
            )),
        }
    }

    /// The reviewer's verdict on an action of `agent`'s, of `scope`.
    async fn review(
        &self,
        agent: &str,
        action: &str,
        scope: Scope,
        reason: &str,
    ) -> Result<(bool, String), String> {
        let task = lock(&self.inner.board)
            .members
            .get(agent)
            .map(|member| member.task.clone())
            .unwrap_or_default();
        let prompt = review::prompt(&task, action, review::place(scope), reason);
        let model = self
            .inner
            .settings
            .reviewer
            .as_ref()
            .ok_or("no reviewer is set")?;
        let mut instructions = review::INSTRUCTIONS.to_owned();
        for extension in self.extensions() {
            if let Some(policy) = extension.review_instructions()? {
                instructions
                    .push_str("\n\nThe person's local review policy, within the limits above:\n");
                instructions.push_str(&policy);
            }
        }
        let request = ChatRequest {
            model: model.clone(),
            messages: vec![Message::system(instructions), Message::user(prompt)],
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
        self.spent("reviewer", model.as_str(), response.usage, cost);
        let answer = response.content.unwrap_or_default();
        Ok(review::verdict(&answer))
    }

    /// Lets the reviewer give a command the internet, the autopilot on, or
    /// leaves that to the person. Publishing and what leaves the workspace
    /// are always the person's.
    pub fn set_autopilot_internet(&self, may: bool) {
        self.inner.gate.set_reviewer_internet(may);
    }

    /// Lets the reviewer let a command run on the machine itself, the
    /// autopilot on, following the policy the person wrote for it, or
    /// leaves that to the person. What changes what others share is always
    /// the person's.
    pub fn set_autopilot_host(&self, may: bool) {
        self.inner.gate.set_reviewer_host(may);
    }

    fn spent(&self, agent: &str, model: &str, usage: iq_core::Usage, cost: Option<Usd>) {
        if let Some(cost) = cost {
            *lock(&self.inner.billed) += cost;
        }
        self.report(Event::Spent {
            agent: agent.to_owned(),
            model: model.to_owned(),
            usage,
            cost,
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
             for an answer. Messages to you also arrive between your calls.\n\n\
             Read before you change, check your work with the project's own tools (its build, \
             tests, linters), and be brief. When the task is done, answer with what you did and \
             call no tool.{}{}",
            if by == PERSON {
                self.known()
            } else {
                String::new()
            },
            self.extended(name, by)
        )
    }

    /// What an agent the person started is told beyond its task: the
    /// person's rules and the project's, and that the agents it starts
    /// know none of what it knows but what it gives them.
    fn known(&self) -> String {
        let mut out = String::new();
        for file in &self.inner.settings.rules {
            if let Some(text) = rules::kept(&file.text) {
                out.push_str(&format!(
                    "\n\nThe person's rules, from their {}, which you follow:\n\n{text}",
                    file.from
                ));
            }
        }
        for (file, text) in self.project_rules() {
            out.push_str(&format!(
                "\n\nThe project's rules for agents, from its {file}, which you follow:\n\n{text}"
            ));
        }
        let scoped = self.scoped_rules();
        if !scoped.is_empty() {
            out.push_str(
                "\n\nThe project's rules for some of its files, which an agent is given as it \
                 first reads or writes one of them: read those that bear on work you give, and \
                 put what it needs of them in its task.",
            );
            for rule in &scoped {
                out.push_str(&format!("\n- {}: {}", rule.file, rule.paths.join(", ")));
            }
        }
        out.push_str(
            "\n\nAn agent or a team you start sees only the task you give it: nothing of your \
             conversation, nor of these rules, nor of what else you are told beside your task. \
             Write into its task what its part needs of them (the rules its work must follow, the \
             facts it must know, the files it works on), and nothing more: it reads all of it on \
             every call.",
        );
        out
    }

    /// Reads `path` in the workspace through the gate, as an agent's reads
    /// are, never asking the person: what the gate does not allow at once
    /// is not read.
    fn read_quietly(&self, path: std::path::PathBuf) -> Option<String> {
        match self.inner.gate.judge(Action::Read { path }).ok()? {
            Judged::Allowed(permit) => String::from_utf8(permit.read().ok()?).ok(),
            Judged::Asked { .. } | Judged::Denied { .. } => None,
        }
    }

    /// The project's rules for agents, each with the file it came from: its
    /// `AGENTS.md`, then its `CLAUDE.md`, whose `@file` lines are the
    /// files they import, a file given already left out; then those of its
    /// `.claude/rules` for the whole project.
    fn project_rules(&self) -> Vec<(String, String)> {
        let workspace = self.inner.gate.workspace().to_path_buf();
        let mut given: Vec<(String, String)> = Vec::new();
        for file in RULES_FILES {
            let path = workspace.join(file);
            if !path.is_file() {
                continue;
            }
            let Some(text) = self.read_quietly(path) else {
                continue;
            };
            let text: String = text
                .lines()
                .map(|line| match rules::import(line) {
                    // As `AGENTS.md` is, imported by `CLAUDE.md`.
                    Some(name) if given.iter().any(|(file, _)| file == name) => "\n".to_owned(),
                    Some(name) => {
                        self.read_quietly(workspace.join(name))
                            .unwrap_or_else(|| line.to_owned())
                            + "\n"
                    }
                    None => line.to_owned() + "\n",
                })
                .collect();
            if let Some(text) = rules::kept(&text) {
                given.push((file.to_owned(), text));
            }
        }
        for (file, text) in self.rules_folder() {
            let (paths, body) = rules::scope(&text);
            if paths.is_empty()
                && let Some(body) = rules::kept(body)
            {
                given.push((file, body));
            }
        }
        given
    }

    /// The files of the project's `.claude/rules`, each with what it says,
    /// in the order of their names.
    fn rules_folder(&self) -> Vec<(String, String)> {
        let workspace = self.inner.gate.workspace().to_path_buf();
        let mut found = Vec::new();
        let mut folders = vec![workspace.join(RULES_FOLDER)];
        while let Some(folder) = folders.pop() {
            let Ok(entries) = std::fs::read_dir(&folder) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    folders.push(path);
                } else if path.extension().is_some_and(|ending| ending == "md") {
                    found.push(path);
                }
            }
        }
        found.sort();
        found
            .into_iter()
            .filter_map(|path| {
                let file = path
                    .strip_prefix(&workspace)
                    .ok()?
                    .to_string_lossy()
                    .into_owned();
                Some((file, self.read_quietly(path)?))
            })
            .collect()
    }

    /// The project's rules for some of its files, read again once they are
    /// older than [`SCOPED_FRESH`].
    fn scoped_rules(&self) -> Vec<rules::Scoped> {
        if let Some((read, rules)) = lock(&self.inner.scoped).as_ref()
            && read.elapsed() < SCOPED_FRESH
        {
            return rules.clone();
        }
        let rules: Vec<rules::Scoped> = self
            .rules_folder()
            .into_iter()
            .filter_map(|(file, text)| {
                let (paths, body) = rules::scope(&text);
                let text = rules::kept(body)?;
                (!paths.is_empty()).then_some(rules::Scoped { file, paths, text })
            })
            .collect();
        *lock(&self.inner.scoped) = Some((Instant::now(), rules.clone()));
        rules
    }

    /// The project's rules for `path` that `agent` was not given yet, as
    /// it reads or writes it, to follow on the tool's answer; marked given.
    fn scoped_for(&self, agent: &str, path: &std::path::Path) -> String {
        let workspace = self.inner.gate.workspace();
        let relative = if path.is_absolute() {
            match path.strip_prefix(workspace) {
                Ok(relative) => relative,
                Err(_) => return String::new(),
            }
        } else {
            path
        };
        let relative = relative.to_string_lossy();
        let matching: Vec<rules::Scoped> = self
            .scoped_rules()
            .into_iter()
            .filter(|rule| {
                rule.paths
                    .iter()
                    .any(|pattern| iq_core::glob_matches(pattern, &relative))
            })
            .collect();
        if matching.is_empty() {
            return String::new();
        }
        let mut board = lock(&self.inner.board);
        let Some(member) = board.members.get_mut(agent) else {
            return String::new();
        };
        let mut out = String::new();
        for rule in matching {
            if member.rules_given.contains(&rule.file) {
                continue;
            }
            out.push_str(&format!(
                "\n\n[iq: the project's rules for {relative}, from its {}, which you follow:]\n{}",
                rule.file, rule.text
            ));
            member.rules_given.push(rule.file);
        }
        out
    }
}

/// `name` as the board writes names: lower case, letters, digits and
/// dashes, never one the board keeps for itself.
fn base_name(name: &str) -> String {
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
    if base.is_empty() || base == ALL || base == PERSON {
        "agent".to_owned()
    } else {
        base
    }
}

/// `base`, or `base-2`, `base-3`…: the first no member has, nor `taken`.
fn free_name(members: &BTreeMap<String, Member>, base: &str, taken: &[String]) -> String {
    let free = |name: &str| !members.contains_key(name) && !taken.iter().any(|t| t == name);
    let mut name = base.to_owned();
    let mut n = 2;
    while !free(&name) {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

/// What an agent is told of its conversation's size: `context` tokens read
/// on each call, and where its model's price rises, when known.
fn context_said(context: TokenCount, rises: Option<TokenCount>) -> String {
    let past = match rises {
        Some(rises) if context >= rises => {
            format!(": past the {rises} where your model's price per token rises")
        }
        Some(rises) => format!("; your model's price per token rises once a call reads {rises}"),
        None => String::new(),
    };
    format!("(iq: your conversation is {context} tokens now, read again on each call{past}.)")
}

/// A conversation compacted: its standing instructions, its task and the
/// summary its agent wrote of the rest, then its last answer.
fn compacted(
    messages: &[Message],
    task: &str,
    summary: &str,
    answer: Option<&String>,
) -> Vec<Message> {
    let mut kept: Vec<Message> = messages.first().cloned().into_iter().collect();
    kept.push(Message::user(format!(
        "{task}\n\n(iq: what came after, as you summed it up when you compacted your \
         conversation:)\n\n{summary}"
    )));
    if let Some(answer) = answer.filter(|answer| !answer.trim().is_empty()) {
        kept.push(Message::Assistant {
            content: Some(answer.clone()),
            tool_calls: Vec::new(),
        });
    }
    kept
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

/// What an agent is told of an action refused: who refused it and why, and
/// what it may do now, so that it asks or takes another way rather than
/// getting around what was refused. What was asked may be asked of the
/// person; what the gate denies, nobody approves.
fn refusal(who: &str, reason: &str, asked: bool) -> String {
    let reason = reason.trim().trim_end_matches('.');
    let then = if asked {
        "tell the person what is blocked and what you need, and wait for their answer"
    } else {
        "tell the person what is blocked: nobody can approve it"
    };
    format!(
        "{who} refused it: {reason}. Do not get around it: no downloading or building a \
         replacement tool, no changing the project to avoid it. Take another way only if the \
         task allows it and it keeps to this refusal; otherwise {then}."
    )
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

/// A turn under way: counted until it ends, or is stopped, as its task
/// drops it either way.
struct Busy<M>(Crew<M>);

impl<M> Drop for Busy<M> {
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
            host,
        } => {
            let more = if *host {
                ", on the machine itself, outside the sandbox"
            } else if *network {
                ", with the internet"
            } else {
                ""
            };
            format!("run `{line}` in {}{more}", folder.display())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_is_told_where_its_models_price_rises() {
        assert_eq!(
            context_said(TokenCount(200_000), Some(TokenCount(272_000))),
            "(iq: your conversation is 200.0k tokens now, read again on each call; your model's \
             price per token rises once a call reads 272.0k.)"
        );
        assert!(
            context_said(TokenCount(280_000), Some(TokenCount(272_000)))
                .ends_with(": past the 272.0k where your model's price per token rises.)")
        );
    }
}
