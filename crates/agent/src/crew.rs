//! The crew: the agents at work in one workspace, the board they write to
//! each other on, and the one way any of them acts on the machine, through
//! the gate.
//!
//! An agent is a model of the provider, which iq runs in a loop with its
//! tools, the board's among them. What the gate asks goes to the person,
//! or, the autopilot being on, to the reviewer, which may answer only what
//! stays within the sandbox.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
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
use crate::keep::{Aside, Keeper, ModelChoice, Saved, Shown};
use crate::place::{Placed, Placer};
use crate::review;
use crate::rules::{self, RulesFile};
use crate::team::{self, Pattern, Phase, Seat, Team};
use crate::tools::{self, Args};

/// Reports what the crew does, as it does it.
pub type Report = Arc<dyn Fn(Event) + Send + Sync>;

/// Everyone on the board: a message to `all` reaches every agent.
pub const ALL: &str = "all";

/// Who writes iq's own messages to an agent: a job it left that ended, a
/// wake it asked for.
const IQ: &str = "iq";

/// How long a command left to run in the background may run.
const BACKGROUND_MOST: Duration = Duration::from_secs(2 * 3600);

/// How long it may write nothing: a CI run watched may be quiet a while.
const BACKGROUND_QUIET: Duration = Duration::from_secs(30 * 60);

/// How much of a job's output an agent is shown, in bytes.
const JOB_SHOWN: usize = 4000;

/// How many wakes an agent may wait for at once.
const WAKES_MOST: usize = 3;

/// How many wakes an agent may ask for within an hour.
const WAKES_PER_HOUR: usize = 12;

/// How often two agents may wake each other in [`PEER_SPAN`], whichever
/// writes: more, and they talk on without the work moving, on the
/// person's money.
const PEER_WAKES: usize = 4;

/// How often agents may wake each other in [`PEER_SPAN`], all of them.
const CREW_WAKES: usize = 16;

/// The span those wakes are counted over.
const PEER_SPAN: Duration = Duration::from_secs(10 * 60);

/// How often an agent waiting for messages looks whether the agents it
/// started still work.
const WAIT_LOOK: Duration = Duration::from_secs(1);

/// How many crews this program made: each numbers its questions apart
/// from the others', so that an answer reaches the crew that asked when
/// one window shows several.
static CREWS: AtomicU64 = AtomicU64::new(0);

/// How many of the person's requests to an agent are kept, for the
/// reviewer to judge its actions by what was asked last.
const ASKED_KEPT: usize = 3;

/// How many of an agent's last actions the reviewer is shown, with how
/// they ended, so that a policy may rest on what was just done.
const DONE_KEPT: usize = 8;

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

/// How long a command may run before it is stopped, unless the settings
/// say.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(600);

/// How many times a call is sent again while its provider is out of
/// capacity, unless the settings say: with the waits doubling, some ten
/// minutes, past what a quota for the minute or an overload lasts.
pub const BUSY_TRIES: u32 = 6;

/// How long the first wait for a provider out of capacity lasts, at
/// least, doubled at each try.
pub const BUSY_WAIT: Duration = Duration::from_secs(10);

/// The longest one wait lasts, whatever the provider asks.
pub const BUSY_MOST: Duration = Duration::from_secs(300);

/// How long a command may write nothing before it is stopped, unless the
/// settings say: one waiting for a login in a browser, or for an answer at
/// a terminal it does not have, would hold its agent until its time ran
/// out.
pub const COMMAND_QUIET: Duration = Duration::from_secs(180);

/// The most a chat's folder of scratch holds, in bytes, unless the
/// settings say: past it, the oldest files go first.
pub const SCRATCH_MOST: u64 = 500 * 1024 * 1024;

/// How a model was chosen, as the audit log and the chats keep it: by
/// the tool an agent started another with.
const START_AGENT: &str = "start_agent";

/// The same, a team's member.
const START_TEAM: &str = "start_team";

/// The same, by the person's `/model`.
const SET_MODEL: &str = "/model";

/// The same, a pattern's agent on its other model: a pair's reviewer, a
/// debater.
const PATTERN_MODEL: &str = "the pattern's other model";

/// The same, by a caller of [`Crew::start`].
const STARTED: &str = "start";

/// How many models the provider lists are offered for a name it does not.
const LIKE_LISTED: usize = 6;

/// How many of the files taken from a folder of scratch an agent is told
/// by name.
const CLEARED_NAMED: usize = 5;

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
    /// How hard the reviewer thinks; low unless the person says.
    pub reviewer_effort: Effort,
    /// How long a command may run before it is stopped.
    pub command_timeout: Duration,
    /// How long a command may write nothing before it is stopped: one
    /// waiting for an answer nobody can give it.
    pub command_quiet: Duration,
    /// How many times a call is sent again while its provider is out of
    /// capacity, before the turn ends saying so; 0 never.
    pub busy_tries: u32,
    /// How long the first wait for a provider out of capacity lasts, at
    /// least, doubled at each try.
    pub busy_wait: Duration,
    /// The longest one wait lasts, whatever the provider asks.
    pub busy_most: Duration,
    /// The model of the provider that names the person's chats after what
    /// their work is about: none, and they keep the name they started
    /// with.
    pub namer: Option<ModelId>,
    /// The only models an agent is started on or given, the person's and
    /// every agent's, as the person's settings list them: any other is
    /// refused, the agent told which there are. Empty: any the provider
    /// lists.
    pub allowed: Vec<ModelId>,
    /// The most a chat's folder of scratch holds, in bytes.
    pub scratch_most: u64,
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
            reviewer_effort: Effort::Low,
            command_timeout: COMMAND_TIMEOUT,
            command_quiet: COMMAND_QUIET,
            busy_tries: BUSY_TRIES,
            busy_wait: BUSY_WAIT,
            busy_most: BUSY_MOST,
            namer: None,
            allowed: Vec::new(),
            scratch_most: SCRATCH_MOST,
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
    /// The crew's own place: its workspace, where agents work unless given
    /// a place of their own.
    place: Arc<Place>,
    /// What starts the sandbox of every place.
    helper: Helper,
    /// What gives the agents the person starts a place of their own, when
    /// they want one.
    placer: Mutex<Option<Arc<dyn Placer>>>,
    /// What keeps the chats between sessions, when the person wants it.
    keeper: Mutex<Option<Arc<dyn Keeper>>>,
    /// Held while a chat is kept, so that what a turn under way keeps and
    /// what is kept of it once it is stopped come in the order they were
    /// taken.
    keeping: Mutex<()>,
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
    /// The names agents were given while they worked, by the names they
    /// work under until their turn ends: what the crew reports of them
    /// shows the new ones at once. Never locked before the board.
    shown: Mutex<HashMap<String, String>>,
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
    /// The project's rules for some of its files, as last read in each
    /// place, and when.
    scoped: Mutex<HashMap<PathBuf, (Instant, Vec<rules::Scoped>)>>,
    /// The images each agent is shown with its next call: what the person
    /// attached, what a file or a tool showed.
    images: Mutex<HashMap<String, Vec<Message>>>,
    /// The questions with choices waiting for the person.
    choices: Mutex<HashMap<u64, oneshot::Sender<Option<String>>>>,
    /// The commands left to run in the background, by number.
    jobs: Mutex<HashMap<u64, Job>>,
    next_job: AtomicU64,
    /// The wakes agents asked for, each agent's.
    wakes: Mutex<HashMap<String, Wakes>>,
    /// The models the provider lists, once read: an agent is started only
    /// on one of them, by its whole name.
    listed: Mutex<Vec<String>>,
    /// What the person was shown of each agent beside its conversation,
    /// with the number of its turn under way or last. Never locked before
    /// the board.
    aside: Mutex<HashMap<String, AsideLog>>,
    /// Which chats wrote in each folder lately, and when; and the pairs
    /// of chats the person was told wrote in the same one.
    writers: Mutex<Writers>,
}

/// The tools that write where their agent works, as far as iq can tell: a
/// command may write too.
const WRITERS: [&str; 3] = ["write_file", "edit_file", "run"];

/// How long a chat that wrote in a folder counts as writing there.
const LATELY: Duration = Duration::from_secs(30 * 60);

/// Which chats wrote in each folder lately, and which pairs of chats the
/// person was told of.
#[derive(Default)]
struct Writers {
    lately: HashMap<PathBuf, HashMap<String, Instant>>,
    told: std::collections::HashSet<(String, String)>,
}

/// Two chats, in the same order whichever comes first.
fn pair_of(one: &str, other: &str) -> (String, String) {
    if one <= other {
        (one.to_owned(), other.to_owned())
    } else {
        (other.to_owned(), one.to_owned())
    }
}

/// What was set aside of one agent, with where its conversation stands.
#[derive(Default)]
struct AsideLog {
    /// Its turn under way, or its last.
    turn: u64,
    /// How many messages its conversation holds, as far as told.
    messages: usize,
    /// What was set aside, in order.
    kept: Vec<Aside>,
}

/// A call to the provider that failed: why, in words, and what the
/// provider said when it refused it.
struct Failure {
    said: String,
    refusal: Option<iq_core::Refusal>,
    /// Whether the provider stayed out of capacity through every try,
    /// which `said` tells whole.
    busy: bool,
}

/// The wakes an agent asked for: how many wait, and when it asked those of
/// the last hour.
#[derive(Debug, Default)]
struct Wakes {
    waiting: usize,
    asked: Vec<Instant>,
}

impl Wakes {
    /// Whether one more may be asked at `now`: as many as [`WAKES_MOST`]
    /// waiting, as many as [`WAKES_PER_HOUR`] asked within an hour, so that
    /// an agent waking itself again and again is held, each wake a call.
    /// Counted when it may.
    fn ask(&mut self, now: Instant) -> Result<(), String> {
        self.asked
            .retain(|at| now.duration_since(*at) < Duration::from_secs(3600));
        if self.waiting >= WAKES_MOST {
            return Err(format!(
                "you already asked to be woken {WAKES_MOST} times: wait for those"
            ));
        }
        if self.asked.len() >= WAKES_PER_HOUR {
            return Err(format!(
                "you asked to be woken {WAKES_PER_HOUR} times within the hour: end your turn, \
                 and what you wait for, or the person, wakes you"
            ));
        }
        self.waiting += 1;
        self.asked.push(now);
        Ok(())
    }
}

/// A command an agent left to run in the background.
struct Job {
    agent: String,
    line: String,
    started: Instant,
    live: iq_gate::Live,
    /// Its task, while it runs.
    running: Option<JoinHandle<()>>,
    /// How it ended, once it did.
    ended: Option<String>,
}

/// Where agents work: the gate that judges their actions there, and the
/// sandbox their commands run in.
struct Place {
    gate: Gate,
    sandbox: Sandbox,
}

/// Where an agent joining works: a place given, the one of the agent that
/// starts it; or, for the person's, one of its own when they set a placer,
/// else the crew's.
enum Placing {
    Given(Arc<Place>),
    Own,
}

#[derive(Default)]
struct Board {
    messages: Vec<Posted>,
    members: BTreeMap<String, Member>,
    /// The names agents were renamed from, and the names they have now:
    /// what an agent writes to an old one reaches it. A name given to an
    /// agent at work leads to the one it works under until its turn ends.
    aliases: HashMap<String, String>,
}

impl Board {
    /// The member `name` names: the one of that name, else the one it was
    /// renamed from or is to take.
    fn named(&self, name: &str) -> Option<String> {
        if self.members.contains_key(name) {
            return Some(name.to_owned());
        }
        self.aliases
            .get(name)
            .filter(|member| self.members.contains_key(member.as_str()))
            .cloned()
    }

    /// The name the member `name` shows: the one it is to take as its turn
    /// ends, else its own.
    fn shown(&self, name: &str) -> String {
        self.members
            .get(name)
            .and_then(|member| member.renaming.clone())
            .unwrap_or_else(|| name.to_owned())
    }

