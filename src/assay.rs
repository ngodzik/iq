//! `iq assay`, built with the `assay` feature: what the assay's folder
//! holds, an exercise frozen, runs of frozen exercises on candidates, and
//! the runs its grader has not graded yet graded. It keeps a folder of its
//! own, with its own settings, audit log and ledger of billed calls, never
//! iq's, so that trying a new model needs no second iq. What spends is
//! said, with what it should cost, and asked of the person first.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::{Context, Result};
use clap::Subcommand;
use iq_agent::{Event, Report};
use iq_assay::{Assay, Phase, Planned, Progress};
use iq_core::Usd;
use iq_gate::{Helper, Rules};
use iq_llm::OpenAiCompatible;
use tracing_subscriber::EnvFilter;

use crate::Cli;
use crate::ledger::Ledger;

/// What the assay is asked from the command line.
#[derive(Subcommand)]
pub(crate) enum Act {
    /// Freezes an exercise as it is, once its check fails on start/ and
    /// passes on the reference: from then on every candidate is given
    /// exactly the same. Asks first.
    Freeze {
        /// The exercise.
        exercise: String,
    },
    /// Runs frozen exercises on candidates, each in a folder of its own,
    /// checked, then graded blind: one exercise, every frozen exercise of
    /// a kind of task, or `all`. Says what it should cost, and asks first.
    Run {
        /// An exercise, a kind of task, or `all`.
        which: String,
        /// A model of the provider (`lab/model`), or a team (`pair: writer
        /// + reviewer`, `lead: lead + cheaper`, `debate: a + b`).
        #[arg(required = true)]
        candidates: Vec<String>,
        /// Runs of each candidate, 3 at most.
        #[arg(long, default_value_t = 1)]
        times: u64,
    },
    /// Grades, with the grader the settings name, the runs of each
    /// exercise's latest version it has not graded yet: after the grader
    /// changed, so that every grade compared is its own. Says what it
    /// should cost, and asks first.
    Grade {
        /// An exercise, a kind of task, or `all`, the default.
        which: Option<String>,
    },
}

/// Runs `iq assay`, apart from the rest of iq: its own folder, its own
/// settings, the provider's key from the environment as iq reads it.
pub(crate) fn main(cli: &Cli, act: Option<&Act>) -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("error"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    let runtime = tokio::runtime::Runtime::new().context("starting the runtime")?;
    runtime.block_on(run(cli, act))
}

async fn run(cli: &Cli, act: Option<&Act>) -> Result<()> {
    let folder = folder()?;
    let store = iq_assay::Store::open(&folder)
        .with_context(|| format!("opening the assay's folder, {}", folder.display()))?;
    let settings = iq_assay::Settings::load(store.root())?;
    // The overview says them with the rest.
    if act.is_some() {
        for notice in &settings.notices {
            eprintln!("iq assay: {notice}");
        }
    }
    // The provider's address: as the command line or the environment
    // names it, else the assay's settings, else Requesty's router.
    let endpoint = cli
        .base_url
        .clone()
        .or_else(|| settings.base_url.clone())
        .unwrap_or_else(|| crate::ROUTER.to_owned());
    let provider = match &cli.api_key {
        Some(key) => OpenAiCompatible::new(&endpoint, key),
        None => OpenAiCompatible::without_key(&endpoint),
    };
    let ledger = Arc::new(Ledger::open(store.root()));
    let billed = Arc::new(Mutex::new(Usd(0.0)));
    let report: Report = {
        let ledger = Arc::clone(&ledger);
        let billed = Arc::clone(&billed);
        Arc::new(move |event: Event| {
            if let Event::Spent {
                agent, model, cost, ..
            } = &event
            {
                ledger.bill(agent, model, *cost);
                *billed.lock().unwrap_or_else(PoisonError::into_inner) += cost.unwrap_or_default();
            }
        })
    };
    let runner = iq_assay::Runner {
        provider: provider.clone(),
        provider_name: Some(provider.name()),
        helper: Helper::this_program()?,
        audit: Some(store.root().join("audit.log")),
        rules: rules(store.root())?,
        effort: settings.effort,
    };
    let assay = Assay::new(store, runner, settings, report, watch());
    let keyed = cli.api_key.is_some();
    match act {
        None => print!("{}", assay.overview()),
        Some(Act::Freeze { exercise }) => freeze(&assay, exercise).await?,
        Some(Act::Run {
            which,
            candidates,
            times,
        }) => {
            assay.set_roster(roster(&provider, keyed).await);
            let named: Vec<&str> = candidates.iter().map(String::as_str).collect();
            run_on(&assay, which, &named, *times).await?;
        }
        Some(Act::Grade { which }) => {
            assay.set_roster(roster(&provider, keyed).await);
            grade(&assay, which.as_deref().unwrap_or("all")).await?;
        }
    }
    let billed = *billed.lock().unwrap_or_else(PoisonError::into_inner);
    if billed.0 > 0.0 {
        println!(
            "\nBilled: {billed} in all, each call kept in {}.",
            assay.store().root().join("billed.jsonl").display()
        );
        ledger.trim();
    }
    Ok(())
}

