//! The `iq` command.

#![deny(unsafe_code)]

mod chats;
mod journal;
mod ledger;
mod lock;
mod settings;
mod sync;
mod terminal;
mod worktree;

use std::io::{BufRead, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use iq_agent::{Crew, Event, Report};
use iq_core::ModelId;
use iq_gate::{Action, Approver, Gate, HELPER_FLAG, HOLD_FLAG, Helper, Judged, Rules};
use iq_llm::OpenAiCompatible;
use iq_ui::Update;
use iq_ui::crew::Request;
use tracing_subscriber::EnvFilter;

use crate::journal::Journal;
use crate::ledger::Ledger;
use crate::settings::Settings;

/// How often iq's journal is looked at again, for the tasks it holds.
const READ_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

/// The lead's name on the board.
const LEAD: &str = "lead";

/// Without a command, iq runs in the terminal: the agents at work, and the
/// cost of every call to a model.
#[derive(Parser)]
#[command(version, about = "An experimental coding agent")]
struct Cli {
    /// Base URL of an OpenAI compatible endpoint, instead of the settings'
    /// `base_url`: Requesty's router when neither names one.
    #[arg(long, env = "IQ_BASE_URL", global = true)]
    base_url: Option<String>,

    /// API key for that endpoint. Read from the environment so that it never
    /// lands in the shell history.
    #[arg(long, env = "REQUESTY_API_KEY", hide_env_values = true, global = true)]
    api_key: Option<String>,

    /// The lead's model, instead of the settings': a model of the provider,
    /// such as `anthropic/claude-opus-5-5`.
    #[arg(long, global = true)]
    model: Option<String>,

    /// Lets the reviewer answer for the person what stays within the
    /// sandbox.
    #[arg(long, global = true)]
    autopilot: bool,

    /// Gives every command the internet, rather than what fetches packages
    /// and what the person allows. Files stay held, secrets out.
    #[arg(long, global = true)]
    internet: bool,

    /// Each chat works in a worktree of its own, on a branch `iq/<chat>` from
    /// what is checked out: the checkout is left as it is.
    #[arg(long, global = true)]
    worktree: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Hands a task to the crew in the terminal: the lead, and the agents
    /// it starts, work until they answer.
    Do {
        /// The task.
        task: String,
        /// How the team works: solo, lead, pair (a writer and a reviewer),
        /// debate (agents answer alone, then read each other).
        #[arg(long, default_value = "solo")]
        pattern: iq_agent::Pattern,
        /// The second model, for pair (the reviewer) or debate (a second
        /// voice); for lead, a cheaper one its agents may use.
        #[arg(long)]
        partner: Option<String>,
    },
    /// The provider's models, with what they charge per million tokens.
    Models,
    /// Syncs iq's folder of data now with the repository the settings'
    /// [sync] names: what changed elsewhere taken, what changed here sent
    /// once the person says so, unless [sync] auto; a new machine takes
    /// everything first. iq does it as it runs too.
    Sync,
    /// Runs a command line in the sandbox, as a model's would be: judged by
    /// the gate, asked when it must be, written to the audit log.
    Run {
        /// The line, as a shell reads it.
        line: String,
        /// Lets it ask for the network, which the sandbox withholds
        /// otherwise.
        #[arg(long)]
        network: bool,
        /// Lets it ask for the machine itself, outside the sandbox, as
        /// what the rules know needs it is asked.
        #[arg(long)]
        host: bool,
    },
}

fn main() -> Result<()> {
    // iq's program run for another purpose than the person's, before
    // anything else starts: the sandbox's helper confines itself and
    // becomes the command.
    let mut args = std::env::args_os();
    match args.nth(1) {
        Some(flag) if flag == HELPER_FLAG => {
            let error = iq_gate::sandboxed(args);
            eprintln!("{}", iq_gate::helper_failure(&error));
            std::process::exit(125);
        }
        Some(flag) if flag == HOLD_FLAG => {
            let error = iq_gate::hold_network();
            eprintln!("{}", iq_gate::helper_failure(&error));
            std::process::exit(125);
        }
        _ => {}
    }
    let mut cli = Cli::parse();
    // The provider's address: as the command line or the environment
    // names it, else the settings, else Requesty's router.
    if cli.base_url.is_none() {
        cli.base_url = home_folder()
            .ok()
            .and_then(|folder| Settings::load(&folder).ok())
            .and_then(|settings| settings.base_url());
    }
    // In the terminal, what is logged would write over what iq draws: it
    // goes to iq's folder instead.
    let log = cli
        .command
        .is_none()
        .then(|| home_folder().ok())
        .flatten()
        .and_then(|folder| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(folder.join("iq.log"))
                .ok()
        });
    // In its file, the warnings too: what went wrong is read there after;
    // on the terminal, the errors alone, unless RUST_LOG says.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(if log.is_some() { "warn" } else { "error" }));
    let logging = tracing_subscriber::fmt().with_env_filter(filter);
    match log {
        Some(file) => logging
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .init(),
        None => logging.with_writer(std::io::stderr).init(),
    }
    let runtime = tokio::runtime::Runtime::new().context("starting the runtime")?;
    match &cli.command {
        Some(Command::Do {
            task,
            pattern,
            partner,
        }) => runtime.block_on(work(&cli, task, *pattern, partner.as_deref())),
        Some(command) => runtime.block_on(run(command, &cli)),
        None => session(&runtime, &cli),
    }
}

