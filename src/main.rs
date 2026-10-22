//! The `iq` command.

#![deny(unsafe_code)]

mod ledger;
mod settings;
mod terminal;

use std::io::{BufRead, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use iq_agent::{Crew, EXEC_FLAG, Event, HOOK_FLAG, MCP_FLAG, PERSON, Report};
use iq_core::{Delegate, ModelId};
use iq_gate::{Action, Approver, Gate, HELPER_FLAG, HOLD_FLAG, Helper, Judged, Rules};
use iq_llm::{Agents, OpenAiCompatible, PriceTable, Tokens};
use iq_ui::Update;
use iq_ui::costs::{Payer, Spent};
use iq_ui::crew::Request;
use tracing_subscriber::EnvFilter;

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
    },
    /// The provider's models, with what they charge per million tokens.
    Models,
    /// What Claude Code's and Codex's subscriptions spent over the last day,
    /// as their own files keep it.
    Subscriptions,
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
        Some(Command::Do { task }) => runtime.block_on(work(&cli, task)),
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
    let provider = match &cli.api_key {
        Some(key) => OpenAiCompatible::new(&cli.base_url, key),
        None => OpenAiCompatible::without_key(&cli.base_url),
    };
    let mut agents = Agents::find();
    agents.claude = agents.claude.with_prices(prices.clone());
    agents.codex = agents.codex.with_prices(prices);
    let crew = Crew::new(
        provider,
        agents,
        gate(settings.internet)?,
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

/// What the crew does, kept in iq's folder as it happens: the calls billed
/// to the provider, the quotas reported.
fn keep(ledger: &Ledger, event: &Event) {
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

/// Hands `task` to the crew in the terminal, until it answers.
async fn work(cli: &Cli, task: &str) -> Result<()> {
    let folder = home_folder()?;
    let (settings, lead) = chosen(cli, &folder)?;
    let prices = iq_llm::load_prices(&folder.join("prices.json"))
        .await
        .map(Arc::new);
    let ledger = Arc::new(Ledger::open(&folder));
    let (send, mut events) = tokio::sync::mpsc::unbounded_channel();
    let kept = Arc::clone(&ledger);
    let crew = crew(
        cli,
        &settings,
        prices,
        Arc::new(move |event: Event| {
            keep(&kept, &event);
            let _ = send.send(event);
        }),
    )?;
    crew.start(LEAD, &lead, task.to_owned(), PERSON);
    let mut tally = terminal::Tally::default();
    let handle = |event: Event, tally: &mut terminal::Tally| {
        if let Some((id, question)) = terminal::show(&event, tally) {
            let crew = crew.clone();
            tokio::task::spawn_blocking(move || crew.answer(id, terminal::ask(&question)));
        }
    };
    loop {
        tokio::select! {
            Some(event) = events.recv() => handle(event, &mut tally),
            () = crew.settled() => {
                while let Ok(event) = events.try_recv() {
                    handle(event, &mut tally);
                }
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
    let (send, feed) = std::sync::mpsc::channel();
    let (ask, requests) = std::sync::mpsc::channel::<Request>();
    let waker = iq_gui::Waker::default();
    let since = iq_ui::now().saturating_sub(iq_ui::subscriptions::KEEP_SECS);
    let _ = send.send(Update::Started {
        lead: lead.to_string(),
        reviewer: settings.crew(keyed)?.reviewer.to_string(),
        autopilot: settings.autopilot,
        internet: settings.internet,
    });
    let _ = send.send(Update::Kept {
        provider: ledger.billed(since),
        quotas: ledger.quotas(),
    });

    let report: Report = {
        let send = send.clone();
        let waker = waker.clone();
        let ledger = Arc::clone(&ledger);
        Arc::new(move |event: Event| {
            keep(&ledger, &event);
            let _ = send.send(Update::Crew(event));
            waker.wake();
        })
    };
    let crew = {
        let _inside = runtime.enter();
        crew(cli, &settings, prices.clone(), report)?
    };

    let files = {
        let waker = waker.clone();
        move || {
            let agents = Agents::find();
            loop {
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

    let handle = runtime.handle().clone();
    let working = crew.clone();
    std::thread::spawn(move || {
        let _inside = handle.enter();
        for request in requests {
            match request {
                Request::Start { task } => {
                    working.start(LEAD, &lead, task, PERSON);
                }
                Request::Tell { to, text } => {
                    let _ = working.tell(&to, text);
                }
                Request::Answer { id, approved } => working.answer(id, approved),
                Request::Autopilot(on) => working.set_autopilot(on),
                Request::Internet(open) => working.set_internet(open),
                Request::Stop { agent } => working.stop(&agent),
            }
        }
    });

    iq_gui::run(feed, ask, waker);
    runtime.block_on(crew.end());
    Ok(())
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
/// some, the internet open to every command or not. Rules are never read
/// from the workspace: a command could write them there.
fn gate(internet: bool) -> Result<Gate> {
    let folder = home_folder()?;
    let mut gate = Gate::builder(std::env::current_dir().context("reading the current folder")?)
        .audit(folder.join("audit.log"))
        .internet(internet);
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
        Command::Run { line, network } => {
            let gate = gate(cli.internet)?;
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
