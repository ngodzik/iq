//! One candidate's run of one exercise: a folder of its own made from the
//! exercise's start, a crew of its own working in it through a gate of its
//! own, for as long as the exercise allows; then what it changed, compared
//! file by file, and its hidden checks run in the sandbox. Nobody answers
//! for the person during a run: what the gate asks is refused, the same
//! for every candidate.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iq_agent::{Crew, Event, PERSON, Report};
use iq_core::{Agent, ChatModel, Delegate, Effort, ModelId, Usd};
use iq_gate::{Action, Approver, Gate, Helper, Judged, Scope};

use crate::candidate::{Candidate, Kind};
use crate::error::AssayError;
use crate::exercise::Exercise;
use crate::record::{Checked, Ending, Record, Spent};
use crate::store::Store;
use crate::tree;

/// The longest the hidden checks run.
const CHECK_MOST: Duration = Duration::from_secs(20 * 60);

/// How much of the checks' output is kept.
const CHECK_OUTPUT: usize = 6_000;

/// How many lines of a tool's output the transcript keeps.
const TRANSCRIPT_LINES: usize = 12;

/// Makes the gate of a run's folder, with the person's rules.
pub type GateFor = Arc<dyn Fn(&Path) -> Result<Gate, String> + Send + Sync>;

/// How the agents with tools of their own reach a run's crew: iq's
/// program, and the folder of iq's own where their sockets are.
#[derive(Debug, Clone)]
pub struct Reach {
    /// iq's program.
    pub program: PathBuf,
    /// The folder of iq's own for the bridge's sockets.
    pub folder: PathBuf,
}

/// What a run is at, for the window to show it as it goes.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    /// The exercise.
    pub exercise: String,
    /// The batch.
    pub batch: String,
    /// The candidate's letter.
    pub label: String,
    /// The candidate, as written.
    pub candidate: String,
    /// What it does.
    pub phase: Phase,
    /// The tools it called.
    pub tools: u32,
    /// What the provider billed it.
    pub billed: Usd,
    /// The tokens it read and wrote.
    pub tokens: u64,
    /// When it started, in seconds since 1970.
    pub since: u64,
}

/// What a run does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// It waits for its turn.
    Waiting,
    /// The candidate works.
    Working,
    /// Its checks run.
    Checking,
    /// Its judges read it.
    Judging,
    /// It is done: whether it passed.
    Done(bool),
}

/// Runs candidates, each with a crew of its own.
pub struct Runner<M, D> {
    /// The models of the provider.
    pub provider: M,
    /// Who serves them, as it names itself: `Requesty`, a host.
    pub provider_name: Option<String>,
    /// The agents with tools of their own.
    pub agents: D,
    /// The sandbox's helper.
    pub helper: Helper,
    /// The gate of a run's folder.
    pub gate: GateFor,
    /// How the agents with tools of their own reach a run's crew, when
    /// they may.
    pub reach: Option<Reach>,
    /// How hard the models think.
    pub effort: Option<Effort>,
}

/// One run asked of a [`Runner`].
pub struct Order<'a> {
    /// The exercise, frozen.
    pub exercise: &'a Exercise,
    /// Who does it.
    pub candidate: &'a Candidate,
    /// The batch it belongs to.
    pub batch: &'a str,
    /// The candidate's letter in it.
    pub label: &'a str,
    /// The model's name as its grade is kept under.
    pub model: String,
}

/// What a run is told of the world around it while it goes.
pub struct Around<'a, M, D> {
    /// Where its costs go, as the crew's report: they are counted with
    /// iq's others.
    pub costs: Report,
    /// Told what the run is at.
    pub progress: Arc<dyn Fn(Progress) + Send + Sync>,
    /// The crews at work, for them to be stopped.
    pub active: &'a Mutex<Vec<(String, Crew<M, D>)>>,
    /// Whether the person asked for the runs to stop.
    pub stopping: &'a AtomicBool,
    /// The payers the person's limits closed.
    pub closed: Vec<String>,
    /// The time now, in seconds since 1970, as the run starts.
    pub now: u64,
}

/// What a run's crew did, gathered as it reports it.
#[derive(Default)]
struct Gathered {
    events: Vec<Event>,
    tools: u32,
    tool_errors: u32,
    refused: u32,
    spent: Spent,
}

