//! An exercise: a folder an agent writes with the person, frozen once
//! they agree on it, so that every candidate, now or in a month, is given
//! exactly the same.
//!
//! ```text
//! exercises/<id>/
//!   exercise.toml   its title, kinds of task, the command that checks it, its limits
//!   task.md         what every candidate is told, word for word
//!   rubric.md       what the grader grades beyond the checks
//!   start/          the files a candidate starts from
//!   checks/         hidden tests, put in only once the candidate is done
//!   reference/      a solution that passes them, laid over start/: never given
//!   frozen.json     its versions, each with the digest of the above but the reference
//!   reference.json  how the checks went on the reference, as it was frozen
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AssayError;

/// The most a candidate may work on an exercise, in minutes.
pub const MOST_MINUTES: u64 = 120;

/// What a version's digest covers: all a candidate and its grader are
/// given. The reference is not: it only checks the checks, and a better
/// one changes no candidate's results.
pub(crate) const FROZEN_PARTS: &[&str] =
    &["exercise.toml", "task.md", "rubric.md", "start", "checks"];

/// `exercise.toml`, as the judge writes it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    title: String,
    #[serde(default)]
    categories: Vec<String>,
    check: Option<String>,
    minutes: Option<u64>,
    dollars: Option<f64>,
    source: Option<String>,
}

/// One frozen version of an exercise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frozen {
    /// Its number, from 1.
    pub version: u32,
    /// The digest of what it holds.
    pub digest: String,
    /// When it was frozen, in seconds since 1970.
    pub at: u64,
}

/// Where an exercise stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Being written: never frozen.
    Draft,
    /// Frozen, as its latest version holds it.
    Frozen(u32),
    /// Changed since its latest version: frozen again, it is a new one.
    Changed(u32),
    /// Its settings cannot be read.
    Broken(String),
}

/// An exercise of the store.
#[derive(Debug, Clone, PartialEq)]
pub struct Exercise {
    /// Its name, its folder's.
    pub id: String,
    /// What it is, in a line.
    pub title: String,
    /// The kinds of task it stands for, as the judge names them.
    pub categories: Vec<String>,
    /// The command run once a candidate is done, its hidden checks put
    /// in: its exit status says whether it passed.
    pub check: Option<String>,
    /// The most a candidate may work on it.
    pub minutes: u64,
    /// The most a candidate may be billed for it, in dollars.
    pub dollars: Option<f64>,
    /// Where it came from, as the judge wrote it.
    pub source: Option<String>,
    /// Its versions, oldest first.
    pub versions: Vec<Frozen>,
    /// Where it stands.
    pub state: State,
    /// The digest of what it holds now.
    pub digest: String,
    /// Its folder.
    pub folder: PathBuf,
}

/// The minutes a candidate gets when the exercise does not say.
const MINUTES: u64 = 15;

impl Exercise {
    /// The exercise in `folder`, its name `id`.
    pub(crate) fn read(id: &str, folder: &Path) -> Result<Self, AssayError> {
        let file = folder.join("exercise.toml");
        let digest = crate::tree::digest(folder, FROZEN_PARTS)?;
        let versions = read_versions(folder)?;
        let written = std::fs::read_to_string(&file)
            .map_err(AssayError::io(&file))
            .and_then(|text| {
                toml::from_str::<Written>(&text).map_err(|source| AssayError::Toml {
                    path: file.clone(),
                    source,
                })
            });
        let (written, broken) = match written {
            Ok(written) => (written, None),
            Err(error) => (Written::default(), Some(error_text(&error))),
        };
        let state = match (broken, versions.last()) {
            (Some(why), _) => State::Broken(why),
            (None, None) => State::Draft,
            (None, Some(last)) if last.digest == digest => State::Frozen(last.version),
            (None, Some(last)) => State::Changed(last.version),
        };
        Ok(Self {
            id: id.to_owned(),
            title: written.title,
            categories: written.categories,
            check: written.check.filter(|c| !c.trim().is_empty()),
            minutes: written.minutes.unwrap_or(MINUTES),
            dollars: written.dollars,
            source: written.source,
            versions,
            state,
            digest,
            folder: folder.to_path_buf(),
        })
    }

