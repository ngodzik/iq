//! The journal of iq's tasks, kept in its folder: each turn of an agent's,
//! the files it wrote, the commands it ran and what it cost, one line of
//! JSON each, for the assay to say what each kind of task spends. Only
//! what iq's agents did is kept, never what they read or were told beyond
//! the start of their task.

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use iq_agent::Event;
use iq_observe::Task;

/// How long tasks are kept: a year and some, for the observations to look
/// back over a year.
const KEEP_SECS: u64 = 400 * 24 * 60 * 60;

/// How much of a task's prompt is kept.
const PROMPT_CHARS: usize = 300;

/// How many files and commands of one task are kept.
const MOST_KEPT: usize = 60;

/// The tools that write files, as iq's agents name them.
const WRITING: &[&str] = &["write_file", "edit_file"];

/// The tools that run commands.
const RUNNING: &[&str] = &["run"];

/// The journal.
pub(crate) struct Journal {
    file: PathBuf,
    /// The project iq works in, as its memory is kept under.
    project: String,
    /// The tasks under way, by agent, and each agent's model.
    open: Mutex<(HashMap<String, Task>, HashMap<String, String>)>,
}

impl Journal {
    /// The journal of `folder`, its tasks done in `project`.
    pub(crate) fn open(folder: &Path, project: &str) -> Self {
        Self {
            file: folder.join("tasks.jsonl"),
            project: project.to_owned(),
            open: Mutex::new((HashMap::new(), HashMap::new())),
        }
    }

    /// Where it is kept.
    pub(crate) fn file(&self) -> &Path {
        &self.file
    }

    /// Notes what `event` says of a task, and keeps the task once its turn
    /// ends, which it gives.
    pub(crate) fn event(&self, event: &Event) -> Option<Task> {
        let Ok(mut open) = self.open.lock() else {
            return None;
        };
        let (tasks, models) = &mut *open;
        match event {
            Event::Joined { agent, model, .. } => {
                models.insert(agent.clone(), model.clone());
            }
            Event::Working { agent, prompt } => {
                tasks.entry(agent.clone()).or_insert_with(|| Task {
                    at: iq_ui::now(),
                    agent: agent.clone(),
                    model: models.get(agent).cloned().unwrap_or_default(),
                    project: self.project.clone(),
                    prompt: prompt.chars().take(PROMPT_CHARS).collect(),
                    ..Task::default()
                });
            }
            Event::Tool {
                agent, name, input, ..
            } => {
                if let Some(task) = tasks.get_mut(agent) {
                    let kept = if WRITING.contains(&name.as_str()) {
                        Some(&mut task.files)
                    } else if RUNNING.contains(&name.as_str()) {
                        Some(&mut task.commands)
                    } else {
                        None
                    };
                    if let Some(kept) = kept
                        && kept.len() < MOST_KEPT
                        && !kept.contains(input)
                    {
                        kept.push(input.chars().take(160).collect());
                    }
                }
            }
            Event::Spent {
                agent, usage, cost, ..
            } => {
                if let Some(task) = tasks.get_mut(agent) {
                    task.tokens += usage.input.0 + usage.output.0;
                    task.billed += cost.map_or(0.0, |c| c.0);
                }
            }
            Event::Done { agent, outcome } => {
                if let Some(mut task) = tasks.remove(agent) {
                    task.seconds = iq_ui::now().saturating_sub(task.at);
                    task.ok = outcome.is_ok();
                    self.keep(&task);
                    return Some(task);
                }
            }
            _ => {}
        }
        None
    }

    fn keep(&self, task: &Task) {
        let Ok(line) = serde_json::to_string(task) else {
            return;
        };
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&self.file);
        if let Ok(mut file) = file {
            let _ = writeln!(file, "{line}");
        }
    }

    /// Drops the tasks older than they are kept.
    pub(crate) fn trim(&self) {
        let since = iq_ui::now().saturating_sub(KEEP_SECS);
        let Ok(text) = std::fs::read_to_string(&self.file) else {
            return;
        };
        let kept: Vec<&str> = text
            .lines()
            .filter(|line| serde_json::from_str::<Task>(line).is_ok_and(|task| task.at >= since))
            .collect();
        if kept.len() == text.lines().count() {
            return;
        }
        let mut out = kept.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = std::fs::write(&self.file, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iq_core::{TokenCount, Usage, Usd};

    #[test]
    fn a_turn_is_kept_with_its_files_commands_and_costs() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::open(dir.path(), "github.com/org/repo");
        let events = [
            Event::Joined {
                agent: "lead".into(),
                model: "z-ai/glm-5".into(),
                by: "person".into(),
            },
            Event::Working {
                agent: "lead".into(),
                prompt: "Add a mul function".into(),
            },
            Event::Tool {
                agent: "lead".into(),
                name: "edit_file".into(),
                input: "calc.py".into(),
                output: "Edited.".into(),
                ok: true,
            },
            Event::Tool {
                agent: "lead".into(),
                name: "run".into(),
                input: "pytest -q".into(),
                output: "ok".into(),
                ok: true,
            },
            Event::Tool {
                agent: "lead".into(),
                name: "read_file".into(),
                input: "README.md".into(),
                output: "…".into(),
                ok: true,
            },
            Event::Spent {
                agent: "lead".into(),
                model: "z-ai/glm-5".into(),
                usage: Usage {
                    input: TokenCount(900),
                    output: TokenCount(100),
                },
                cost: Some(Usd(0.01)),
            },
            Event::Done {
                agent: "lead".into(),
                outcome: Ok("done".into()),
            },
        ];
        let ended: Vec<Task> = events.iter().filter_map(|e| journal.event(e)).collect();
        assert_eq!(ended.len(), 1);
        let tasks = iq_observe::read(journal.file(), 0);
        assert_eq!(tasks.len(), 1);
        let task = &tasks[0];
        assert_eq!(task.model, "z-ai/glm-5");
        assert_eq!(task.files, ["calc.py"]);
        assert_eq!(task.commands, ["pytest -q"]);
        assert_eq!(task.tokens, 1_000);
        assert_eq!(task.project, "github.com/org/repo");
        assert!(task.ok);
        assert!((task.billed - 0.01).abs() < 1e-12);
        journal.trim();
        assert_eq!(iq_observe::read(journal.file(), 0).len(), 1);
    }
}
