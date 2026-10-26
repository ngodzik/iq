//! The assay's folder: its exercises, its runs, the kinds of task, kept as
//! plain files the person and the judge read and write, and that a copy
//! elsewhere can be made from.
//!
//! ```text
//! README.md          this layout, for whoever opens the folder
//! exercises/<id>/    see `exercise`
//! runs/<id>/<batch>/<label>/
//!   run.json         the record
//!   transcript.md    what the candidate said and did
//!   changes.diff     what it changed
//!   work/            the folder it worked in, its checks put in after
//! ```

use std::path::{Path, PathBuf};

use crate::error::AssayError;
use crate::exercise::{self, Exercise, Frozen};
use crate::record::Record;

/// What the folder's README says.
const README: &str = "# iq's assay

The same exercises given to agents, checked by hidden tests, then judged blind, to find the cheapest agent that does each kind of task well. These files are the reference: a copy kept elsewhere only receives them.

- `exercises/<id>/`: `exercise.toml` (title, categories, the `check` command, `minutes`, `dollars`), `task.md` (what every candidate is told), `rubric.md` (what the judges grade beyond the checks), `start/` (the files a candidate starts from), `checks/` (hidden tests, put in once the candidate is done), `frozen.json` (its versions). Change a frozen exercise and freeze it again: it is a new version, and its results start anew.
- `runs/<id>/<batch>/<label>/`: `run.json` (the record), `transcript.md`, `changes.diff`, `work/` (the folder the candidate worked in).
";

/// Writes `text` to `file` whole or not at all: beside it first, then put
/// in its place, so that a reader at the same time reads the one before.
pub(crate) fn write_whole(file: &Path, text: &str) -> std::io::Result<()> {
    let mut beside = file.as_os_str().to_owned();
    beside.push(".new");
    let beside = PathBuf::from(beside);
    std::fs::write(&beside, text)?;
    std::fs::rename(&beside, file)
}

/// The assay's folder.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// The store in `root`, made with its folders when it is not there.
    ///
    /// # Errors
    ///
    /// The folders cannot be made.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, AssayError> {
        let root = root.into();
        for folder in [root.join("exercises"), root.join("runs")] {
            std::fs::create_dir_all(&folder).map_err(AssayError::io(&folder))?;
        }
        let readme = root.join("README.md");
        if !readme.exists() {
            std::fs::write(&readme, README).map_err(AssayError::io(&readme))?;
        }
        let root = root.canonicalize().map_err(AssayError::io(&root))?;
        Ok(Self { root })
    }

    /// Its folder.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder of exercise `id`.
    fn folder(&self, id: &str) -> Result<PathBuf, AssayError> {
        if !exercise::valid_id(id) {
            return Err(AssayError::NotReady(format!(
                "`{id}` cannot name an exercise: use lowercase letters, digits and dashes"
            )));
        }
        Ok(self.root.join("exercises").join(id))
    }

    /// Every exercise, by name.
    ///
    /// # Errors
    ///
    /// The folder of exercises cannot be read.
    pub fn exercises(&self) -> Result<Vec<Exercise>, AssayError> {
        let folder = self.root.join("exercises");
        let entries = std::fs::read_dir(&folder).map_err(AssayError::io(&folder))?;
        let mut ids: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
            .filter(|id| exercise::valid_id(id))
            .collect();
        ids.sort();
        ids.iter().map(|id| self.exercise(id)).collect()
    }

    /// Exercise `id`.
    ///
    /// # Errors
    ///
    /// There is none of that name, or its files cannot be read.
    pub fn exercise(&self, id: &str) -> Result<Exercise, AssayError> {
        let folder = self.folder(id)?;
        if !folder.is_dir() {
            return Err(AssayError::NoExercise(id.to_owned()));
        }
        Exercise::read(id, &folder)
    }

    /// Freezes exercise `id` as it is now, at `now`: a new version when it
    /// changed since its last, the last again when it did not.
    ///
    /// # Errors
    ///
    /// It cannot be frozen as it is, and why.
    pub fn freeze(&self, id: &str, now: u64) -> Result<Frozen, AssayError> {
        let exercise = self.exercise(id)?;
        let problems = exercise.problems();
        if !problems.is_empty() {
            return Err(AssayError::NotReady(format!(
                "{id} cannot be frozen yet: {}",
                problems.join("; ")
            )));
        }
        if let Some(last) = exercise.versions.last()
            && last.digest == exercise.digest
        {
            return Ok(last.clone());
        }
        let frozen = Frozen {
            version: exercise.versions.last().map_or(1, |v| v.version + 1),
            digest: exercise.digest.clone(),
            at: now,
        };
        let mut versions = exercise.versions;
        versions.push(frozen.clone());
        exercise::write_versions(&exercise.folder, &versions)?;
        Ok(frozen)
    }

    /// The folder of one run.
    pub(crate) fn run_folder(&self, exercise: &str, batch: &str, label: &str) -> PathBuf {
        self.root
            .join("runs")
            .join(exercise)
            .join(batch)
            .join(label)
    }

    /// Keeps `record` in its run's folder, whole or not at all: it is
    /// written beside, then put in place, so that whoever reads it while it
    /// is graded again reads the one before.
    ///
    /// # Errors
    ///
    /// It cannot be written.
    pub fn save(&self, record: &Record) -> Result<(), AssayError> {
        let folder = self.run_folder(&record.exercise, &record.batch, &record.label);
        std::fs::create_dir_all(&folder).map_err(AssayError::io(&folder))?;
        let file = folder.join("run.json");
        let text = serde_json::to_string_pretty(record).map_err(|source| AssayError::Json {
            path: file.clone(),
            source,
        })?;
        write_whole(&file, &(text + "\n")).map_err(AssayError::io(file))
    }

    /// The runs of exercise `id`, oldest first. A record that cannot be
    /// read is left out, as one that never ended.
    #[must_use]
    pub fn records(&self, id: &str) -> Vec<Record> {
        let mut records = Vec::new();
        let Ok(batches) = std::fs::read_dir(self.root.join("runs").join(id)) else {
            return records;
        };
        for batch in batches.flatten() {
            let Ok(runs) = std::fs::read_dir(batch.path()) else {
                continue;
            };
            for run in runs.flatten() {
                let file = run.path().join("run.json");
                if let Some(record) = std::fs::read_to_string(&file)
                    .ok()
                    .and_then(|text| serde_json::from_str::<Record>(&text).ok())
                {
                    records.push(record);
                }
            }
        }
        records
            .sort_by(|a, b| (a.started, &a.batch, &a.label).cmp(&(b.started, &b.batch, &b.label)));
        records
    }
}