/// The crew of the current folder, what it does reported to `report`.
/// Must run within the runtime.
fn crew(cli: &Cli, settings: &Settings, report: Report) -> Result<Crew<OpenAiCompatible>> {
    let home = home_dir()?;
    let workspace = std::env::current_dir().context("reading the current folder")?;
    let mut crew_settings = settings.crew()?;
    crew_settings.rules = person_rules(&settings.rules_files(&home), &home);
    let crew = Crew::new(
        provider(cli),
        gate(
            settings.internet,
            settings.autopilot_internet(),
            settings.autopilot_host(),
        )?,
        Helper::this_program()?,
        crew_settings,
        report,
    );
    crew.set_autopilot(settings.autopilot);
    if settings.worktree {
        let placer = worktrees(settings, &home, &workspace)?;
        crew.set_placer(Some(Arc::new(placer)));
    }
    // Each chat kept at every turn's end, to come back when iq starts.
    crew.set_keeper(Some(Arc::new(chats::Chats::open(
        &home_folder()?,
        &project(),
    ))));
    // What the agents the person starts remember of this project, kept
    // with iq's data, and the notes kept of it under ~/.claude.
    let claude = settings
        .claude_memory()
        .then(|| iq_memory::claude_folder(&home.join(".claude/projects"), &workspace))
        .flatten();
    let memory = iq_memory::Memory::open(&settings::data_folder(&home).join("memory"), &workspace)
        .with_claude(claude);
    crew.extend(Arc::new(memory));
    // And the skills the person wrote for them.
    let mut skills = iq_memory::Skills::new(settings.skill_folders(&home, &workspace));
    if let Some(name) = &settings.autopilot_skill {
        skills = skills.with_reviewer(name, &workspace);
    }
    crew.extend(Arc::new(skills));
    Ok(crew)
}

/// The worktrees of the repository `workspace` is in, as the settings say
/// they are made.
fn worktrees(settings: &Settings, home: &Path, workspace: &Path) -> Result<worktree::Worktrees> {
    let folder = settings::data_folder(home).join("worktrees");
    worktree::Worktrees::open(
        workspace,
        &folder,
        settings.worktree_from(),
        settings.worktree_setup(),
    )
    .map_err(|why| anyhow::anyhow!("no worktree for the chats: {why}"))
}

/// Puts the project's chats kept from earlier sessions back on the board,
/// resting, each in its worktree when it still has one; the chats and the
/// worktrees untouched for as long as the settings say forgotten first.
/// What was forgotten is said; what came back is said to each chat.
fn resume_chats(
    settings: &Settings,
    crew: &Crew<OpenAiCompatible>,
    tell: &std::sync::mpsc::Sender<Update>,
) -> Vec<String> {
    let (Ok(folder), Ok(home), Ok(workspace)) =
        (home_folder(), home_dir(), std::env::current_dir())
    else {
        return Vec::new();
    };
    let chats = chats::Chats::open(&folder, &project());
    let mut said = Vec::new();
    if let Some(keep) = settings.chat_keep() {
        said.extend(chats.prune(keep));
    }
    let worktrees = settings
        .worktree
        .then(|| worktrees(settings, &home, &workspace).ok())
        .flatten();
    let now = iq_ui::now();
    for found in chats.list() {
        let name = found.chat.name.clone();
        let place = match (&found.chat.folder, &worktrees) {
            (Some(folder), Some(worktrees)) if folder.is_dir() => Some(iq_agent::Placed {
                folder: folder.clone(),
                writable: vec![worktrees.common().to_path_buf()],
                said: String::new(),
            }),
            (Some(folder), _) => {
                said.push(format!(
                    "the chat {name} worked in {}, which is gone: it goes on in this folder",
                    folder.display()
                ));
                None
            }
            (None, _) => None,
        };
        let task: String = found.chat.task.chars().take(80).collect();
        match crew.resume(found.chat, place) {
            Ok(name) => {
                let days = now.saturating_sub(found.at) / (24 * 3600);
                let ago = match days {
                    0 => "today".to_owned(),
                    1 => "yesterday".to_owned(),
                    days => format!("{days} days ago"),
                };
                let _ = tell.send(Update::Crew(Event::Notice {
                    agent: name,
                    text: format!("resumed from {ago}: its task was \"{task}\""),
                }));
            }
            Err(why) => said.push(format!("the chat {name} could not be resumed: {why}")),
        }
    }
    if let (Some(keep), Some(worktrees)) = (settings.chat_keep(), &worktrees) {
        said.extend(worktrees.prune(keep, &chats.places()));
    }
    said
}