    /// Whether `name` is free for an agent: none has it, nor is to take it.
    fn free(&self, name: &str) -> bool {
        !self.members.contains_key(name)
            && !self
                .members
                .values()
                .any(|member| member.renaming.as_deref() == Some(name))
    }

    /// The name of the chat `agent` is part of, as it shows: its own when
    /// the person started it, else that of the chat of the agent that
    /// started it.
    fn chat_of(&self, agent: &str) -> String {
        let mut name = agent.to_owned();
        // As many steps as there are members, so that a loop ends.
        for _ in 0..=self.members.len() {
            match self.members.get(&name) {
                Some(member) if member.by != PERSON => name.clone_from(&member.by),
                _ => break,
            }
        }
        let shown = self.shown(&name);
        // An agent kept by an older iq, named apart from its chat.
        shown.split('/').next().unwrap_or(&shown).to_owned()
    }
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
    /// What the person asked of it since, the latest last.
    asked: Vec<String>,
    /// When the person last wrote to it, in seconds since 1970.
    written: Option<u64>,
    /// What started its turn under way, or its last.
    prompt: Option<String>,
    /// Its last actions and how they ended, the latest last.
    done: VecDeque<String>,
    /// What it wrote last in its turn under way, for the reviewer: its own
    /// reason for the actions it calls after.
    reason: Option<String>,
    /// The calls to its model it made, over all its turns.
    calls: u64,
    /// What those cost, as far as their prices are known.
    spent: Usd,
    /// The first message it has not read.
    read: usize,
    /// Its turn under way, when it works.
    turn: Option<JoinHandle<()>>,
    /// How many turns it was woken for: the number of its last.
    turns: u64,
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
    /// Where it works.
    place: Arc<Place>,
    /// The name the person gave it while it worked, which it takes as its
    /// turn ends.
    renaming: Option<String>,
    /// Whether the person named it: iq names it no more.
    chosen: bool,
    /// Each choice of its model, the latest last.
    models_chosen: Vec<ModelChoice>,
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
        let sandbox = gate
            .sandbox(helper.clone())
            .with_timeout(settings.command_timeout)
            .with_quiet(settings.command_quiet);
        Self {
            inner: Arc::new(Inner {
                provider,
                place: Arc::new(Place { gate, sandbox }),
                helper,
                placer: Mutex::new(None),
                keeper: Mutex::new(None),
                keeping: Mutex::new(()),
                settings,
                report,
                autopilot: AtomicBool::new(false),
                refused: Mutex::new(HashMap::new()),
                busy: AtomicUsize::new(0),
                next_question: AtomicU64::new((CREWS.fetch_add(1, Ordering::Relaxed) << 32) | 1),
                questions: Mutex::new(HashMap::new()),
                board: Mutex::new(Board::default()),
                shown: Mutex::new(HashMap::new()),
                images: Mutex::new(HashMap::new()),
                choices: Mutex::new(HashMap::new()),
                jobs: Mutex::new(HashMap::new()),
                next_job: AtomicU64::new(1),
                wakes: Mutex::new(HashMap::new()),
                listed: Mutex::new(Vec::new()),
                aside: Mutex::new(HashMap::new()),
                writers: Mutex::new(Writers::default()),
                posted: watch::Sender::new(0),
                billed: Mutex::new(Usd(0.0)),
                peer_wakes: Mutex::new(Vec::new()),
                teams: Mutex::new(Vec::new()),
                extensions: Mutex::new(Vec::new()),
                scoped: Mutex::new(HashMap::new()),
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

    /// The gate the crew acts through, in its own workspace.
    pub fn gate(&self) -> &Gate {
        &self.inner.place.gate
    }

    /// Gives each agent the person starts from now on a place of its own,
    /// made by `placer`; or none, and they work in the crew's workspace.
    /// The agents one starts work where it does.
    pub fn set_placer(&self, placer: Option<Arc<dyn Placer>>) {
        *lock(&self.inner.placer) = placer;
    }

    /// Gives each chat to `keeper` as it joins and at every turn's end,
    /// to be resumed with [`Self::resume`] when iq starts again; or to
    /// nobody.
    pub fn set_keeper(&self, keeper: Option<Arc<dyn Keeper>>) {
        *lock(&self.inner.keeper) = keeper;
    }

    /// Gives `agent` to the keeper, as it stands now.
    fn keep(&self, agent: &str) {
        self.keep_as(agent, None);
    }

    /// Gives `agent` to the keeper with its conversation as its turn
    /// `turn` holds it so far, `messages`, so that iq stopped in the middle
    /// of it, or crashed, finds it where it stopped. A turn stopped, or
    /// another begun, keeps nothing more.
    fn keep_turn(&self, agent: &str, turn: u64, messages: &[Message]) {
        self.keep_as(agent, Some((turn, messages)));
    }

    /// Gives `agent` to the keeper, with the conversation of its turn under
    /// way when `turn` gives it.
    fn keep_as(&self, agent: &str, turn: Option<(u64, &[Message])>) {
        let Some(keeper) = lock(&self.inner.keeper).clone() else {
            return;
        };
        let _keeping = lock(&self.inner.keeping);
        let saved = {
            let board = lock(&self.inner.board);
            let Some(member) = board.members.get(agent) else {
                return;
            };
            let messages = match turn {
                Some((number, messages)) => {
                    if member.turns != number || member.turn.is_none() {
                        return;
                    }
                    messages.to_vec()
                }
                None => member.messages.clone(),
            };
            Saved {
                name: agent.to_owned(),
                model: member.model.to_string(),
                by: member.by.clone(),
                role: member.role.clone(),
                task: member.task.clone(),
                asked: member.asked.clone(),
                written: member.written,
                effort: member.effort,
                folder: (!Arc::ptr_eq(&member.place, &self.inner.place))
                    .then(|| member.place.gate.workspace().to_path_buf()),
                context: member.context,
                models_chosen: member.models_chosen.clone(),
                calls: member.calls,
                spent: member.spent,
                aside: lock(&self.inner.aside)
                    .get(agent)
                    .map(|log| log.kept.clone())
                    .unwrap_or_default(),
                messages,
            }
        };
        keeper.keep(&saved);
    }

    /// Puts a chat kept from an earlier session back on the board, resting
    /// until it is written to: its conversation as kept, its standing
    /// instructions written again, in `place` when it had one of its own
    /// that is still there, else in the crew's workspace. Returns its name:
    /// the one kept, unless an agent holds it.
    ///
    /// # Errors
    ///
    /// Its model is no model's name, or its place cannot be resolved.
    pub fn resume(&self, saved: Saved, place: Option<Placed>) -> Result<String, String> {
        let model = ModelId::new(saved.model.as_str()).map_err(|e| e.to_string())?;
        let place = match place {
            Some(placed) => {
                let gate = self
                    .inner
                    .place
                    .gate
                    .within(&placed.folder, placed.writable)
                    .map_err(|e| chain(&e))?;
                let sandbox = gate
                    .sandbox(self.inner.helper.clone())
                    .with_timeout(self.inner.settings.command_timeout)
                    .with_quiet(self.inner.settings.command_quiet);
                Arc::new(Place { gate, sandbox })
            }
            None => Arc::clone(&self.inner.place),
        };
        let mut board = lock(&self.inner.board);
        let kept = if saved.by == PERSON {
            base_name(&saved.name)
        } else {
            board_name(&saved.name)
        };
        let name = free_name(&board, &kept, &[]);
        let instructions =
            Message::system(self.instructions(&name, &saved.by, saved.role.as_deref(), &place));
        let mut messages = saved.messages;
        match messages.first_mut() {
            Some(first @ Message::System(_)) => *first = instructions,
            _ => messages.insert(0, instructions),
        }
        if messages.len() == 1 {
            messages.push(Message::user(saved.task.clone()));
        }
        let read = board.messages.len();
        let own = !Arc::ptr_eq(&place, &self.inner.place);
        let workspace = place.gate.workspace().to_path_buf();
        board.members.insert(
            name.clone(),
            Member {
                model: model.clone(),
                by: saved.by.clone(),
                role: saved.role,
                task: saved.task,
                asked: saved.asked,
                written: saved.written,
                prompt: None,
                done: VecDeque::new(),
                reason: None,
                calls: saved.calls,
                spent: saved.spent,
                read,
                turn: None,
                turns: 0,
                messages,
                effort: saved.effort,
                context: saved.context,
                told: saved.context,
                compacted: None,
                rules_given: Vec::new(),
                place,
                renaming: None,
                chosen: false,
                models_chosen: saved.models_chosen,
            },
        );
        drop(board);
        let log = AsideLog {
            turn: saved
                .aside
                .iter()
                .map(|aside| aside.turn)
                .max()
                .unwrap_or(0),
            messages: 0,
            kept: saved.aside,
        };
        lock(&self.inner.aside).insert(name.clone(), log);
        self.report(Event::Joined {
            agent: name.clone(),
            model: model.to_string(),
            by: saved.by,
        });
        if own {
            self.report(Event::Placed {
                agent: name.clone(),
                folder: workspace,
                said: None,
            });
        }
        self.report(Event::Effort {
            agent: name.clone(),
            effort: saved.effort.or(self.inner.settings.effort),
        });
        Ok(name)
    }

    /// Where `agent` works: its place, or the crew's own for one unknown.
    fn place_of(&self, agent: &str) -> Arc<Place> {
        lock(&self.inner.board)
            .members
            .get(agent)
            .map_or_else(|| Arc::clone(&self.inner.place), |m| Arc::clone(&m.place))
    }

    /// The sandbox `agent`'s commands run in: its place's, given its chat's
    /// folder of scratch when the gate has one.
    fn sandbox_of(&self, agent: &str) -> Sandbox {
        let place = self.place_of(agent);
        let sandbox = place.sandbox.clone();
        match place.gate.scratch() {
            Some(root) => {
                let chat = lock(&self.inner.board).chat_of(agent);
                sandbox.with_scratch(root.join(chat), self.inner.settings.scratch_most)
            }
            None => sandbox,
        }
    }

    /// A place of its own for `name`, from the placer when the person set
    /// one: its gate as the crew's, in its folder, and its sandbox; and
    /// what the person is told of it.
    fn own_place(&self, name: &str) -> Result<Option<(Arc<Place>, String)>, String> {
        let Some(placer) = lock(&self.inner.placer).clone() else {
            return Ok(None);
        };
        let placed = placer.place(name)?;
        let gate = self
            .inner
            .place
            .gate
            .within(&placed.folder, placed.writable.clone())
            .map_err(|e| chain(&e))?;
        let sandbox = gate
            .sandbox(self.inner.helper.clone())
            .with_timeout(self.inner.settings.command_timeout)
            .with_quiet(self.inner.settings.command_quiet);
        Ok(Some((Arc::new(Place { gate, sandbox }), placed.said)))
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
        self.inner.place.gate.internet()
    }

    /// Gives every command the internet, or only what a rule gives it to
    /// and what the person allows: in every place at once.
    pub fn set_internet(&self, open: bool) {
        self.inner.place.gate.set_internet(open);
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

    /// Answers the question with choices `id`: a choice, or the person's
    /// own words; none when they gave none.
    pub fn choose(&self, id: u64, answer: Option<String>) {
        if let Some(reply) = lock(&self.inner.choices).remove(&id) {
            let _ = reply.send(answer);
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
    /// the settings say when `None`. `how` says how the person chose the
    /// first model, as the audit log and the chat keep it. Returns the
    /// first agent's name: `name`, made unique.
    #[allow(clippy::too_many_arguments)]
    pub fn launch(
        &self,
        pattern: Pattern,
        name: &str,
        models: &[ModelId],
        task: String,
        role: Option<String>,
        effort: Option<Effort>,
        images: Vec<Message>,
        how: &str,
    ) -> String {
        let Some(first) = models.first() else {
            return String::new();
        };
        // Held while the team starts, so that a turn ending at once finds
        // its team.
        let mut teams = lock(&self.inner.teams);
        match pattern {
            Pattern::Solo => self.start_with(name, first, task, PERSON, role, effort, images, how),
            Pattern::Lead => {
                let cheaper: Vec<String> = models[1..].iter().map(ToString::to_string).collect();
                let role = Some(team::lead_role(&cheaper));
                self.start_with(name, first, task, PERSON, role, effort, images, how)
            }
            Pattern::Pair => {
                let writer = self.start_with(
                    name,
                    first,
                    task.clone(),
                    PERSON,
                    Some(team::WRITER_ROLE.to_owned()),
                    effort,
                    images,
                    how,
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
                        self.start_with(
                            name,
                            &models[at % models.len()],
                            task.clone(),
                            PERSON,
                            Some(team::debater_role(count)),
                            None,
                            images.clone(),
                            if at % models.len() == 0 {
                                how
                            } else {
                                PATTERN_MODEL
                            },
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
                                let started = self.start_with(
                                    "review",
                                    reviewer_model,
                                    prompt,
                                    writer,
                                    Some(team::REVIEWER_ROLE.to_owned()),
                                    None,
                                    Vec::new(),
                                    PATTERN_MODEL,
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
    /// Returns its name: for the person's, `name`, made unique; for
    /// another's, its chat's name and `name` as its part in it,
    /// `<chat>/<name>`, or a number when `name` is empty.
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
        self.start_with(name, model, task, by, role, None, Vec::new(), STARTED)
    }

    /// Starts an agent as [`Self::start_as`] does, thinking with `effort`,
    /// as the settings say when `None`, shown `images` with its task, its
    /// model chosen by `by` as `how` says.
    #[allow(clippy::too_many_arguments)]
    fn start_with(
        &self,
        name: &str,
        model: &ModelId,
        task: String,
        by: &str,
        role: Option<String>,
        effort: Option<Effort>,
        images: Vec<Message>,
        how: &str,
    ) -> String {
        // The person's agent works in a place of its own when they want
        // one; an agent's works where it does, and is named after its
        // chat.
        let (name, placing) = if by == PERSON {
            (base_name(name), Placing::Own)
        } else {
            let name = agent_name(&lock(&self.inner.board), by, name, &[]);
            (name, Placing::Given(self.place_of(by)))
        };
        let (name, joined) = self.join(
            &name,
            model,
            task.clone(),
            by,
            role,
            effort,
            true,
            placing,
            how,
        );
        if joined {
            // Before its turn begins, so that its first call shows them.
            for image in images {
                self.show(&name, image);
            }
            self.wake(&name, task);
        }
        name
    }

    /// Shows the image `bytes` of `path`, a `mime` image, to `agent` after
    /// this step's results, and says so; one larger than a model is shown
    /// is not, and it is told why.
    fn shown_image(
        &self,
        agent: &str,
        path: &Path,
        mime: &str,
        bytes: &[u8],
    ) -> Result<String, String> {
        use base64::Engine;
        let kb = bytes.len().div_ceil(1024);
        if bytes.len() > iq_core::IMAGE_MOST {
            return Err(format!(
                "{} is an image of {kb} KB, more than the {} KB a model is shown: make a smaller \
                 copy of it in the workspace with a command, and read that",
                path.display(),
                iq_core::IMAGE_MOST / 1024
            ));
        }
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        self.show(
            agent,
            Message::Image {
                mime: mime.to_owned(),
                data,
            },
        );
        Ok(format!(
            "{} is an image ({mime}, {kb} KB): it is shown to you after this step's results.",
            path.display()
        ))
    }

    /// Shows `image`, an image message, to `agent` with its next call to
    /// its model: after the messages and the results of its tools that
    /// came before.
    pub fn show(&self, agent: &str, image: Message) {
        lock(&self.inner.images)
            .entry(agent.to_owned())
            .or_default()
            .push(image);
    }

    /// Puts an agent on the board, its name `name`, as the board writes
    /// names, made unique, in the place `placing` says, on `model`, which
    /// `by` chose as `how` says, written to the audit log and kept with the
    /// chat. One that `works` is about to be woken on `task`; another holds
    /// its task and waits until it is written to. Whether it joined: one on
    /// a model the person does not allow, or that could not be given a
    /// place of its own, is not on the board, and the person is told why.
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
        placing: Placing,
        how: &str,
    ) -> (String, bool) {
        let mut board = lock(&self.inner.board);
        let name = free_name(&board, &board_name(name), &[]);
        if let Some(why) = self.refused_model(model) {
            drop(board);
            self.not_started(&name, model, by, &why, "its model is not one it may run on");
            return (name, false);
        }
        let (place, said) = match placing {
            Placing::Given(place) => (place, None),
            Placing::Own => match self.own_place(&name) {
                Ok(Some((place, said))) => (place, Some(said)),
                Ok(None) => (Arc::clone(&self.inner.place), None),
                Err(why) => {
                    drop(board);
                    self.not_started(
                        &name,
                        model,
                        by,
                        &format!("no place of its own for it, {why}"),
                        "it has no place to work in",
                    );
                    return (name, false);
                }
            },
        };
        let workspace = place.gate.workspace().to_path_buf();
        let mut messages = vec![Message::system(self.instructions(
            &name,
            by,
            role.as_deref(),
            &place,
        ))];
        if !works {
            messages.push(Message::user(task.clone()));
        }
        let read = board.messages.len();
        let own = !Arc::ptr_eq(&place, &self.inner.place);
        board.members.insert(
            name.clone(),
            Member {
                model: model.clone(),
                by: by.to_owned(),
                role,
                task: task.clone(),
                asked: Vec::new(),
                written: (by == PERSON).then(now),
                prompt: None,
                done: VecDeque::new(),
                reason: None,
                calls: 0,
                spent: Usd(0.0),
                read,
                turn: None,
                turns: 0,
                messages,
                effort,
                context: 0,
                told: 0,
                compacted: None,
                rules_given: Vec::new(),
                place: Arc::clone(&place),
                renaming: None,
                chosen: false,
                models_chosen: vec![ModelChoice {
                    model: model.to_string(),
                    by: by.to_owned(),
                    how: how.to_owned(),
                    at: now(),
                }],
            },
        );
        drop(board);
        self.report(Event::Joined {
            agent: name.clone(),
            model: model.to_string(),
            by: by.to_owned(),
        });
        self.audit_choice(&place.gate, &name, model, by, how);
        if own {
            self.report(Event::Placed {
                agent: name.clone(),
                folder: workspace,
                said,
            });
        }
        self.report(Event::Effort {
            agent: name.clone(),
            effort: effort.or(self.inner.settings.effort),
        });
        self.keep(&name);
        (name, true)
    }

    /// Says that `name`, on `model`, which `by` asked for, is not started,
    /// and `why`: it joins and ends at once, its turn failed as `ended`
    /// says.
    fn not_started(&self, name: &str, model: &ModelId, by: &str, why: &str, ended: &str) {
        self.report(Event::Joined {
            agent: name.to_owned(),
            model: model.to_string(),
            by: by.to_owned(),
        });
        self.report(Event::Notice {
            agent: name.to_owned(),
            text: format!("{name} is not started: {why}"),
        });
        self.report(Event::Done {
            agent: name.to_owned(),
            outcome: Err(ended.to_owned()),
        });
    }

    /// Writes to the audit log that `agent` runs on `model`, as `by` chose
    /// it, `how`; a line that cannot be written is said.
    fn audit_choice(&self, gate: &Gate, agent: &str, model: &ModelId, by: &str, how: &str) {
        if let Err(error) = gate.chose(agent, model.as_str(), by, how) {
            self.report(Event::Notice {
                agent: agent.to_owned(),
                text: format!("the audit log misses a choice of model: {}", chain(&error)),
            });
        }
    }

    /// The models the provider lists, as read: from now on an agent is
    /// started only on one of them, by its whole name, unless the person's
    /// settings allow others. An empty list, when it could not be read,
    /// holds none back.
    pub fn set_listed(&self, models: Vec<String>) {
        *lock(&self.inner.listed) = models;
    }

    /// Why no agent may run on `model`, if none may: the person's settings
    /// allow others only, said with them, so that the agent that asked
    /// chooses again; or the provider lists no model of that name, which
    /// is never left to the provider to guess, said with those it lists
    /// that look like it.
    fn refused_model(&self, model: &ModelId) -> Option<String> {
        let allowed = &self.inner.settings.allowed;
        if !allowed.is_empty() {
            if allowed.contains(model) {
                return None;
            }
            let names: Vec<String> = allowed.iter().map(ToString::to_string).collect();
            return Some(format!(
                "{model} is not among the models the person allows: {}. Choose one of them, by \
                 its whole name.",
                names.join(", ")
            ));
        }
        let listed = lock(&self.inner.listed);
        if listed.is_empty() || listed.iter().any(|known| known == model.as_str()) {
            return None;
        }
        let last = model.as_str().rsplit('/').next().unwrap_or(model.as_str());
        let base = last.split(['@', ':']).next().unwrap_or(last).to_lowercase();
        let like: Vec<&str> = listed
            .iter()
            .filter(|known| !base.is_empty() && known.to_lowercase().contains(&base))
            .take(LIKE_LISTED)
            .map(String::as_str)
            .collect();
        Some(if like.is_empty() {
            format!(
                "the provider lists no model {model}: give one by its whole name, as it lists it"
            )
        } else {
            format!(
                "the provider lists no model {model}: give one by its whole name, as it lists it, \
                 such as {}",
                like.join(", ")
            )
        })
    }

    /// Starts a team on `task` for `by`: one agent for each seat, named
    /// after `by`'s chat and its role, told the others. The first works at
    /// once and answers `by`; the others wait until a teammate writes to
    /// them, and answer the first. Says what was started, as the agent that
    /// asked reads it.
    fn start_team(&self, by: &str, task: &str, seats: &[Seat]) -> String {
        let names: Vec<String> = {
            let board = lock(&self.inner.board);
            let mut names: Vec<String> = Vec::new();
            for seat in seats {
                let name = agent_name(&board, by, &seat.role, &names);
                names.push(name);
            }
            names
        };
        let brief = |at: usize| format!("{task}\n\n{}", team::roster(&names, seats, at, by));
        let role = |seat: &Seat| {
            Some(format!(
                "{} in a team of {}: {}",
                seat.role,
                seats.len(),
                seat.does
            ))
        };
        // Those who wait join first, so that the first finds them when it
        // writes to them; all work where the agent that asked does.
        let place = self.place_of(by);
        for (at, seat) in seats.iter().enumerate().skip(1) {
            self.join(
                &names[at],
                &seat.model,
                brief(at),
                &names[0],
                role(seat),
                seat.effort,
                false,
                Placing::Given(Arc::clone(&place)),
                START_TEAM,
            );
        }
        let first = &seats[0];
        let (lead, joined) = self.join(
            &names[0],
            &first.model,
            brief(0),
            by,
            role(first),
            first.effort,
            true,
            Placing::Given(Arc::clone(&place)),
            START_TEAM,
        );
        if joined {
            self.wake(&lead, brief(0));
        }
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
            "The team is started: {lead} works on it now, on {}.{waits} {lead}'s answer will \
             come to you as a message.",
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
        let name = board
            .named(agent)
            .ok_or_else(|| format!("no agent is named {agent:?}"))?;
        if let Some(member) = board.members.get_mut(&name) {
            member.effort = Some(effort);
        }
        drop(board);
        self.report(Event::Effort {
            agent: name,
            effort: Some(effort),
        });
        Ok(())
    }

    /// Has `agent` answered by `model` from its next turn, its
    /// conversation going on: the person chose another, or started it on
    /// one the provider does not have.
    ///
    /// # Errors
    ///
    /// No agent of that name.
    pub fn set_model(&self, agent: &str, model: &ModelId) -> Result<(), String> {
        if let Some(why) = self.refused_model(model) {
            return Err(why);
        }
        let mut board = lock(&self.inner.board);
        let name = board
            .named(agent)
            .ok_or_else(|| format!("no agent is named {agent:?}"))?;
        let mut place = None;
        if let Some(member) = board.members.get_mut(&name) {
            member.model = model.clone();
            member.models_chosen.push(ModelChoice {
                model: model.to_string(),
                by: PERSON.to_owned(),
                how: SET_MODEL.to_owned(),
                at: now(),
            });
            place = Some(Arc::clone(&member.place));
        }
        drop(board);
        if let Some(place) = place {
            self.audit_choice(&place.gate, &name, model, PERSON, SET_MODEL);
        }
        self.report(Event::Model {
            agent: name.clone(),
            model: model.to_string(),
        });
        self.keep(&name);
        Ok(())
    }

    /// The person writes to `to`: an agent, woken if it rests, or all.
    pub fn tell(&self, to: &str, text: String) -> Result<(), String> {
        self.post(PERSON, to, text)
    }

    /// Gives `agent` the name `name`, as the person calls it now, made a
    /// name as the crew makes them, and names the agents of its chat after
    /// it: what was written to it, the agents it started and its teams
    /// follow it, what is written to its old name reaches it, and it is
    /// kept under the new one. One at work shows it and is written to under
    /// it at once, and takes it as its turn ends. iq names it no more.
    /// Returns its new name.
    ///
    /// # Errors
    ///
    /// No agent is named `agent`, or another holds the name.
    pub fn rename(&self, agent: &str, name: &str) -> Result<String, String> {
        let (member, new) = {
            let board = lock(&self.inner.board);
            let member = board
                .named(agent)
                .ok_or_else(|| format!("no agent is named {agent:?}"))?;
            // An agent another started stays part of its chat.
            let new = match board.members.get(&member) {
                Some(started) if started.by != PERSON => {
                    format!("{}/{}", board.chat_of(&member), base_name(&part_of(name)))
                }
                _ => base_name(name),
            };
            (member, new)
        };
        self.rename_chat(&member, &new)?;
        // Under its new name, or its old while it works.
        let mut board = lock(&self.inner.board);
        if let Some(named) = board.named(&member)
            && let Some(member) = board.members.get_mut(&named)
        {
            member.chosen = true;
        }
        Ok(new)
    }

    /// Names the member `member` `new`, and the agents of its chat after
    /// it: `<chat>/review` follows its chat.
    fn rename_chat(&self, member: &str, new: &str) -> Result<(), String> {
        let (shown, theirs) = {
            let board = lock(&self.inner.board);
            let shown = board.shown(member);
            let prefix = format!("{shown}/");
            let theirs: Vec<(String, String)> = board
                .members
                .keys()
                .filter_map(|name| {
                    let part = board.shown(name).strip_prefix(&prefix)?.to_owned();
                    Some((name.clone(), format!("{new}/{part}")))
                })
                .collect();
            (shown, theirs)
        };
        self.name_again(member, new)?;
        if shown != new {
            for (agent, name) in theirs {
                // Its name, if taken since, stays as it was.
                let _ = self.name_again(&agent, &name);
            }
        }
        Ok(())
    }

    /// Names the member `member` `new`: at once when it rests; when it
    /// works, shown and written to under it at once, and taken as its turn
    /// ends, which holds its name until then.
    fn name_again(&self, member: &str, new: &str) -> Result<(), String> {
        let mut board = lock(&self.inner.board);
        let Some(current) = board.members.get(member) else {
            return Err(format!("no agent is named {member:?}"));
        };
        let shown = board.shown(member);
        if new == shown {
            return Ok(());
        }
        let working = current
            .turn
            .as_ref()
            .is_some_and(|turn| !turn.is_finished());
        let pending = current.renaming.clone();
        let theirs =
            |name: &str| name == member || board.aliases.get(name).is_some_and(|to| to == member);
        if !theirs(new) && (!board.free(new) || board.aliases.contains_key(new)) {
            return Err(format!("an agent is named {new} already"));
        }
        if !working {
            // Moved under the same lock it was found resting under: no turn
            // of it may act under the old name meanwhile.
            let folder = self.move_on_board(&mut board, member, new).flatten();
            drop(board);
            self.moved(member, new);
            self.report(Event::Renamed {
                from: shown,
                to: new.to_owned(),
            });
            // Told again where it works, under its new name: what the
            // person's plugins show of its place follows it.
            if let Some(folder) = folder {
                self.report(Event::Placed {
                    agent: new.to_owned(),
                    folder,
                    said: None,
                });
            }
            return Ok(());
        }
        if let Some(before) = pending
            && board.aliases.get(&before).is_some_and(|to| to == member)
        {
            board.aliases.remove(&before);
        }
        let taking = (new != member).then(|| new.to_owned());
        if let Some(new) = &taking {
            board.aliases.insert(new.clone(), member.to_owned());
        }
        let folder = board.members.get_mut(member).and_then(|current| {
            current.renaming.clone_from(&taking);
            (!Arc::ptr_eq(&current.place, &self.inner.place))
                .then(|| current.place.gate.workspace().to_path_buf())
        });
        drop(board);
        let mut names = lock(&self.inner.shown);
        match taking {
            Some(new) => names.insert(member.to_owned(), new),
            None => names.remove(member),
        };
        // Reported as it is said, the name it showed and the one it shows,
        // before anything reported under the new: what the crew reports
        // waits for the names.
        (self.inner.report)(Event::Renamed {
            from: shown,
            to: new.to_owned(),
        });
        drop(names);
        if let Some(folder) = folder {
            self.report(Event::Placed {
                agent: member.to_owned(),
                folder,
                said: None,
            });
        }
        Ok(())
    }

    /// Moves the member `agent` to the name `new` on `board`, where no turn
    /// of it works: its instructions written again, what names it following
    /// it. None when no member has the name; else its folder, when it has a
    /// place of its own. [`Self::moved`] does the rest once the board is
    /// let go.
    fn move_on_board(&self, board: &mut Board, agent: &str, new: &str) -> Option<Option<PathBuf>> {
        let mut member = board.members.remove(agent)?;
        if let Some(pending) = member.renaming.take()
            && board.aliases.get(&pending).is_some_and(|to| to == agent)
        {
            board.aliases.remove(&pending);
        }
        if board.aliases.get(new).is_some_and(|to| to == agent) {
            board.aliases.remove(new);
        }
        let instructions = Message::system(self.instructions(
            new,
            &member.by,
            member.role.as_deref(),
            &member.place,
        ));
        if let Some(first @ Message::System(_)) = member.messages.first_mut() {
            *first = instructions;
        }
        let own = !Arc::ptr_eq(&member.place, &self.inner.place);
        let folder = own.then(|| member.place.gate.workspace().to_path_buf());
        board.members.insert(new.to_owned(), member);
        for other in board.members.values_mut() {
            if other.by == agent {
                new.clone_into(&mut other.by);
            }
        }
        for posted in &mut board.messages {
            if posted.from == agent {
                new.clone_into(&mut posted.from);
            }
            if posted.to == agent {
                new.clone_into(&mut posted.to);
            }
        }
        for named in board.aliases.values_mut() {
            if named == agent {
                new.clone_into(named);
            }
        }
        board.aliases.insert(agent.to_owned(), new.to_owned());
        Some(folder)
    }

    /// What follows an agent moved from `agent` to `new` off the board: its
    /// teams, the count of its refusals, and what is kept of it and of
    /// those it started, under the new name.
    fn moved(&self, agent: &str, new: &str) {
        lock(&self.inner.shown).remove(agent);
        let mut aside = lock(&self.inner.aside);
        if let Some(kept) = aside.remove(agent) {
            aside.insert(new.to_owned(), kept);
        }
        drop(aside);
        for team in lock(&self.inner.teams).iter_mut() {
            team.rename(agent, new);
        }
        let mut refused = lock(&self.inner.refused);
        if let Some(count) = refused.remove(agent) {
            refused.insert(new.to_owned(), count);
        }
        drop(refused);
        if let Some(keeper) = lock(&self.inner.keeper).clone() {
            keeper.forget(agent);
        }
        self.keep(new);
        // Those it started are kept with their starter's new name, to come
        // back under it.
        let started: Vec<String> = lock(&self.inner.board)
            .members
            .iter()
            .filter(|(_, member)| member.by == new)
            .map(|(name, _)| name.clone())
            .collect();
        for name in &started {
            self.keep(name);
        }
    }

    /// Gives `agent`, resting, the name it was given while it worked;
    /// a turn begun since takes it as it ends.
    fn take_name(&self, agent: &str) {
        let mut board = lock(&self.inner.board);
        let pending = board.members.get(agent).and_then(|member| {
            let working = member.turn.as_ref().is_some_and(|turn| !turn.is_finished());
            member.renaming.clone().filter(|_| !working)
        });
        let Some(new) = pending else {
            return;
        };
        self.move_on_board(&mut board, agent, &new);
        drop(board);
        self.moved(agent, &new);
    }

    /// Asks the cheap model the settings give (`namer`) once, with
    /// `instructions` and `prompt`, what it costs billed to `agent`, as any
    /// call's: what a tool asks of a model rather than the agent's own,
    /// such as a question on a page fetched.
    ///
    /// # Errors
    ///
    /// No cheap model is set, or its provider failed.
    pub async fn ask_cheap(
        &self,
        agent: &str,
        instructions: &str,
        prompt: String,
    ) -> Result<String, String> {
        let model = self
            .inner
            .settings
            .namer
            .clone()
            .ok_or("no cheap model is set")?;
        crate::once::ask_once(
            &self.inner.provider,
            &model,
            instructions,
            prompt,
            crate::once::Asked {
                agent,
                effort: Effort::Low,
                costs: &self.inner.report,
            },
        )
        .await
    }

    /// Names the chat `chat` after what its work is about, as the namer's
    /// model reads it from its task, its cost reported as any call's: a
    /// name made of the first words of a request says little of it. Its
    /// agents follow it. Left as it is when the person names it before the
    /// model answers, or when no namer is set.
    pub fn name_after_work(&self, chat: &str) {
        let Some(model) = self.inner.settings.namer.clone() else {
            return;
        };
        let Some(task) = lock(&self.inner.board)
            .members
            .get(chat)
            .map(|member| member.task.clone())
        else {
            return;
        };
        let crew = self.clone();
        let chat = chat.to_owned();
        tokio::spawn(async move {
            let answer = crate::once::ask_once(
                &crew.inner.provider,
                &model,
                NAMING,
                task.clone(),
                crate::once::Asked {
                    agent: NAMER,
                    effort: Effort::Low,
                    costs: &crew.inner.report,
                },
            )
            .await;
            let Some(name) = answer.ok().and_then(|answer| title(&answer)) else {
                return;
            };
            // The person's name wins, and so does a chat of the same name
            // started since on another task.
            let still = lock(&crew.inner.board)
                .members
                .get(&chat)
                .is_some_and(|member| !member.chosen && member.task == task);
            if still {
                let _ = crew.rename_chat(&chat, &name);
            }
        });
    }

    /// Stops `agent`'s turn, if it works.
    ///
    /// It is left as it was before that turn, and kept so; then it takes
    /// the name it was given while it worked.
    pub fn stop(&self, agent: &str) {
        let Some(agent) = lock(&self.inner.board).named(agent) else {
            return;
        };
        // What it left to run stops with it.
        self.stop_jobs(Some(&agent));
        let Some(turn) = self.halt(&agent) else {
            return;
        };
        self.keep(&agent);
        let renamed = lock(&self.inner.board)
            .members
            .get(&agent)
            .is_some_and(|member| member.renaming.is_some());
        if renamed {
            // Once the turn is over: until then, it may still act under its
            // name.
            let crew = self.clone();
            tokio::spawn(async move {
                let _ = turn.await;
                crew.take_name(&agent);
            });
        }
    }

    /// Stops every agent, as iq ends: what each did in its turn stays kept,
    /// to come back to, under the name it worked under.
    pub fn stop_all(&self) {
        let names: Vec<String> = lock(&self.inner.board).members.keys().cloned().collect();
        for name in names {
            self.halt(&name);
        }
    }

    /// Stops `agent`'s turn, if it works; the turn, aborted, when it did.
    fn halt(&self, agent: &str) -> Option<JoinHandle<()>> {
        let turn = lock(&self.inner.board)
            .members
            .get_mut(agent)
            .and_then(|member| member.turn.take())?;
        turn.abort();
        self.report(Event::Done {
            agent: agent.to_owned(),
            outcome: Err("stopped by the person".into()),
        });
        Some(turn)
    }

    /// Ends the crew: every agent stopped, and what their commands left
    /// running stopped with the sandboxes, in every place.
    pub async fn end(&self) {
        self.stop_all();
        self.stop_jobs(None);
        let mut places = vec![Arc::clone(&self.inner.place)];
        for member in lock(&self.inner.board).members.values() {
            if !places.iter().any(|place| Arc::ptr_eq(place, &member.place)) {
                places.push(Arc::clone(&member.place));
            }
        }
        for place in places {
            place.sandbox.end().await;
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
        // A name an agent was renamed from leads to it, unless another
        // agent took that name since.
        let renamed = {
            let board = lock(&self.inner.board);
            (!board.members.contains_key(to))
                .then(|| board.aliases.get(to).cloned())
                .flatten()
        };
        let to = renamed.as_deref().unwrap_or(to);
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
            if from == PERSON {
                for (name, member) in &mut board.members {
                    if name == to || to == ALL {
                        member.asked.push(text.clone());
                        let over = member.asked.len().saturating_sub(ASKED_KEPT);
                        member.asked.drain(..over);
                        member.written = Some(now());
                    }
                }
            }
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
        // Kept as the person writes, so that iq opens again on the chat
        // they wrote to last.
        if from == PERSON && to != ALL {
            self.keep(to);
        }
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

    /// Whether a message to `agent` waits for it, left unread.
    fn has_unread(&self, agent: &str) -> bool {
        let board = lock(&self.inner.board);
        board.members.get(agent).is_some_and(|member| {
            board.messages[member.read..]
                .iter()
                .any(|m| m.from != agent && (m.to == agent || m.to == ALL))
        })
    }

    /// The messages to `agent` it has not read, as one text, marked read.
    fn unread(&self, agent: &str) -> Option<String> {
        let mut board = lock(&self.inner.board);
        let Board {
            messages, members, ..
        } = &mut *board;
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

    /// Waits for a message to `agent`: up to `seconds`, or, while agents
    /// it started work, until one of them writes, as long as
    /// [`tools::WAIT_MOST`] at most, whatever `seconds` it gave. Their
    /// answers are what it waits for, and each call to wait again reads
    /// its whole conversation. Once they no longer work, it waits
    /// `seconds` more for what they wrote last.
    async fn wait(&self, agent: &str, seconds: u64) -> String {
        let mut posted = self.inner.posted.subscribe();
        let started = tokio::time::Instant::now();
        // Said once, unless a message is there already: who it waits for.
        if !self.has_unread(agent) {
            let on = self.at_work_by(agent);
            if !on.is_empty() {
                self.report(Event::Waiting {
                    agent: agent.to_owned(),
                    on,
                });
            }
        }
        let most = started + Duration::from_secs(tools::WAIT_MOST);
        let mut seen: Option<tokio::time::Instant> = None;
        loop {
            posted.borrow_and_update();
            if let Some(news) = self.unread(agent) {
                return news;
            }
            let now = tokio::time::Instant::now();
            let theirs = self.started_at_work(agent);
            if theirs {
                seen = Some(now);
            }
            let until = (seen.unwrap_or(started) + Duration::from_secs(seconds)).min(most);
            if now >= until {
                return match seen {
                    Some(_) if theirs => format!(
                        "No message came in {} seconds, and the agents you started still work. \
                         End your turn: their answers will wake you.",
                        tools::WAIT_MOST
                    ),
                    Some(_) => "The agents you started no longer work, and wrote nothing more \
                                to you."
                        .to_owned(),
                    None => format!("No message came in {seconds} seconds."),
                };
            }
            // Looked at again now and then: an agent stopped writes
            // nothing.
            let next = until.min(now + WAIT_LOOK);
            let _ = tokio::time::timeout_at(next, posted.changed()).await;
        }
    }

    /// Whether an agent `agent` started works.
    fn started_at_work(&self, agent: &str) -> bool {
        !self.at_work_by(agent).is_empty()
    }

    /// The agents `agent` started that are at work, by name.
    fn at_work_by(&self, agent: &str) -> Vec<String> {
        let board = lock(&self.inner.board);
        let mut names: Vec<String> = board
            .members
            .iter()
            .filter(|(_, member)| {
                member.by == agent && member.turn.as_ref().is_some_and(|turn| !turn.is_finished())
            })
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
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
        member.prompt = Some(prompt.clone());
        member.turns += 1;
        let number = member.turns;
        self.report(Event::Working {
            agent: name.clone(),
            prompt: prompt.clone(),
        });
        self.inner.busy.fetch_add(1, Ordering::SeqCst);
        let busy = Busy(self.clone());
        member.turn = Some(tokio::spawn(async move {
            crew.turn(name, prompt, number).await;
            drop(busy);
        }));
    }

    /// One turn of an agent's, until it answers. Its state is taken as it
    /// was, and kept again only when the turn ends: a turn stopped halfway
    /// leaves the agent as it was before it.
    async fn turn(self, name: String, prompt: String, number: u64) {
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
        let (messages, outcome) = self.own_turn(&name, &model, messages, prompt, number).await;
        let mut compacted_from = None;
        let mut board = lock(&self.inner.board);
        let mut renamed = None;
        if let Some(member) = board.members.get_mut(&name) {
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
            renamed.clone_from(&member.renaming);
        }
        // The name the person gave it as it worked, taken as it rests,
        // under the same lock: no turn of it acts under the old one then.
        if let Some(new) = &renamed {
            self.move_on_board(&mut board, &name, new);
        }
        drop(board);
        let name = match renamed {
            Some(new) => {
                self.moved(&name, &new);
                new
            }
            None => name,
        };
        // Its starter may have taken a name of its own since.
        let by = lock(&self.inner.board)
            .members
            .get(&name)
            .map_or(by, |member| member.by.clone());
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
        self.keep(&name);
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
                // Told by iq: the agent that failed did not write it.
                Err(error) => format!(
                    "(iq: {name} stopped without answering: {error}. This is iq's account of \
                     what happened, not {name}'s.)"
                ),
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

    /// Notes that `agent` wrote where it works, and tells the person, once
    /// for two chats, when another chat of this session wrote there lately:
    /// two chats in one checkout may undo each other's changes.
    fn wrote(&self, agent: &str) {
        let (chat, folder) = {
            let board = lock(&self.inner.board);
            let Some(member) = board.members.get(agent) else {
                return;
            };
            (
                board.chat_of(agent),
                member.place.gate.workspace().to_path_buf(),
            )
        };
        let now = Instant::now();
        let mut writers = lock(&self.inner.writers);
        let Writers { lately, told } = &mut *writers;
        let wrote = lately.entry(folder.clone()).or_default();
        wrote.retain(|_, at| now.duration_since(*at) < LATELY);
        let others: Vec<(String, Duration)> = wrote
            .iter()
            .filter(|(other, _)| **other != chat)
            .filter(|(other, _)| {
                let pair = pair_of(&chat, other);
                !told.contains(&pair)
            })
            .map(|(other, at)| (other.clone(), now.duration_since(*at)))
            .collect();
        wrote.insert(chat.clone(), now);
        for (other, _) in &others {
            told.insert(pair_of(&chat, other));
        }
        drop(writers);
        for (other, ago) in others {
            self.report(Event::Notice {
                agent: chat.clone(),
                text: format!(
                    "the chat {other} wrote in {} {} ago, as this one does now: two chats in one \
                     checkout may undo each other's changes. Let one finish first, or give the \
                     next a worktree of its own (worktree = true)",
                    folder.display(),
                    lasted(ago),
                ),
            });
        }
    }

    /// Reports the call of `agent`'s to `model` that `failed`, its turn
    /// with it, with where it stood (its last call's size and `added`
    /// since, what it spent until then), and gives what iq says of it: the
    /// facts, to be told as iq's, not left to an agent to tell.
    fn failed(&self, agent: &str, model: &ModelId, failed: Failure, added: &[Message]) -> String {
        let (context, calls, spent) = lock(&self.inner.board)
            .members
            .get(agent)
            .map_or((0, 0, Usd(0.0)), |member| {
                (member.context, member.calls, member.spent)
            });
        // About four characters a token: what was added since the last
        // call that answered, which said how much it read.
        let added: usize = added
            .iter()
            .map(|message| serde_json::to_string(message).map_or(0, |text| text.len()))
            .sum();
        let context = context + u64::try_from(added / 4).unwrap_or(u64::MAX);
        let (status, message) = match &failed.refusal {
            Some(refusal) => (Some(refusal.status), refusal.message.clone()),
            None => (None, failed.said.clone()),
        };
        let said = match &failed.refusal {
            _ if failed.busy => failed.said.clone(),
            Some(refusal) => format!(
                "the provider refused its call to {model}, answering HTTP {}: \"{}\"",
                refusal.status, refusal.message
            ),
            None => format!("its call to {model} failed: {}", failed.said),
        };
        self.report(Event::Failed {
            agent: agent.to_owned(),
            model: model.to_string(),
            status,
            message,
            context,
            calls,
            spent,
        });
        let made = if calls == 1 {
            "1 call".to_owned()
        } else {
            format!("{calls} calls")
        };
        format!(
            "{said}. The call held about {} tokens; {agent} had made {made} before it, for {spent}",
            TokenCount(context)
        )
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

    /// Sends `request` for `agent`, again while its provider is out of
    /// capacity, as many times as the settings say: after as long as the
    /// provider asks, and at least a wait that doubles at each try, each
    /// wait said. Then the call fails saying what the person may do: the
    /// conversation is kept, to go on with later or on another model.
    async fn ask_model(
        &self,
        agent: &str,
        request: &ChatRequest,
    ) -> Result<iq_core::ChatResponse, Failure> {
        let settings = &self.inner.settings;
        let started = Instant::now();
        let mut wait = settings.busy_wait;
        let mut tries = 0;
        loop {
            let error = match self.inner.provider.complete(request).await {
                Ok(response) => {
                    if tries > 0 {
                        self.report(Event::Notice {
                            agent: agent.to_owned(),
                            text: format!("the provider of {} answers again", request.model),
                        });
                    }
                    return Ok(response);
                }
                Err(error) => error,
            };
            let Some(busy) = self.inner.provider.busy(&error) else {
                return Err(Failure {
                    said: chain(&error),
                    refusal: self.inner.provider.refusal(&error),
                    busy: false,
                });
            };
            let said = cut_to(&chain(&error), BUSY_SAID);
            if tries >= settings.busy_tries {
                let refusal = self.inner.provider.refusal(&error);
                return Err(Failure {
                    refusal,
                    busy: true,
                    said: format!(
                        "the provider of {} stayed out of capacity for {}, through {} tries ({said}). \
                     The conversation is kept: write to the agent to try again later, or give \
                     the task to an agent on another model",
                        request.model,
                        lasted(started.elapsed()),
                        tries + 1,
                    ),
                });
            }
            tries += 1;
            let delay = busy
                .after
                .unwrap_or_default()
                .max(wait)
                .min(settings.busy_most);
            wait = (wait * 2).min(settings.busy_most);
            self.report(Event::Notice {
                agent: agent.to_owned(),
                text: format!(
                    "the provider of {} is out of capacity ({said}): it is asked again in {}, \
                     try {tries} of {}",
                    request.model,
                    lasted(delay),
                    settings.busy_tries,
                ),
            });
            tokio::time::sleep(delay).await;
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
        number: u64,
    ) -> (Vec<Message>, Result<String, String>) {
        // What the last call that answered read: all but the answer it
        // gave, which is read from the next.
        let mut counted = messages.len().saturating_sub(1);
        messages.push(Message::user(prompt));
        if let Some(member) = lock(&self.inner.board).members.get_mut(name) {
            member.reason = None;
        }
        let mut urged = false;
        for _ in 0..self.inner.settings.steps {
            if let Some(news) = self.unread(name) {
                messages.push(Message::user(news));
            }
            if let Some(note) = self.context_note(name, model).await {
                messages.push(Message::user(note));
            }
            // What it is shown: after its prompt or its tools' results.
            if let Some(images) = lock(&self.inner.images).remove(name) {
                messages.extend(images);
            }
            // Read at each call, so that a change reaches a turn under way.
            let effort = lock(&self.inner.board)
                .members
                .get(name)
                .and_then(|member| member.effort)
                .or(self.inner.settings.effort);
            // Read at each call too: an extension may offer more once the
            // agent asked for it.
            let mut tools = tools::work();
            tools.extend(self.tools_for(name));
            let request = ChatRequest {
                model: model.clone(),
                messages: messages.clone(),
                tools,
                effort,
            };
            self.set_length(name, messages.len());
            let response = match self.ask_model(name, &request).await {
                Ok(response) => response,
                Err(failed) => {
                    let said = self.failed(name, model, failed, &messages[counted..]);
                    return (messages, Err(said));
                }
            };
            counted = messages.len();
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
                member.calls += 1;
                if let Some(cost) = cost {
                    member.spent += cost;
                }
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
            if let Some(text) = response.content.as_ref().filter(|t| !t.trim().is_empty())
                && let Some(member) = lock(&self.inner.board).members.get_mut(name)
            {
                member.reason = Some(text.clone());
            }
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
            self.set_length(name, messages.len());
            for call in &response.tool_calls {
                let content = self.call(name, call).await;
                messages.push(Message::Tool {
                    call_id: call.id.clone(),
                    content,
                });
            }
            self.set_length(name, messages.len());
            // Its calls answered, the turn is kept as it stands.
            self.keep_turn(name, number, &messages);
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
        if ok && WRITERS.contains(&call.name.as_str()) {
            self.wrote(agent);
        }
        if let Some(done) = review::done(&call.name, &input, &output, ok)
            && let Some(member) = lock(&self.inner.board).members.get_mut(agent)
        {
            member.done.push_back(done);
            while member.done.len() > DONE_KEPT {
                member.done.pop_front();
            }
        }
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
        let workspace = self.place_of(agent).gate.workspace().to_path_buf();
        match name {
            "read_file" => {
                let path: std::path::PathBuf = args.text("path")?.into();
                let permit = self.act(agent, Action::Read { path: path.clone() }).await?;
                let judged = match permit.action() {
                    Action::Read { path } => path.clone(),
                    _ => path.clone(),
                };
                if judged.is_dir() {
                    let listed = permit.list().map_err(|e| chain(&e))?;
                    return Ok(listing(&judged, &listed));
                }
                let bytes = permit.read().map_err(|e| chain(&e))?;
                if let Some(mime) = iq_core::image_type(&bytes) {
                    return self.shown_image(agent, &judged, mime, &bytes);
                }
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
                    line: line.clone(),
                    folder: workspace,
                    network: args.flag("network"),
                    host: args.flag("host"),
                };
                let permit = self.act(agent, action).await?;
                if args.flag("background") {
                    return Ok(self.background(agent, line, permit));
                }
                self.run(agent, permit, OWN_ASK).await.map(|(text, _)| text)
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
                if let Some(why) = self.refused_model(&model) {
                    return Err(why);
                }
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
                // Named after its chat and its part, which it keeps to.
                let part = role.clone().unwrap_or_default();
                let started = self.start_with(
                    &part,
                    &model,
                    args.text("task")?.to_owned(),
                    agent,
                    role,
                    effort,
                    Vec::new(),
                    START_AGENT,
                );
                Ok(format!(
                    "{started} started on {model}. Its answer will come to you as a message."
                ))
            }
            "start_team" => {
                let seats = team::seats(&args.value()["members"])?;
                if let Some(why) = seats
                    .iter()
                    .find_map(|seat| self.refused_model(&seat.model))
                {
                    return Err(why);
                }
                Ok(self.start_team(agent, args.text("task")?, &seats))
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
            "job" => {
                let id = args.number("id").ok_or("`id` names the job")?;
                Ok(self.job(agent, id, args.flag("stop")))
            }
            "wake_me" => {
                let seconds = args.number("seconds").unwrap_or(60).clamp(10, 3600);
                let why = args.text("why")?.trim().to_owned();
                self.wake_me(agent, seconds, why)
            }
            "ask_person" => {
                let question = args.text("question")?.trim().to_owned();
                let choices: Vec<String> = args.value()["choices"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.as_str().map(str::trim).filter(|c| !c.is_empty()))
                    .map(str::to_owned)
                    .collect();
                if question.is_empty() || !(2..=4).contains(&choices.len()) {
                    return Err("ask one question with two to four choices".into());
                }
                Ok(self.ask_person(agent, question, choices).await)
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
                    .find(|extension| extension.handles(agent, name));
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

    /// Runs the command `permit` allows for `agent`, in its place, and says
    /// how it went, and whether it succeeded. A command that failed without
    /// the internet, as one that needs it does, is told how to ask for it:
    /// `ask`.
    pub(crate) async fn run(
        &self,
        agent: &str,
        permit: Permit,
        ask: &str,
    ) -> Result<(String, bool), String> {
        let offline = matches!(
            permit.action(),
            Action::Run {
                network: false,
                host: false,
                ..
            }
        );
        let ran = self
            .sandbox_of(agent)
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
        if !ran.cleared.is_empty() {
            let named: Vec<String> = ran
                .cleared
                .iter()
                .take(CLEARED_NAMED)
                .map(|path| path.display().to_string())
                .collect();
            let more = ran.cleared.len().saturating_sub(CLEARED_NAMED);
            out.push_str(&format!(
                "[your folder of scratch grew past its size: its oldest files went, {}{}]\n",
                named.join(", "),
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
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
    /// when the gate asks: an extension's tool acts through it, as the
    /// crew's own do, so that what it does was judged.
    ///
    /// # Errors
    ///
    /// The gate denied it, whoever answers for the person refused it, or
    /// the audit log could not be written.
    pub async fn act(&self, agent: &str, action: Action) -> Result<Permit, String> {
        let judged = self
            .place_of(agent)
            .gate
            .judge(action)
            .map_err(|e| chain(&e))?;
        match judged {
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
        let gate = self.place_of(agent).gate.clone();
        let reviewable = match pending.scope() {
            Scope::Sandbox => true,
            Scope::Internet => gate.reviewer_internet(),
            Scope::Host => gate.reviewer_host(),
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
            // A make target is judged by its recipe, not by its name.
            let does = match pending.action() {
                Action::Run { line, .. } => self.make_recipes(agent, line),
                _ => None,
            };
            match self
                .review(
                    agent,
                    &action,
                    review::place(pending.action(), pending.scope()),
                    &reason,
                    does.as_deref(),
                )
                .await
            {
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
                        gate.approve(pending, Approver::Reviewer)
                            .map_err(|e| chain(&e))
                    } else {
                        self.refused(agent, &gate, pending, Approver::Reviewer, &why);
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
            Ok(Ok(true)) => gate
                .approve(pending, Approver::Person)
                .map_err(|e| chain(&e)),
            Ok(Ok(false)) => {
                self.refused(agent, &gate, pending, Approver::Person, &reason);
                Err(refusal("the person", &reason, true))
            }
            _ => {
                let why = format!("nobody answered in time ({reason})");
                self.refused(agent, &gate, pending, Approver::Person, &why);
                Err(refusal("the gate", &why, true))
            }
        }
    }

    /// Leaves the command `permit` allows, `line`, to run in the background
    /// for `agent`, as long as two hours: it is woken with how it ended.
    fn background(&self, agent: &str, line: String, permit: Permit) -> String {
        let id = self.inner.next_job.fetch_add(1, Ordering::Relaxed);
        let live = iq_gate::Live::default();
        let sandbox = self
            .sandbox_of(agent)
            .with_timeout(BACKGROUND_MOST)
            .with_quiet(BACKGROUND_QUIET);
        let crew = self.clone();
        let (name, kept_line, watched) = (agent.to_owned(), line.clone(), live.clone());
        let running = tokio::spawn(async move {
            let said = match sandbox.run_live(permit, &watched).await {
                Ok(ran) if ran.timed_out => "it was stopped, out of time".to_owned(),
                Ok(ran) => match ran.status {
                    Some(status) => format!("exit status {status}"),
                    None => "ended by a signal".to_owned(),
                },
                Err(error) => format!("it could not run: {}", chain(&error)),
            };
            crew.job_ended(&name, id, &kept_line, said);
        });
        lock(&self.inner.jobs).insert(
            id,
            Job {
                agent: agent.to_owned(),
                line: line.clone(),
                started: Instant::now(),
                live,
                running: Some(running),
                ended: None,
            },
        );
        self.report(Event::JobStarted {
            agent: agent.to_owned(),
            id,
            line: line.clone(),
        });
        format!(
            "Job {id} runs `{line}` in the background. `job` with id {id} reads its output so \
             far or stops it; you are woken when it ends."
        )
    }

    /// Writes down how the job `id` of `agent`'s ended, and wakes it with
    /// its status and the end of its output.
    fn job_ended(&self, agent: &str, id: u64, line: &str, said: String) {
        let last = {
            let mut jobs = lock(&self.inner.jobs);
            let Some(job) = jobs.get_mut(&id) else {
                return;
            };
            if job.ended.is_some() {
                return;
            }
            job.ended = Some(said.clone());
            job.running = None;
            job.live.so_far(JOB_SHOWN)
        };
        self.report(Event::JobEnded {
            agent: agent.to_owned(),
            id,
            said: said.clone(),
        });
        self.iq_says(
            agent,
            format!("Job {id}, `{line}`, ended: {said}. The end of its output:\n{last}"),
        );
    }

    /// The job `id` of `agent`'s: how long it has run or how it ended, and
    /// the end of its output; stopped first when `stop`.
    fn job(&self, agent: &str, id: u64, stop: bool) -> String {
        let (line, started, running) = {
            let jobs = lock(&self.inner.jobs);
            let Some(job) = jobs.get(&id).filter(|job| job.agent == agent) else {
                return format!("you have no job {id}");
            };
            (job.line.clone(), job.started, job.running.is_some())
        };
        if stop && running {
            if let Some(task) = lock(&self.inner.jobs)
                .get_mut(&id)
                .and_then(|job| job.running.take())
            {
                task.abort();
            }
            self.job_ended(agent, id, &line, "stopped, as you asked".to_owned());
        }
        let jobs = lock(&self.inner.jobs);
        let Some(job) = jobs.get(&id) else {
            return format!("you have no job {id}");
        };
        let state = match &job.ended {
            Some(said) => format!("ended: {said}"),
            None => format!("running for {}", short_span(started.elapsed().as_secs())),
        };
        format!(
            "Job {id}, `{line}`, {state}. The end of its output so far:\n{}",
            job.live.so_far(JOB_SHOWN)
        )
    }

    /// Stops the jobs of `agent`, or every one when none.
    fn stop_jobs(&self, agent: Option<&str>) {
        let stopped: Vec<(u64, String, String)> = {
            let mut jobs = lock(&self.inner.jobs);
            jobs.iter_mut()
                .filter(|(_, job)| agent.is_none_or(|a| job.agent == a))
                .filter_map(|(id, job)| {
                    let task = job.running.take()?;
                    task.abort();
                    Some((*id, job.agent.clone(), job.line.clone()))
                })
                .collect()
        };
        for (id, agent, line) in stopped {
            self.job_ended(&agent, id, &line, "stopped with its agent".to_owned());
        }
    }

    /// Wakes `agent` after `seconds`, saying `why`; a few at a time.
    fn wake_me(&self, agent: &str, seconds: u64, why: String) -> Result<String, String> {
        lock(&self.inner.wakes)
            .entry(agent.to_owned())
            .or_default()
            .ask(Instant::now())?;
        self.report(Event::Notice {
            agent: agent.to_owned(),
            text: format!("asked to be woken in {}: {why}", short_span(seconds)),
        });
        let crew = self.clone();
        let name = agent.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            if let Some(wakes) = lock(&crew.inner.wakes).get_mut(&name) {
                wakes.waiting = wakes.waiting.saturating_sub(1);
            }
            crew.iq_says(&name, format!("You asked to be woken now: {why}"));
        });
        Ok(format!(
            "You will be woken in {}. End your turn now.",
            short_span(seconds)
        ))
    }

    /// Writes `text` to `agent` as iq's own message, waking it when it
    /// rests: what a job or a wake it asked for says.
    fn iq_says(&self, agent: &str, text: String) {
        let resting = {
            let mut board = lock(&self.inner.board);
            if !board.members.contains_key(agent) {
                return;
            }
            board.messages.push(Posted {
                from: IQ.to_owned(),
                to: agent.to_owned(),
                text: text.clone(),
            });
            board
                .members
                .get(agent)
                .is_some_and(|m| m.turn.as_ref().is_none_or(JoinHandle::is_finished))
        };
        self.inner.posted.send_modify(|count| *count += 1);
        if resting {
            let prompt = self.unread(agent).unwrap_or(text);
            self.wake(agent, prompt);
        }
    }

    /// Asks the person `question`, offering `choices`, and waits for their
    /// answer as long as the settings' patience. What they answered is
    /// kept with what they asked of `agent`, so that the reviewer reads a
    /// "yes" with the question it answered.
    async fn ask_person(&self, agent: &str, question: String, choices: Vec<String>) -> String {
        let id = self.inner.next_question.fetch_add(1, Ordering::Relaxed);
        let (reply, answer) = oneshot::channel();
        lock(&self.inner.choices).insert(id, reply);
        self.report(Event::Choose {
            id,
            agent: agent.to_owned(),
            question: question.clone(),
            choices: choices.clone(),
        });
        let answer = tokio::time::timeout(self.inner.settings.patience, answer)
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
            .map(|a| a.trim().to_owned())
            .filter(|a| !a.is_empty());
        lock(&self.inner.choices).remove(&id);
        self.report(Event::Chosen {
            id,
            answer: answer.clone(),
        });
        match answer {
            Some(answer) => {
                if let Some(member) = lock(&self.inner.board).members.get_mut(agent) {
                    member
                        .asked
                        .push(format!("To your question \"{question}\": {answer}"));
                }
                if choices.contains(&answer) {
                    format!("The person chose: {answer}")
                } else {
                    format!("The person answered in their own words: {answer}")
                }
            }
            None => "The person gave no answer: go on as best you can, and say what you \
                     needed to know."
                .to_owned(),
        }
    }

    /// Writes a refusal of `agent`'s action to the audit log; one that
    /// cannot be written is said, as nothing is done either way.
    fn refused(&self, agent: &str, gate: &Gate, pending: Pending, by: Approver, why: &str) {
        if let Err(error) = gate.refuse(pending, by, why) {
            self.report(Event::Notice {
                agent: agent.to_owned(),
                text: format!("the audit log misses a refusal: {}", chain(&error)),
            });
        }
    }

    /// What the make targets of `line` do, from the Makefiles of the place
    /// `agent` works in that make them, when `line` runs make and they have
    /// them. A Makefile is read only within the place, wherever a link in
    /// its path leads.
    fn make_recipes(&self, agent: &str, line: &str) -> Option<String> {
        if crate::make::makes(line).is_empty() {
            return None;
        }
        let place = self.place_of(agent);
        let workspace = place.gate.workspace().to_path_buf();
        let real = workspace.canonicalize().ok()?;
        crate::make::recipes(line, |relative| {
            let path = workspace.join(relative);
            if !path.is_file() || !path.canonicalize().ok()?.starts_with(&real) {
                return None;
            }
            // By its own name, as its folder lists it: where names are not
            // told apart by case, `makefile` would find `Makefile`.
            let name = path.file_name()?;
            let listed = std::fs::read_dir(path.parent()?)
                .ok()?
                .flatten()
                .any(|entry| entry.file_name() == name);
            if !listed {
                return None;
            }
            self.read_quietly(&place.gate, path)
        })
    }

    /// The reviewer's verdict on an action of `agent`'s, which runs at
    /// `place`, told what the action does when its name alone does not say
    /// (`does`).
    async fn review(
        &self,
        agent: &str,
        action: &str,
        place: &str,
        reason: &str,
        does: Option<&str>,
    ) -> Result<(bool, String), String> {
        let work = lock(&self.inner.board)
            .members
            .get(agent)
            .map(|member| review::Work {
                task: member.task.clone(),
                asked: member.asked.clone(),
                prompt: member.prompt.clone(),
                done: member.done.iter().cloned().collect(),
                reason: member.reason.clone(),
            })
            .unwrap_or_default();
        let prompt = review::prompt(&work, action, place, reason, does);
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
            effort: Some(self.inner.settings.reviewer_effort),
        };
        let response = self
            .ask_model(agent, &request)
            .await
            .map_err(|failed| failed.said)?;
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
        self.inner.place.gate.set_reviewer_internet(may);
    }

    /// Lets the reviewer let a command run on the machine itself, the
    /// autopilot on, following the policy the person wrote for it, or
    /// leaves that to the person. What changes what others share is always
    /// the person's.
    pub fn set_autopilot_host(&self, may: bool) {
        self.inner.place.gate.set_reviewer_host(may);
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

    fn report(&self, mut event: Event) {
        self.set_aside(&event);
        for (name, shown) in lock(&self.inner.shown).iter() {
            event.rename(name, shown);
        }
        (self.inner.report)(event);
    }

    /// Keeps what `event` shows the person of an agent beside its
    /// conversation, to be kept with its chat: a notice, a question, an
    /// action asked or answered for them, an error; and the answers to
    /// those asked before. Counts its turns.
    fn set_aside(&self, event: &Event) {
        let mut aside = lock(&self.inner.aside);
        let (agent, shown) = match event {
            Event::Working { agent, .. } => {
                aside.entry(agent.clone()).or_default().turn += 1;
                return;
            }
            Event::Chosen { id, answer } => {
                for log in aside.values_mut() {
                    for kept in log.kept.iter_mut().rev() {
                        if let Shown::Question {
                            id: asked,
                            answered,
                            answer: given,
                            ..
                        } = &mut kept.shown
                            && asked == id
                        {
                            *answered = true;
                            given.clone_from(answer);
                            return;
                        }
                    }
                }
                return;
            }
            Event::Answered { id, approved } => {
                for log in aside.values_mut() {
                    for kept in log.kept.iter_mut().rev() {
                        if let Shown::Asked {
                            id: asked,
                            approved: given,
                            ..
                        } = &mut kept.shown
                            && asked == id
                        {
                            *given = Some(*approved);
                            return;
                        }
                    }
                }
                return;
            }
            Event::Notice { agent, text } => (agent, Shown::Notice { text: text.clone() }),
            Event::Choose {
                id,
                agent,
                question,
                choices,
            } => (
                agent,
                Shown::Question {
                    id: *id,
                    question: question.clone(),
                    choices: choices.clone(),
                    answered: false,
                    answer: None,
                },
            ),
            Event::Asked {
                id,
                agent,
                action,
                reason,
                ..
            } => (
                agent,
                Shown::Asked {
                    id: *id,
                    action: action.clone(),
                    reason: reason.clone(),
                    approved: None,
                },
            ),
            Event::Reviewed {
                agent,
                action,
                approved,
                why,
                ..
            } => (
                agent,
                Shown::Reviewed {
                    action: action.clone(),
                    approved: *approved,
                    why: why.clone(),
                },
            ),
            Event::Failed {
                agent,
                model,
                status,
                message,
                context,
                calls,
                spent,
            } => (
                agent,
                Shown::Failed {
                    model: model.clone(),
                    status: *status,
                    message: message.clone(),
                    context: *context,
                    calls: *calls,
                    spent: *spent,
                },
            ),
            Event::Done {
                agent,
                outcome: Err(text),
            } => {
                // A failed call was kept already, with what it said.
                let log = aside.entry(agent.clone()).or_default();
                if log.kept.last().is_some_and(|last| {
                    last.turn == log.turn && matches!(last.shown, Shown::Failed { .. })
                }) {
                    return;
                }
                (agent, Shown::Error { text: text.clone() })
            }
            _ => return,
        };
        let log = aside.entry(agent.clone()).or_default();
        let (turn, after) = (log.turn, log.messages);
        log.kept.push(Aside {
            at: now(),
            turn,
            after,
            shown,
        });
    }

    /// Says that `agent`'s conversation holds `messages` now, for what is
    /// set aside next to stand after them.
    fn set_length(&self, agent: &str, messages: usize) {
        lock(&self.inner.aside)
            .entry(agent.to_owned())
            .or_default()
            .messages = messages;
    }

    /// The standing instructions of one of iq's agents, working in
    /// `workspace`.
    fn instructions(&self, name: &str, by: &str, role: Option<&str>, place: &Place) -> String {
        let workspace = place.gate.workspace().display();
        let started = if by == PERSON {
            "The person gave you your task.".to_owned()
        } else {
            format!("The agent {by} started you and gets your answer.")
        };
        let part = role_line(role);
        // A name on the board is the chat's, or that of a part of it: the
        // agent is not who it names.
        let named = if by == PERSON {
            "the name of this chat, which the person gave it or iq made".to_owned()
        } else {
            let chat = name.split('/').next().unwrap_or(name);
            format!("the name of your part in the chat {chat}, which iq gave it")
        };
        // Its chat's folder of scratch, by the name the chat has now.
        let scratch = place
            .gate
            .scratch()
            .map(|root| {
                let chat = name.split('/').next().unwrap_or(name);
                format!(
                    "\n\nKeep what outlives one command (a long output to read in parts, a \
                     snapshot to compare with, notes of your own) in your chat's folder of \
                     scratch, {}, which commands find as $IQ_SCRATCH: they write there, and you \
                     read and write its files without asking. Never leave such files in the \
                     workspace, where they would be committed with the work: `make check > \
                     \"$IQ_SCRATCH/check.log\" 2>&1`, then read what you need of it. Its oldest \
                     files go once it grows past {} MB.",
                    root.join(chat).display(),
                    self.inner.settings.scratch_most / (1024 * 1024)
                )
            })
            .unwrap_or_default();
        let allowed = &self.inner.settings.allowed;
        let models = if allowed.is_empty() {
            String::new()
        } else {
            let names: Vec<String> = allowed.iter().map(ToString::to_string).collect();
            format!(
                " The person allows these models only, for any agent: {}; start_agent and \
                 start_team refuse any other.",
                names.join(", ")
            )
        };
        format!(
            "You are a coding agent of iq, a coding harness, working in {workspace}. On iq's board, \
             where the person and the agents write to each other, you are listed as {name}: \
             {named}, not who you are; what is written to {name} reaches you. \
             {started}{part}\n\n\
             Your tools act on the machine through iq's gate: files are read and written in the \
             workspace, and commands run in a sandbox that writes only the workspace. Commands \
             reach localhost (servers you start, the tests that talk to them); the internet is \
             given to what fetches packages and sources (npm install, cargo fetch, pip install, \
             git fetch), and asked of the person for others with `network: true`, unless the \
             person opened it to every command. What the gate holds waits for the person, or for \
             a reviewer acting for them; when it is refused, you are told why: find another way, \
             or say what you need.{scratch}\n\n\
             Other agents may work beside you. start_agent hands a task to a new one, \
             send_message writes to one, to all, or to the person, and wait_for_messages waits \
             for an answer. Messages to you also arrive between your calls.{models}\n\n\
             Read before you change, check your work with the project's own tools (its build, \
             tests, linters), and be brief. When the task is done, answer with what you did and \
             call no tool.{}{}",
            if by == PERSON {
                self.known(place)
            } else {
                HANDING_ON.to_owned()
            },
            self.extended(name, by)
        )
    }

    /// What an agent the person started is told beyond its task: the
    /// person's rules and the project's, and that the agents it starts
    /// know none of what it knows but what it gives them.
    fn known(&self, place: &Place) -> String {
        let mut out = String::new();
        for file in &self.inner.settings.rules {
            if let Some(text) = rules::kept(&file.text) {
                out.push_str(&format!(
                    "\n\nThe person's rules, from their {}, which you follow:\n\n{text}",
                    file.from
                ));
            }
        }
        for (file, text) in self.project_rules(place) {
            out.push_str(&format!(
                "\n\nThe project's rules for agents, from its {file}, which you follow:\n\n{text}"
            ));
        }
        let scoped = self.scoped_rules(place);
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

    /// Reads `path` through `gate`, as an agent's reads are, never asking
    /// the person: what the gate does not allow at once is not read.
    fn read_quietly(&self, gate: &Gate, path: std::path::PathBuf) -> Option<String> {
        match gate.judge(Action::Read { path }).ok()? {
            Judged::Allowed(permit) => String::from_utf8(permit.read().ok()?).ok(),
            Judged::Asked { .. } | Judged::Denied { .. } => None,
        }
    }

    /// The project's rules for agents, each with the file it came from: its
    /// `AGENTS.md`, then its `CLAUDE.md`, whose `@file` lines are the
    /// files they import, a file given already left out; then those of its
    /// `.claude/rules` for the whole project. Read where the agent works:
    /// a place of its own may hold other rules than the person's checkout.
    fn project_rules(&self, place: &Place) -> Vec<(String, String)> {
        let gate = &place.gate;
        let workspace = gate.workspace().to_path_buf();
        let mut given: Vec<(String, String)> = Vec::new();
        for file in RULES_FILES {
            let path = workspace.join(file);
            if !path.is_file() {
                continue;
            }
            let Some(text) = self.read_quietly(gate, path) else {
                continue;
            };
            let text: String = text
                .lines()
                .map(|line| match rules::import(line) {
                    // As `AGENTS.md` is, imported by `CLAUDE.md`.
                    Some(name) if given.iter().any(|(file, _)| file == name) => "\n".to_owned(),
                    Some(name) => {
                        self.read_quietly(gate, workspace.join(name))
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
        for (file, text) in self.rules_folder(place) {
            let (paths, body) = rules::scope(&text);
            if paths.is_empty()
                && let Some(body) = rules::kept(body)
            {
                given.push((file, body));
            }
        }
        given
    }

    /// The files of the project's `.claude/rules` in `place`, each with
    /// what it says, in the order of their names.
    fn rules_folder(&self, place: &Place) -> Vec<(String, String)> {
        let workspace = place.gate.workspace().to_path_buf();
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
                Some((file, self.read_quietly(&place.gate, path)?))
            })
            .collect()
    }

    /// The project's rules for some of its files, in `place`, read again
    /// once they are older than [`SCOPED_FRESH`].
    fn scoped_rules(&self, place: &Place) -> Vec<rules::Scoped> {
        let workspace = place.gate.workspace().to_path_buf();
        if let Some((read, rules)) = lock(&self.inner.scoped).get(&workspace)
            && read.elapsed() < SCOPED_FRESH
        {
            return rules.clone();
        }
        let rules: Vec<rules::Scoped> = self
            .rules_folder(place)
            .into_iter()
            .filter_map(|(file, text)| {
                let (paths, body) = rules::scope(&text);
                let text = rules::kept(body)?;
                (!paths.is_empty()).then_some(rules::Scoped { file, paths, text })
            })
            .collect();
        lock(&self.inner.scoped).insert(workspace, (Instant::now(), rules.clone()));
        rules
    }

    /// The project's rules for `path` that `agent` was not given yet, as
    /// it reads or writes it, to follow on the tool's answer; marked given.
    fn scoped_for(&self, agent: &str, path: &std::path::Path) -> String {
        let place = self.place_of(agent);
        let relative = if path.is_absolute() {
            match path.strip_prefix(place.gate.workspace()) {
                Ok(relative) => relative,
                Err(_) => return String::new(),
            }
        } else {
            path
        };
        let relative = relative.to_string_lossy();
        let matching: Vec<rules::Scoped> = self
            .scoped_rules(&place)
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

/// A name for an agent `by` starts, with `part` its part in the crew:
/// its chat's name and its part, `<chat>/review`, or a number when it has
/// none, `<chat>/2`; never one an agent has, nor `taken`. An agent is part
/// of a chat: named after it, it never takes the name of one.
fn agent_name(board: &Board, by: &str, part: &str, taken: &[String]) -> String {
    let chat = board.chat_of(by);
    let part = part_of(part);
    if !part.is_empty() {
        return free_name(board, &format!("{chat}/{part}"), taken);
    }
    let free = |name: &str| board.free(name) && !taken.iter().any(|t| t == name);
    let mut n = 2;
    while !free(&format!("{chat}/{n}")) {
        n += 1;
    }
    format!("{chat}/{n}")
}

/// The longest part of an agent's name, after its chat's.
const PART_CHARS: usize = 20;

/// `role` as the part of an agent's name: its first words, lower case,
/// joined by dashes, as many as fit in [`PART_CHARS`]. Empty for none.
fn part_of(role: &str) -> String {
    dashed(role, PART_CHARS)
}

/// The first words of `text`, lower case, joined by dashes: three at most,
/// as many as fit in `most` characters.
fn dashed(text: &str, most: usize) -> String {
    let mut out = String::new();
    let words = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(3);
    for word in words {
        let word = word.to_lowercase();
        let more = usize::from(!out.is_empty()) + word.chars().count();
        if out.chars().count() + more > most {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&word);
    }
    out
}

/// Who the cost of naming a chat is reported for, as the costs show it.
const NAMER: &str = "namer";

/// The longest name the namer gives a chat.
const TITLE_CHARS: usize = 24;

/// What the namer's model is told.
const NAMING: &str = "You name a chat of a coding harness after what its work is about, from the \
    first request the person wrote in it, which follows. Answer with two or three words in the \
    request's own language, lower case, joined by dashes, that name its subject (the feature, the \
    module, the bug), not the way it is asked: for \"could you code the payment part of the \
    invoice module so that a refund can be made?\", payment-refunds. The name alone, nothing \
    else.";

/// The name a namer's `answer` gives a chat, as the board writes names;
/// none when it gives none.
fn title(answer: &str) -> Option<String> {
    let line = answer.lines().find(|line| !line.trim().is_empty())?;
    let name = base_name(&dashed(line, TITLE_CHARS));
    (name != "agent").then_some(name)
}

/// `name` as the board writes an agent's: each part between slashes as
/// [`base_name`] writes it.
fn board_name(name: &str) -> String {
    name.split('/').map(base_name).collect::<Vec<_>>().join("/")
}

/// Seconds since 1970.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// `base`, or `base-2`, `base-3`…: the first no member has nor is to
/// take, nor `taken`.
fn free_name(board: &Board, base: &str, taken: &[String]) -> String {
    let free = |name: &str| board.free(name) && !taken.iter().any(|t| t == name);
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
            reasoning_signature: None,
        });
    }
    kept
}

/// The line of an agent's instructions that gives it its part, if any.
/// What an agent another started is told of those it starts in turn: they
/// know of its task only what it writes into theirs, so that the rules it
/// was given reach them only if it passes them on.
const HANDING_ON: &str = "\n\nAn agent or a team you start sees only the task you give it: \
    nothing of your conversation, nor of your own task. Write into its task what its part needs \
    of yours (the rules its work must follow, the facts it must know, the files it works on), \
    and nothing more: it reads all of it on every call.";

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
        Action::Tool {
            server,
            tool,
            arguments,
            reads,
        } => {
            let only = if *reads {
                ", which it says only reads"
            } else {
                ""
            };
            format!(
                "call {tool} of the server {server}{only}, with {}",
                cut_to(arguments, TOOL_SAID)
            )
        }
    }
}

/// The most characters of a server's tool's arguments said as its call is
/// asked: enough for what it writes to be read.
const TOOL_SAID: usize = 2000;

/// The entries of `folder` as a model reads them: each with its size and
/// how long ago it changed, the latest first, a folder's name ending in `/`.
fn listing(folder: &Path, listed: &[iq_gate::Listed]) -> String {
    if listed.is_empty() {
        return format!("{} is an empty folder.", folder.display());
    }
    let now = std::time::SystemTime::now();
    let mut lines = vec![format!("{}, the latest changed first:", folder.display())];
    for entry in listed {
        let ago = entry
            .modified
            .and_then(|at| now.duration_since(at).ok())
            .map(|d| format!(", changed {} ago", short_span(d.as_secs())))
            .unwrap_or_default();
        if entry.folder {
            lines.push(format!("{}/{ago}", entry.name));
        } else {
            lines.push(format!(
                "{} ({} KB{ago})",
                entry.name,
                entry.bytes.div_ceil(1024)
            ));
        }
    }
    lines.join("\n")
}

/// `seconds` as a short span: `42s`, `5m`, `3h`, `2d`.
fn short_span(seconds: u64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

/// An error with its causes, as one line.
/// The most characters of a provider's error said while it is waited for.
const BUSY_SAID: usize = 200;

/// `text` within `most` characters, an ellipsis where it was cut.
fn cut_to(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(most.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// A length of time as said to the person: `350ms`, `20s`, `4m 05s`.
fn lasted(time: Duration) -> String {
    let seconds = time.as_secs();
    if seconds == 0 {
        format!("{}ms", time.as_millis())
    } else if seconds < 120 {
        format!("{seconds}s")
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

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
    fn an_agent_waking_itself_again_and_again_is_held_within_the_hour() {
        let mut wakes = Wakes::default();
        let start = Instant::now();
        // Three at a time.
        for _ in 0..WAKES_MOST {
            assert!(wakes.ask(start).is_ok());
        }
        assert!(wakes.ask(start).unwrap_err().contains("wait for those"));
        // Each fired, it asks again: as many as twelve within the hour.
        let mut at = start;
        loop {
            wakes.waiting = 0;
            at += Duration::from_secs(60);
            if wakes.ask(at).is_err() {
                break;
            }
        }
        assert_eq!(wakes.asked.len(), WAKES_PER_HOUR);
        assert!(wakes.ask(at).unwrap_err().contains("within the hour"));
        // An hour on, it may again.
        assert!(wakes.ask(at + Duration::from_secs(3600)).is_ok());
    }

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
