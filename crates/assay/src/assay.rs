//! The assay as iq runs it: the judge's tools, the batches of runs they
//! start once the person approves, the judging that follows, and what the
//! window shows of it all.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use iq_agent::{Called, Crew, Extension, Report};
use iq_core::{ChatModel, Delegate, ModelId, ToolSpec};
use iq_gate::Action;
use serde_json::{Value, json};

use crate::candidate::Candidate;
use crate::error::AssayError;
use crate::estimate;
use crate::exercise::{Exercise, State, error_text, valid_id};
use crate::judge;
use crate::record::{Record, Verdict};
use crate::report::{self, Row, Table};
use crate::roster::{Allowed, Roster};
use crate::run::{Around, Order, Phase, Progress, Runner, lock};
use crate::store::{Store, batch_name};

/// The judge's name on the board, the agent the person talks with.
pub const JUDGE: &str = "assay";

/// Who writes to the judge when its runs are done: not the judge's own
/// name, which it does not read messages from.
const WRITER: &str = "results";

/// The most times a candidate runs in one batch.
const MOST_TIMES: u64 = 3;

/// The most candidates in one batch.
const MOST_CANDIDATES: usize = 12;

/// The most files of a commit's folder made into an exercise.
const MOST_FILES: usize = 20_000;

/// What a judge reads of a run, in tokens at a guess, for the cost told
/// before judging.
const READ_PER_RUN: u64 = 6_000;

/// The assay's settings.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Who judges: the first talks with the person, all grade.
    pub judges: Vec<ModelId>,
    /// Which candidates may be tested.
    pub allowed: Allowed,
    /// The reference: the person's lead.
    pub reference: ModelId,
    /// How many runs go at once.
    pub parallel: usize,
    /// Where a copy of the store is kept, as the judge is told: a server
    /// and where in it.
    pub mirrors: Vec<String>,
    /// The MCP servers the copies are made through, handed to the judge
    /// when it is Claude Code, which takes them.
    pub servers: Vec<iq_core::McpServer>,
    /// The journal of iq's tasks, for what each category of work spent.
    pub journal: Option<PathBuf>,
    /// Where the categories models gave tasks are kept.
    pub labels: Option<PathBuf>,
    /// The person's categories of work.
    pub categories: iq_observe::Categories,
    /// What is wrong in the settings the assay was made from, for the
    /// person to see: a server that cannot be handed.
    pub notices: Vec<String>,
}

/// What changed, for the program to show it.
#[derive(Debug, Clone, PartialEq)]
pub enum Told {
    /// The store's files: exercises, records, verdicts.
    Store,
    /// The runs at work.
    Running(Vec<Progress>),
}

/// What an action of the assay will do, for the person to agree to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The action, in words.
    pub action: String,
    /// What it does and costs, said after "held because".
    pub reason: String,
}

/// An action ready for the person to agree to, or already done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Planned {
    /// It waits for the person.
    Ready(Plan),
    /// Nothing to do, and why.
    Done(String),
}

/// Runs of a frozen exercise, ready for the person to agree to.
#[derive(Debug, Clone)]
pub struct RunPlan {
    exercise: String,
    candidates: Vec<Candidate>,
    times: usize,
    /// What they do and cost.
    pub plan: Plan,
}

impl RunPlan {
    /// How many runs.
    #[must_use]
    pub fn runs(&self) -> usize {
        self.candidates.len() * self.times
    }
}

/// The assay. Cheap to clone: every clone is the same.
pub struct Assay<M, D> {
    inner: Arc<Inner<M, D>>,
}

impl<M, D> Clone for Assay<M, D> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct Inner<M, D> {
    store: Store,
    runner: Runner<M, D>,
    settings: Settings,
    roster: Mutex<Roster>,
    /// The agents whose judge it is.
    served: Mutex<Vec<String>>,
    running: Mutex<Vec<Progress>>,
    active: Mutex<Vec<(String, Crew<M, D>)>>,
    stopping: AtomicBool,
    batches: AtomicUsize,
    closed: Mutex<Vec<String>>,
    costs: Report,
    told: Arc<dyn Fn(Told) + Send + Sync>,
}

/// An exercise, as the window lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// Its name.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Where it stands.
    pub state: State,
    /// Its kinds of task.
    pub categories: Vec<String>,
    /// Its runs, all versions.
    pub runs: usize,
    /// What keeps it from being frozen.
    pub problems: Vec<String>,
}

/// The assay as the window shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overview {
    /// The store's folder.
    pub folder: PathBuf,
    /// Who judges.
    pub judges: Vec<String>,
    /// The reference.
    pub reference: String,
    /// Which candidates may be tested, in words.
    pub allowed: String,
    /// The exercises.
    pub exercises: Vec<Summary>,
    /// Their results.
    pub tables: Vec<Table>,
    /// Each kind of task's results across its exercises, with the
    /// cheapest candidate that does as well as the reference.
    pub categories: Vec<(String, Vec<Row>, Option<String>)>,
    /// What went wrong reading the store, if anything did.
    pub trouble: Option<String>,
    /// What is wrong in the assay's settings.
    pub notices: Vec<String>,
}

