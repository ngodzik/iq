//! What iq's agents spent, by kind of task. iq keeps each task an agent
//! worked on: the files it touched, the commands it ran, the words it was
//! given, and what it cost. The kinds are the person's own, which the judge
//! writes with them in `categories.toml` as rules iq applies itself; the
//! code knows none of them.
//!
//! ```toml
//! [[category]]
//! name = "Python"
//! files = ["*.py", "pyproject.toml"]
//! commands = ["pytest", "uv"]
//! words = ["django"]
//! ```
//!
//! A task is of the first kind one of whose rules it meets: a file it
//! touched matches a pattern, a command it ran starts with one, or its
//! words hold one. A task no rule places is of none, and is shown so.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// One task an agent of iq's worked on, as the journal keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// When it started, in seconds since 1970.
    pub at: u64,
    /// The agent.
    pub agent: String,
    /// Its model.
    pub model: String,
    /// The start of what it was given.
    pub prompt: String,
    /// The files it wrote or edited, as its tools named them.
    #[serde(default)]
    pub files: Vec<String>,
    /// The commands it ran, their first words.
    #[serde(default)]
    pub commands: Vec<String>,
    /// What the provider billed it, in dollars.
    #[serde(default)]
    pub billed: f64,
    /// What its calls under a subscription would cost at list prices.
    #[serde(default)]
    pub listed: f64,
    /// The tokens it read and wrote.
    #[serde(default)]
    pub tokens: u64,
    /// How long it worked, in seconds.
    #[serde(default)]
    pub seconds: u64,
}

/// One kind of task, as `categories.toml` writes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    /// Its name.
    pub name: String,
    /// Patterns of the files a task of it touches: `*` for any letters
    /// but `/`, `**` for any, matched against the path or its name.
    #[serde(default)]
    pub files: Vec<String>,
    /// The starts of the commands it runs, as `git rebase`.
    #[serde(default)]
    pub commands: Vec<String>,
    /// Words found in what it was given, in any case.
    #[serde(default)]
    pub words: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    #[serde(default)]
    category: Vec<Category>,
}

/// The kinds of task, in the order they are tried.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules {
    /// The kinds.
    pub categories: Vec<Category>,
}

impl Rules {
    /// The rules `text` writes, `categories.toml`'s.
    ///
    /// # Errors
    ///
    /// What is wrong with them.
    pub fn parse(text: &str) -> Result<Self, String> {
        let written: Written = toml::from_str(text).map_err(|e| e.to_string())?;
        Ok(Self {
            categories: written.category,
        })
    }

    /// The kind `task` is of, if a rule places it.
    #[must_use]
    pub fn classify(&self, task: &Task) -> Option<&str> {
        let prompt = task.prompt.to_lowercase();
        self.categories
            .iter()
            .find(|category| {
                category
                    .files
                    .iter()
                    .any(|pattern| task.files.iter().any(|file| matches(pattern, file)))
                    || category
                        .commands
                        .iter()
                        .any(|start| task.commands.iter().any(|command| starts(command, start)))
                    || category.words.iter().any(|word| {
                        !word.trim().is_empty() && prompt.contains(&word.to_lowercase())
                    })
            })
            .map(|category| category.name.as_str())
    }
}

/// Whether `command` starts with the words of `start`.
fn starts(command: &str, start: &str) -> bool {
    let mut words = command.split_whitespace();
    start.split_whitespace().all(|word| {
        words
            .next()
            .is_some_and(|w| w == word || w.ends_with(&format!("/{word}")))
    }) && !start.trim().is_empty()
}

/// Whether `path`, or its name, matches `pattern`: `*` any letters but
/// `/`, `**` any, `?` one.
fn matches(pattern: &str, path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    glob(pattern.as_bytes(), path.as_bytes())
        || (!pattern.contains('/') && glob(pattern.as_bytes(), name.as_bytes()))
}

fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match pattern {
        [] => text.is_empty(),
        [b'*', b'*', rest @ ..] => {
            let rest = rest.strip_prefix(b"/").unwrap_or(rest);
            (0..=text.len()).any(|at| glob(rest, &text[at..]))
        }
        [b'*', rest @ ..] => (0..=text.len())
            .take_while(|at| *at == 0 || text[at - 1] != b'/')
            .any(|at| glob(rest, &text[at..])),
        [b'?', rest @ ..] => !text.is_empty() && text[0] != b'/' && glob(rest, &text[1..]),
        [c, rest @ ..] => text.first() == Some(c) && glob(rest, &text[1..]),
    }
}

/// The tasks of the journal at `file` since `since`, oldest first.
#[must_use]
pub fn read(file: &Path, since: u64) -> Vec<Task> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Task>(line).ok())
        .filter(|task| task.at >= since)
        .collect()
}

/// What one kind of task spent.
#[derive(Debug, Clone, PartialEq)]
pub struct Spent {
    /// The kind, `None` for the tasks no rule places.
    pub category: Option<String>,
    /// How many tasks.
    pub tasks: usize,
    /// What the provider billed them.
    pub billed: f64,
    /// What their calls under a subscription would cost at list prices.
    pub listed: f64,
    /// The tokens they read and wrote.
    pub tokens: u64,
}

/// What each kind of task spent among `tasks`, the most first, those of
/// no kind last.
#[must_use]
pub fn spent(tasks: &[Task], rules: &Rules) -> Vec<Spent> {
    let mut sums: Vec<Spent> = Vec::new();
    for task in tasks {
        let category = rules.classify(task).map(str::to_owned);
        let at = match sums.iter().position(|s| s.category == category) {
            Some(at) => at,
            None => {
                sums.push(Spent {
                    category: category.clone(),
                    tasks: 0,
                    billed: 0.0,
                    listed: 0.0,
                    tokens: 0,
                });
                sums.len() - 1
            }
        };
        let sum = &mut sums[at];
        sum.tasks += 1;
        sum.billed += task.billed;
        sum.listed += task.listed;
        sum.tokens += task.tokens;
    }
    sums.sort_by(|a, b| {
        a.category
            .is_none()
            .cmp(&b.category.is_none())
            .then((b.billed + b.listed).total_cmp(&(a.billed + a.listed)))
    });
    sums
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(prompt: &str, files: &[&str], commands: &[&str], billed: f64) -> Task {
        Task {
            at: 1,
            agent: "lead".into(),
            model: "m".into(),
            prompt: prompt.into(),
            files: files.iter().map(|f| (*f).to_owned()).collect(),
            commands: commands.iter().map(|c| (*c).to_owned()).collect(),
            billed,
            listed: 0.0,
            tokens: 10,
            seconds: 5,
        }
    }

    #[test]
    fn tasks_are_placed_by_the_first_rule_they_meet() {
        let rules = Rules::parse(
            r#"
[[category]]
name = "Rebase"
commands = ["git rebase"]

[[category]]
name = "Python"
files = ["*.py", "**/pyproject.toml"]
words = ["Django"]

[[category]]
name = "Front end"
files = ["src/**/*.tsx"]
"#,
        )
        .unwrap();
        let python = task("fix it", &["app/models.py"], &["pytest -q"], 0.2);
        let rebase = task(
            "rebase on main",
            &["app/models.py"],
            &["git rebase main"],
            0.1,
        );
        let worded = task("a django view", &[], &[], 0.05);
        let front = task("the page", &["src/pages/home/Home.tsx"], &[], 0.3);
        let none = task("hello", &["README.md"], &["ls"], 0.01);
        assert_eq!(rules.classify(&python), Some("Python"));
        assert_eq!(rules.classify(&rebase), Some("Rebase"));
        assert_eq!(rules.classify(&worded), Some("Python"));
        assert_eq!(rules.classify(&front), Some("Front end"));
        assert_eq!(rules.classify(&none), None);
        let sums = spent(&[python, rebase, worded, front, none], &rules);
        let names: Vec<Option<&str>> = sums.iter().map(|s| s.category.as_deref()).collect();
        assert_eq!(
            names,
            [Some("Front end"), Some("Python"), Some("Rebase"), None]
        );
        assert_eq!(sums[1].tasks, 2);
        assert!(Rules::parse("[[category]]\nnom = \"x\"").is_err());
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