impl<M, D> Runner<M, D>
where
    M: ChatModel + Clone + 'static,
    D: Delegate + Clone + 'static,
{
    /// Runs `order` and keeps its record in `store`.
    ///
    /// # Errors
    ///
    /// The exercise is not frozen, its folder cannot be made, or the gate
    /// of the run's folder cannot be.
    pub async fn run(
        &self,
        store: &Store,
        order: Order<'_>,
        around: Around<'_, M, D>,
    ) -> Result<Record, AssayError> {
        let exercise = order.exercise;
        let version = exercise.version().ok_or_else(|| {
            AssayError::NotReady(format!("{} is not frozen as it is now", exercise.id))
        })?;
        let folder = store.run_folder(&exercise.id, order.batch, order.label);
        let work = folder.join("work");
        if work.exists() {
            return Err(AssayError::NotReady(format!(
                "{} holds a run already",
                folder.display()
            )));
        }
        tree::copy(&exercise.start(), &work)?;
        let gate = (self.gate)(&work).map_err(AssayError::Gate)?;
        let task = exercise.task()?;
        let started = Instant::now();
        let mut progress = Progress {
            exercise: exercise.id.clone(),
            batch: order.batch.to_owned(),
            label: order.label.to_owned(),
            candidate: order.candidate.to_string(),
            phase: Phase::Working,
            tools: 0,
            billed: Usd(0.0),
            tokens: 0,
            since: around.now,
        };
        (around.progress)(progress.clone());

        let gathered = Arc::new(Mutex::new(Gathered::default()));
        let report: Report = {
            let gathered = Arc::clone(&gathered);
            let costs = Arc::clone(&around.costs);
            let told = Arc::clone(&around.progress);
            let shown = progress.clone();
            let payer = format!("assay {} {}", exercise.id, order.label);
            Arc::new(move |event: Event| {
                let mut kept = lock(&gathered);
                match &event {
                    Event::Tool { ok, .. } => {
                        kept.tools += 1;
                        kept.tool_errors += u32::from(!ok);
                    }
                    Event::Asked { .. } => kept.refused += 1,
                    Event::Spent {
                        model,
                        usage,
                        cost,
                        billed,
                        ..
                    } => {
                        let tokens = usage.input.0 + usage.output.0;
                        kept.spent.calls += 1;
                        kept.spent.tokens += tokens;
                        if *billed {
                            kept.spent.billed += cost.unwrap_or_default();
                        } else {
                            kept.spent.listed += cost.unwrap_or_default();
                            kept.spent.subscription_tokens += tokens;
                        }
                        // Every cost is counted with iq's others, under the
                        // run's name.
                        costs(Event::Spent {
                            agent: payer.clone(),
                            model: model.clone(),
                            usage: *usage,
                            cost: *cost,
                            billed: *billed,
                        });
                    }
                    Event::Quota(quota) => costs(Event::Quota(quota.clone())),
                    _ => {}
                }
                let moved = matches!(event, Event::Tool { .. } | Event::Spent { .. });
                kept.events.push(event);
                if moved {
                    told(Progress {
                        tools: kept.tools,
                        billed: kept.spent.billed,
                        tokens: kept.spent.tokens,
                        ..shown.clone()
                    });
                }
            })
        };
        let settings = iq_agent::Settings {
            reviewer: ModelId::agent(Agent::ClaudeCode),
            effort: self.effort,
            budget: exercise.dollars.map(Usd),
            // Nobody answers for the person during a run.
            patience: Duration::ZERO,
            ..iq_agent::Settings::default()
        };
        let crew = Crew::new(
            self.provider.clone(),
            self.agents.clone(),
            gate.clone(),
            self.helper.clone(),
            settings,
            report,
        );
        crew.close_payers(around.closed.clone());
        if let Some(reach) = &self.reach {
            crew.serve(reach.program.clone(), reach.folder.clone())
                .map_err(AssayError::io(&reach.folder))?;
        }
        let key = format!("{}/{}/{}", exercise.id, order.batch, order.label);
        lock(around.active).push((key.clone(), crew.clone()));
        let candidate = order.candidate;
        let lead = crew.launch(
            candidate.pattern,
            "candidate",
            &candidate.models,
            task,
            None,
        );
        let limit = Duration::from_secs(exercise.minutes * 60);
        let in_time = tokio::time::timeout(limit, crew.settled()).await.is_ok();
        if !in_time {
            crew.stop_all();
        }
        crew.end().await;
        lock(around.active).retain(|(k, _)| *k != key);
        let seconds = started.elapsed().as_secs();

        let (events, tools, tool_errors, refused, spent) = {
            let kept = lock(&gathered);
            (
                kept.events.clone(),
                kept.tools,
                kept.tool_errors,
                kept.refused,
                kept.spent,
            )
        };
        let ending = if around.stopping.load(Ordering::Relaxed) {
            Ending::Stopped
        } else if !in_time {
            Ending::OutOfTime
        } else {
            ending(&events, &lead)
        };
        let answer = answer(&events, &lead);
        let changed = tree::compare(&exercise.start(), &work)?;
        write(&folder.join("changes.diff"), &changed.patch())?;
        write(&folder.join("transcript.md"), &transcript(&events))?;

        let checked = match &exercise.check {
            Some(line) if ending != Ending::Stopped => {
                progress.phase = Phase::Checking;
                (around.progress)(progress.clone());
                Some(self.check(exercise, &gate, &work, line, limit).await?)
            }
            _ => None,
        };
        let kind = candidate.kind();
        let record = Record {
            exercise: exercise.id.clone(),
            version,
            digest: exercise.digest.clone(),
            batch: order.batch.to_owned(),
            label: order.label.to_owned(),
            candidate: candidate.clone(),
            kind,
            model: order.model,
            provider: match kind {
                Kind::Harness => None,
                Kind::Model | Kind::Team => self.provider_name.clone(),
            },
            harness: match candidate.lead().delegate() {
                Some((agent, _)) if kind == Kind::Harness => agent.prefix().to_owned(),
                _ => "iq".to_owned(),
            },
            started: around.now,
            seconds,
            ending,
            answer,
            tools,
            tool_errors,
            refused,
            spent,
            changed: (
                changed.files() as u32,
                changed.added() as u32,
                changed.removed() as u32,
            ),
            checked,
            verdicts: Vec::new(),
        };
        store.save(&record)?;
        progress.phase = Phase::Done(record.passed());
        progress.tools = tools;
        progress.billed = spent.billed;
        progress.tokens = spent.tokens;
        (around.progress)(progress);
        Ok(record)
    }

    /// Puts the hidden checks in `work` and runs `line` there, in the
    /// sandbox. The person approved this line with the run, so what the
    /// gate asks of it is theirs, as long as it stays within the sandbox.
    async fn check(
        &self,
        exercise: &Exercise,
        gate: &Gate,
        work: &Path,
        line: &str,
        limit: Duration,
    ) -> Result<Checked, AssayError> {
        tree::copy(&exercise.checks(), work)?;
        let refused = |why: String| Checked {
            line: line.to_owned(),
            passed: false,
            tally: None,
            output: String::new(),
            refused: Some(why),
        };
        let action = Action::Run {
            line: line.to_owned(),
            folder: work.to_path_buf(),
            network: false,
        };
        let permit = match gate.judge(action) {
            Ok(Judged::Allowed(permit)) => permit,
            Ok(Judged::Asked { pending, .. }) if matches!(pending.scope(), Scope::Sandbox) => {
                match gate.approve(pending, Approver::Person) {
                    Ok(permit) => permit,
                    Err(error) => return Ok(refused(crate::exercise::error_text(&error))),
                }
            }
            Ok(Judged::Asked { reason, .. }) => {
                return Ok(refused(format!(
                    "the gate asks for it ({reason}), which a run cannot answer"
                )));
            }
            Ok(Judged::Denied { reason }) => return Ok(refused(format!("refused, {reason}"))),
            Err(error) => return Ok(refused(crate::exercise::error_text(&error))),
        };
        let sandbox = gate
            .sandbox(self.helper.clone())
            .with_timeout(limit.min(CHECK_MOST));
        let ran = sandbox.run(permit).await;
        sandbox.end().await;
        Ok(match ran {
            Ok(ran) => Checked {
                line: line.to_owned(),
                passed: ran.status == Some(0) && !ran.timed_out,
                tally: crate::tally::tally(&ran.output),
                output: tail(&ran.output, CHECK_OUTPUT),
                refused: None,
            },
            Err(error) => refused(crate::exercise::error_text(&error)),
        })
    }
}