    /// The version a candidate is given: the latest, when it holds.
    #[must_use]
    pub fn version(&self) -> Option<u32> {
        match self.state {
            State::Frozen(version) => Some(version),
            _ => None,
        }
    }

    /// What every candidate is told.
    ///
    /// # Errors
    ///
    /// The file cannot be read.
    pub fn task(&self) -> Result<String, AssayError> {
        let file = self.folder.join("task.md");
        std::fs::read_to_string(&file).map_err(AssayError::io(file))
    }

    /// What the grader grades beyond the checks, empty when not written.
    #[must_use]
    pub fn rubric(&self) -> String {
        std::fs::read_to_string(self.folder.join("rubric.md")).unwrap_or_default()
    }

    /// The files a candidate starts from.
    #[must_use]
    pub fn start(&self) -> PathBuf {
        self.folder.join("start")
    }

    /// The hidden checks.
    #[must_use]
    pub fn checks(&self) -> PathBuf {
        self.folder.join("checks")
    }

    /// A solution that passes the checks, laid over the start, when the
    /// exercise has one: `reference/`, or `judge/reference/` as an assay
    /// graded by hand kept it.
    #[must_use]
    pub fn reference(&self) -> Option<PathBuf> {
        ["reference", "judge/reference"]
            .iter()
            .map(|at| self.folder.join(at))
            .find(|folder| folder.is_dir())
    }

    /// What keeps it from being frozen, none when it may be.
    #[must_use]
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if let State::Broken(why) = &self.state {
            problems.push(format!("exercise.toml: {why}"));
            return problems;
        }
        if self.title.trim().is_empty() {
            problems.push("exercise.toml gives no title".into());
        }
        if !self.task().is_ok_and(|task| !task.trim().is_empty()) {
            problems.push("task.md, what every candidate is told, is missing or empty".into());
        }
        let has_checks = crate::tree::files(&self.checks(), true).is_ok_and(|f| !f.is_empty());
        if has_checks && self.check.is_none() {
            problems.push(
                "checks/ holds hidden tests, but exercise.toml gives no `check` command to run them"
                    .into(),
            );
        }
        if self.check.is_none() && self.rubric().trim().is_empty() {
            problems.push("nothing judges it: give a `check` command, a rubric.md, or both".into());
        }
        if self.minutes == 0 || self.minutes > MOST_MINUTES {
            problems.push(format!(
                "`minutes` is {}: give from 1 to {MOST_MINUTES}",
                self.minutes
            ));
        }
        if self.dollars.is_some_and(|d| !d.is_finite() || d <= 0.0) {
            problems.push("`dollars` must be more than 0".into());
        }
        if self.categories.is_empty() {
            problems.push("`categories` names no kind of task it stands for".into());
        }
        problems
    }
}

/// An error and its causes, as one line.
pub(crate) fn error_text(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

fn read_versions(folder: &Path) -> Result<Vec<Frozen>, AssayError> {
    let file = folder.join("frozen.json");
    match std::fs::read_to_string(&file) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|source| AssayError::Json { path: file, source })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(AssayError::Io { path: file, source }),
    }
}

/// Writes `versions` as an exercise's own.
pub(crate) fn write_versions(folder: &Path, versions: &[Frozen]) -> Result<(), AssayError> {
    let file = folder.join("frozen.json");
    let text = serde_json::to_string_pretty(versions).map_err(|source| AssayError::Json {
        path: file.clone(),
        source,
    })?;
    std::fs::write(&file, text + "\n").map_err(AssayError::io(file))
}

/// Whether `id` may name an exercise: lowercase letters, digits and
/// dashes, so that it is a folder of the store and nothing else.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 60
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}