/// The person's rules, from the files the settings list, as written and
/// where they are: each with the files it imports, `~/` in `home`, others
/// beside it. A file that is not there, or that its name says holds
/// secrets, is left out; its folder may be one of credentials, as
/// `~/.claude` is, which holds its rules beside them.
fn person_rules(files: &[(String, PathBuf)], home: &Path) -> Vec<iq_agent::RulesFile> {
    files
        .iter()
        .filter(|(_, path)| !iq_gate::holds_secrets(path, None))
        .filter_map(|(written, path)| {
            let text = std::fs::read_to_string(path).ok()?;
            let folder = path.parent().unwrap_or(home);
            let text = iq_agent::with_imports(&text, |name| {
                let imported = match name.strip_prefix("~/") {
                    Some(rest) => home.join(rest),
                    None => folder.join(name),
                };
                if iq_gate::holds_secrets(&imported, None) {
                    return None;
                }
                std::fs::read_to_string(imported).ok()
            });
            Some(iq_agent::RulesFile {
                from: written.clone(),
                text,
            })
        })
        .collect()
}

/// The person's home.
fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

/// This machine's sync, when its settings name a repository.
fn machine_sync() -> Result<Option<Arc<iq_sync::Sync>>> {
    let folder = home_folder()?;
    let home = home_dir()?;
    Ok(Settings::load(&folder)?
        .sync(&folder, &settings::data_folder(&home), &home)
        .map(Arc::new))
}

/// Syncs once; on a machine new to the repository, the files of the home
/// that the settings it took keep are put back too, what it had kept
/// beside.
async fn sync_now(sync: &iq_sync::Sync) -> Result<iq_sync::Outcome, iq_sync::SyncError> {
    put_back_taken(sync.run().await?)
}

/// Takes what the repository holds, on a machine new to it, and sends
/// nothing: what the machine has is asked about after. The files of the
/// home the settings it took keep are put back, as [`sync_now`] does.
async fn take_first(sync: &iq_sync::Sync) -> Result<iq_sync::Outcome, iq_sync::SyncError> {
    put_back_taken(sync.take_first().await?)
}

/// `outcome`, the files of the home put back first when a machine new to
/// the repository took settings that keep some.
fn put_back_taken(outcome: iq_sync::Outcome) -> Result<iq_sync::Outcome, iq_sync::SyncError> {
    if outcome.fresh
        && outcome.taken.iter().any(|f| f.starts_with("config/"))
        && let Some(taken) = machine_sync().ok().flatten()
    {
        taken.put_back()?;
    }
    Ok(outcome)
}

/// How a sync went, as the status line says it.
fn synced(result: &Result<iq_sync::Outcome, iq_sync::SyncError>) -> iq_ui::Synced {
    iq_ui::Synced {
        at: iq_ui::now(),
        ok: result.is_ok(),
        said: match result {
            Ok(outcome) => outcome.said(),
            Err(error) => {
                let mut text = error.to_string();
                let mut source = std::error::Error::source(error);
                while let Some(cause) = source {
                    text.push_str(": ");
                    text.push_str(&cause.to_string());
                    source = cause.source();
                }
                text
            }
        },
    }
}

/// The provider's models, with its key when there is one.
fn provider(cli: &Cli) -> OpenAiCompatible {
    match &cli.api_key {
        Some(key) => OpenAiCompatible::new(cli.endpoint(), key),
        None => OpenAiCompatible::without_key(cli.endpoint()),
    }
}

/// Requesty's router, which serves hundreds of models behind one key.
const ROUTER: &str = "https://router.requesty.ai/v1";

impl Cli {
    /// The provider's address.
    fn endpoint(&self) -> &str {
        self.base_url.as_deref().unwrap_or(ROUTER)
    }
}

/// The models the provider lists, which the person may name; none
/// without a key, or when the list cannot be had.
async fn listed(provider: &OpenAiCompatible, keyed: bool) -> Vec<String> {
    if !keyed {
        return Vec::new();
    }
    match provider.list().await {
        Ok(listed) => listed.into_iter().map(|model| model.id).collect(),
        Err(error) => {
            tracing::warn!("the provider's models: {:#}", anyhow::Error::new(error));
            Vec::new()
        }
    }
}

/// What iq says of each model in use, `in_use` as the setting that names
/// it and the model, that the provider at `endpoint` does not list among
/// `listed`; nothing when the list could not be had.
fn unlisted(endpoint: &str, in_use: &[(&str, String)], listed: &[String]) -> Vec<String> {
    if listed.is_empty() {
        return Vec::new();
    }
    in_use
        .iter()
        .filter(|(_, model)| !listed.contains(model))
        .map(|(key, model)| {
            format!(
                "{endpoint} lists no {model}, which iq uses for `{key}`: set it in \
                 ~/.iq/settings.toml to a model it lists (iq models)"
            )
        })
        .collect()
}