/// How the lead's last turn ended.
fn ending(events: &[Event], lead: &str) -> Ending {
    let last = events.iter().rev().find_map(|event| match event {
        Event::Done { agent, outcome } if agent == lead => Some(outcome),
        _ => None,
    });
    match last {
        Some(Ok(_)) => Ending::Answered,
        Some(Err(why)) => Ending::Failed(why.clone()),
        None => Ending::Failed("it never answered".into()),
    }
}

/// What the candidate said last: what the team wrote to the person, else
/// the lead's last answer.
fn answer(events: &[Event], lead: &str) -> String {
    events
        .iter()
        .rev()
        .find_map(|event| match event {
            Event::Posted { to, text, .. } if to == PERSON => Some(text.clone()),
            _ => None,
        })
        .or_else(|| {
            events.iter().rev().find_map(|event| match event {
                Event::Done {
                    agent,
                    outcome: Ok(text),
                } if agent == lead => Some(text.clone()),
                _ => None,
            })
        })
        .unwrap_or_default()
}

/// What the crew said and did, as the person reads it.
fn transcript(events: &[Event]) -> String {
    let mut out = String::new();
    let mut texting: Option<String> = None;
    for event in events {
        if !matches!(event, Event::Text { .. } | Event::TextStart { .. })
            && texting.take().is_some()
        {
            out.push_str("\n\n");
        }
        match event {
            Event::Joined { agent, model, by } => {
                out.push_str(&format!("## {agent} joins on {model}, started by {by}\n\n"));
            }
            Event::Working { agent, prompt } => {
                out.push_str(&format!("### {agent} is given\n\n{}\n\n", quote(prompt)));
            }
            Event::TextStart { agent } => {
                out.push_str(&format!("**{agent}:** "));
                texting = Some(agent.clone());
            }
            Event::Text { text, .. } => out.push_str(text),
            Event::Tool {
                agent,
                name,
                input,
                output,
                ok,
            } => {
                let how = if *ok { "" } else { " (failed)" };
                out.push_str(&format!("- {agent} `{name}` {input}{how}\n"));
                let lines: Vec<&str> = output.lines().collect();
                for line in lines.iter().take(TRANSCRIPT_LINES) {
                    out.push_str(&format!("      {line}\n"));
                }
                if lines.len() > TRANSCRIPT_LINES {
                    out.push_str(&format!(
                        "      … {} more lines\n",
                        lines.len() - TRANSCRIPT_LINES
                    ));
                }
            }
            Event::Asked {
                agent,
                action,
                reason,
                ..
            } => out.push_str(&format!(
                "- {agent} asked to {action} ({reason}): refused, as nobody answers in a run\n"
            )),
            Event::Posted { from, to, text } => {
                out.push_str(&format!("**{from} → {to}:** {}\n\n", text.trim()));
            }
            Event::Done { agent, outcome } => match outcome {
                Ok(_) => out.push_str(&format!("### {agent} answers\n\n")),
                Err(why) => out.push_str(&format!("### {agent} stops: {why}\n\n")),
            },
            Event::Notice { agent, text } => out.push_str(&format!("- {agent}: {text}\n")),
            Event::Spent { .. }
            | Event::Answered { .. }
            | Event::Reviewed { .. }
            | Event::Quota(_) => {}
        }
    }
    out
}

/// `text` as a Markdown quote.
fn quote(text: &str) -> String {
    text.lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The last `most` bytes of `text`, cut at a character.
pub(crate) fn tail(text: &str, most: usize) -> String {
    if text.len() <= most {
        return text.to_owned();
    }
    let mut from = text.len() - most;
    while !text.is_char_boundary(from) {
        from += 1;
    }
    format!("…{}", &text[from..])
}

fn write(file: &Path, text: &str) -> Result<(), AssayError> {
    std::fs::write(file, text).map_err(AssayError::io(file))
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
