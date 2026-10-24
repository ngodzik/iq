//! The `iq` command.

#![deny(unsafe_code)]

mod journal;
mod ledger;
mod settings;
mod terminal;

use std::io::{BufRead, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use iq_agent::{Crew, EXEC_FLAG, Event, HOOK_FLAG, MCP_FLAG, PERSON, Report};
use iq_assay::{Assay, Told};
use iq_core::{Agent, Delegate, ModelId};
use iq_gate::{Action, Approver, Gate, HELPER_FLAG, HOLD_FLAG, Helper, Judged, Rules};
use iq_llm::{Agents, OpenAiCompatible, PriceTable, Tokens};
use iq_ui::Update;
use iq_ui::costs::{Payer, Spent};
use iq_ui::crew::{AssayAsk, Request};
use tracing_subscriber::EnvFilter;

use crate::journal::Journal;
use crate::ledger::Ledger;
use crate::settings::Settings;

/// How often the window reads the agents' files again.
const READ_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

/// The lead's name on the board.
const LEAD: &str = "lead";

/// Without a command, iq opens its window: the agents at work, the costs
/// of every call to a model, and what is left of the subscriptions.
#[derive(Parser)]
#[command(version, about = "An experimental coding agent")]
struct Cli {
    /// Base URL of an OpenAI compatible endpoint: Requesty's router by
    /// default, which serves hundreds of models behind one key.
    #[arg(
        long,
        env = "IQ_BASE_URL",
        default_value = "https://router.requesty.ai/v1",
        global = true
    )]
    base_url: String,

    /// API key for that endpoint. Read from the environment so that it never
    /// lands in the shell history.
    #[arg(long, env = "REQUESTY_API_KEY", hide_env_values = true, global = true)]
    api_key: Option<String>,

    /// The lead's model, instead of the settings': a model of the provider,
    /// or `claude-code`, `codex`, `claude-code/opus`.
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
    /// What Claude Code's and Codex's subscriptions spent over the last day,
    /// as their own files keep it.
    Subscriptions,
    /// The assay's exercises and their results, as its folder holds them.
    Assay,
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

fn main() -> Result<()> {
    // iq's program run for another purpose than the person's, before
    // anything else starts: the sandbox's helper confines itself and
    // becomes the command; the others speak to the running iq.
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
        Some(flag) if flag == HOOK_FLAG => std::process::exit(iq_agent::hook_main()),
        Some(flag) if flag == MCP_FLAG => std::process::exit(iq_agent::mcp_main()),
        Some(flag) if flag == EXEC_FLAG => std::process::exit(iq_agent::exec_main(args.next())),
        _ => {}
    }
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let runtime = tokio::runtime::Runtime::new().context("starting the runtime")?;
    match &cli.command {
        Some(Command::Do {
            task,
            pattern,
            partner,
        }) => runtime.block_on(work(&cli, task, *pattern, partner.as_deref())),
        Some(command) => runtime.block_on(run(command, &cli)),
        None => window(&runtime, &cli),
    }
}

/// The crew of the current folder, its other agents reaching it through
/// iq's program, what it does reported to `report`. Must run within the
/// runtime.
fn crew(
    cli: &Cli,
    settings: &Settings,
    prices: Option<Arc<PriceTable>>,
    report: Report,
) -> Result<Crew<OpenAiCompatible, Agents>> {
    let folder = home_folder()?;
    let crew = Crew::new(
        provider(cli),
        agents(prices),
        gate(settings.internet, settings.autopilot_internet())?,
        Helper::this_program()?,
        settings.crew(cli.api_key.is_some())?,
        report,
    );
    crew.set_autopilot(settings.autopilot);
    let program = std::env::current_exe().context("finding iq's own program")?;
    crew.serve(program, folder.join("run"))
        .context("opening the bridge for the other agents")?;
    Ok(crew)
}

/// The provider's models, with its key when there is one.
fn provider(cli: &Cli) -> OpenAiCompatible {
    match &cli.api_key {
        Some(key) => OpenAiCompatible::new(&cli.base_url, key),
        None => OpenAiCompatible::without_key(&cli.base_url),
    }
}

/// The agents with tools of their own, their messages priced at `prices`.
fn agents(prices: Option<Arc<PriceTable>>) -> Agents {
    let mut agents = Agents::find();
    agents.claude = agents.claude.with_prices(prices.clone());
    agents.codex = agents.codex.with_prices(prices);
    agents
}

