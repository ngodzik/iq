//! The kinds of task the person names in their settings, each with what
//! it covers and, if they wish, rules iq applies itself: the files a task
//! touches, the commands it runs, the words it is given.
//!
//! ```toml
//! [[observe.category]]
//! name = "Python"
//! description = "code in Python: the back end, the scripts"
//! files = ["*.py", "pyproject.toml"]
//! commands = ["pytest", "uv"]
//! words = ["django"]
//! ```

use serde::{Deserialize, Serialize};

use crate::task::Task;

/// What a task no kind fits is placed in.
pub const OTHER: &str = "other";

/// One category of task.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    /// Its name.
    pub name: String,
    /// What it covers, for the model that places tasks.
    #[serde(default)]
    pub description: String,
    /// Patterns of the files a task of it touches: `*` for any letters but
    /// `/`, `**` for any, matched against the path or its name.
    #[serde(default)]
    pub files: Vec<String>,
    /// The starts of the commands it runs, as `git rebase`.
    #[serde(default)]
    pub commands: Vec<String>,
    /// Words found in what it was given, in any case.
    #[serde(default)]
    pub words: Vec<String>,
}

/// The categories of task, in the order their rules are tried.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Categories {
    /// The categories.
    pub categories: Vec<Category>,
}

impl Categories {
    /// The kinds named.
    #[must_use]
    pub fn new(categories: Vec<Category>) -> Self {
        Self { categories }
    }

    /// Whether the person named none: nothing is placed then.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.categories.is_empty()
    }

    /// Whether `name` is one of the kinds, or [`OTHER`].
    #[must_use]
    pub fn knows(&self, name: &str) -> bool {
        name == OTHER || self.categories.iter().any(|category| category.name == name)
    }

    /// The kind `task` is of by the rules, if one places it: the first
    /// one of whose rules it meets.
    #[must_use]
    pub fn by_rule(&self, task: &Task) -> Option<&str> {
        let prompt = task.prompt.to_lowercase();
        self.categories
            .iter()
            .find(|kind| {
                kind.files
                    .iter()
                    .any(|pattern| task.files.iter().any(|file| matches(pattern, file)))
                    || kind
                        .commands
                        .iter()
                        .any(|start| task.commands.iter().any(|command| starts(command, start)))
                    || kind.words.iter().any(|word| {
                        !word.trim().is_empty() && prompt.contains(&word.to_lowercase())
                    })
            })
            .map(|kind| kind.name.as_str())
    }
}

impl Categories {
    /// The kind a task given as `text` is of by the rules, before it ran:
    /// its words, and the files it names.
    ///
    /// # Examples
    ///
    /// ```
    /// use iq_observe::{Categories, Category};
    ///
    /// let python = Category {
    ///     name: "python".into(),
    ///     files: vec!["*.py".into()],
    ///     ..Category::default()
    /// };
    /// let kinds = Categories::new(vec![python]);
    /// assert_eq!(kinds.of_text("Fix add() in calc.py"), Some("python"));
    /// assert_eq!(kinds.of_text("Rebase the branch"), None);
    /// ```
    #[must_use]
    pub fn of_text(&self, text: &str) -> Option<&str> {
        let files: Vec<String> = text
            .split_whitespace()
            .map(|word| {
                word.trim_matches(|c: char| !(c.is_alphanumeric() || "/._-".contains(c)))
                    .trim_end_matches('.')
                    .to_owned()
            })
            .filter(|word| {
                word.rsplit_once('.').is_some_and(|(stem, ext)| {
                    !stem.is_empty() && !ext.is_empty() && ext.chars().all(char::is_alphanumeric)
                })
            })
            .collect();
        let task = Task {
            prompt: text.to_owned(),
            files,
            ..Task::default()
        };
        self.by_rule(&task)
    }
}

/// Whether `command` starts with the words of `start`.
fn starts(command: &str, start: &str) -> bool {
    let mut words = command.split_whitespace();
    !start.trim().is_empty()
        && start.split_whitespace().all(|word| {
            words
                .next()
                .is_some_and(|w| w == word || w.ends_with(&format!("/{word}")))
        })
}

/// Whether `path`, or its name, matches `pattern`, as a shell reads it.
fn matches(pattern: &str, path: &str) -> bool {
    iq_core::glob_matches(pattern, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(prompt: &str, files: &[&str], commands: &[&str]) -> Task {
        Task {
            prompt: prompt.into(),
            files: files.iter().map(|f| (*f).to_owned()).collect(),
            commands: commands.iter().map(|c| (*c).to_owned()).collect(),
            ..Task::default()
        }
    }

    #[test]
    fn a_task_is_of_the_first_kind_whose_rules_it_meets() {
        let kind = |name: &str, files: &[&str], commands: &[&str], words: &[&str]| Category {
            name: name.into(),
            files: files.iter().map(|f| (*f).to_owned()).collect(),
            commands: commands.iter().map(|c| (*c).to_owned()).collect(),
            words: words.iter().map(|w| (*w).to_owned()).collect(),
            ..Category::default()
        };
        let kinds = Categories::new(vec![
            kind("Rebase", &[], &["git rebase"], &[]),
            kind("Python", &["*.py", "**/pyproject.toml"], &[], &["Django"]),
            kind("Front end", &["src/**/*.tsx"], &[], &[]),
            kind("Reviews", &[], &[], &[]),
        ]);
        let by = |t: &Task| kinds.by_rule(t).map(str::to_owned);
        assert_eq!(
            by(&task("fix it", &["app/models.py"], &["pytest -q"])).as_deref(),
            Some("Python")
        );
        assert_eq!(
            by(&task("rebase", &["a.py"], &["git rebase main"])).as_deref(),
            Some("Rebase")
        );
        assert_eq!(
            by(&task("a django view", &[], &[])).as_deref(),
            Some("Python")
        );
        assert_eq!(
            by(&task("the page", &["src/pages/home/Home.tsx"], &[])).as_deref(),
            Some("Front end")
        );
        // A kind without rules is the model's to place.
        assert_eq!(by(&task("review the PR", &["README.md"], &["ls"])), None);
        assert!(kinds.knows("Reviews") && kinds.knows(OTHER) && !kinds.knows("Go"));
    }

    #[test]
    fn patterns_read_as_a_shell_reads_them() {
        assert!(matches("*.py", "a/b/c.py"));
        assert!(!matches("src/*.py", "src/a/b.py"));
        assert!(matches("src/**/*.py", "src/a/b.py"));
        assert!(matches("src/**/*.py", "src/b.py"));
        assert!(matches("test_?.py", "test_a.py"));
        assert!(starts("/usr/bin/git rebase -i", "git rebase"));
        assert!(!starts("git status", "git rebase"));
    }
}