/// The models the settings put to use, each with the setting that names
/// it: the lead's, the reviewer's and the observer's.
fn in_use(settings: &Settings, lead: &ModelId) -> Result<Vec<(&'static str, String)>> {
    let mut models = vec![("model", lead.to_string())];
    if let Some(reviewer) = settings.crew()?.reviewer {
        models.push(("reviewer", reviewer.to_string()));
    }
    models.push(("[observe] model", settings.observing()?.model.to_string()));
    Ok(models)
}

/// What the crew does, kept in iq's folder as it happens: the calls billed
/// to the provider, each task's files, commands and costs. Gives the task
/// that `event` ended, if it ended one.
fn keep(ledger: &Ledger, journal: &Journal, event: &Event) -> Option<iq_observe::Task> {
    let ended = journal.event(event);
    if let Event::Spent {
        agent, model, cost, ..
    } = event
    {
        ledger.bill(agent, model, *cost);
    }
    ended
}

/// The project iq works in, as its memory and its tasks are kept under.
fn project() -> String {
    std::env::current_dir()
        .map(|folder| iq_memory::project(&folder))
        .unwrap_or_default()
}

/// The settings, as the command line changes them, and the lead's model.
fn chosen(cli: &Cli, folder: &std::path::Path) -> Result<(Settings, ModelId)> {
    let mut settings = Settings::load(folder)?;
    settings.autopilot |= cli.autopilot;
    settings.internet |= cli.internet;
    settings.worktree |= cli.worktree;
    let lead = match &cli.model {
        Some(model) => ModelId::new(model.as_str())?,
        None => settings.lead()?,
    };
    Ok((settings, lead))
}

/// Hands `task` to the crew in the terminal, a team working as `pattern`
/// says, until it answers.
async fn work(
    cli: &Cli,
    task: &str,
    pattern: iq_agent::Pattern,
    partner: Option<&str>,
) -> Result<()> {
    let folder = home_folder()?;
    let (settings, lead) = chosen(cli, &folder)?;
    for note in retired(&settings).into_iter().chain(left_over(&settings)) {
        eprintln!("iq: {note}");
    }
    let ledger = Arc::new(Ledger::open(&folder));
    let journal = Arc::new(Journal::open(
        &folder,
        &project(),
        &std::env::current_dir()?,
    ));
    let (send, mut events) = tokio::sync::mpsc::unbounded_channel();
    let kept = Arc::clone(&ledger);
    let noted = Arc::clone(&journal);
    let crew = crew(
        cli,
        &settings,
        Arc::new(move |event: Event| {
            let _ = keep(&kept, &noted, &event);
            let _ = send.send(event);
        }),
    )?;
    let listed = listed(&provider(cli), cli.api_key.is_some()).await;
    for said in unlisted(cli.endpoint(), &in_use(&settings, &lead)?, &listed) {
        eprintln!("iq: {said}");
    }
    let mut models = vec![lead];
    if let Some(partner) = partner {
        models.push(ModelId::new(partner)?);
    }
    crew.launch(pattern, LEAD, &models, task.to_owned(), None, None);
    let mut tally = terminal::Tally::default();
    let handle = |event: Event, tally: &mut terminal::Tally| {
        if let Some((id, question)) = terminal::show(&event, tally) {
            let crew = crew.clone();
            tokio::task::spawn_blocking(move || crew.answer(id, terminal::ask(&question)));
        }
    };
    let mut stopped = std::pin::pin!(stop_asked());
    loop {
        tokio::select! {
            Some(event) = events.recv() => handle(event, &mut tally),
            () = crew.settled() => {
                while let Ok(event) = events.try_recv() {
                    handle(event, &mut tally);
                }
                break;
            }
            () = &mut stopped => {
                println!("\nStopped: the agents are stopped with iq.");
                crew.stop_all();
                break;
            }
        }
    }
    crew.end().await;
    println!("\n{}", tally.summary());
    ledger.trim();
    Ok(())
}