impl<M, D> Assay<M, D>
where
    M: ChatModel + Clone + 'static,
    D: Delegate + Clone + 'static,
{
    /// The assay of `store`, its runs made by `runner`, their costs told
    /// to `costs` as a crew's report, what changes told to `told`.
    pub fn new(
        store: Store,
        runner: Runner<M, D>,
        settings: Settings,
        costs: Report,
        told: Arc<dyn Fn(Told) + Send + Sync>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                runner,
                settings,
                roster: Mutex::new(Roster::default()),
                served: Mutex::new(Vec::new()),
                running: Mutex::new(Vec::new()),
                active: Mutex::new(Vec::new()),
                stopping: AtomicBool::new(false),
                batches: AtomicUsize::new(0),
                closed: Mutex::new(Vec::new()),
                costs,
                told,
            }),
        }
    }

    /// The store.
    #[must_use]
    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    /// The settings.
    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.inner.settings
    }

    /// The judge the person talks with: the first of the settings'.
    #[must_use]
    pub fn judge(&self) -> &ModelId {
        self.inner
            .settings
            .judges
            .first()
            .unwrap_or(&self.inner.settings.reference)
    }

    /// Who may be tested, as the program read it.
    pub fn set_roster(&self, roster: Roster) {
        *lock(&self.inner.roster) = roster;
    }

    /// Serves `agent` as the assay's judge: it gets the assay's tools.
    pub fn serve(&self, agent: &str) {
        let mut served = lock(&self.inner.served);
        if !served.iter().any(|a| a == agent) {
            served.push(agent.to_owned());
        }
    }

    fn serves(&self, agent: &str) -> bool {
        lock(&self.inner.served).iter().any(|a| a == agent)
    }

    /// Whether the judge the person talks with takes MCP servers beside
    /// iq's: Claude Code does.
    fn takes_servers(&self) -> bool {
        matches!(
            self.judge().delegate(),
            Some((iq_core::Agent::ClaudeCode, _))
        )
    }

    /// What the judge is given as the person opens the assay.
    #[must_use]
    pub fn opening(&self) -> String {
        crate::prompt::OPENING.to_owned()
    }

    /// Closes the payers named, as the person's limits say: no run starts
    /// on them.
    pub fn close_payers(&self, payers: Vec<String>) {
        for (_, crew) in lock(&self.inner.active).iter() {
            crew.close_payers(payers.clone());
        }
        *lock(&self.inner.closed) = payers;
    }

    /// Stops the runs at work and those waiting.
    pub fn stop(&self) {
        if self.inner.batches.load(Ordering::SeqCst) == 0 {
            return;
        }
        self.inner.stopping.store(true, Ordering::SeqCst);
        for (_, crew) in lock(&self.inner.active).iter() {
            crew.stop_all();
        }
    }

    /// The runs at work.
    #[must_use]
    pub fn running(&self) -> Vec<Progress> {
        lock(&self.inner.running).clone()
    }

    /// The assay as the window shows it, read from the store.
    #[must_use]
    pub fn overview(&self) -> Overview {
        let settings = &self.inner.settings;
        let mut overview = Overview {
            folder: self.inner.store.root().to_path_buf(),
            judges: settings.judges.iter().map(ToString::to_string).collect(),
            reference: settings.reference.to_string(),
            allowed: settings.allowed.said(),
            notices: settings.notices.clone(),
            ..Overview::default()
        };
        let exercises = match self.inner.store.exercises() {
            Ok(exercises) => exercises,
            Err(error) => {
                overview.trouble = Some(error_text(&error));
                return overview;
            }
        };
        let with_records: Vec<(Exercise, Vec<Record>)> = exercises
            .into_iter()
            .map(|exercise| {
                let records = self.inner.store.records(&exercise.id);
                (exercise, records)
            })
            .collect();
        let reference = self.reference_key();
        let mut categories: Vec<String> = Vec::new();
        for (exercise, records) in &with_records {
            overview.exercises.push(Summary {
                id: exercise.id.clone(),
                title: exercise.title.clone(),
                state: exercise.state.clone(),
                categories: exercise.categories.clone(),
                runs: records.len(),
                problems: exercise.problems(),
            });
            if !records.is_empty() {
                overview.tables.push(report::table(exercise, records));
            }
            for category in &exercise.categories {
                if !records.is_empty()
                    && !categories.iter().any(|c| c.eq_ignore_ascii_case(category))
                {
                    categories.push(category.clone());
                }
            }
        }
        for category in categories {
            let rows = report::category(&category, &with_records);
            let fit =
                report::cheapest_fit(&rows, &reference).map(|row| fit_text(row, &rows, &reference));
            overview.categories.push((category, rows, fit));
        }
        overview
    }

    /// The reference's name as its grades are kept.
    fn reference_key(&self) -> String {
        lock(&self.inner.roster).key(&self.inner.settings.reference)
    }

    fn tell(&self, told: Told) {
        (self.inner.told)(told);
    }

    fn show_running(&self) {
        self.tell(Told::Running(self.running()));
    }

    /// The text of `rows` with the cheapest that does as well as the
    /// reference, for the judge.
    fn results_text(&self, rows: &[Row]) -> String {
        let reference = self.reference_key();
        let mut out = report::text(rows);
        match report::cheapest_fit(rows, &reference) {
            Some(row) => {
                out.push_str(&format!("\n{}\n", fit_text(row, rows, &reference)));
            }
            None if !rows.is_empty() => out.push_str(&format!(
                "\nNo candidate billed by the provider does as well as the reference ({reference}) yet.\n"
            )),
            None => {}
        }
        out
    }

    fn roster_text(&self, args: &Value) -> String {
        let roster = lock(&self.inner.roster).clone();
        let settings = &self.inner.settings;
        let search = args["search"].as_str().unwrap_or("").to_lowercase();
        let limit = args["limit"].as_u64().unwrap_or(40).clamp(1, 200) as usize;
        let mut out = format!(
            "The reference: {}. Judges: {}. The settings allow: {}.\n",
            settings.reference,
            settings
                .judges
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            settings.allowed.said()
        );
        out.push_str(&format!(
            "\nAgents with tools of their own, installed (paid by their subscriptions): {}\n",
            if roster.harnesses.is_empty() {
                "none".to_owned()
            } else {
                roster.harnesses.join(", ")
            }
        ));
        let provider = roster
            .provider
            .clone()
            .unwrap_or_else(|| "the provider".into());
        if let Some(why) = &roster.missing {
            out.push_str(&format!(
                "\nThe models of {provider} are not listed: {why}\n"
            ));
            return out;
        }
        let mut offers: Vec<_> = roster
            .models
            .iter()
            .filter(|offer| offer.tools != Some(false))
            .filter(|offer| {
                search.is_empty()
                    || offer.id.to_lowercase().contains(&search)
                    || offer
                        .canonical
                        .as_deref()
                        .is_some_and(|c| c.to_lowercase().contains(&search))
                    || offer
                        .description
                        .as_deref()
                        .is_some_and(|d| d.to_lowercase().contains(&search))
            })
            .collect();
        let price = |offer: &&crate::roster::Offer| {
            offer.input.unwrap_or(f64::MAX / 4.0) * 10.0 + offer.output.unwrap_or(f64::MAX / 4.0)
        };
        offers.sort_by(|a, b| price(a).total_cmp(&price(b)));
        let total = offers.len();
        out.push_str(&format!(
            "\nModels of {provider} that call tools{}, cheapest first ({total}), dollars per million tokens in / out:\n",
            if search.is_empty() {
                String::new()
            } else {
                format!(" matching `{search}`")
            }
        ));
        for offer in offers.iter().take(limit) {
            let per = |p: Option<f64>| p.map_or("?".to_owned(), |p| format!("{:.2}", p * 1e6));
            let context = offer
                .context
                .map(|c| format!(", reads {}", iq_core::TokenCount(c)))
                .unwrap_or_default();
            out.push_str(&format!(
                "- {}: {} / {}{context}\n",
                offer.id,
                per(offer.input),
                per(offer.output)
            ));
        }
        if total > limit {
            out.push_str(&format!(
                "… {} more: narrow with `search`, or raise `limit`.\n",
                total - limit
            ));
        }
        out
    }

    fn exercises_text(&self) -> Result<String, AssayError> {
        let exercises = self.inner.store.exercises()?;
        if exercises.is_empty() {
            return Ok(format!(
                "No exercise yet. Write one in {}/exercises/<id>/ as README.md says.",
                self.inner.store.root().display()
            ));
        }
        let mut out = String::new();
        for exercise in exercises {
            let state = match &exercise.state {
                State::Draft => "draft".to_owned(),
                State::Frozen(v) => format!("frozen, version {v}"),
                State::Changed(v) => format!("changed since version {v}: freeze it again"),
                State::Broken(why) => format!("broken: {why}"),
            };
            let runs = self.inner.store.records(&exercise.id).len();
            out.push_str(&format!(
                "- {} ({}): {state}; kinds: {}; check: {}; {} min{}; {runs} runs\n",
                exercise.id,
                if exercise.title.is_empty() {
                    "no title"
                } else {
                    &exercise.title
                },
                if exercise.categories.is_empty() {
                    "none".to_owned()
                } else {
                    exercise.categories.join(", ")
                },
                exercise
                    .check
                    .as_deref()
                    .map_or("none".to_owned(), |c| format!("`{c}`")),
                exercise.minutes,
                exercise
                    .dollars
                    .map(|d| format!(", at most ${d} billed"))
                    .unwrap_or_default(),
            ));
            for problem in exercise.problems() {
                out.push_str(&format!("    to fix: {problem}\n"));
            }
        }
        Ok(out)
    }

    fn results(&self, args: &Value) -> Result<String, AssayError> {
        if let Some(id) = args["exercise"].as_str().filter(|s| !s.is_empty()) {
            let exercise = self.inner.store.exercise(id)?;
            let mut table = report::table(&exercise, &self.inner.store.records(id));
            if args["providers"].as_bool().unwrap_or(false) {
                let records = self.inner.store.records(id);
                let kept: Vec<&Record> = records
                    .iter()
                    .filter(|r| Some(r.version) == table.version)
                    .collect();
                table.rows = report::rows_by(&kept, true);
            }
            return Ok(format!(
                "{} ({}), version {}:\n{}",
                table.exercise,
                table.title,
                table
                    .version
                    .map_or("none run".to_owned(), |v| v.to_string()),
                self.results_text(&table.rows)
            ));
        }
        let exercises: Vec<(Exercise, Vec<Record>)> = self
            .inner
            .store
            .exercises()?
            .into_iter()
            .map(|e| {
                let records = self.inner.store.records(&e.id);
                (e, records)
            })
            .collect();
        if let Some(category) = args["category"].as_str().filter(|s| !s.is_empty()) {
            let rows = report::category(category, &exercises);
            return Ok(format!(
                "Kind of task {category}:\n{}",
                self.results_text(&rows)
            ));
        }
        let mut out = String::new();
        for (exercise, records) in &exercises {
            if records.is_empty() {
                continue;
            }
            let table = report::table(exercise, records);
            out.push_str(&format!(
                "{} ({}), version {}:\n{}\n",
                table.exercise,
                table.title,
                table.version.map_or("?".to_owned(), |v| v.to_string()),
                report::text(&table.rows)
            ));
        }
        if out.is_empty() {
            out.push_str("No runs yet.\n");
        }
        Ok(out)
    }

    async fn freeze(&self, crew: &Crew<M, D>, agent: &str, args: &Value) -> Result<String, String> {
        let id = text(args, "exercise")?;
        let plan = match self.plan_freeze(id)? {
            Planned::Ready(plan) => plan,
            Planned::Done(said) => return Ok(said),
        };
        if !crew
            .confirm(agent, plan.action.clone(), plan.reason.clone())
            .await
        {
            return Ok("The person did not agree: it is not frozen.".into());
        }
        self.freeze_now(id)
    }

    /// What freezing exercise `id` would do, for the person to agree to,
    /// or why it cannot be frozen.
    ///
    /// # Errors
    ///
    /// The exercise cannot be read, or is not ready to be frozen.
    pub fn plan_freeze(&self, id: &str) -> Result<Planned, String> {
        let exercise = self.inner.store.exercise(id).map_err(|e| error_text(&e))?;
        let problems = exercise.problems();
        if !problems.is_empty() {
            return Err(format!(
                "{id} cannot be frozen yet: {}",
                problems.join("; ")
            ));
        }
        if let State::Frozen(version) = exercise.state {
            return Ok(Planned::Done(format!(
                "{id} is frozen as it is, version {version}."
            )));
        }
        let next = exercise.versions.last().map_or(1, |v| v.version + 1);
        // The terminal reads it after "held because".
        let reason = format!(
            "freezing it is for good: every candidate will be told task.md as it is now, start \
             from start/, {} within {} min{}; changed later, it becomes a new version whose \
             results start anew",
            exercise
                .check
                .as_deref()
                .map_or("and be judged without checks".to_owned(), |c| format!(
                    "and be checked by `{c}`"
                )),
            exercise.minutes,
            exercise
                .dollars
                .map(|d| format!(" and ${d} billed"))
                .unwrap_or_default()
        );
        let action = format!(
            "freeze the exercise {id} ({}) as version {next}",
            exercise.title
        );
        Ok(Planned::Ready(Plan { action, reason }))
    }

    /// Freezes exercise `id`, which the person agreed to.
    ///
    /// # Errors
    ///
    /// It cannot be frozen.
    pub fn freeze_now(&self, id: &str) -> Result<String, String> {
        let frozen = self
            .inner
            .store
            .freeze(id, crate::now())
            .map_err(|e| error_text(&e))?;
        self.tell(Told::Store);
        Ok(format!(
            "{id} is frozen as version {}, digest {}.",
            frozen.version, frozen.digest
        ))
    }

    /// The candidates `args` names, each checked against the roster and
    /// the settings.
    fn candidates(&self, named: &[&str]) -> Result<Vec<Candidate>, String> {
        if named.is_empty() {
            return Err("name a candidate at least".into());
        }
        if named.len() > MOST_CANDIDATES {
            return Err(format!("{MOST_CANDIDATES} candidates at most in one batch"));
        }
        let roster = lock(&self.inner.roster).clone();
        let mut candidates: Vec<Candidate> = Vec::new();
        let mut refused = Vec::new();
        for &name in named {
            let candidate: Candidate = match name.parse() {
                Ok(candidate) => candidate,
                Err(error) => {
                    refused.push(error);
                    continue;
                }
            };
            let shown = candidate.to_string();
            if !self.inner.settings.allowed.allows(candidate.kind(), &shown) {
                refused.push(format!(
                    "{shown} is a {} candidate, which the person's settings do not allow ({})",
                    candidate.kind(),
                    self.inner.settings.allowed.said()
                ));
                continue;
            }
            if let Err(why) = candidate.models.iter().try_for_each(|m| roster.check(m)) {
                refused.push(why);
                continue;
            }
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
        if !refused.is_empty() {
            return Err(format!("Nothing ran: {}", refused.join("; ")));
        }
        Ok(candidates)
    }

    /// The judges' cost for `runs` runs, in words.
    fn judging_said(&self, runs: usize) -> String {
        let roster = lock(&self.inner.roster).clone();
        let read = READ_PER_RUN * runs as u64;
        let mut dollars = 0.0;
        let mut tokens = 0;
        for judge in &self.inner.settings.judges {
            let prices = if judge.delegate().is_some() {
                None
            } else {
                roster
                    .offer(judge.as_str())
                    .and_then(|o| Some((o.input?, o.output?)))
            };
            let (billed, subscription) = estimate::judging(read, runs, prices);
            dollars += billed.map_or(0.0, |b| b.0);
            tokens += subscription;
        }
        let mut parts = Vec::new();
        if dollars > 0.0 {
            parts.push(format!("about {} billed", iq_core::Usd(dollars)));
        }
        if tokens > 0 {
            parts.push(format!(
                "about {} tokens of subscriptions",
                iq_core::TokenCount(tokens)
            ));
        }
        if parts.is_empty() {
            parts.push("little".into());
        }
        parts.join(" and ")
    }

    async fn run(&self, crew: &Crew<M, D>, agent: &str, args: &Value) -> Result<String, String> {
        let id = text(args, "exercise")?;
        let named: Vec<&str> = args["candidates"]
            .as_array()
            .ok_or("`candidates` must be a list of candidates")?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let times = args["times"].as_u64().unwrap_or(1);
        let plan = self.plan_run(id, &named, times)?;
        if !crew
            .confirm(agent, plan.plan.action.clone(), plan.plan.reason.clone())
            .await
        {
            return Ok("The person did not agree: nothing ran.".into());
        }
        let runs = plan.runs();
        let assay = self.clone();
        let crew = crew.clone();
        let agent = agent.to_owned();
        tokio::spawn(async move {
            let summary = assay.run_now(plan).await;
            let _ = crew.deliver(WRITER, &agent, summary);
        });
        Ok(format!(
            "Started: {runs} run{}, {} at a time. Their results come to you as a message from {WRITER} once they are judged; meanwhile, talk with the person.",
            if runs == 1 { "" } else { "s" },
            self.inner.settings.parallel.max(1)
        ))
    }

    /// What running the frozen exercise `id` on the candidates `named`,
    /// `times` each, would do and cost, for the person to agree to.
    ///
    /// # Errors
    ///
    /// The exercise is not frozen as it is, a candidate cannot be tested,
    /// or the provider's limit is reached.
    pub fn plan_run(&self, id: &str, named: &[&str], times: u64) -> Result<RunPlan, String> {
        let exercise = self.inner.store.exercise(id).map_err(|e| error_text(&e))?;
        let version = match &exercise.state {
            State::Frozen(version) => *version,
            State::Changed(version) => {
                return Err(format!(
                    "{id} changed since version {version}: freeze it again, as a new version"
                ));
            }
            State::Draft => return Err(format!("{id} is a draft: freeze it first")),
            State::Broken(why) => return Err(format!("{id} is broken: {why}")),
        };
        let candidates = self.candidates(named)?;
        let times = times.clamp(1, MOST_TIMES) as usize;
        if lock(&self.inner.closed)
            .iter()
            .any(|p| p == iq_agent::PROVIDER)
            && candidates
                .iter()
                .any(|c| c.models.iter().any(|m| m.delegate().is_none()))
        {
            return Err("the provider's limit, as the person set it, is reached: no run of its models starts".into());
        }
        let records = self.inner.store.records(id);
        let roster = lock(&self.inner.roster).clone();
        let guesses: Vec<estimate::Guess> = candidates
            .iter()
            .map(|c| estimate::guess(c, &records, &roster))
            .collect();
        let runs = candidates.len() * times;
        let names: Vec<String> = candidates.iter().map(ToString::to_string).collect();
        // The terminal reads it after "held because".
        let reason = format!(
            "it spends {}, and the judges ({}) {}: {} run{} of {id} version {version} ({}), {}{}, \
             each in a folder of its own for up to {} min{}, then {}",
            estimate::said(&guesses, times),
            names_of(&self.inner.settings.judges),
            self.judging_said(runs),
            runs,
            if runs == 1 { "" } else { "s" },
            exercise.title,
            names.join(", "),
            if times > 1 {
                format!(", {times} times each")
            } else {
                String::new()
            },
            exercise.minutes,
            exercise
                .dollars
                .map(|d| format!(" and ${d} billed"))
                .unwrap_or_default(),
            exercise
                .check
                .as_deref()
                .map_or("no checks".to_owned(), |c| format!(
                    "`{c}` checks it in the sandbox"
                )),
        );
        let action = format!(
            "run the exercise {id} on {} candidate{}",
            candidates.len(),
            if candidates.len() == 1 { "" } else { "s" }
        );
        Ok(RunPlan {
            exercise: id.to_owned(),
            candidates,
            times,
            plan: Plan { action, reason },
        })
    }

    /// Runs what `plan` says, which the person agreed to, and says how it
    /// went once every run is checked and judged.
    pub async fn run_now(&self, plan: RunPlan) -> String {
        self.inner.batches.fetch_add(1, Ordering::SeqCst);
        let summary = self
            .batch(&plan.exercise, &plan.candidates, plan.times)
            .await;
        if self.inner.batches.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.inner.stopping.store(false, Ordering::SeqCst);
        }
        summary
    }

    /// Runs `candidates` on exercise `id`, `times` each, judges them, and
    /// says how it went.
    async fn batch(&self, id: &str, candidates: &[Candidate], times: usize) -> String {
        let exercise = match self.inner.store.exercise(id) {
            Ok(exercise) => exercise,
            Err(error) => {
                return format!("The runs of {id} could not start: {}", error_text(&error));
            }
        };
        let mut batch = batch_name(crate::now());
        while self.inner.store.run_folder(id, &batch, "").exists() {
            batch.push('x');
        }
        let orders: Vec<&Candidate> = candidates
            .iter()
            .flat_map(|c| std::iter::repeat_n(c, times))
            .collect();
        let labels = judge::labels(orders.len(), &format!("{batch}{}", exercise.digest));
        {
            let mut running = lock(&self.inner.running);
            for (candidate, label) in orders.iter().zip(&labels) {
                running.push(Progress {
                    exercise: id.to_owned(),
                    batch: batch.clone(),
                    label: label.clone(),
                    candidate: candidate.to_string(),
                    phase: Phase::Waiting,
                    tools: 0,
                    billed: iq_core::Usd(0.0),
                    tokens: 0,
                    since: crate::now(),
                });
            }
        }
        self.show_running();
        let gate = Arc::new(tokio::sync::Semaphore::new(
            self.inner.settings.parallel.max(1),
        ));
        let mut set = tokio::task::JoinSet::new();
        for (candidate, label) in orders.iter().zip(&labels) {
            let assay = self.clone();
            let exercise = exercise.clone();
            let candidate = (*candidate).clone();
            let label = label.clone();
            let batch = batch.clone();
            let gate = Arc::clone(&gate);
            set.spawn(async move {
                let _turn = gate.acquire_owned().await;
                if assay.inner.stopping.load(Ordering::SeqCst) {
                    return (label, Err("stopped before it started".to_owned()));
                }
                let outcome = assay.one(&exercise, &candidate, &batch, &label).await;
                (label, outcome.map_err(|e| error_text(&e)))
            });
        }
        let mut records = Vec::new();
        let mut failures = Vec::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((_, Ok(record))) => records.push(record),
                Ok((label, Err(why))) => failures.push(format!("{label}: {why}")),
                Err(error) => failures.push(error.to_string()),
            }
        }
        self.tell(Told::Store);
        if !records.is_empty() && !self.inner.stopping.load(Ordering::SeqCst) {
            for record in &records {
                self.set_phase(&record.batch, &record.label, Phase::Judging);
            }
            let notes = self.grade(&exercise, &mut records).await;
            failures.extend(notes);
            self.tell(Told::Store);
        }
        lock(&self.inner.running).retain(|p| p.batch != batch || p.exercise != id);
        self.show_running();
        let mut out = format!("The runs of {id} (batch {batch}) are done.\n\n");
        records.sort_by(|a, b| a.label.cmp(&b.label));
        for record in &records {
            let checks = match &record.checked {
                Some(checked) => match (&checked.refused, checked.tally) {
                    (Some(why), _) => format!("checks not run ({why})"),
                    (None, Some((p, t))) => {
                        format!(
                            "checks {} ({p}/{t})",
                            if checked.passed { "passed" } else { "failed" }
                        )
                    }
                    (None, None) => format!(
                        "checks {}",
                        if checked.passed { "passed" } else { "failed" }
                    ),
                },
                None => "no checks".to_owned(),
            };
            let score = record
                .score()
                .map_or("not judged".to_owned(), |s| format!("score {s:.1}"));
            let billed = if record.spent.billed.0 > 0.0 {
                format!("{} billed, ", record.spent.billed)
            } else {
                String::new()
            };
            out.push_str(&format!(
                "- {} was {}: {}, {checks}, {score}, {billed}{} tokens, {} s\n",
                record.label,
                record.candidate,
                record.ending.said(),
                iq_core::TokenCount(record.spent.tokens),
                record.seconds
            ));
            for verdict in &record.verdicts {
                out.push_str(&format!("  - {}: {}\n", verdict.judge, verdict.notes));
            }
        }
        for failure in &failures {
            out.push_str(&format!("- {failure}\n"));
        }
        let all = self.inner.store.records(id);
        let table = report::table(&exercise, &all);
        out.push_str(&format!(
            "\nAll runs of version {}:\n{}",
            table.version.map_or("?".to_owned(), |v| v.to_string()),
            self.results_text(&table.rows)
        ));
        out
    }

    /// One run.
    async fn one(
        &self,
        exercise: &Exercise,
        candidate: &Candidate,
        batch: &str,
        label: &str,
    ) -> Result<Record, AssayError> {
        let model = lock(&self.inner.roster).key(candidate.lead());
        let model = match candidate.kind() {
            crate::Kind::Model => model,
            _ => candidate.to_string(),
        };
        let assay = self.clone();
        let progress: Arc<dyn Fn(Progress) + Send + Sync> = Arc::new(move |progress: Progress| {
            {
                let mut running = lock(&assay.inner.running);
                if let Some(at) = running
                    .iter_mut()
                    .find(|p| p.batch == progress.batch && p.label == progress.label)
                {
                    *at = progress;
                }
            }
            assay.show_running();
        });
        let closed = lock(&self.inner.closed).clone();
        self.inner
            .runner
            .run(
                &self.inner.store,
                Order {
                    exercise,
                    candidate,
                    batch,
                    label,
                    model,
                },
                Around {
                    costs: Arc::clone(&self.inner.costs),
                    progress,
                    active: &self.inner.active,
                    stopping: &self.inner.stopping,
                    closed,
                    now: crate::now(),
                },
            )
            .await
    }

    fn set_phase(&self, batch: &str, label: &str, phase: Phase) {
        if let Some(at) = lock(&self.inner.running)
            .iter_mut()
            .find(|p| p.batch == batch && p.label == label)
        {
            at.phase = phase;
        }
        self.show_running();
    }

    /// Has every judge grade `records`, the runs of one batch, blind, and
    /// keeps their verdicts. Returns what went wrong.
    async fn grade(&self, exercise: &Exercise, records: &mut [Record]) -> Vec<String> {
        self.grade_by(exercise, records, &self.inner.settings.judges.clone())
            .await
    }

    async fn grade_by(
        &self,
        exercise: &Exercise,
        records: &mut [Record],
        judges: &[ModelId],
    ) -> Vec<String> {
        let mut notes = Vec::new();
        let task = match exercise.task() {
            Ok(task) => task,
            Err(error) => {
                return vec![format!(
                    "the task could not be read: {}",
                    error_text(&error)
                )];
            }
        };
        // Its folder holds the work and nothing that names who did it.
        let batch = records.first().map(|r| r.batch.clone()).unwrap_or_default();
        let blind = self
            .inner
            .store
            .root()
            .join("judging")
            .join(format!("{}-{batch}", exercise.id));
        let patches: Vec<String> = records
            .iter()
            .map(|record| {
                std::fs::read_to_string(
                    self.inner
                        .store
                        .run_folder(&record.exercise, &record.batch, &record.label)
                        .join("changes.diff"),
                )
                .unwrap_or_default()
            })
            .collect();
        if std::fs::create_dir_all(&blind).is_ok() {
            for (record, patch) in records.iter().zip(&patches) {
                let _ = std::fs::write(blind.join(format!("{}.diff", record.label)), patch);
            }
        }
        let labels: Vec<String> = records.iter().map(|r| r.label.clone()).collect();
        let prompt = {
            let works: Vec<judge::Work<'_>> = records
                .iter()
                .zip(&patches)
                .map(|(record, patch)| judge::Work {
                    record,
                    patch: patch.clone(),
                })
                .collect();
            judge::prompt(exercise, &task, &works)
        };
        for judge in judges {
            if judge.delegate().is_none()
                && lock(&self.inner.closed)
                    .iter()
                    .any(|p| p == iq_agent::PROVIDER)
            {
                notes.push(format!(
                    "{judge} did not judge: the provider's limit is reached"
                ));
                continue;
            }
            match self.ask_judge(judge, &prompt, &labels, &blind).await {
                Ok(grades) => {
                    for grade in grades {
                        if let Some(record) = records.iter_mut().find(|r| r.label == grade.label) {
                            record.verdicts.retain(|v| v.judge != judge.as_str());
                            record.verdicts.push(Verdict {
                                judge: judge.to_string(),
                                score: grade.score,
                                notes: grade.notes,
                                at: crate::now(),
                            });
                        }
                    }
                }
                Err(why) => notes.push(format!("{judge} could not judge: {why}")),
            }
        }
        for record in records.iter() {
            if let Err(error) = self.inner.store.save(record) {
                notes.push(error_text(&error));
            }
        }
        notes
    }

    /// One judge's grades, asked again once when its answer cannot be
    /// read.
    async fn ask_judge(
        &self,
        judge: &ModelId,
        prompt: &str,
        labels: &[String],
        folder: &Path,
    ) -> Result<Vec<judge::Grade>, String> {
        let runner = &self.inner.runner;
        let answer = judge::ask(
            &runner.provider,
            &runner.agents,
            judge,
            prompt.to_owned(),
            folder,
            &self.inner.costs,
        )
        .await?;
        match judge::read(&answer, labels) {
            Ok(grades) => Ok(grades),
            Err(why) => {
                let again = format!(
                    "{prompt}\n\nAn answer of yours to this could not be read: {why}. Answer with the JSON object only."
                );
                let answer = judge::ask(
                    &runner.provider,
                    &runner.agents,
                    judge,
                    again,
                    folder,
                    &self.inner.costs,
                )
                .await?;
                judge::read(&answer, labels)
            }
        }
    }

    async fn regrade(
        &self,
        crew: &Crew<M, D>,
        agent: &str,
        args: &Value,
    ) -> Result<String, String> {
        let id = text(args, "exercise")?;
        let exercise = self.inner.store.exercise(id).map_err(|e| error_text(&e))?;
        let judges: Vec<ModelId> = match args["judges"].as_array() {
            Some(named) if !named.is_empty() => named
                .iter()
                .filter_map(Value::as_str)
                .map(|j| ModelId::new(j).map_err(|e| e.to_string()))
                .collect::<Result<_, _>>()?,
            _ => self.inner.settings.judges.clone(),
        };
        let records = self.inner.store.records(id);
        let version = report::table(&exercise, &records).version;
        let waiting: Vec<Record> = records
            .into_iter()
            .filter(|r| Some(r.version) == version)
            .filter(|r| judges.iter().any(|j| !r.judged_by(j.as_str())))
            .collect();
        if waiting.is_empty() {
            return Ok(format!(
                "Every run of {id}'s latest version is judged by {}.",
                names_of(&judges)
            ));
        }
        let action = format!(
            "have {} judge {} run{} of {id}",
            names_of(&judges),
            waiting.len(),
            if waiting.len() == 1 { "" } else { "s" }
        );
        let reason = format!("it spends {}", self.judging_said(waiting.len()));
        if !crew.confirm(agent, action, reason).await {
            return Ok("The person did not agree: nothing was judged.".into());
        }
        let assay = self.clone();
        let crew = crew.clone();
        let agent = agent.to_owned();
        tokio::spawn(async move {
            let mut by_batch: Vec<Vec<Record>> = Vec::new();
            for record in waiting {
                match by_batch.iter_mut().find(|b| b[0].batch == record.batch) {
                    Some(batch) => batch.push(record),
                    None => by_batch.push(vec![record]),
                }
            }
            let mut notes = Vec::new();
            for mut batch in by_batch {
                notes.extend(assay.grade_by(&exercise, &mut batch, &judges).await);
            }
            assay.tell(Told::Store);
            let table = report::table(&exercise, &assay.inner.store.records(&exercise.id));
            let mut out = format!("The judging of {} is done.\n", exercise.id);
            for note in notes {
                out.push_str(&format!("- {note}\n"));
            }
            out.push_str(&assay.results_text(&table.rows));
            let _ = crew.deliver(WRITER, &agent, out);
        });
        Ok(format!(
            "Judging {}: the results come to you as a message.",
            id
        ))
    }

    async fn of_commit(
        &self,
        crew: &Crew<M, D>,
        agent: &str,
        args: &Value,
    ) -> Result<String, String> {
        let repository = PathBuf::from(text(args, "repository")?);
        let commit = text(args, "commit")?.trim().to_owned();
        let id = text(args, "exercise")?;
        if !valid_id(id) {
            return Err(format!(
                "`{id}` cannot name an exercise: lowercase letters, digits and dashes"
            ));
        }
        if !repository.is_absolute() {
            return Err("give the repository's folder as an absolute path".into());
        }
        let shaped = !commit.is_empty()
            && !commit.starts_with('-')
            && commit
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._/^~-".contains(c));
        if !shaped {
            return Err(format!("`{commit}` does not read as a commit"));
        }
        let folder = self.inner.store.root().join("exercises").join(id);
        if folder.exists() {
            return Err(format!("{id} exists already: choose another name"));
        }
        // Reading another folder is the gate's to judge, so the person's
        // to allow: it is outside the judge's workspace.
        crew.permit(
            agent,
            Action::Read {
                path: repository.clone(),
            },
        )
        .await?;
        let made = crate::commit::exercise(&repository, &commit, &folder, MOST_FILES).await;
        match made {
            Ok(text) => {
                self.tell(Told::Store);
                Ok(text)
            }
            Err(why) => {
                let _ = std::fs::remove_dir_all(&folder);
                Err(why)
            }
        }
    }

    fn usage_text(&self, args: &Value) -> String {
        let Some(journal) = &self.inner.settings.journal else {
            return "iq keeps no journal of its tasks here.".into();
        };
        let days = args["days"].as_u64().unwrap_or(14).clamp(1, 35);
        let since = crate::now().saturating_sub(days * 86_400);
        let tasks = iq_observe::read(journal, since);
        let categories = &self.inner.settings.categories;
        let labels = self
            .inner
            .settings
            .labels
            .as_ref()
            .map(|file| iq_observe::Labels::at(file).load())
            .unwrap_or_default();
        let mut out = format!("iq's tasks of the last {days} days: {}.\n", tasks.len());
        if categories.is_empty() {
            out.push_str(
                "The person's settings name no category of work yet ([observe], each with a name, \
                 a description, and rules if they wish).\n",
            );
        }
        let mut sums: Vec<(Option<String>, usize, f64, f64, u64)> = Vec::new();
        let mut unplaced = Vec::new();
        for task in &tasks {
            let category = iq_observe::category_of(task, categories, &labels);
            if category.is_none() {
                unplaced.push(task);
            }
            match sums.iter_mut().find(|sum| sum.0 == category) {
                Some(sum) => {
                    sum.1 += 1;
                    sum.2 += task.billed;
                    sum.3 += task.listed;
                    sum.4 += task.tokens;
                }
                None => sums.push((category, 1, task.billed, task.listed, task.tokens)),
            }
        }
        sums.sort_by(|a, b| {
            a.0.is_none()
                .cmp(&b.0.is_none())
                .then((b.2 + b.3).total_cmp(&(a.2 + a.3)))
        });
        for (category, count, billed, listed, tokens) in &sums {
            out.push_str(&format!(
                "- {}: {count} tasks, {} billed, {} at list prices under subscriptions, {} tokens\n",
                category.as_deref().unwrap_or("(not placed yet)"),
                iq_core::Usd(*billed),
                iq_core::Usd(*listed),
                iq_core::TokenCount(*tokens)
            ));
        }
        if !unplaced.is_empty() {
            out.push_str("\nTasks not placed yet, the latest first:\n");
            for task in unplaced.iter().rev().take(40) {
                let mut files: Vec<String> = task
                    .files
                    .iter()
                    .map(|f| {
                        f.rsplit('/')
                            .next()
                            .and_then(|name| {
                                name.rsplit_once('.').map(|(_, ext)| format!("*.{ext}"))
                            })
                            .unwrap_or_else(|| f.clone())
                    })
                    .collect();
                files.sort();
                files.dedup();
                let mut commands: Vec<String> = task
                    .commands
                    .iter()
                    .map(|c| c.split_whitespace().take(2).collect::<Vec<_>>().join(" "))
                    .collect();
                commands.sort();
                commands.dedup();
                let prompt: String = task.prompt.chars().take(100).collect();
                out.push_str(&format!(
                    "- {} on {}, {} billed, {} listed: files {}; commands {}; given: {}\n",
                    task.agent,
                    task.model,
                    iq_core::Usd(task.billed),
                    iq_core::Usd(task.listed),
                    if files.is_empty() {
                        "none".into()
                    } else {
                        files.join(" ")
                    },
                    if commands.is_empty() {
                        "none".into()
                    } else {
                        commands.join(", ")
                    },
                    prompt.replace('\n', " ")
                ));
            }
        }
        out
    }

    async fn call_tool(
        &self,
        crew: Crew<M, D>,
        agent: String,
        name: String,
        args: Value,
    ) -> Result<String, String> {
        match name.as_str() {
            "assay_roster" => Ok(self.roster_text(&args)),
            "assay_exercises" => self.exercises_text().map_err(|e| error_text(&e)),
            "assay_results" => self.results(&args).map_err(|e| error_text(&e)),
            "assay_usage" => Ok(self.usage_text(&args)),
            "assay_freeze" => self.freeze(&crew, &agent, &args).await,
            "assay_run" => self.run(&crew, &agent, &args).await,
            "assay_grade" => self.regrade(&crew, &agent, &args).await,
            "assay_from_commit" => self.of_commit(&crew, &agent, &args).await,
            _ => Err(format!("the assay has no tool named {name:?}")),
        }
    }
}