/// A batch's name: when it was asked, in UTC, as `20261024-191500`.
#[must_use]
pub fn batch_name(now: u64) -> String {
    let (year, month, day) = civil(now / 86_400);
    let seconds = now % 86_400;
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

/// The date of a day counted from 1970, as the proleptic Gregorian
/// calendar has it.
fn civil(days: u64) -> (u64, u64, u64) {
    // Howard Hinnant's days_from_civil, inverted, for days after 1970.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exercise::State;

    fn write(root: &Path, path: &str, text: &str) {
        let at = root.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, text).unwrap();
    }

    #[test]
    fn an_exercise_is_frozen_once_whole_and_again_once_changed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("assay")).unwrap();
        let folder = store.root().join("exercises/add-mul");
        write(
            &folder,
            "exercise.toml",
            "title = \"Add mul\"\ncheck = \"python3 -m pytest -q\"\n",
        );
        write(
            &folder,
            "start/calc.py",
            "def add(a, b):\n    return a + b\n",
        );
        write(&folder, "checks/test_hidden.py", "from calc import mul\n");
        let draft = store.exercise("add-mul").unwrap();
        assert_eq!(draft.state, State::Draft);
        let problems = draft.problems().join("; ");
        assert!(
            problems.contains("task.md") && problems.contains("categories"),
            "{problems}"
        );
        assert!(store.freeze("add-mul", 10).is_err());
        write(&folder, "task.md", "Add a mul function to calc.py.\n");
        write(
            &folder,
            "exercise.toml",
            "title = \"Add mul\"\ncategories = [\"python\"]\ncheck = \"python3 -m pytest -q\"\nminutes = 10\n",
        );
        let first = store.freeze("add-mul", 10).unwrap();
        assert_eq!(first.version, 1);
        assert_eq!(store.exercise("add-mul").unwrap().state, State::Frozen(1));
        // Frozen again unchanged, it is the same version.
        assert_eq!(store.freeze("add-mul", 11).unwrap(), first);
        write(
            &folder,
            "checks/test_hidden.py",
            "from calc import mul, add\n",
        );
        assert_eq!(store.exercise("add-mul").unwrap().state, State::Changed(1));
        assert_eq!(store.freeze("add-mul", 12).unwrap().version, 2);
        // Notes beside it change nothing.
        write(&folder, "reference.diff", "+x\n");
        assert_eq!(store.exercise("add-mul").unwrap().state, State::Frozen(2));
        assert_eq!(store.exercises().unwrap().len(), 1);
    }

    #[test]
    fn names_that_leave_the_store_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        for id in ["../x", "a/b", "", "Upper", "-x"] {
            assert!(store.exercise(id).is_err(), "{id}");
        }
        assert!(matches!(
            store.exercise("missing"),
            Err(AssayError::NoExercise(_))
        ));
    }

    #[test]
    fn a_broken_exercise_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        write(store.root(), "exercises/odd/exercise.toml", "title = 3\n");
        let odd = store.exercise("odd").unwrap();
        assert!(matches!(odd.state, State::Broken(_)));
        assert!(odd.problems()[0].starts_with("exercise.toml"));
    }

    #[test]
    fn batches_are_named_by_their_time_in_utc() {
        assert_eq!(batch_name(0), "19700101-000000");
        assert_eq!(batch_name(1_792_869_300), "20261024-191500");
        assert_eq!(batch_name(951_782_400), "20000229-000000");
    }
}