/// Runs iq in the terminal: the crew works on the runtime, the agents'
/// files are read on a thread of their own, and the terminal is drawn on
/// the main thread.
fn session(runtime: &tokio::runtime::Runtime, cli: &Cli) -> Result<()> {
    let folder = home_folder()?;
    // What other machines sent comes first, the settings among it: a new
    // machine takes everything before it sends anything. What this one
    // changed is sent only as the person says, unless the settings send it
    // on their own.
    let sync = machine_sync()?;
    let auto = Arc::new(AtomicBool::new(
        Settings::load(&folder).is_ok_and(|settings| settings.sync_auto()),
    ));
    let first = sync.as_ref().and_then(|sync| {
        if sync.fresh() {
            // New to the repository: it takes what is there, and what it
            // has is asked about once iq is open.
            eprintln!(
                "iq: this machine is new to {}: it takes what is there first…",
                sync.remote()
            );
            let result = runtime
                .block_on(async { tokio::time::timeout(sync::OPENING, take_first(sync)).await })
                .unwrap_or_else(|_| {
                    Err(iq_sync::SyncError::Refused(
                        "it took too long; iq opened without it".into(),
                    ))
                });
            match &result {
                Ok(outcome) => eprintln!("iq: synced, {}", outcome.said()),
                Err(error) => eprintln!("iq: the sync failed, iq opens without it: {error}"),
            }
            return Some(synced(&result));
        }
        let waiting = runtime.block_on(sync.changes());
        if !waiting.is_empty() && !auto.load(Ordering::Relaxed) {
            // Asked once iq is open.
            return None;
        }
        eprintln!("iq: syncing with {}…", sync.remote());
        let result = runtime
            .block_on(async { tokio::time::timeout(sync::OPENING, sync_now(sync)).await })
            .unwrap_or_else(|_| {
                Err(iq_sync::SyncError::Refused(
                    "it took too long; iq opened without it".into(),
                ))
            });
        match &result {
            Ok(outcome) => eprintln!("iq: synced, {}", outcome.said()),
            Err(error) => eprintln!("iq: the sync failed, iq opens without it: {error}"),
        }
        Some(synced(&result))
    });
    let (settings, lead) = chosen(cli, &folder)?;
    // Said in the conversation, under its header: what is printed before
    // the terminal is taken is cleared with it.
    let mut notes = retired(&settings);
    notes.extend(left_over(&settings));
    if cli.api_key.is_none() {
        notes.push("REQUESTY_API_KEY is not set: no model will answer.".to_owned());
    }
    let keyed = cli.api_key.is_some();
    let ledger = Arc::new(Ledger::open(&folder));
    ledger.trim();
    let journal = Arc::new(Journal::open(
        &folder,
        &project(),
        &std::env::current_dir()?,
    ));
    journal.trim();
    let (send, feed) = std::sync::mpsc::channel();
    let quit = send.clone();
    let (ask, requests) = std::sync::mpsc::channel::<Request>();
    let _ = send.send(Update::Started {
        lead: lead.to_string(),
        reviewer: settings
            .crew()?
            .reviewer
            .map(|r| r.to_string())
            .unwrap_or_default(),
        autopilot: settings.autopilot,
        internet: settings.internet,
    });
    for note in notes {
        let _ = send.send(Update::Said(note));
    }
    let _ = send.send(Update::Kept(
        ledger.billed(iq_ui::now().saturating_sub(ledger::BILLED_KEEP_SECS)),
    ));
    if let Some(first) = first {
        let _ = send.send(Update::Synced(first));
    }
    // While iq runs, what changed is asked of the person, or sent and said
    // when the settings say so, and what other machines sent is taken now
    // and then.
    if let Some(sync) = sync.clone() {
        let send = send.clone();
        let auto = Arc::clone(&auto);
        runtime.spawn(async move {
            let mut last = std::time::Instant::now();
            let mut asked: Vec<String> = Vec::new();
            loop {
                let changes = sync.changes().await;
                let update = if sync.fresh() {
                    // Its first sync failed as iq opened: it takes first
                    // again, now and then, and sends nothing.
                    if last.elapsed() < sync::TAKE_EVERY {
                        None
                    } else {
                        last = std::time::Instant::now();
                        Some(Update::Synced(synced(&take_first(&sync).await)))
                    }
                } else if changes.is_empty() {
                    asked.clear();
                    if last.elapsed() < sync::TAKE_EVERY {
                        None
                    } else {
                        last = std::time::Instant::now();
                        Some(Update::Synced(synced(&sync.run().await)))
                    }
                } else if auto.load(Ordering::Relaxed) {
                    last = std::time::Instant::now();
                    Some(Update::Synced(synced(&sync.run().await)))
                } else if changes != asked {
                    // Asked once for what changed: what the person put off
                    // is asked again only as more changes.
                    asked = changes.clone();
                    Some(Update::SyncAsk(iq_ui::SyncAsk {
                        remote: sync.remote().to_owned(),
                        said: iq_sync::named(&changes),
                        files: changes,
                    }))
                } else {
                    None
                };
                if let Some(update) = update
                    && send.send(update).is_err()
                {
                    return;
                }
                tokio::time::sleep(sync::LOOK_EVERY).await;
            }
        });
    }

    // As each task ends, the person's rules place it, or a cheap model in
    // the background; what that model spends is counted as any call.
    let observer = {
        let costs: Report = {
            let send = send.clone();
            let ledger = Arc::clone(&ledger);
            Arc::new(move |event: Event| {
                if let Event::Spent {
                    agent, model, cost, ..
                } = &event
                {
                    ledger.bill(agent, model, *cost);
                }
                let _ = send.send(Update::Crew(event));
            })
        };
        iq_observe::Observer::new(
            provider(cli),
            settings.observing()?,
            journal.file().to_path_buf(),
            iq_observe::Labels::at(folder.join(LABELS)),
            costs,
        )
    };
    {
        // What iq left unplaced while it was away, or what the categories
        // no longer fit, a week back.
        let _inside = runtime.enter();
        observer.catch_up(iq_ui::now().saturating_sub(7 * 24 * 3600));
    }
    let report: Report = {
        let send = send.clone();
        let ledger = Arc::clone(&ledger);
        let journal = Arc::clone(&journal);
        let observer = observer.clone();
        Arc::new(move |event: Event| {
            if let Some(task) = keep(&ledger, &journal, &event) {
                observer.done(task);
            }
            let _ = send.send(Update::Crew(event));
        })
    };
    let crew = {
        let _inside = runtime.enter();
        crew(cli, &settings, report)?
    };
    // The chats of earlier sessions come back before the person writes.
    for note in resume_chats(&settings, &crew, &send) {
        let _ = send.send(Update::Said(note));
    }
    {
        let provider = provider(cli);
        let send = send.clone();
        let endpoint = cli.endpoint().to_owned();
        let in_use = in_use(&settings, &lead)?;
        runtime.spawn(async move {
            // The models the person may name in what they write; and those
            // in use the provider does not have, said before a call fails.
            let models = listed(&provider, keyed).await;
            for said in unlisted(&endpoint, &in_use, &models) {
                let _ = send.send(Update::Said(said));
            }
            let _ = send.send(Update::Models(models));
        });
    }

    let told = send.clone();
    let files = {
        let journal = journal.file().to_path_buf();
        let categories = settings.categories();
        let labels = iq_observe::Labels::at(folder.join(LABELS));
        let mut seen = None;
        move || {
            loop {
                // Read again only once the journal or the labels changed.
                let changed = [&journal, labels.file()]
                    .iter()
                    .map(|file| std::fs::metadata(file).and_then(|m| m.modified()).ok())
                    .collect::<Vec<_>>();
                if seen.as_ref() != Some(&changed) {
                    seen = Some(changed);
                    let tasks = tasks(&journal, &categories, &labels);
                    if send.send(Update::Tasks(tasks)).is_err() {
                        return;
                    }
                }
                std::thread::sleep(READ_EVERY);
            }
        }
    };
    std::thread::spawn(files);

    // Asked to stop from outside, iq ends as if the person had quit, so
    // that the agents are stopped with it.
    {
        runtime.spawn(async move {
            stop_asked().await;
            let _ = quit.send(Update::Quit);
        });
    }
    let handle = runtime.handle().clone();
    let working = crew.clone();
    let syncing = sync.clone();
    let always = Arc::clone(&auto);
    std::thread::spawn(move || {
        let _inside = handle.enter();
        for request in requests {
            match request {
                Request::Start {
                    name,
                    model,
                    task,
                    role,
                    pattern,
                    partner,
                    named,
                    effort,
                } => match ModelId::new(&model) {
                    Ok(model) => {
                        let mut models = vec![model];
                        models.extend(partner.and_then(|p| ModelId::new(&p).ok()));
                        let task = with_named(task, &named[named.len().min(1)..]);
                        working.launch(pattern, &name, &models, task, role, effort);
                    }
                    Err(error) => tracing::warn!("no chat on `{model}`: {error}"),
                },
                Request::Tell { to, text, named } => {
                    let _ = working.tell(&to, with_named(text, &named));
                }
                Request::Answer { id, approved } => working.answer(id, approved),
                Request::Autopilot(on) => working.set_autopilot(on),
                Request::Internet(open) => working.set_internet(open),
                Request::Effort { agent, effort } => {
                    let _ = working.set_effort(&agent, effort);
                }
                Request::Stop { agent } => working.stop(&agent),
                Request::Sync { send, always: on } => {
                    if on {
                        always.store(true, Ordering::Relaxed);
                    }
                    if !send {
                        continue;
                    }
                    let told = told.clone();
                    match syncing.clone() {
                        Some(sync) => {
                            tokio::spawn(async move {
                                let result = sync.run().await;
                                let _ = told.send(Update::Synced(synced(&result)));
                            });
                        }
                        None => {
                            let _ = told.send(Update::Synced(iq_ui::Synced {
                                at: iq_ui::now(),
                                ok: false,
                                said: "no repository is set: [sync] remote in ~/.iq/settings.toml"
                                    .into(),
                            }));
                        }
                    }
                }
            }
        }
    });

    let board = iq_ui::Board::new(settings.keymap()?, settings.look()?);
    let ran = iq_tui::run(feed, ask, board).context("running in the terminal");
    runtime.block_on(async {
        crew.end().await;
        // What changed since the last sync leaves with iq, as the person
        // says.
        let changes = match &sync {
            Some(sync) => sync.changes().await,
            None => Vec::new(),
        };
        if let Some(sync) = &sync
            && !changes.is_empty()
            && (auto.load(Ordering::Relaxed) || send_changes(sync.remote(), &changes))
        {
            match tokio::time::timeout(sync::CLOSING, sync.run()).await {
                Ok(Ok(outcome)) => eprintln!("iq: synced, {}", outcome.said()),
                Ok(Err(error)) => eprintln!("iq: the last sync failed: {error}"),
                Err(_) => eprintln!("iq: the last sync took too long and was left"),
            }
        }
    });
    ran
}