impl<M, D> Extension<M, D> for Assay<M, D>
where
    M: ChatModel + Clone + 'static,
    D: Delegate + Clone + 'static,
{
    fn tools(&self, agent: &str) -> Vec<ToolSpec> {
        if self.serves(agent) {
            tools()
        } else {
            Vec::new()
        }
    }

    fn instructions(&self, agent: &str) -> Option<String> {
        if !self.serves(agent) {
            return None;
        }
        let settings = &self.inner.settings;
        let mirrors = if self.takes_servers() {
            settings.mirrors.clone()
        } else {
            Vec::new()
        };
        Some(crate::prompt::instructions(
            settings.reference.as_str(),
            &settings.allowed.said(),
            &settings
                .judges
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            &mirrors,
        ))
    }

    fn servers(&self, agent: &str) -> Vec<iq_core::McpServer> {
        if self.serves(agent) && self.takes_servers() {
            self.inner.settings.servers.clone()
        } else {
            Vec::new()
        }
    }

    fn call(&self, crew: Crew<M, D>, agent: String, name: String, args: Value) -> Called {
        let assay = self.clone();
        Box::pin(async move { assay.call_tool(crew, agent, name, args).await })
    }
}

fn text<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args[name]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("`{name}` is missing or not text"))
}

fn names_of(models: &[ModelId]) -> String {
    models
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The cheapest fit in words, against the reference when it ran.
fn fit_text(row: &Row, rows: &[Row], reference: &str) -> String {
    let cost = row.per_success.map_or("?".to_owned(), |c| c.to_string());
    match rows.iter().find(|r| r.candidate == reference) {
        Some(best) if best.candidate == row.candidate => format!(
            "The reference ({reference}) is the cheapest that does the work: {cost} a success."
        ),
        Some(best) => {
            let theirs = best.per_success.map_or("?".to_owned(), |c| c.to_string());
            format!(
                "{} does as well as the reference ({reference}) for {cost} a success, against {theirs}.",
                row.candidate
            )
        }
        None => format!(
            "{} passes most of its runs for {cost} a success; the reference ({reference}) has not run.",
            row.candidate
        ),
    }
}

/// The assay's tools.
fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "assay_roster".into(),
            description: "Who may be tested: the provider's models that call tools, cheapest first, with their prices, the agents with tools of their own installed, the judges, the reference, and what the person's settings allow.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "search": {"type": "string", "description": "Letters in a model's name or description: glm, mini, coder."},
                    "limit": {"type": "integer", "description": "How many models to list, 40 by default."},
                },
            }),
        },
        ToolSpec {
            name: "assay_exercises".into(),
            description: "The exercises of the store: where each stands (draft, frozen, changed), its kinds of task, its check, its limits, its runs, and what keeps a draft from being frozen.".into(),
            parameters: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "assay_results".into(),
            description: "The results summed by iq: an exercise's, a kind of task's across its exercises, or all; each candidate's runs, passes, judges' score and cost, and the cheapest that does as well as the reference.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "exercise": {"type": "string"},
                    "category": {"type": "string"},
                    "providers": {"type": "boolean", "description": "An exercise's runs apart by the provider that served each model."},
                },
            }),
        },
        ToolSpec {
            name: "assay_usage".into(),
            description: "What iq's agents worked on lately and what it cost, by the person's categories of work, and the tasks not placed yet: to propose categories, and rules for them, for the person to write in their settings.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "days": {"type": "integer", "description": "How far back, 14 days by default, 35 at most."},
                },
            }),
        },
        ToolSpec {
            name: "assay_freeze".into(),
            description: "Freezes an exercise as it is, once the person agrees, which it asks them: from then on every candidate gets exactly the same. Changed later, it is frozen again as a new version.".into(),
            parameters: json!({
                "type": "object",
                "properties": {"exercise": {"type": "string"}},
                "required": ["exercise"],
            }),
        },
        ToolSpec {
            name: "assay_run".into(),
            description: "Runs a frozen exercise on candidates, once the person agrees to what it should cost, which it asks them; each in a folder of its own, checked, then judged blind. Returns at once; the results come to you as a message. A candidate is a model (`z-ai/glm-5`), an agent (`claude-code/sonnet`), or a team (`pair: writer + reviewer`, `lead: lead + cheaper`, `debate: a + b`).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "exercise": {"type": "string"},
                    "candidates": {"type": "array", "items": {"type": "string"}},
                    "times": {"type": "integer", "description": "Runs of each candidate, 1 by default, 3 at most."},
                },
                "required": ["exercise", "candidates"],
            }),
        },
        ToolSpec {
            name: "assay_grade".into(),
            description: "Has judges grade the runs of an exercise's latest version they did not grade yet, once the person agrees, which it asks them: the settings' judges, or those named.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "exercise": {"type": "string"},
                    "judges": {"type": "array", "items": {"type": "string"}},
                },
                "required": ["exercise"],
            }),
        },
        ToolSpec {
            name: "assay_from_commit".into(),
            description: "Makes a draft exercise from a commit of a repository on this machine, which the person allows: start/ as the folder was before it, reference.diff (what it changed), after/ (those files after it), source.md. You then write task.md, rubric.md and the checks, from the tests the commit added.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "repository": {"type": "string", "description": "The repository's folder, an absolute path."},
                    "commit": {"type": "string"},
                    "exercise": {"type": "string", "description": "The new exercise's name: lowercase letters, digits and dashes."},
                },
                "required": ["repository", "commit", "exercise"],
            }),
        },
    ]
}
