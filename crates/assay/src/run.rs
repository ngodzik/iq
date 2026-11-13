//! One candidate's run of one exercise: a folder of its own made from the
//! exercise's start, a crew of its own working in it through a gate of its
//! own, for as long and as much as the exercise allows; then what it
//! changed, compared file by file, and its hidden checks run in the
//! sandbox. Nobody answers for the person during a run: what the gate asks
//! is refused, the same for every candidate.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iq_agent::{Crew, Event, PERSON, Report};
use iq_core::{ChatModel, Effort, Usd};
use iq_gate::{Action, Approver, Gate, Helper, Judged, Rules, Scope};

use crate::candidate::Candidate;
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

/// The name the candidate's first agent is started under.
const CANDIDATE: &str = "candidate";

/// How its models were chosen, as the audit log writes it.
const CHOSEN: &str = "the assay's run";

/// Told what a run is at, as it changes.
pub type Watch = Arc<dyn Fn(Progress) + Send + Sync>;

/// What a run is at, for the person to follow it as it goes.
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
    /// The grader reads it.
    Grading,
    /// It is done: whether it passed.
    Done(bool),
}

impl Phase {
    /// In a word or two.
    #[must_use]
    pub fn said(self) -> &'static str {
        match self {
            Phase::Waiting => "waiting",
            Phase::Working => "working",
            Phase::Checking => "checking",
            Phase::Grading => "grading",
            Phase::Done(true) => "done, passed",
            Phase::Done(false) => "done, not passed",
        }
    }
}

/// Runs candidates, each with a crew of its own.
pub struct Runner<M> {
    /// The models of the provider.
    pub provider: M,
    /// Who serves them, as it names itself: `Requesty`, a host.
    pub provider_name: Option<String>,
    /// The sandbox's helper.
    pub helper: Helper,
    /// The audit log the gates of the runs write to, the assay's own;
    /// none, for tests.
    pub audit: Option<PathBuf>,
    /// Rules for the candidates' commands beside the defaults, the
    /// assay's own.
    pub rules: Option<Rules>,
    /// How hard the candidates think; `None` leaves it to each model.
    pub effort: Option<Effort>,
}

/// One run asked of a [`Runner`].
pub(crate) struct Order<'a> {
    /// The exercise, frozen.
    pub(crate) exercise: &'a Exercise,
    /// Who does it.
    pub(crate) candidate: &'a Candidate,
    /// The batch it belongs to.
    pub(crate) batch: &'a str,
    /// The candidate's letter in it.
    pub(crate) label: &'a str,
    /// The model's name as its grade is kept under.
    pub(crate) model: String,
}

/// What a run is told of the world around it while it goes.
pub(crate) struct Around<'a, M> {
    /// Where its costs go, as a crew's report.
    pub(crate) costs: Report,
    /// Told what the run is at.
    pub(crate) watch: Watch,
    /// The crews at work, for them to be stopped.
    pub(crate) active: &'a Mutex<Vec<(String, Crew<M>)>>,
    /// Whether the person asked for the runs to stop.
    pub(crate) stopping: &'a AtomicBool,
    /// The time now, in seconds since 1970, as the run starts.
    pub(crate) now: u64,
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

/// How a run's crew stopped working.
enum Stopped {
    /// No agent of it works any more.
    Settled,
    /// It spent what the exercise allows.
    Spent,
    /// Its time ran out.
    OutOfTime,
}