/// `text`, the models the person named in it told to the agent: "with
/// opus" is the person's choice, and what they name is what works.
fn with_named(text: String, named: &[String]) -> String {
    let models: Vec<ModelId> = named.iter().filter_map(|m| ModelId::new(m).ok()).collect();
    if models.is_empty() {
        return text;
    }
    let list = models
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{text}\n\n(iq: the person names {list} for this. Do as they say with them: start an agent \
         on each, with start_agent, for the part they name it for.)"
    )
}

/// Resolves once the person or the system asks iq to stop: Ctrl-C in its
/// terminal, the terminal closed, or a termination asked. iq then stops
/// its agents before it ends: killed outright, it would leave them working
/// on, spending on the provider.
async fn stop_asked() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut interrupt), Ok(mut terminate), Ok(mut hangup)) = (
            signal(SignalKind::interrupt()),
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) else {
            // Without them, iq stops as the system kills it.
            return std::future::pending().await;
        };
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
            _ = hangup.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Where the categories models gave tasks are kept, beside the journal.
const LABELS: &str = "labels.jsonl";

/// iq's tasks, all the journal keeps, each in its category: by the rules
/// of `categories`, else as a model placed it.
fn tasks(
    journal: &Path,
    categories: &iq_observe::Categories,
    labels: &iq_observe::Labels,
) -> Vec<iq_ui::costs::TaskSpent> {
    let placed = labels.load();
    iq_observe::read(journal, 0)
        .iter()
        .map(|task| iq_ui::costs::TaskSpent {
            at: task.at,
            kind: iq_observe::category_of(task, categories, &placed),
            billed: iq_core::Usd(task.billed),
            tokens: task.tokens,
            model: task.model.clone(),
            agent: task.agent.clone(),
            project: task.project.clone(),
            seconds: task.seconds,
            ok: task.ok,
            prompt: task.prompt.chars().take(160).collect(),
        })
        .collect()
}