/// The assay's folder, made readable by its owner only when it is made:
/// it holds the audit log and the ledger.
fn folder() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let folder = iq_assay::folder(|name| std::env::var_os(name), home.as_deref())
        .with_context(|| format!("neither HOME nor {} is set", iq_assay::VARIABLE))?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&folder)
        .with_context(|| format!("making {}", folder.display()))?;
    Ok(folder)
}

/// The rules for the candidates' commands the assay's folder holds,
/// beside the defaults; none unless written there. The runs' folders are
/// elsewhere in it: no command of theirs writes these.
fn rules(folder: &Path) -> Result<Option<Rules>> {
    let file = folder.join("rules.toml");
    if !file.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    Ok(Some(
        Rules::parse(&text).with_context(|| format!("in {}", file.display()))?,
    ))
}

/// Says each run's phase as it changes, with what it was billed once
/// done.
fn watch() -> iq_assay::Watch {
    let said: Mutex<HashMap<String, Phase>> = Mutex::new(HashMap::new());
    Arc::new(move |progress: Progress| {
        let key = format!(
            "{}/{}/{}",
            progress.exercise, progress.batch, progress.label
        );
        let mut said = said.lock().unwrap_or_else(PoisonError::into_inner);
        if said.get(&key) == Some(&progress.phase) {
            return;
        }
        let billed = match progress.phase {
            Phase::Done(_) => format!(", {} billed", progress.billed),
            _ => String::new(),
        };
        println!(
            "  {} {} on {}: {}{billed}",
            progress.label,
            progress.candidate,
            progress.exercise,
            progress.phase.said()
        );
        said.insert(key, progress.phase);
    })
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
    if let Some(why) = &roster.missing {
        eprintln!("iq assay: the provider's models are not listed, nor their prices: {why}");
    }
    roster
}

/// Freezes `exercise` once its checks hold and the person agrees.
async fn freeze(assay: &Assay<OpenAiCompatible>, exercise: &str) -> Result<()> {
    match assay.plan_freeze(exercise).await? {
        Planned::Done(said) => println!("{said}"),
        Planned::Ready(plan) => {
            if agree(&plan.action, &plan.reason)? {
                println!("{}", assay.freeze_now(exercise)?);
            } else {
                println!("Not frozen.");
            }
        }
    }
    Ok(())
}

/// Runs the frozen exercises `which` names on the candidates `named`,
/// `times` each, once the person agrees to what they should cost. Asked
/// to stop, the runs at work stop and the others do not start.
async fn run_on(
    assay: &Assay<OpenAiCompatible>,
    which: &str,
    named: &[&str],
    times: u64,
) -> Result<()> {
    let plans = assay
        .select(which, true)?
        .iter()
        .map(|id| assay.plan_run(id, named, times))
        .collect::<Result<Vec<_>, _>>()?;
    let agreed = match plans.as_slice() {
        [plan] => agree(&plan.plan.action, &plan.plan.reason)?,
        _ => {
            for plan in &plans {
                println!("To {}: {}.", plan.plan.action, plan.plan.reason);
            }
            agree("run them all", "they spend what each says")?
        }
    };
    if !agreed {
        println!("Nothing ran.");
        return Ok(());
    }
    let stopped = Arc::new(AtomicBool::new(false));
    let stopper = {
        let assay = assay.clone();
        let stopped = Arc::clone(&stopped);
        tokio::spawn(async move {
            crate::stop_asked().await;
            eprintln!("\niq assay: the runs stop.");
            stopped.store(true, Ordering::SeqCst);
            assay.stop();
        })
    };
    for plan in plans {
        if stopped.load(Ordering::SeqCst) {
            break;
        }
        println!("\n{}", assay.run_now(plan).await);
    }
    stopper.abort();
    Ok(())
}

/// Grades the runs of the exercises `which` names that the grader has not
/// graded yet, once the person agrees to what it should cost.
async fn grade(assay: &Assay<OpenAiCompatible>, which: &str) -> Result<()> {
    let mut plans = Vec::new();
    for id in assay.select(which, false)? {
        match assay.plan_grade(&id)? {
            Some(plan) => plans.push(plan),
            None => println!("No run of {id}'s latest version waits for a grade."),
        }
    }
    let agreed = match plans.as_slice() {
        [] => return Ok(()),
        [plan] => agree(&plan.plan.action, &plan.plan.reason)?,
        _ => {
            for plan in &plans {
                println!("To {}: {}.", plan.plan.action, plan.plan.reason);
            }
            agree("grade them all", "they spend what each says")?
        }
    };
    if !agreed {
        println!("Nothing was graded.");
        return Ok(());
    }
    for plan in plans {
        println!("\n{}", assay.grade_now(plan).await);
    }
    Ok(())
}

/// Asks the person on the terminal whether `action` may be done.
fn agree(action: &str, reason: &str) -> Result<bool> {
    let mut chars = action.chars();
    let action: String = chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    let reason = reason.trim_end_matches('.');
    print!("{action}? It is asked because {reason}. [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}