/// The agents with tools of their own installed on the machine.
fn installed() -> Vec<Agent> {
    let mut found = Vec::new();
    if iq_llm::ClaudeCode::find().is_some() {
        found.push(Agent::ClaudeCode);
    }
    if iq_llm::Codex::find().is_some() {
        found.push(Agent::Codex);
    }
    found
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
    installed: &[Agent],
    prices: Option<Arc<PriceTable>>,
    report: Report,
    told: std::sync::mpsc::Sender<Told>,
    journal: &Path,
) -> Result<(
    Assay<OpenAiCompatible, Agents>,
    Crew<OpenAiCompatible, Agents>,
)> {
    let folder = home_folder()?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")?;
    let store = iq_assay::Store::open(settings.assay_folder(&home))
        .context("opening the assay's folder")?;
    let keyed = cli.api_key.is_some();
    let provider = provider(cli);
    let agents = agents(prices);
    let program = std::env::current_exe().context("finding iq's own program")?;
    let crew_settings = settings.crew(keyed)?;
    let (mirrors, notices) = settings.mirrors(|name| std::env::var(name).ok());
    let runner = iq_assay::Runner {
        provider: provider.clone(),
        provider_name: Some(provider.name()),
        agents: agents.clone(),
        helper: Helper::this_program()?,
        gate: Arc::new(|work: &Path| gate_in(work, false, false).map_err(|e| format!("{e:#}"))),
        reach: Some(iq_assay::Reach {
            program: program.clone(),
            folder: folder.join("run"),
        }),
        effort: crew_settings.effort,
    };
    let assay = Assay::new(
        store,
        runner,
        iq_assay::Settings {
            judges: settings.assay_judges(installed, lead)?,
            allowed: settings.assay_allowed(),
            reference: lead.clone(),
            parallel: settings.assay_parallel(),
            mirrors: mirrors
                .iter()
                .map(|(server, place)| format!("{}: {place}", server.name))
                .collect(),
            servers: mirrors.into_iter().map(|(server, _)| server).collect(),
            journal: Some(journal.to_path_buf()),
            notices,
        },
        Arc::clone(&report),
        Arc::new(move |news| {
            let _ = told.send(news);
        }),
    );
    let crew = Crew::new(
        provider,
        agents,
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
    crew.serve(program, folder.join("run"))
        .context("opening the bridge for the assay's judge")?;
    assay.serve(iq_assay::JUDGE);
    crew.extend(Arc::new(assay.clone()));
    Ok((assay, crew))
}

/// Who the assay may test: the provider's models as it lists them, with
/// their prices, and the agents installed.
async fn roster(provider: &OpenAiCompatible, keyed: bool, installed: &[Agent]) -> iq_assay::Roster {
    let mut roster = iq_assay::Roster {
        provider: Some(provider.name()),
        harnesses: iq_assay::Roster::harnesses_of(installed),
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
/// to the provider, the quotas reported, each task's files, commands and
/// costs.
fn keep(ledger: &Ledger, journal: &Journal, event: &Event) {
    journal.event(event);
    match event {
        Event::Spent {
            agent,
            model,
            cost,
            billed: true,
            ..
        } => ledger.bill(agent, model, *cost),
        Event::Quota(quota) => ledger.quota(quota.clone()),
        _ => {}
    }
}

/// The settings, as the command line changes them, and the lead's model.
fn chosen(cli: &Cli, folder: &std::path::Path) -> Result<(Settings, ModelId)> {
    let mut settings = Settings::load(folder)?;
    settings.autopilot |= cli.autopilot;
    settings.internet |= cli.internet;
    let lead = match &cli.model {
        Some(model) => ModelId::new(model.as_str())?,
        None => settings.lead(cli.api_key.is_some())?,
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
    let prices = iq_llm::load_prices(&folder.join("prices.json"))
        .await
        .map(Arc::new);
    let ledger = Arc::new(Ledger::open(&folder));
    let journal = Arc::new(Journal::open(&folder));
    let (send, mut events) = tokio::sync::mpsc::unbounded_channel();
    let kept = Arc::clone(&ledger);
    let noted = Arc::clone(&journal);
    let crew = crew(
        cli,
        &settings,
        prices,
        Arc::new(move |event: Event| {
            keep(&kept, &noted, &event);
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

/// Opens the window: the crew works on the runtime, the agents' files are
/// read on a thread of their own, and the window runs on the main thread,
/// as the system wants it.
fn window(runtime: &tokio::runtime::Runtime, cli: &Cli) -> Result<()> {
    let folder = home_folder()?;
    let (settings, lead) = chosen(cli, &folder)?;
    let keyed = cli.api_key.is_some();
    let prices = runtime
        .block_on(iq_llm::load_prices(&folder.join("prices.json")))
        .map(Arc::new);
    let ledger = Arc::new(Ledger::open(&folder));
    ledger.trim();
    let journal = Arc::new(Journal::open(&folder));
    journal.trim();
    let (send, feed) = std::sync::mpsc::channel();
    let quit = send.clone();
    let (ask, requests) = std::sync::mpsc::channel::<Request>();
    let waker = iq_gui::Waker::default();
    let since = iq_ui::now().saturating_sub(iq_ui::subscriptions::KEEP_SECS);
    // The provider's calls of the whole month too, for the steward.
    let billed_since =
        iq_ui::steward::month(iq_ui::now()).map_or(since, |(start, _)| start.min(since));
    let _ = send.send(Update::Started {
        lead: lead.to_string(),
        reviewer: settings.crew(keyed)?.reviewer.to_string(),
        autopilot: settings.autopilot,
        internet: settings.internet,
    });
    let _ = send.send(Update::Kept {
        provider: ledger.billed(billed_since),
        quotas: ledger.quotas(),
    });

    let report: Report = {
        let send = send.clone();
        let waker = waker.clone();
        let ledger = Arc::clone(&ledger);
        let journal = Arc::clone(&journal);
        Arc::new(move |event: Event| {
            keep(&ledger, &journal, &event);
            let _ = send.send(Update::Crew(event));
            waker.wake();
        })
    };
    let crew = {
        let _inside = runtime.enter();
        crew(cli, &settings, prices.clone(), Arc::clone(&report))?
    };
    let installed = installed();
    let (told, heard) = std::sync::mpsc::channel::<Told>();
    let (assay, assay_crew) = {
        let _inside = runtime.enter();
        assay(
            cli,
            &settings,
            &lead,
            &installed,
            prices.clone(),
            report,
            told,
            journal.file(),
        )?
    };
    // What the assay's folder holds is read on a thread of its own, never
    // in the window, at once and whenever it changes.
    {
        let send = send.clone();
        let waker = waker.clone();
        let assay = assay.clone();
        std::thread::spawn(move || {
            let _ = send.send(Update::Assay(Box::new(assay.overview())));
            waker.wake();
            for news in heard {
                let update = match news {
                    Told::Store => Update::Assay(Box::new(assay.overview())),
                    Told::Running(running) => Update::AssayRunning(running),
                };
                if send.send(update).is_err() {
                    return;
                }
                waker.wake();
            }
        });
    }
    {
        let assay = assay.clone();
        let provider = provider(cli);
        let installed = installed.clone();
        runtime.spawn(async move {
            assay.set_roster(roster(&provider, keyed, &installed).await);
        });
    }

    let files = {
        let waker = waker.clone();
        let journal = journal.file().to_path_buf();
        let categories = assay.store().root().join("categories.toml");
        move || {
            let agents = Agents::find();
            loop {
                let tasks = tasks(&journal, &categories);
                if send.send(Update::Tasks(tasks)).is_err() {
                    return;
                }
                let since = iq_ui::now().saturating_sub(iq_ui::subscriptions::KEEP_SECS);
                let calls = agents.subscription_calls(since);
                let spent = calls
                    .iter()
                    .map(|call| spent(call, prices.as_deref()))
                    .collect();
                if send
                    .send(Update::Read(iq_ui::Read { calls, spent }))
                    .is_err()
                {
                    return;
                }
                waker.wake();
                std::thread::sleep(READ_EVERY);
            }
        }
    };
    std::thread::spawn(files);

    // Asked to stop from outside, the window closes as if the person had
    // closed it, so that the agents are stopped with it.
    {
        let waker = waker.clone();
        runtime.spawn(async move {
            stop_asked().await;
            let _ = quit.send(Update::Quit);
            waker.wake();
        });
    }
    let handle = runtime.handle().clone();
    let working = crew.clone();
    let judging = assay_crew.clone();
    let assaying = assay.clone();
    std::thread::spawn(move || {
        let _inside = handle.enter();
        // The judge works in a crew of its own, in the assay's folder:
        // what is asked of an agent goes to the crew it is in.
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
                } => match ModelId::new(&model) {
                    Ok(model) => {
                        let mut models = vec![model];
                        models.extend(partner.and_then(|p| ModelId::new(&p).ok()));
                        working.launch(pattern, &name, &models, task, role);
                    }
                    Err(error) => tracing::warn!("no chat on `{model}`: {error}"),
                },
                Request::Tell { to, text } => {
                    let _ = if judge_has(&to) {
                        judging.tell(&to, text)
                    } else {
                        working.tell(&to, text)
                    };
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
                Request::Advise { to, text } => {
                    let _ = if judge_has(&to) {
                        judging.advise(&to, text)
                    } else {
                        working.advise(&to, text)
                    };
                }
                Request::Closed(payers) => {
                    working.close_payers(payers.clone());
                    judging.close_payers(payers.clone());
                    assaying.close_payers(payers);
                }
                Request::Assay(AssayAsk::Talk) => {
                    if !judge_has(iq_assay::JUDGE) {
                        judging.start(
                            iq_assay::JUDGE,
                            assaying.judge(),
                            assaying.opening(),
                            PERSON,
                        );
                    }
                }
                Request::Assay(AssayAsk::Stop) => assaying.stop(),
            }
        }
    });

    let board = iq_ui::Board::new(settings.keymap()?, settings.look()?)
        .with_limits(settings.limits(), settings.steward);
    iq_gui::run(feed, ask, waker, board);
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
    });
    Ok(())
}

/// Resolves once the person or the system asks iq to stop: Ctrl-C in its
/// terminal, the terminal closed, or a termination asked. iq then stops
/// its agents before it ends: killed outright, it would leave them working
/// on, spending the person's subscription.
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

/// iq's tasks of the longest period the costs offer, each placed by the
/// assay's rules in `categories`, its kinds of task.
fn tasks(journal: &Path, categories: &Path) -> Vec<iq_ui::costs::TaskSpent> {
    let longest = iq_ui::PERIODS.iter().max().copied().unwrap_or(0);
    let since = iq_ui::now().saturating_sub(longest);
    let rules = std::fs::read_to_string(categories)
        .ok()
        .and_then(|text| iq_assay::usage::Rules::parse(&text).ok())
        .unwrap_or_default();
    iq_assay::usage::read(journal, since)
        .iter()
        .map(|task| iq_ui::costs::TaskSpent {
            at: task.at,
            kind: rules.classify(task).map(str::to_owned),
            billed: iq_core::Usd(task.billed),
            listed: iq_core::Usd(task.listed),
            tokens: task.tokens,
        })
        .collect()
}

/// What a subscription's call would cost at the API's list prices.
fn spent(call: &iq_core::SubscriptionCall, prices: Option<&PriceTable>) -> Spent {
    let tokens = Tokens {
        input: call.input,
        cache_read: call.cache_read,
        cache_write: call.cache_written,
        output: call.output,
    };
    Spent {
        at: call.at,
        payer: Payer::Subscription(call.agent),
        model: call.model.clone(),
        cost: prices.and_then(|prices| prices.cost(&call.model, tokens)),
        tokens: call.input + call.cache_read + call.cache_written + call.output,
        by_iq: call.by_iq,
    }
}

/// iq's own folder, readable by its owner only: the audit log, the
/// person's rules and settings, what iq keeps of its sessions, and the
/// bridge's socket, where no sandboxed command reaches.
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

/// Asks the person on the terminal whether `reason` may be overridden.
fn ask(reason: &str, line: &str) -> Result<bool> {
    print!("{line}\nThis is asked because {reason}. Run it? [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

async fn run(command: &Command, cli: &Cli) -> Result<()> {
    let (base_url, api_key) = (cli.base_url.as_str(), cli.api_key.clone());
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
        Command::Subscriptions => {
            let since = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("the clock is before 1970")?
                .as_secs()
                .saturating_sub(24 * 3600);
            let calls = Agents::find().subscription_calls(since);
            for call in &calls {
                println!(
                    "{:?} {:<24} in {:>8} cached {:>9} out {:>7}",
                    call.agent, call.model, call.input, call.cache_read, call.output
                );
            }
            let quotas = calls.iter().rev().filter_map(|call| call.quota.as_ref());
            for agent in [iq_core::Agent::ClaudeCode, iq_core::Agent::Codex] {
                if let Some(quota) = quotas.clone().find(|quota| quota.agent == agent) {
                    for window in &quota.windows {
                        println!("{agent:?} {}: {:.0}% used", window.name, window.used);
                    }
                }
            }
        }
        Command::Assay => {
            let folder = home_folder()?;
            let settings = Settings::load(&folder)?;
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .context("HOME is not set")?;
            let store = iq_assay::Store::open(settings.assay_folder(&home))?;
            println!("The assay's folder: {}", store.root().display());
            let exercises = store.exercises()?;
            if exercises.is_empty() {
                println!("No exercise yet: talk to the judge in the window's assay (Ctrl-L).");
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