/// iq's own folder, readable by its owner only: the audit log, the
/// person's rules and settings, and what iq keeps of its sessions, where
/// no sandboxed command reaches.
fn home_folder() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let folder = PathBuf::from(home).join(".iq");
    std::fs::create_dir_all(&folder).with_context(|| format!("making {}", folder.display()))?;
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("shutting {}", folder.display()))?;
    Ok(folder)
}

/// The gate of the current folder, with the person's rules when they wrote
/// some, the internet open to every command or not.
fn gate(internet: bool, reviewer_internet: bool, reviewer_host: bool) -> Result<Gate> {
    gate_in(
        &std::env::current_dir().context("reading the current folder")?,
        internet,
        reviewer_internet,
        reviewer_host,
    )
}

/// The gate of `workspace`, as [`gate`]'s. Rules are never read from the
/// workspace: a command could write them there.
fn gate_in(
    workspace: &Path,
    internet: bool,
    reviewer_internet: bool,
    reviewer_host: bool,
) -> Result<Gate> {
    let folder = home_folder()?;
    let mut gate = Gate::builder(workspace)
        .audit(folder.join("audit.log"))
        .internet(internet)
        .reviewer_internet(reviewer_internet)
        .reviewer_host(reviewer_host);
    let rules = folder.join("rules.toml");
    if rules.exists() {
        let text = std::fs::read_to_string(&rules)
            .with_context(|| format!("reading {}", rules.display()))?;
        gate = gate.rules(Rules::parse(&text).with_context(|| format!("in {}", rules.display()))?);
    }
    Ok(gate.build()?)
}

/// What iq says at start of what it left behind that the person may want
/// gone, and of where the rules the lead follows come from when they are
/// those of `~/.claude`.
fn left_over(settings: &Settings) -> Vec<String> {
    let Ok(home) = home_dir() else {
        return Vec::new();
    };
    let mut said = Vec::new();
    let assay = settings::data_folder(&home).join("assay");
    if assay.is_dir() {
        said.push(format!(
            "{} is the store of the assay iq no longer has: it is synced no more, and the next \
             sync takes it out of the repository, whose history keeps it. Move it out of iq's \
             folder, or remove it.",
            assay.display()
        ));
    }
    if settings.claude_rules_by_default(&home) {
        said.push(
            "the lead follows ~/.claude/CLAUDE.md, which iq does not sync: \
             rules of iq's own go in ~/.iq/AGENTS.md, read instead once written."
                .to_owned(),
        );
    }
    said
}

