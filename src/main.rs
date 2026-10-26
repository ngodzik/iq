//! The `iq` command.

#![deny(unsafe_code)]

mod journal;
mod ledger;
mod settings;
mod sync;
mod terminal;

use std::io::{BufRead, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use iq_agent::{Crew, Event, Report};
use iq_assay::{Assay, Told};
use iq_core::ModelId;
use iq_gate::{Action, Approver, Gate, HELPER_FLAG, HOLD_FLAG, Helper, Judged, Rules};
use iq_llm::OpenAiCompatible;
use iq_ui::Update;
use iq_ui::crew::{AssayAsk, Request};
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
    /// The assay's exercises and their results, as its folder holds them;
    /// or, named after it, an exercise frozen or run.
    Assay {
        #[command(subcommand)]
        act: Option<AssayAct>,
    },
    /// Syncs iq's folder of data now with the repository the settings'
    /// [sync] names: what changed here sent, what changed elsewhere taken;
    /// a new machine takes everything first. iq does it on its own as it runs.
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
    },
}

/// What the assay is asked from the command line.
#[derive(Subcommand)]
enum AssayAct {
    /// Freezes an exercise as it is: from then on every candidate is given
    /// exactly the same. Asks first.
    Freeze {
        /// The exercise.
        exercise: String,
    },
    /// Runs frozen exercises on candidates, each in a folder of its own,
    /// checked, then judged blind: one exercise, every exercise of a kind
    /// of task, or `all`. Says what it should cost, and asks first.
    Run {
        /// An exercise, a kind of task, or `all`.
        which: String,
        /// A model (`z-ai/glm-5`), or a team (`pair: writer + reviewer`).
        #[arg(required = true)]
        candidates: Vec<String>,
        /// Runs of each candidate, 3 at most.
        #[arg(long, default_value_t = 1)]
        times: u64,
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
    let logging = tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env());
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
    let crew = Crew::new(
        provider(cli),
        gate(settings.internet, settings.autopilot_internet())?,
        Helper::this_program()?,
        settings.crew()?,
        report,
    );
    crew.set_autopilot(settings.autopilot);
    // What the agents remember of this project, kept with iq's data.
    let memory = iq_memory::Memory::open(
        &settings::data_folder(&home_dir()?).join("memory"),
        &std::env::current_dir().context("reading the current folder")?,
    );
    crew.extend(Arc::new(memory));
    Ok(crew)
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
    let outcome = sync.run().await?;
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

/// The assay of the settings' folder, and the crew its judge works in,
/// in that folder: its runs' gates hold the person's rules, its costs go
/// to `report` with the rest, what changes in it is told on `told`. Must
/// run within the runtime.
#[allow(clippy::too_many_arguments)]
fn assay(
    cli: &Cli,
    settings: &Settings,
    lead: &ModelId,
    report: Report,
    told: std::sync::mpsc::Sender<Told>,
    journal: &Path,
) -> Result<(Assay<OpenAiCompatible>, Crew<OpenAiCompatible>)> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")?;
    let store = iq_assay::Store::open(settings.assay_folder(&home))
        .context("opening the assay's folder")?;
    let provider = provider(cli);
    let crew_settings = settings.crew()?;
    let runner = iq_assay::Runner {
        provider: provider.clone(),
        provider_name: Some(provider.name()),
        helper: Helper::this_program()?,
        gate: Arc::new(|work: &Path| gate_in(work, false, false).map_err(|e| format!("{e:#}"))),
        effort: crew_settings.effort,
    };
    let assay = Assay::new(
        store,
        runner,
        iq_assay::Settings {
            grader: settings.assay_grader(lead)?,
            allowed: settings.assay_allowed(),
            reference: lead.clone(),
            parallel: settings.assay_parallel(),
            journal: Some(journal.to_path_buf()),
            labels: Some(journal.with_file_name(LABELS)),
            categories: settings.categories(),
            notices: Vec::new(),
        },
        Arc::clone(&report),
        Arc::new(move |news| {
            let _ = told.send(news);
        }),
    );
    let crew = Crew::new(
        provider,
        gate_in(
            assay.store().root(),
            settings.internet,
            settings.autopilot_internet(),
        )?,
        Helper::this_program()?,
        crew_settings,
        report,
    );
    crew.set_autopilot(settings.autopilot);
    assay.serve(iq_assay::CHAT);
    crew.extend(Arc::new(assay.clone()));
    Ok((assay, crew))
}

