//! The assay as the command line runs it: what its folder holds, an
//! exercise frozen once its checks hold, the batches of runs and their
//! grading, by one grader whose grades alone are compared. What spends is
//! planned first, with what it should cost, for the person to agree to.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use iq_agent::{Crew, Report};
use iq_core::{ChatModel, ModelId};

use crate::candidate::{Candidate, Kind};
use crate::error::AssayError;
use crate::estimate;
use crate::exercise::{Exercise, State, error_text};
use crate::judge;
use crate::record::{Checked, Record, Verdict};
use crate::report::{self, Row, Table};
use crate::roster::Roster;
use crate::run::{Around, Order, Phase, Progress, Runner, Watch, lock};
use crate::settings::{FILE, Settings};
use crate::store::{Store, batch_name};

/// Below this grade of the reference, the grader is said to be doubtful:
/// it changed, or the rubric asks what the reference does not do.
const REFERENCE_LEAST: f32 = 8.0;

/// The most times a candidate runs in one batch.
const MOST_TIMES: u64 = 3;

/// The most candidates in one batch.
const MOST_CANDIDATES: usize = 12;

/// What the grader reads of a run, in tokens at a guess, for the cost told
/// before grading.
const READ_PER_RUN: u64 = 6_000;

/// Where a batch keeps what the grader gave the exercise's reference, and
/// an exercise how its checks went on its reference as it was frozen.
const REFERENCE_FILE: &str = "reference.json";

/// What the grader gave an exercise's reference in one batch, graded
/// blind beside the runs.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ReferenceGrades {
    /// Its letter in the batch.
    label: String,
    /// Each grader's grade.
    verdicts: Vec<Verdict>,
}

/// `records` with only the grades the grader `key` names gave: grades by
/// another model do not compare with them.
fn graded(mut records: Vec<Record>, key: Option<&str>) -> Vec<Record> {
    for record in &mut records {
        record.verdicts.retain(|v| Some(v.key()) == key);
    }
    records
}

/// The reference's changes to the exercise's start, as a unified diff, as
/// the grader reads a run's: the start laid with the reference in a
/// folder made for it under `root`.
fn reference_patch(
    exercise: &Exercise,
    reference: &Path,
    root: &Path,
) -> Result<String, AssayError> {
    let folder = root
        .join("judging")
        .join(format!("{}-reference", exercise.id));
    let _ = std::fs::remove_dir_all(&folder);
    crate::tree::copy(&exercise.start(), &folder)?;
    crate::tree::copy(reference, &folder)?;
    let changed = crate::tree::compare(&exercise.start(), &folder);
    let _ = std::fs::remove_dir_all(&folder);
    Ok(changed?.patch())
}