impl<M> Runner<M>
where
    M: ChatModel + Clone + 'static,
{
    /// The gate of `work`, a run's folder or one its checks are checked
    /// in, with the assay's rules and its audit log, its agents' folder of
    /// scratch `scratch` when they have one.
    fn gate(&self, work: &Path, scratch: Option<&Path>) -> Result<Gate, AssayError> {
        let mut gate = Gate::builder(work);
        if let Some(audit) = &self.audit {
            gate = gate.audit(audit);
        }
        if let Some(rules) = &self.rules {
            gate = gate.rules(rules.clone());
        }
        if let Some(scratch) = scratch {
            gate = gate.scratch(scratch);
        }
        gate.build().map_err(|source| AssayError::Gate {
            path: work.to_path_buf(),
            source,
        })
    }

    /// Runs `order` and keeps its record in `store`.
    ///
    /// # Errors
    ///
    /// The exercise is not frozen, its folder cannot be made, or the gate
    /// of the run's folder cannot be.
    pub(crate) async fn run(
        &self,
        store: &Store,
        order: Order<'_>,
        around: Around<'_, M>,
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
        // Its own, so that runs side by side never read each other's.
        let scratch = folder.join("scratch");
        let gate = self.gate(&work, Some(&scratch))?;
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
        (around.watch)(progress.clone());

        let gathered = Arc::new(Mutex::new(Gathered::default()));
        let spent_all = Arc::new(tokio::sync::Notify::new());
        let report: Report = {
            let gathered = Arc::clone(&gathered);
            let spent_all = Arc::clone(&spent_all);
            let budget = exercise.dollars;
            let costs = Arc::clone(&around.costs);
            let told = Arc::clone(&around.watch);
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
                        model, usage, cost, ..
                    } => {
                        kept.spent.calls += 1;
                        kept.spent.tokens += usage.input.0 + usage.output.0;
                        kept.spent.billed += cost.unwrap_or_default();
                        // Every cost is counted with the assay's others,
                        // under the run's name.
                        costs(Event::Spent {
                            agent: payer.clone(),
                            model: model.clone(),
                            usage: *usage,
                            cost: *cost,
                            cache: None,
                            chat: None,
                        });
                        if budget.is_some_and(|most| kept.spent.billed.0 >= most) {
                            spent_all.notify_one();
                        }
                    }
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
        let candidate = order.candidate;
        let settings = iq_agent::Settings {
            effort: self.effort,
            // Nobody answers for the person during a run.
            patience: Duration::ZERO,
            // A candidate is what it is: its agents start on its own
            // models, never on another that would do the work for it.
            allowed: candidate.models.clone(),
            ..iq_agent::Settings::default()
        };
        let crew = Crew::new(
            self.provider.clone(),
            gate.clone(),
            self.helper.clone(),
            settings,
            report,
        );
        let key = format!("{}/{}/{}", exercise.id, order.batch, order.label);
        lock(around.active).push((key.clone(), crew.clone()));
        let lead = crew.launch(
            candidate.pattern,
            CANDIDATE,
            &candidate.models,
            task,
            None,
            None,
            Vec::new(),
            CHOSEN,
        );
        let limit = Duration::from_secs(exercise.minutes * 60);
        let working = async {
            tokio::select! {
                () = crew.settled() => Stopped::Settled,
                () = spent_all.notified() => Stopped::Spent,
            }
        };
        let stopped = tokio::time::timeout(limit, working)
            .await
            .unwrap_or(Stopped::OutOfTime);
        if !matches!(stopped, Stopped::Settled) {
            crew.stop_all();
        }
        crew.end().await;
        lock(around.active).retain(|(k, _)| *k != key);
        // What the agents kept there is no part of their work.
        let _ = std::fs::remove_dir_all(&scratch);
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
        } else {
            match (stopped, exercise.dollars) {
                (Stopped::OutOfTime, _) => Ending::OutOfTime,
                (Stopped::Spent, Some(most)) => {
                    Ending::Failed(format!("it spent the {} it was allowed", Usd(most)))
                }
                _ => ending(&events, &lead),
            }
        };
        let answer = answer(&events, &lead);
        let changed = tree::compare(&exercise.start(), &work)?;
        write(&folder.join("changes.diff"), &changed.patch())?;
        write(
            &folder.join("transcript.md"),
            &format!(
                "{}### The run ends: {}\n",
                transcript(&events),
                ending.said()
            ),
        )?;

        let checked = match &exercise.check {
            Some(line) if ending != Ending::Stopped => {
                progress.phase = Phase::Checking;
                (around.watch)(progress.clone());
                Some(self.check(exercise, &gate, &work, line, limit).await?)
            }
            _ => None,
        };
        let record = Record {
            exercise: exercise.id.clone(),
            version,
            digest: exercise.digest.clone(),
            batch: order.batch.to_owned(),
            label: order.label.to_owned(),
            candidate: candidate.clone(),
            kind: candidate.kind(),
            model: order.model,
            provider: self.provider_name.clone(),
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
        (around.watch)(progress);
        Ok(record)
    }

    /// Runs the exercise's checks on `work`, a folder made for it, in a
    /// gate and a sandbox of its own: to check the checks, on the start
    /// and on the reference.
    pub(crate) async fn check_folder(
        &self,
        exercise: &Exercise,
        work: &Path,
    ) -> Result<Checked, AssayError> {
        let Some(line) = &exercise.check else {
            return Err(AssayError::NotReady(format!(
                "{} has no check to run",
                exercise.id
            )));
        };
        let gate = self.gate(work, None)?;
        let limit = Duration::from_secs(exercise.minutes * 60);
        self.check(exercise, &gate, work, line, limit).await
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
            host: false,
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
        // A check prints little while it builds: only its time limits it.
        let most = limit.min(CHECK_MOST);
        let sandbox = gate
            .sandbox(self.helper.clone())
            .with_timeout(most)
            .with_quiet(most);
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
    let mut texting = false;
    for event in events {
        if !matches!(event, Event::Text { .. } | Event::TextStart { .. })
            && std::mem::take(&mut texting)
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
                texting = true;
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
            Event::Choose {
                agent, question, ..
            } => out.push_str(&format!(
                "- {agent} asked the person: {question}: nobody answers in a run\n"
            )),
            Event::JobStarted { agent, id, line } => {
                out.push_str(&format!("- {agent} left `{line}` running, as job {id}\n"));
            }
            Event::JobEnded {
                agent, id, said, ..
            } => {
                out.push_str(&format!("- {agent}'s job {id} ended: {said}\n"));
            }
            Event::Posted { from, to, text } => {
                out.push_str(&format!("**{from} → {to}:** {}\n\n", text.trim()));
            }
            Event::Done { agent, outcome } => match outcome {
                Ok(_) => out.push_str(&format!("### {agent} answers\n\n")),
                Err(why) => out.push_str(&format!("### {agent} stops: {why}\n\n")),
            },
            Event::Notice { agent, text } => out.push_str(&format!("- {agent}: {text}\n")),
            // What says nothing of the work: its costs, its places, the
            // answers nobody gave.
            _ => {}
        }
    }
    if texting {
        out.push_str("\n\n");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_transcript_tells_what_the_crew_said_and_did() {
        let events = [
            Event::Joined {
                agent: "candidate".into(),
                model: "lab/quill".into(),
                by: PERSON.into(),
            },
            Event::TextStart {
                agent: "candidate".into(),
            },
            Event::Text {
                agent: "candidate".into(),
                text: "Adding mul.".into(),
            },
            Event::Tool {
                agent: "candidate".into(),
                name: "write_file".into(),
                input: "calc.py".into(),
                output: "written".into(),
                ok: true,
            },
            Event::Choose {
                id: 1,
                agent: "candidate".into(),
                question: "Which name?".into(),
                choices: vec!["mul".into(), "times".into()],
            },
            Event::Done {
                agent: "candidate".into(),
                outcome: Ok("Added mul.".into()),
            },
        ];
        assert_eq!(
            transcript(&events),
            "## candidate joins on lab/quill, started by person\n\n\
             **candidate:** Adding mul.\n\n\
             - candidate `write_file` calc.py\n      written\n\
             - candidate asked the person: Which name?: nobody answers in a run\n\
             ### candidate answers\n\n"
        );
        assert_eq!(ending(&events, "candidate"), Ending::Answered);
        assert_eq!(answer(&events, "candidate"), "Added mul.");
        assert_eq!(
            ending(&events[..1], "candidate"),
            Ending::Failed("it never answered".into())
        );
        assert_eq!(tail("abcdef", 3), "…def");
    }
}