/// What iq says at start of what the settings hold that it reads no more,
/// for the person to change it.
fn retired(settings: &Settings) -> Vec<String> {
    let mut said = Vec::new();
    let retired = settings.retired();
    if !retired.is_empty() {
        said.push(format!(
            "~/.iq/settings.toml holds {}, which iq reads no more: it may go.",
            retired.join(", ")
        ));
    }
    said
}

/// Asks the person on the terminal whether what changed, `files`, goes to
/// `remote` as iq ends; no when they cannot answer.
fn send_changes(remote: &str, files: &[String]) -> bool {
    print!(
        "iq: changed since the last sync: {}. Send to {remote}? [y/N] ",
        iq_sync::named(files)
    );
    if std::io::stdout().flush().is_err() {
        return false;
    }
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer).is_ok()
        && matches!(answer.trim(), "y" | "Y" | "yes")
}

/// Asks the person on the terminal whether `reason` may be overridden.
fn ask(reason: &str, line: &str) -> Result<bool> {
    print!("{line}\nThis is asked because {reason}. Run it? [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

async fn run(command: &Command, cli: &Cli) -> Result<()> {
    let (base_url, api_key) = (cli.endpoint(), cli.api_key.clone());
    match command {
        Command::Do { .. } => bail!("a task is handed to the crew"),
        Command::Models => {
            let provider = match api_key {
                Some(key) => OpenAiCompatible::new(base_url, key),
                None => OpenAiCompatible::without_key(base_url),
            };
            let models = provider
                .list()
                .await
                .with_context(|| format!("listing the models of {base_url}"))?;
            for model in models {
                let per_million = |price: Option<f64>| {
                    price.map_or("?".to_owned(), |p| format!("{:.2}", p * 1e6))
                };
                println!(
                    "{:<50} in ${:>7}  out ${:>7}",
                    model.id,
                    per_million(model.input_price),
                    per_million(model.output_price)
                );
            }
        }
        Command::Sync => {
            let Some(sync) = machine_sync()? else {
                bail!(
                    "the settings name no repository to sync with: make a private one \
                     (gh repo create <name> --private) and give it in ~/.iq/settings.toml, \
                     [sync] remote = \"<its address>\""
                );
            };
            for why in sync.refused() {
                println!("Not kept: {why}.");
            }
            if sync.fresh() {
                println!(
                    "This machine is new to {}: it takes what is there first…",
                    sync.remote()
                );
                let outcome = take_first(&sync).await?;
                println!("Taken: {}.", outcome.said());
            }
            let changes = sync.changes().await;
            let auto = Settings::load(&home_folder()?).is_ok_and(|s| s.sync_auto());
            if !changes.is_empty() && !auto && !send_changes(sync.remote(), &changes) {
                println!("Nothing sent.");
                return Ok(());
            }
            println!("Syncing {} with {}…", sync.root().display(), sync.remote());
            let outcome = sync_now(&sync).await?;
            println!("Synced: {}.", outcome.said());
            if !outcome.taken.is_empty() {
                println!("  taken: {}", iq_sync::named(&outcome.taken));
            }
            let kept = sync.kept();
            if !kept.is_empty() {
                println!("Kept from the home: {}", kept.join(", "));
            }
        }
        Command::Run {
            line,
            network,
            host,
        } => {
            let gate = gate(cli.internet, false, false)?;
            let action = Action::Run {
                line: line.clone(),
                folder: gate.workspace().to_path_buf(),
                network: *network,
                host: *host,
            };
            let permit = match gate.judge(action)? {
                Judged::Allowed(permit) => permit,
                Judged::Asked { reason, pending } => {
                    if !ask(&reason, line)? {
                        bail!("not run");
                    }
                    gate.approve(pending, Approver::Person)?
                }
                Judged::Denied { reason } => bail!("refused: {reason}"),
            };
            let sandbox = gate.sandbox(Helper::this_program()?);
            let ran = sandbox.run(permit).await?;
            sandbox.end().await;
            print!("{}", ran.output);
            for path in &ran.restored {
                eprintln!("put back as it was: {}", path.display());
            }
            for path in &ran.touched {
                eprintln!("changed where code runs later: {}", path.display());
            }
            if ran.status != Some(0) {
                std::process::exit(ran.status.unwrap_or(1));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_in_use_the_provider_does_not_list_is_named_with_its_setting() {
        let in_use = [
            ("model", "lab/lead".to_owned()),
            ("reviewer", "lab/gone".to_owned()),
        ];
        let listed = ["lab/lead".to_owned(), "lab/other".to_owned()];
        let said = unlisted("https://router.example/v1", &in_use, &listed);
        assert_eq!(said.len(), 1);
        assert!(said[0].starts_with("https://router.example/v1 lists no lab/gone"));
        assert!(said[0].contains("`reviewer`"));
        // No list, nothing said: the provider may simply not answer.
        assert!(unlisted("x", &in_use, &[]).is_empty());
    }
}