/// Who the assay may test: the provider's models as it lists them, with
/// their prices.
async fn roster(provider: &OpenAiCompatible, keyed: bool) -> iq_assay::Roster {
    let mut roster = iq_assay::Roster {
        provider: Some(provider.name()),
        ..iq_assay::Roster::default()
    };
    if !keyed {
        roster.missing =
            Some("there is no key for the provider: iq reads it from REQUESTY_API_KEY".into());
        return roster;
    }
    match provider.list().await {
        Ok(listed) => {
            roster.models = listed
                .into_iter()
                .map(|model| iq_assay::Offer {
                    id: model.id,
                    canonical: model.canonical,
                    input: model.input_price,
                    output: model.output_price,
                    context: model.context_window,
                    tools: model.tool_calling,
                    description: model.description,
                })
                .collect();
        }
        Err(error) => roster.missing = Some(format!("{:#}", anyhow::Error::new(error))),
    }
    roster
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
    retired(&settings);
    let ledger = Arc::new(Ledger::open(&folder));
    let journal = Arc::new(Journal::open(&folder, &project()));
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
    let mut models = vec![lead];
    if let Some(partner) = partner {
        models.push(ModelId::new(partner)?);
    }
    crew.launch(pattern, LEAD, &models, task.to_owned(), None);
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
    // machine takes everything before it sends anything.
    let sync = machine_sync()?;
    let first = sync.as_ref().map(|sync| {
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
        synced(&result)
    });
    let (settings, lead) = chosen(cli, &folder)?;
    retired(&settings);
    if cli.api_key.is_none() {
        eprintln!("iq: REQUESTY_API_KEY is not set: no model will answer.");
    }
    let keyed = cli.api_key.is_some();
    let ledger = Arc::new(Ledger::open(&folder));
    ledger.trim();
    let journal = Arc::new(Journal::open(&folder, &project()));
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
    let _ = send.send(Update::Kept(
        ledger.billed(iq_ui::now().saturating_sub(ledger::BILLED_KEEP_SECS)),
    ));
    if let Some(first) = first {
        let _ = send.send(Update::Synced(first));
    }
    // While iq runs, what changed is sent every few minutes, and what
    // other machines sent is taken now and then.
    if let Some(sync) = sync.clone() {
        let send = send.clone();
        runtime.spawn(async move {
            let mut last = std::time::Instant::now();
            loop {
                tokio::time::sleep(sync::LOOK_EVERY).await;
                if !sync.pending().await && last.elapsed() < sync::TAKE_EVERY {
                    continue;
                }
                let result = sync.run().await;
                last = std::time::Instant::now();
                if send.send(Update::Synced(synced(&result))).is_err() {
                    return;
                }
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
        crew(cli, &settings, Arc::clone(&report))?
    };
    let (told, heard) = std::sync::mpsc::channel::<Told>();
    let (assay, assay_crew) = {
        let _inside = runtime.enter();
        assay(cli, &settings, &lead, report, told, journal.file())?
    };
    // The routing, as its file says it: made again only as the assay's
    // records change, never as iq starts, but for the first time.
    let routing = Arc::new(std::sync::Mutex::new(
        iq_assay::Routing::load(assay.store().root()).unwrap_or_else(|| assay.refresh_routing()),
    ));
    let book = {
        let categories = settings.categories();
        let choices = settings.route_choices();
        move |routing: &iq_assay::Routing| iq_ui::routes::RouteBook {
            categories: categories.clone(),
            routing: routing.clone(),
            choices: choices.clone(),
        }
    };
    if let Ok(now) = routing.lock() {
        let _ = send.send(Update::Routes(Box::new(book(&now))));
    }
    // Agents are told the routing each turn, for the lead to decide whom
    // it starts: iq holds no agent to it.
    crew.extend(Arc::new(iq_assay::Card {
        routing: Arc::clone(&routing),
        choices: settings.route_choices(),
    }));
    // What the assay's folder holds is read on a thread of its own, never
    // where the terminal is drawn, at once and whenever it changes, the
    // routing made again with it.
    {
        let send = send.clone();
        let assay = assay.clone();
        let routing = Arc::clone(&routing);
        std::thread::spawn(move || {
            let _ = send.send(Update::Assay(Box::new(assay.overview())));
            for news in heard {
                let update = match news {
                    Told::Store => {
                        let made = assay.refresh_routing();
                        let _ = send.send(Update::Routes(Box::new(book(&made))));
                        if let Ok(mut now) = routing.lock() {
                            *now = made;
                        }
                        Update::Assay(Box::new(assay.overview()))
                    }
                    Told::Running(running) => Update::AssayRunning(running),
                };
                if send.send(update).is_err() {
                    return;
                }
            }
        });
    }
    {
        let assay = assay.clone();
        let provider = provider(cli);
        let send = send.clone();
        runtime.spawn(async move {
            let roster = roster(&provider, keyed).await;
            // The models the person may name in what they write.
            let _ = send.send(Update::Models(
                roster.models.iter().map(|offer| offer.id.clone()).collect(),
            ));
            assay.set_roster(roster);
        });
    }

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
    let judging = assay_crew.clone();
    let assaying = assay.clone();
    std::thread::spawn(move || {
        let _inside = handle.enter();
        // The assay's chat works in a crew of its own, in the assay's
        // folder: what is asked of an agent goes to the crew it is in.
        let judge_has = |name: &str| judging.list().iter().any(|a| a.name == name);
        for request in requests {
            match request {
                Request::Start {
                    name,
                    model,
                    task,
                    role,
                    pattern,
                    partner,
                    assay,
                    named,
                    ..
                } => match ModelId::new(&model) {
                    Ok(model) => {
                        let mut models = vec![model];
                        models.extend(partner.and_then(|p| ModelId::new(&p).ok()));
                        // The assay's chat works in the assay's folder, in
                        // a crew of its own, with the assay's tools.
                        let crew = if assay { &judging } else { &working };
                        let task = with_named(task, &named[named.len().min(1)..], assay);
                        crew.launch(pattern, &name, &models, task, role);
                    }
                    Err(error) => tracing::warn!("no chat on `{model}`: {error}"),
                },
                Request::Tell { to, text, named } => {
                    let assay = judge_has(&to);
                    let crew = if assay { &judging } else { &working };
                    let _ = crew.tell(&to, with_named(text, &named, assay));
                }
                // Each crew numbers its questions apart: the one that
                // asked takes the answer.
                Request::Answer { id, approved } => {
                    working.answer(id, approved);
                    judging.answer(id, approved);
                }
                Request::Autopilot(on) => {
                    working.set_autopilot(on);
                    judging.set_autopilot(on);
                }
                Request::Internet(open) => {
                    working.set_internet(open);
                    judging.set_internet(open);
                }
                Request::Stop { agent } => {
                    if judge_has(&agent) {
                        judging.stop(&agent);
                    } else {
                        working.stop(&agent);
                    }
                }
                Request::Assay(AssayAsk::Stop) => assaying.stop(),
            }
        }
    });

    let board = iq_ui::Board::new(settings.keymap()?, settings.look()?);
    let ran = iq_tui::run(feed, ask, board).context("running in the terminal");
    runtime.block_on(async {
        // The runs stop, and end their crews, before iq does.
        assay.stop();
        for _ in 0..100 {
            if assay.running().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assay_crew.end().await;
        crew.end().await;
        // What changed since the last sync leaves with iq.
        if let Some(sync) = &sync
            && sync.pending().await
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
/// opus" is the person's choice, and what they name is what works. In the
/// assay's chat, a model named is one to test: only its whole name is
/// told.
fn with_named(text: String, named: &[String], assay: bool) -> String {
    let models: Vec<ModelId> = named.iter().filter_map(|m| ModelId::new(m).ok()).collect();
    if models.is_empty() {
        return text;
    }
    let list = models
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    if assay {
        return format!("{text}\n\n(iq: the models named are, whole: {list}.)");
    }
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
fn gate(internet: bool, reviewer_internet: bool) -> Result<Gate> {
    gate_in(
        &std::env::current_dir().context("reading the current folder")?,
        internet,
        reviewer_internet,
    )
}

/// The gate of `workspace`, as [`gate`]'s. Rules are never read from the
/// workspace: a command could write them there.
fn gate_in(workspace: &Path, internet: bool, reviewer_internet: bool) -> Result<Gate> {
    let folder = home_folder()?;
    let mut gate = Gate::builder(workspace)
        .audit(folder.join("audit.log"))
        .internet(internet)
        .reviewer_internet(reviewer_internet);
    let rules = folder.join("rules.toml");
    if rules.exists() {
        let text = std::fs::read_to_string(&rules)
            .with_context(|| format!("reading {}", rules.display()))?;
        gate = gate.rules(Rules::parse(&text).with_context(|| format!("in {}", rules.display()))?);
    }
    Ok(gate.build()?)
}

/// Freezes or runs exercises of the assay from the command line, each
/// action told and asked first, what it spends kept as iq keeps any call.
async fn assay_act(cli: &Cli, act: &AssayAct) -> Result<()> {
    let folder = home_folder()?;
    let (settings, lead) = chosen(cli, &folder)?;
    let ledger = Arc::new(Ledger::open(&folder));
    let journal = Arc::new(Journal::open(&folder, &project()));
    let report: Report = {
        let ledger = Arc::clone(&ledger);
        let journal = Arc::clone(&journal);
        Arc::new(move |event: Event| {
            let _ = keep(&ledger, &journal, &event);
        })
    };
    let (told, heard) = std::sync::mpsc::channel::<Told>();
    let (assay, crew) = assay(cli, &settings, &lead, report, told, journal.file())?;
    // Each run's step, said as it changes.
    std::thread::spawn(move || {
        let mut said: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for news in heard {
            let Told::Running(running) = news else {
                continue;
            };
            for progress in running {
                let phase = format!("{:?}", progress.phase).to_lowercase();
                let key = format!("{}/{}", progress.batch, progress.label);
                if said.get(&key) != Some(&phase) {
                    println!(
                        "  {} {} on {}: {phase}",
                        progress.label, progress.candidate, progress.exercise
                    );
                    said.insert(key, phase);
                }
            }
        }
    });
    match act {
        AssayAct::Freeze { exercise } => {
            match assay
                .plan_freeze(exercise)
                .await
                .map_err(anyhow::Error::msg)?
            {
                iq_assay::Planned::Done(said) => println!("{said}"),
                iq_assay::Planned::Ready(plan) => {
                    if agree(&plan.action, &plan.reason)? {
                        println!(
                            "{}",
                            assay.freeze_now(exercise).map_err(anyhow::Error::msg)?
                        );
                    } else {
                        println!("Not frozen.");
                    }
                }
            }
        }
        AssayAct::Run {
            which,
            candidates,
            times,
        } => {
            assay.set_roster(roster(&provider(cli), cli.api_key.is_some()).await);
            let exercises = assay_exercises(assay.store(), which)?;
            let named: Vec<&str> = candidates.iter().map(String::as_str).collect();
            let plans = exercises
                .iter()
                .map(|id| assay.plan_run(id, &named, *times))
                .collect::<Result<Vec<_>, String>>()
                .map_err(anyhow::Error::msg)?;
            for plan in &plans {
                println!("To {}: {}.", plan.plan.action, plan.plan.reason);
            }
            if !agree("run them all", "they spend what each says")? {
                println!("Nothing ran.");
            } else {
                for plan in plans {
                    println!("\n{}", assay.run_now(plan).await);
                }
            }
        }
    }
    crew.end().await;
    ledger.trim();
    Ok(())
}

/// The exercises `which` names: one, every frozen one of a kind of task,
/// or every frozen one.
fn assay_exercises(store: &iq_assay::Store, which: &str) -> Result<Vec<String>> {
    let exercises = store.exercises()?;
    if exercises.iter().any(|e| e.id == which) {
        return Ok(vec![which.to_owned()]);
    }
    let frozen = |e: &&iq_assay::Exercise| matches!(e.state, iq_assay::State::Frozen(_));
    let chosen: Vec<String> = exercises
        .iter()
        .filter(frozen)
        .filter(|e| which == "all" || e.categories.iter().any(|c| c.eq_ignore_ascii_case(which)))
        .map(|e| e.id.clone())
        .collect();
    if chosen.is_empty() {
        let names: Vec<&str> = exercises.iter().map(|e| e.id.as_str()).collect();
        bail!(
            "`{which}` names no exercise, and no frozen exercise is of that kind of task; the exercises are {}",
            if names.is_empty() {
                "none yet".to_owned()
            } else {
                names.join(", ")
            }
        );
    }
    Ok(chosen)
}

/// Says at start what the settings hold that iq reads no more, and the
/// models they name that it runs no more, for the person to change them.
fn retired(settings: &Settings) {
    let retired = settings.retired();
    if !retired.is_empty() {
        eprintln!(
            "iq: ~/.iq/settings.toml holds {}, which iq reads no more: it may go.",
            retired.join(", ")
        );
    }
    for (key, model) in settings.agents_named() {
        eprintln!(
            "iq: ~/.iq/settings.toml names {model} for {key}, which was Claude Code's or Codex's: \
             iq runs the provider's models only, such as anthropic/claude-opus-5-5."
        );
    }
}

/// Asks the person on the terminal whether `action` may be done.
fn agree(action: &str, reason: &str) -> Result<bool> {
    let mut chars = action.chars();
    let action: String = chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    print!("{action}? It is asked because {reason}. [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
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
            println!("Syncing {} with {}…", sync.root().display(), sync.remote());
            let outcome = sync_now(&sync).await?;
            if outcome.fresh && !outcome.taken.is_empty() {
                println!("This machine was new to it: it took what was there first.");
            }
            println!("Synced: {}.", outcome.said());
            for file in &outcome.taken {
                println!("  taken: {file}");
            }
            for file in &outcome.sent {
                println!("  sent: {file}");
            }
            let kept = sync.kept();
            if !kept.is_empty() {
                println!("Kept from the home: {}", kept.join(", "));
            }
        }
        Command::Assay { act: Some(act) } => assay_act(cli, act).await?,
        Command::Assay { act: None } => {
            let folder = home_folder()?;
            let settings = Settings::load(&folder)?;
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .context("HOME is not set")?;
            let store = iq_assay::Store::open(settings.assay_folder(&home))?;
            println!("The assay's folder: {}", store.root().display());
            let exercises = store.exercises()?;
            if exercises.is_empty() {
                println!("No exercise yet: write one with an agent, in iq's assay (Ctrl-L).");
            }
            for exercise in &exercises {
                let state = match &exercise.state {
                    iq_assay::State::Draft => "draft".to_owned(),
                    iq_assay::State::Frozen(v) => format!("frozen, version {v}"),
                    iq_assay::State::Changed(v) => format!("changed since version {v}"),
                    iq_assay::State::Broken(why) => format!("broken: {why}"),
                };
                println!("\n{} ({}): {state}", exercise.id, exercise.title);
                let records = store.records(&exercise.id);
                if !records.is_empty() {
                    let table = iq_assay::table(exercise, &records);
                    print!("{}", iq_assay::report::text(&table.rows));
                }
            }
        }
        Command::Run { line, network } => {
            let gate = gate(cli.internet, false)?;
            let action = Action::Run {
                line: line.clone(),
                folder: gate.workspace().to_path_buf(),
                network: *network,
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
