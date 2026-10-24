//! The categories models gave tasks, kept beside the journal, one line of
//! JSON each: what the rules place needs no keeping, as they are applied
//! again each time.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::categories::Categories;
use crate::task::Task;

/// A task's category, as a model gave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    /// The task, by its name.
    pub task: String,
    /// Its category.
    pub category: String,
    /// The model that placed it.
    pub by: String,
    /// When, in seconds since 1970.
    pub at: u64,
}

/// The labels kept in a file.
#[derive(Debug, Clone)]
pub struct Labels {
    file: PathBuf,
}

impl Labels {
    /// The labels kept in `file`.
    #[must_use]
    pub fn at(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into() }
    }

    /// Where they are kept.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Each task's latest label, by the task's name.
    #[must_use]
    pub fn load(&self) -> HashMap<String, Label> {
        std::fs::read_to_string(&self.file)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Label>(line).ok())
            .map(|label| (label.task.clone(), label))
            .collect()
    }

    /// Keeps `labels`.
    ///
    /// # Errors
    ///
    /// The file could not be written.
    pub fn add(&self, labels: &[Label]) -> std::io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        if labels.is_empty() {
            return Ok(());
        }
        if let Some(parent) = self.file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&self.file)?;
        for label in labels {
            if let Ok(line) = serde_json::to_string(label) {
                writeln!(file, "{line}")?;
            }
        }
        Ok(())
    }
}

/// `task`'s category: by the rules, else as a model placed it, when that
/// is still one of the categories; none when neither places it.
#[must_use]
pub fn category_of(
    task: &Task,
    categories: &Categories,
    labels: &HashMap<String, Label>,
) -> Option<String> {
    if let Some(name) = categories.by_rule(task) {
        return Some(name.to_owned());
    }
    labels
        .get(&task.id())
        .map(|label| label.category.clone())
        .filter(|name| categories.knows(name))
}