/// How the checks went on the exercise's reference as it was frozen.
fn reference_checked(exercise: &Exercise) -> Option<Checked> {
    std::fs::read_to_string(exercise.folder.join(REFERENCE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

/// What an action of the assay will do, for the person to agree to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The action, in words.
    pub action: String,
    /// What it does and costs, said after "it is asked because".
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

/// Runs the grader has not graded yet, ready for the person to agree to.
#[derive(Debug, Clone)]
pub struct GradePlan {
    exercise: Exercise,
    records: Vec<Record>,
    /// What it does and costs.
    pub plan: Plan,
}

impl GradePlan {
    /// How many runs.
    #[must_use]
    pub fn runs(&self) -> usize {
        self.records.len()
    }
}

/// An exercise, as the overview gives it.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// The exercise.
    pub exercise: Exercise,
    /// Its runs, all versions.
    pub runs: usize,
    /// The runs of its latest version the grader has not graded.
    pub waiting: usize,
    /// What the grader gave its reference solution lately, when it graded
    /// it beside the runs.
    pub reference_grade: Option<f32>,
    /// Its results, the grader's grades alone, when it has runs.
    pub table: Option<Table>,
}

/// What the assay's folder holds, summed.
#[derive(Debug, Clone, PartialEq)]
pub struct Overview {
    /// The folder.
    pub folder: PathBuf,
    /// The settings.
    pub settings: Settings,
    /// Whose grades are summed: the grader's, or, when none is set, the
    /// one that graded last.
    pub graded_by: Option<String>,
    /// The reference, as its runs are kept under.
    pub reference: Option<String>,
    /// The exercises.
    pub exercises: Vec<Summary>,
    /// Each kind of task's results across its exercises, with the
    /// cheapest candidate that does as well as the reference.
    pub categories: Vec<(String, Vec<Row>, Option<String>)>,
    /// What went wrong reading the exercises, if anything did.
    pub trouble: Option<String>,
    /// What the person should know of the grades.
    pub notices: Vec<String>,
}

/// The assay. Cheap to clone: every clone is the same.
pub struct Assay<M> {
    inner: Arc<Inner<M>>,
}

impl<M> Clone for Assay<M> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct Inner<M> {
    store: Store,
    runner: Runner<M>,
    settings: Settings,
    roster: Mutex<Roster>,
    running: Mutex<Vec<Progress>>,
    active: Mutex<Vec<(String, Crew<M>)>>,
    stopping: AtomicBool,
    batches: AtomicUsize,
    costs: Report,
    watch: Watch,
}

impl<M> Assay<M>
where
    M: ChatModel + Clone + 'static,
{
    /// The assay of `store`, its runs made by `runner`, as `settings` say,
    /// what every call costs told to `costs` as a crew's report, what each
    /// run is at told to `watch`.
    pub fn new(
        store: Store,
        runner: Runner<M>,
        settings: Settings,
        costs: Report,
        watch: Watch,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                runner,
                settings,
                roster: Mutex::new(Roster::default()),
                running: Mutex::new(Vec::new()),
                active: Mutex::new(Vec::new()),
                stopping: AtomicBool::new(false),
                batches: AtomicUsize::new(0),
                costs,
                watch,
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

    /// Who may be tested, and at what prices, as the provider lists them.
    pub fn set_roster(&self, roster: Roster) {
        *lock(&self.inner.roster) = roster;
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

    /// The exercises `which` names: one by its name, those of a kind of
    /// task, or `all`; of a kind or all, the frozen ones alone when
    /// `frozen`, as only they run.
    ///
    /// # Errors
    ///
    /// The exercises cannot be read, or `which` names none.
    pub fn select(&self, which: &str, frozen: bool) -> Result<Vec<String>, AssayError> {
        let exercises = self.inner.store.exercises()?;
        if exercises.iter().any(|e| e.id == which) {
            return Ok(vec![which.to_owned()]);
        }
        let chosen: Vec<String> = exercises
            .iter()
            .filter(|e| !frozen || matches!(e.state, State::Frozen(_)))
            .filter(|e| {
                which == "all" || e.categories.iter().any(|c| c.eq_ignore_ascii_case(which))
            })
            .map(|e| e.id.clone())
            .collect();
        if chosen.is_empty() {
            let names: Vec<&str> = exercises.iter().map(|e| e.id.as_str()).collect();
            return Err(AssayError::NotReady(format!(
                "`{which}` names no exercise, and no {}exercise is of that kind of task; the \
                 exercises are {}",
                if frozen { "frozen " } else { "" },
                if names.is_empty() {
                    "none yet".to_owned()
                } else {
                    names.join(", ")
                }
            )));
        }
        Ok(chosen)
    }

    /// What the folder holds, each run's grade the grader's alone.
    #[must_use]
    pub fn overview(&self) -> Overview {
        let key = self.grader_key();
        let mut overview = Overview {
            folder: self.inner.store.root().to_path_buf(),
            settings: self.inner.settings.clone(),
            graded_by: key.clone(),
            reference: self.reference_key(),
            exercises: Vec::new(),
            categories: Vec::new(),
            trouble: None,
            notices: Vec::new(),
        };
        let exercises = match self.inner.store.exercises() {
            Ok(exercises) => exercises,
            Err(error) => {
                overview.trouble = Some(error_text(&error));
                return overview;
            }
        };
        let mut with_grades: Vec<(Exercise, Vec<Record>)> = Vec::new();
        let mut categories: Vec<String> = Vec::new();
        for exercise in exercises {
            let all = self.inner.store.records(&exercise.id);
            let version = report::table(&exercise, &all).version;
            let waiting = all
                .iter()
                .filter(|r| Some(r.version) == version)
                .filter(|r| key.as_deref().is_none_or(|key| !r.graded_by(key)))
                .count();
            let runs = all.len();
            let records = graded(all, key.as_deref());
            let reference_grade = key
                .as_deref()
                .and_then(|key| self.reference_grade(&exercise.id, key));
            if let Some(grade) = reference_grade.filter(|g| *g < REFERENCE_LEAST) {
                overview.notices.push(format!(
                    "the grader gave {}'s reference only {grade:.1}: it may have changed, or the \
                     rubric asks what the reference does not do",
                    exercise.id
                ));
            }
            if !records.is_empty() {
                for category in &exercise.categories {
                    if !categories.iter().any(|c| c.eq_ignore_ascii_case(category)) {
                        categories.push(category.clone());
                    }
                }
            }
            overview.exercises.push(Summary {
                table: (!records.is_empty()).then(|| report::table(&exercise, &records)),
                exercise: exercise.clone(),
                runs,
                waiting,
                reference_grade,
            });
            with_grades.push((exercise, records));
        }
        let reference = overview.reference.clone();
        for category in categories {
            let rows = report::category(&category, &with_grades);
            let fit = report::cheapest_fit(&rows, reference.as_deref().unwrap_or_default())
                .map(|row| fit_text(row, &rows, reference.as_deref()));
            overview.categories.push((category, rows, fit));
        }
        overview
    }

    /// The model that grades every run.
    fn grader(&self) -> Result<ModelId, AssayError> {
        self.inner.settings.grader.clone().ok_or_else(|| {
            AssayError::NotReady(format!(
                "no grader is set: write grader = \"<provider>/<model>\" in {}, the model that \
                 grades every run",
                self.inner.store.root().join(FILE).display()
            ))
        })
    }

    /// Whose grades are compared: the grader's, else, when the settings
    /// name none, the one that graded last. Grades by another model are
    /// kept apart, never mixed with these.
    fn grader_key(&self) -> Option<String> {
        if let Some(grader) = &self.inner.settings.grader {
            return Some(grader.to_string());
        }
        let mut latest: Option<(u64, String)> = None;
        for exercise in self.inner.store.exercises().unwrap_or_default() {
            for record in self.inner.store.records(&exercise.id) {
                for verdict in &record.verdicts {
                    if latest.as_ref().is_none_or(|(at, _)| verdict.at > *at) {
                        latest = Some((verdict.at, verdict.key().to_owned()));
                    }
                }
            }
        }
        latest.map(|(_, key)| key)
    }

    /// The grades of exercise `id`'s reference, batch by batch.
    fn reference_grades(&self, id: &str) -> Vec<ReferenceGrades> {
        let folder = self.inner.store.root().join("runs").join(id);
        let Ok(entries) = std::fs::read_dir(&folder) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| std::fs::read_to_string(entry.path().join(REFERENCE_FILE)).ok())
            .filter_map(|text| serde_json::from_str(&text).ok())
            .collect()
    }

    /// What the grader `key` names gave exercise `id`'s reference last.
    fn reference_grade(&self, id: &str, key: &str) -> Option<f32> {
        self.reference_grades(id)
            .iter()
            .flat_map(|batch| batch.verdicts.iter())
            .filter(|v| v.key() == key)
            .max_by_key(|v| v.at)
            .map(|v| v.score)
    }

    /// The reference's name as its runs are kept, when the settings name
    /// one.
    fn reference_key(&self) -> Option<String> {
        let reference = self.inner.settings.reference.as_ref()?;
        Some(lock(&self.inner.roster).key(reference))
    }

    /// Exercise `id`'s records, each with the grader's grades alone.
    fn graded_records(&self, id: &str) -> Vec<Record> {
        graded(self.inner.store.records(id), self.grader_key().as_deref())
    }

    /// Tells the watch what `progress` says, and keeps it.
    fn show(&self, progress: Progress) {
        {
            let mut running = lock(&self.inner.running);
            match running
                .iter_mut()
                .find(|p| p.batch == progress.batch && p.label == progress.label)
            {
                Some(at) => *at = progress.clone(),
                None => running.push(progress.clone()),
            }
        }
        (self.inner.watch)(progress);
    }

    fn set_phase(&self, batch: &str, label: &str, phase: Phase) {
        let found = lock(&self.inner.running)
            .iter()
            .find(|p| p.batch == batch && p.label == label)
            .cloned();
        if let Some(progress) = found {
            self.show(Progress { phase, ..progress });
        }
    }

    /// The text of `rows` with the cheapest that does as well as the
    /// reference.
    fn results_text(&self, rows: &[Row]) -> String {
        let reference = self.reference_key();
        let mut out = report::text(rows);
        match report::cheapest_fit(rows, reference.as_deref().unwrap_or_default()) {
            Some(row) => {
                out.push_str(&format!(
                    "\n{}\n",
                    fit_text(row, rows, reference.as_deref())
                ));
            }
            None if !rows.is_empty() => match &reference {
                Some(reference) => out.push_str(&format!(
                    "\nNo candidate billed by the provider does as well as the reference \
                     ({reference}) yet.\n"
                )),
                None => out.push_str(
                    "\nNo candidate billed by the provider passes most of its runs yet.\n",
                ),
            },
            None => {}
        }
        out
    }

    /// What freezing exercise `id` would do, for the person to agree to,
    /// or why it cannot be frozen. Its check is run first, in the
    /// sandbox, on its start, where it must fail, and on its reference,
    /// where it must pass.
    ///
    /// # Errors
    ///
    /// The exercise cannot be read, is not ready to be frozen, or its
    /// checks do not hold: they pass on the start, fail on the reference,
    /// or cannot run in the sandbox.
    pub async fn plan_freeze(&self, id: &str) -> Result<Planned, AssayError> {
        let exercise = self.inner.store.exercise(id)?;
        let problems = exercise.problems();
        if !problems.is_empty() {
            return Err(AssayError::NotReady(format!(
                "{id} cannot be frozen yet: {}",
                problems.join("; ")
            )));
        }
        if let State::Frozen(version) = exercise.state {
            return Ok(Planned::Done(format!(
                "{id} is frozen as it is, version {version}."
            )));
        }
        let next = exercise.versions.last().map_or(1, |v| v.version + 1);
        let checked = self
            .check_checks(&exercise)
            .await
            .map_err(AssayError::NotReady)?;
        let reason = format!(
            "freezing it is for good: every candidate will be told task.md as it is now, start \
             from start/, {} within {} min{}; changed later, it becomes a new version whose \
             results start anew. {checked}",
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

    /// Runs the exercise's check in the sandbox on its start, where it must
    /// fail, and on its reference laid over the start, where it must pass:
    /// checks that pass on the start test nothing a candidate has to do,
    /// and checks the reference fails expect what they should not. What
    /// the reference's checks said is kept beside the exercise, for the
    /// grader to read with the reference. Says how it went.
    async fn check_checks(&self, exercise: &Exercise) -> Result<String, String> {
        if exercise.check.is_none() {
            return Ok("It has no check: the grader alone judges it.".into());
        }
        let scratch = self.inner.store.root().join("checking").join(format!(
            "{}-{}",
            exercise.id,
            crate::now()
        ));
        let checked = self.check_both(exercise, &scratch).await;
        let _ = std::fs::remove_dir_all(&scratch);
        // Left when no other check is under way.
        if let Some(checking) = scratch.parent() {
            let _ = std::fs::remove_dir(checking);
        }
        checked
    }

    async fn check_both(&self, exercise: &Exercise, scratch: &Path) -> Result<String, String> {
        let runner = &self.inner.runner;
        let made = |folder: &Path, over: Option<&Path>| -> Result<(), String> {
            crate::tree::copy(&exercise.start(), folder).map_err(|e| error_text(&e))?;
            if let Some(over) = over {
                crate::tree::copy(over, folder).map_err(|e| error_text(&e))?;
            }
            Ok(())
        };
        let start = scratch.join("start");
        made(&start, None)?;
        let first = runner
            .check_folder(exercise, &start)
            .await
            .map_err(|e| error_text(&e))?;
        if let Some(why) = &first.refused {
            return Err(format!("its check cannot run in the sandbox: {why}"));
        }
        if first.passed {
            return Err(format!(
                "its check passes on start/ already: it tests nothing a candidate has to do. It ends:\n{}",
                crate::run::tail(first.output.trim(), 1_500)
            ));
        }
        let Some(reference) = exercise.reference() else {
            return Ok(
                "Its check fails on start/, in the sandbox; with no reference/, nothing shows it can pass."
                    .into(),
            );
        };
        let solved = scratch.join("reference");
        made(&solved, Some(&reference))?;
        let second = runner
            .check_folder(exercise, &solved)
            .await
            .map_err(|e| error_text(&e))?;
        if let Some(why) = &second.refused {
            return Err(format!(
                "its check cannot run on the reference in the sandbox: {why}"
            ));
        }
        if !second.passed {
            return Err(format!(
                "its check fails on the reference: the tests or the reference are wrong. It ends:\n{}",
                crate::run::tail(second.output.trim(), 1_500)
            ));
        }
        let file = exercise.folder.join(REFERENCE_FILE);
        if let Ok(text) = serde_json::to_string_pretty(&second) {
            let _ = crate::store::write_whole(&file, &(text + "\n"));
        }
        let tally = second
            .tally
            .map(|(passed, total)| format!(", {passed} of {total} tests"))
            .unwrap_or_default();
        Ok(format!(
            "Its check fails on start/ and passes on the reference{tally}, in the sandbox."
        ))
    }

    /// Freezes exercise `id`, which the person agreed to.
    ///
    /// # Errors
    ///
    /// It cannot be frozen.
    pub fn freeze_now(&self, id: &str) -> Result<String, AssayError> {
        let frozen = self.inner.store.freeze(id, crate::now())?;
        Ok(format!(
            "{id} is frozen as version {}, digest {}.",
            frozen.version, frozen.digest
        ))
    }

    /// The candidates `named`, each checked against the provider's list
    /// and the settings.
    fn candidates(&self, named: &[&str]) -> Result<Vec<Candidate>, AssayError> {
        if named.is_empty() {
            return Err(AssayError::NotReady("name a candidate at least".into()));
        }
        if named.len() > MOST_CANDIDATES {
            return Err(AssayError::NotReady(format!(
                "{MOST_CANDIDATES} candidates at most in one batch"
            )));
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
                    "{shown} is a {} candidate, which the assay's settings do not allow ({})",
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
            return Err(AssayError::NotReady(format!(
                "nothing runs: {}",
                refused.join("; ")
            )));
        }
        Ok(candidates)
    }

    /// The grader's cost for `runs` runs, in words.
    fn grading_said(&self, grader: &ModelId, runs: usize) -> String {
        let roster = lock(&self.inner.roster).clone();
        let read = READ_PER_RUN * runs as u64;
        let prices = roster
            .offer(grader.as_str())
            .and_then(|o| Some((o.input?, o.output?)));
        match estimate::judging(read, runs, prices) {
            Some(billed) if billed.0 > 0.0 => format!("about {billed} billed"),
            Some(_) => "little".into(),
            None => "an amount unknown, its prices not listed".into(),
        }
    }

    /// What running the frozen exercise `id` on the candidates `named`,
    /// `times` each, would do and cost, for the person to agree to.
    ///
    /// # Errors
    ///
    /// No grader is set, the exercise is not frozen as it is, or a
    /// candidate cannot be tested.
    pub fn plan_run(&self, id: &str, named: &[&str], times: u64) -> Result<RunPlan, AssayError> {
        let grader = self.grader()?;
        let exercise = self.inner.store.exercise(id)?;
        let version = match &exercise.state {
            State::Frozen(version) => *version,
            State::Changed(version) => {
                return Err(AssayError::NotReady(format!(
                    "{id} changed since version {version}: freeze it again, as a new version"
                )));
            }
            State::Draft => {
                return Err(AssayError::NotReady(format!(
                    "{id} is a draft: freeze it first"
                )));
            }
            State::Broken(why) => {
                return Err(AssayError::NotReady(format!("{id} is broken: {why}")));
            }
        };
        let candidates = self.candidates(named)?;
        let times = times.clamp(1, MOST_TIMES) as usize;
        let records = self.inner.store.records(id);
        let roster = lock(&self.inner.roster).clone();
        let guesses: Vec<estimate::Guess> = candidates
            .iter()
            .map(|c| estimate::guess(c, &records, &roster))
            .collect();
        let runs = candidates.len() * times;
        let names: Vec<String> = candidates.iter().map(ToString::to_string).collect();
        let reason = format!(
            "it spends {}, and the grader ({grader}) {}: {} run{} of {id} version {version} ({}), {}{}, \
             each in a folder of its own for up to {} min{}, then {}",
            estimate::said(&guesses, times),
            self.grading_said(&grader, runs),
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
    /// went once every run is checked and graded.
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

    /// Runs `candidates` on exercise `id`, `times` each, grades them, and
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
        // One letter more than runs for the reference, graded among them.
        let extra = usize::from(exercise.reference().is_some());
        let labels = judge::labels(orders.len() + extra, &format!("{batch}{}", exercise.digest));
        for (candidate, label) in orders.iter().zip(&labels) {
            self.show(Progress {
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
        if !records.is_empty() && !self.inner.stopping.load(Ordering::SeqCst) {
            for record in &records {
                self.set_phase(&record.batch, &record.label, Phase::Grading);
            }
            let notes = self.grade(&exercise, &mut records).await;
            failures.extend(notes);
        }
        lock(&self.inner.running).retain(|p| p.batch != batch || p.exercise != id);
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
                .map_or("not graded".to_owned(), |s| format!("score {s:.1}"));
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
                out.push_str(&format!("  - {}: {}\n", verdict.key(), verdict.notes));
            }
        }
        for failure in &failures {
            out.push_str(&format!("- {failure}\n"));
        }
        let all = self.graded_records(id);
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
        let model = match candidate.kind() {
            Kind::Model => lock(&self.inner.roster).key(candidate.lead()),
            Kind::Team => candidate.to_string(),
        };
        let assay = self.clone();
        let watch: Watch = Arc::new(move |progress: Progress| assay.show(progress));
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
                    watch,
                    active: &self.inner.active,
                    stopping: &self.inner.stopping,
                    now: crate::now(),
                },
            )
            .await
    }

    /// Has the grader grade `records`, the runs of one batch, blind, the
    /// exercise's reference among them when it has one, and keeps its
    /// grades. Returns what went wrong.
    async fn grade(&self, exercise: &Exercise, records: &mut [Record]) -> Vec<String> {
        let grader = match self.grader() {
            Ok(grader) => grader,
            Err(error) => return vec![error_text(&error)],
        };
        let task = match exercise.task() {
            Ok(task) => task,
            Err(error) => {
                return vec![format!(
                    "the task could not be read: {}",
                    error_text(&error)
                )];
            }
        };
        let batch = records.first().map(|r| r.batch.clone()).unwrap_or_default();
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
        // The reference, under the batch's letter no run took.
        let reference = match exercise.reference() {
            Some(folder) => {
                let taken: Vec<&str> = records.iter().map(|r| r.label.as_str()).collect();
                let label =
                    judge::labels(records.len() + 1, &format!("{batch}{}", exercise.digest))
                        .into_iter()
                        .find(|l| !taken.contains(&l.as_str()));
                let patch = reference_patch(exercise, &folder, self.inner.store.root());
                let checked = reference_checked(exercise);
                label
                    .zip(patch.ok())
                    .map(|(label, patch)| (label, patch, checked))
            }
            None => None,
        };
        // Its folder holds the work and nothing that names who did it.
        let blind = self
            .inner
            .store
            .root()
            .join("judging")
            .join(format!("{}-{batch}", exercise.id));
        if std::fs::create_dir_all(&blind).is_ok() {
            for (record, patch) in records.iter().zip(&patches) {
                let _ = std::fs::write(blind.join(format!("{}.diff", record.label)), patch);
            }
            if let Some((label, patch, _)) = &reference {
                let _ = std::fs::write(blind.join(format!("{label}.diff")), patch);
            }
        }
        let mut labels: Vec<String> = records.iter().map(|r| r.label.clone()).collect();
        let prompt = {
            let mut works: Vec<judge::Work<'_>> = records
                .iter()
                .zip(&patches)
                .map(|(record, patch)| judge::Work::of(record, patch.clone()))
                .collect();
            if let Some((label, patch, checked)) = &reference {
                labels.push(label.clone());
                works.push(judge::Work {
                    label,
                    run: None,
                    checked: checked.as_ref(),
                    patch: patch.clone(),
                });
            }
            judge::prompt(exercise, &task, &works)
        };
        let mut notes = Vec::new();
        match self.ask_grader(&grader, &prompt, &labels).await {
            Ok(grades) => {
                let verdict = |grade: &judge::Grade| Verdict {
                    judge: grader.to_string(),
                    model: grader.to_string(),
                    score: grade.score,
                    notes: grade.notes.clone(),
                    at: crate::now(),
                };
                for grade in &grades {
                    if let Some(record) = records.iter_mut().find(|r| r.label == grade.label) {
                        let given = verdict(grade);
                        record.verdicts.retain(|v| v.key() != given.key());
                        record.verdicts.push(given);
                    }
                }
                if let Some((label, _, _)) = &reference
                    && let Some(grade) = grades.iter().find(|g| &g.label == label)
                {
                    let file = self
                        .inner
                        .store
                        .run_folder(&exercise.id, &batch, "")
                        .join(REFERENCE_FILE);
                    let mut kept: ReferenceGrades = std::fs::read_to_string(&file)
                        .ok()
                        .and_then(|text| serde_json::from_str(&text).ok())
                        .unwrap_or_default();
                    let given = verdict(grade);
                    kept.label.clone_from(label);
                    kept.verdicts.retain(|v| v.key() != given.key());
                    kept.verdicts.push(given);
                    if let Ok(text) = serde_json::to_string_pretty(&kept) {
                        let _ = crate::store::write_whole(&file, &(text + "\n"));
                    }
                    if grade.score < REFERENCE_LEAST {
                        notes.push(format!(
                            "the grader gave the reference only {:.1}: it may have changed, or the \
                             rubric asks what the reference does not do",
                            grade.score
                        ));
                    }
                }
            }
            Err(why) => notes.push(format!("{grader} could not grade: {why}")),
        }
        for record in records.iter() {
            if let Err(error) = self.inner.store.save(record) {
                notes.push(error_text(&error));
            }
        }
        notes
    }

    /// The grader's grades, asked again once when its answer cannot be
    /// read.
    async fn ask_grader(
        &self,
        grader: &ModelId,
        prompt: &str,
        labels: &[String],
    ) -> Result<Vec<judge::Grade>, String> {
        let provider = &self.inner.runner.provider;
        let effort = self.inner.settings.grader_effort;
        let costs = &self.inner.costs;
        let answer = judge::ask(provider, grader, effort, prompt.to_owned(), costs).await?;
        match judge::read(&answer, labels) {
            Ok(grades) => Ok(grades),
            Err(why) => {
                let again = format!(
                    "{prompt}\n\nAn answer of yours to this could not be read: {why}. Answer with the JSON object only."
                );
                let answer = judge::ask(provider, grader, effort, again, costs).await?;
                judge::read(&answer, labels)
            }
        }
    }

    /// What grading the runs of exercise `id`'s latest version the grader
    /// has not graded yet would cost, for the person to agree to; none
    /// when it graded them all. After the grader changed, so that every
    /// grade compared is its own.
    ///
    /// # Errors
    ///
    /// No grader is set, or the exercise cannot be read.
    pub fn plan_grade(&self, id: &str) -> Result<Option<GradePlan>, AssayError> {
        let grader = self.grader()?;
        let exercise = self.inner.store.exercise(id)?;
        let records = self.inner.store.records(id);
        let version = report::table(&exercise, &records).version;
        let waiting: Vec<Record> = records
            .into_iter()
            .filter(|r| Some(r.version) == version)
            .filter(|r| !r.graded_by(grader.as_str()))
            .collect();
        if waiting.is_empty() {
            return Ok(None);
        }
        let runs = waiting.len();
        let plan = Plan {
            action: format!(
                "have {grader} grade {runs} run{} of {id}",
                if runs == 1 { "" } else { "s" }
            ),
            reason: format!("it spends {}", self.grading_said(&grader, runs)),
        };
        Ok(Some(GradePlan {
            exercise,
            records: waiting,
            plan,
        }))
    }

    /// Grades what `plan` says, which the person agreed to, batch by
    /// batch, and says how it went.
    pub async fn grade_now(&self, plan: GradePlan) -> String {
        let GradePlan {
            exercise, records, ..
        } = plan;
        let mut by_batch: Vec<Vec<Record>> = Vec::new();
        for record in records {
            match by_batch
                .iter_mut()
                .find(|b| b.first().is_some_and(|r| r.batch == record.batch))
            {
                Some(batch) => batch.push(record),
                None => by_batch.push(vec![record]),
            }
        }
        let mut notes = Vec::new();
        for mut batch in by_batch {
            notes.extend(self.grade(&exercise, &mut batch).await);
        }
        let table = report::table(&exercise, &self.graded_records(&exercise.id));
        let mut out = format!("The grading of {} is done.\n", exercise.id);
        for note in notes {
            out.push_str(&format!("- {note}\n"));
        }
        out.push_str(&self.results_text(&table.rows));
        out
    }
}

/// The cheapest fit in words, against the reference when there is one
/// and it ran.
fn fit_text(row: &Row, rows: &[Row], reference: Option<&str>) -> String {
    let cost = row.per_success.map_or("?".to_owned(), |c| c.to_string());
    let best = reference.and_then(|reference| rows.iter().find(|r| r.candidate == reference));
    match (best, reference) {
        (Some(best), Some(reference)) if best.candidate == row.candidate => format!(
            "The reference ({reference}) is the cheapest that does the work: {cost} a success."
        ),
        (Some(best), Some(reference)) => {
            let theirs = best.per_success.map_or("?".to_owned(), |c| c.to_string());
            format!(
                "{} does as well as the reference ({reference}) for {cost} a success, against {theirs}.",
                row.candidate
            )
        }
        (_, Some(reference)) => format!(
            "{} passes most of its runs for {cost} a success; the reference ({reference}) has not run.",
            row.candidate
        ),
        (_, None) => format!(
            "{} is the cheapest that passes most of its runs: {cost} a success.",
            row.candidate
        ),
    }
}

/// Where an exercise stands, in words.
fn state_said(state: &State) -> String {
    match state {
        State::Draft => "draft".to_owned(),
        State::Frozen(v) => format!("frozen, version {v}"),
        State::Changed(v) => format!("changed since version {v}: freeze it again"),
        State::Broken(why) => format!("broken: {why}"),
    }
}

/// `text`, each line indented by two spaces.
fn indented(text: &str) -> String {
    text.lines().map(|line| format!("  {line}\n")).collect()
}

impl fmt::Display for Overview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let settings = &self.settings;
        writeln!(f, "The assay's folder: {}", self.folder.display())?;
        match &settings.grader {
            Some(grader) => write!(f, "Graded by {grader}, thinking {}", settings.grader_effort)?,
            None => {
                write!(
                    f,
                    "No grader is set: `grader` in {} names the model that grades every run, \
                     needed to run and to grade",
                    self.folder.join(FILE).display()
                )?;
                if let Some(last) = &self.graded_by {
                    write!(f, "; the scores below are {last}'s, who graded last")?;
                }
            }
        }
        writeln!(
            f,
            ". Candidates allowed: {}. The reference: {}. Runs at once: {}.",
            settings.allowed.said(),
            settings
                .reference
                .as_ref()
                .map_or("none".to_owned(), ToString::to_string),
            settings.parallel
        )?;
        for notice in settings.notices.iter().chain(&self.notices) {
            writeln!(f, "Note: {notice}.")?;
        }
        if let Some(trouble) = &self.trouble {
            writeln!(f, "\nThe exercises could not be read: {trouble}")?;
        } else if self.exercises.is_empty() {
            writeln!(
                f,
                "\nNo exercise yet: write one in {}, as README.md there says.",
                self.folder.join("exercises/<id>/").display()
            )?;
        }
        for summary in &self.exercises {
            let exercise = &summary.exercise;
            writeln!(
                f,
                "\n{} ({}): {}. Kinds of task: {}. {}, {} min{}.",
                exercise.id,
                if exercise.title.is_empty() {
                    "no title"
                } else {
                    &exercise.title
                },
                state_said(&exercise.state),
                if exercise.categories.is_empty() {
                    "none".to_owned()
                } else {
                    exercise.categories.join(", ")
                },
                exercise
                    .check
                    .as_deref()
                    .map_or("No check".to_owned(), |c| format!("Checked by `{c}`")),
                exercise.minutes,
                exercise
                    .dollars
                    .map(|d| format!(", at most ${d} billed"))
                    .unwrap_or_default(),
            )?;
            if !matches!(exercise.state, State::Frozen(_)) {
                for problem in exercise.problems() {
                    writeln!(f, "  To fix: {problem}.")?;
                }
            }
            if let Some(grade) = summary.reference_grade {
                writeln!(f, "  Its reference was graded {grade:.1} lately.")?;
            }
            if summary.waiting > 0 {
                writeln!(
                    f,
                    "  {} run{} of its latest version wait{} for {}: iq assay grade {}.",
                    summary.waiting,
                    if summary.waiting == 1 { "" } else { "s" },
                    if summary.waiting == 1 { "s" } else { "" },
                    settings
                        .grader
                        .as_ref()
                        .map_or("a grader".to_owned(), |g| format!("{g}'s grade")),
                    exercise.id
                )?;
            }
            if let Some(table) = &summary.table {
                writeln!(
                    f,
                    "  Version {}, {} run{} in all:",
                    table.version.map_or("?".to_owned(), |v| v.to_string()),
                    summary.runs,
                    if summary.runs == 1 { "" } else { "s" }
                )?;
                write!(f, "{}", indented(&report::text(&table.rows)))?;
            }
        }
        for (category, rows, fit) in &self.categories {
            writeln!(f, "\nKind of task {category}, across its exercises:")?;
            write!(f, "{}", indented(&report::text(rows)))?;
            if let Some(fit) = fit {
                writeln!(f, "  {fit}")?;
            }
        }
        Ok(())
    }
}
