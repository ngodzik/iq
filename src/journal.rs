//! The journal of iq's tasks, kept in its folder: each turn of an agent's,
//! the files it wrote, the commands it ran and what it cost, one line of
//! JSON each, for the assay to say what each kind of task spends. Only
//! what iq's agents did is kept, never what they read or were told beyond
//! the start of their task.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread::JoinHandle;

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
    open: Mutex<(HashMap<String, OpenTask>, HashMap<String, String>)>,
    workspace: PathBuf,
}

impl Journal {
    /// The journal of `folder`, its tasks done in `project`.
    pub(crate) fn open(folder: &Path, project: &str, workspace: &Path) -> Self {
        Self {
            file: folder.join("tasks.jsonl"),
            project: project.to_owned(),
            workspace: workspace.to_owned(),
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
                tasks.entry(agent.clone()).or_insert_with(|| OpenTask {
                    snapshot: {
                        let workspace = self.workspace.clone();
                        std::thread::spawn(move || Snapshot::take(&workspace))
                    },
                    task: Task {
                        at: iq_ui::now(),
                        agent: agent.clone(),
                        model: models.get(agent).cloned().unwrap_or_default(),
                        project: self.project.clone(),
                        prompt: prompt.chars().take(PROMPT_CHARS).collect(),
                        ..Task::default()
                    },
                });
            }
            Event::Tool {
                agent, name, input, ..
            } => {
                if let Some(open) = tasks.get_mut(agent) {
                    let task = &mut open.task;
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
                if let Some(open) = tasks.get_mut(agent) {
                    let task = &mut open.task;
                    task.tokens += usage.input.0 + usage.output.0;
                    task.billed += cost.map_or(0.0, |c| c.0);
                }
            }
            Event::Done { agent, outcome } => {
                if let Some(open) = tasks.remove(agent) {
                    let mut task = open.task;
                    if let Ok(Some(start)) = open.snapshot.join()
                        && let Some(end) = Snapshot::take(&self.workspace)
                    {
                        for path in start.changed(&end, &self.workspace) {
                            if task.files.len() < MOST_KEPT && !task.files.contains(&path) {
                                task.files.push(path);
                            }
                        }
                    }
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
        crate::lock::holding(&self.file, || {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&self.file);
            if let Ok(mut file) = file {
                let _ = writeln!(file, "{line}");
            }
        });
    }

    /// Drops the tasks older than they are kept. Another iq keeping a task
    /// meanwhile waits for it.
    pub(crate) fn trim(&self) {
        crate::lock::holding(&self.file, || self.trim_held());
    }

    fn trim_held(&self) {
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

struct OpenTask {
    task: Task,
    snapshot: JoinHandle<Option<Snapshot>>,
}

struct Snapshot {
    entries: BTreeMap<String, (Vec<u8>, Vec<u8>)>,
    head: Option<String>,
    prefix: String,
}

fn git(workspace: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn hash_file(root: &Path, path: &str) -> Option<Vec<u8>> {
    // Git's quoted input preserves spaces, newlines and non-ASCII path bytes.
    let quoted: String = path.bytes().map(|byte| format!("\\{byte:03o}")).collect();
    let mut child = Command::new("git")
        .current_dir(root)
        .args(["hash-object", "--no-filters", "--stdin-paths"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let written = child
        .stdin
        .take()?
        .write_all(format!("\"{quoted}\"\n").as_bytes());
    let output = child.wait_with_output().ok()?;
    (written.is_ok() && output.status.success()).then_some(output.stdout)
}

impl Snapshot {
    fn take(workspace: &Path) -> Option<Self> {
        let prefix = String::from_utf8(git(workspace, &["rev-parse", "--show-prefix"])?).ok()?;
        let prefix = prefix.trim_end_matches('\n').to_owned();
        let root = String::from_utf8(git(workspace, &["rev-parse", "--show-toplevel"])?).ok()?;
        let root = Path::new(root.trim_end_matches('\n'));
        let status = git(
            workspace,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?;
        let mut entries = BTreeMap::new();
        let mut records = status
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty());
        while let Some(record) = records.next() {
            if record.len() < 4 {
                return None;
            }
            let path = String::from_utf8(record[3..].to_vec()).ok()?;
            let hash = hash_file(root, &path).unwrap_or_default();
            entries.insert(path, (record[..2].to_vec(), hash));
            if record[..2].iter().any(|byte| matches!(byte, b'R' | b'C')) {
                let source = String::from_utf8(records.next()?.to_vec()).ok()?;
                entries.insert(source, (b"D ".to_vec(), Vec::new()));
            }
        }
        let head = git(workspace, &["rev-parse", "--verify", "HEAD"])
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|head| head.trim().to_owned());
        Some(Self {
            entries,
            head,
            prefix,
        })
    }

    fn changed(&self, end: &Self, workspace: &Path) -> BTreeSet<String> {
        let mut paths: BTreeSet<String> = self
            .entries
            .keys()
            .chain(end.entries.keys())
            .filter(|path| self.entries.get(*path) != end.entries.get(*path))
            .cloned()
            .collect();
        if self.head != end.head {
            let diff = match (&self.head, &end.head) {
                (Some(start), Some(end)) => git(
                    workspace,
                    &[
                        "diff",
                        "--name-only",
                        "-z",
                        "--no-renames",
                        &format!("{start}..{end}"),
                        "--",
                    ],
                ),
                (None, Some(end)) => git(
                    workspace,
                    &["ls-tree", "-r", "--full-tree", "--name-only", "-z", end],
                ),
                _ => None,
            };
            if let Some(diff) = diff {
                paths.extend(
                    diff.split(|byte| *byte == 0)
                        .filter(|path| !path.is_empty())
                        .map(|path| String::from_utf8_lossy(path).into_owned()),
                );
            }
        }
        paths
            .into_iter()
            .filter_map(|path| path.strip_prefix(&self.prefix).map(str::to_owned))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iq_core::{TokenCount, Usage, Usd};

    fn run_git(dir: &Path, args: &[&str]) {
        assert!(git(dir, args).is_some(), "git {args:?}");
    }

    fn repository(dir: &Path) {
        run_git(dir, &["init", "-q"]);
        run_git(dir, &["config", "user.name", "Test"]);
        run_git(dir, &["config", "user.email", "test@example.invalid"]);
        run_git(dir, &["config", "commit.gpgsign", "false"]);
    }

    #[test]
    fn a_first_commit_in_a_subdirectory_keeps_new_files() {
        let dir = tempfile::tempdir().unwrap();
        repository(dir.path());
        let workspace = dir.path().join("sub");
        std::fs::create_dir(&workspace).unwrap();
        let start = Snapshot::take(&workspace).unwrap();
        std::fs::write(workspace.join("new"), "new").unwrap();
        std::fs::write(dir.path().join("outside"), "outside").unwrap();
        run_git(dir.path(), &["add", "."]);
        run_git(dir.path(), &["commit", "-qm", "Initial files"]);
        let end = Snapshot::take(&workspace).unwrap();
        assert_eq!(
            start.changed(&end, &workspace),
            ["new".to_owned()].into_iter().collect()
        );
    }

    #[test]
    fn git_tracks_dirty_deleted_renamed_untracked_and_committed_files() {
        let dir = tempfile::tempdir().unwrap();
        repository(dir.path());
        for name in ["dirty", "unchanged", "deleted", "renamed", "committed"] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        run_git(dir.path(), &["add", "."]);
        run_git(dir.path(), &["commit", "-qm", "Initial files"]);
        std::fs::write(dir.path().join("dirty"), "before").unwrap();
        std::fs::write(dir.path().join("unchanged"), "before").unwrap();
        let start = Snapshot::take(dir.path()).unwrap();
        std::fs::write(dir.path().join("committed"), "after").unwrap();
        run_git(dir.path(), &["add", "committed"]);
        run_git(dir.path(), &["commit", "-qm", "Change a file"]);
        std::fs::write(dir.path().join("dirty"), "after").unwrap();
        std::fs::remove_file(dir.path().join("deleted")).unwrap();
        run_git(dir.path(), &["mv", "renamed", "new name"]);
        std::fs::write(dir.path().join("new\nfile"), "new").unwrap();
        let end = Snapshot::take(dir.path()).unwrap();
        assert_eq!(
            start.changed(&end, dir.path()),
            [
                "committed",
                "deleted",
                "dirty",
                "new\nfile",
                "new name",
                "renamed"
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );
    }

    #[test]
    fn journal_merges_git_changes_and_tool_writes_relative_to_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let logs = tempfile::tempdir().unwrap();
        repository(dir.path());
        let workspace = dir.path().join("sub");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("file"), "before").unwrap();
        run_git(dir.path(), &["add", "."]);
        run_git(dir.path(), &["commit", "-qm", "Initial files"]);
        let journal = Journal::open(logs.path(), "project", &workspace);
        journal.event(&Event::Working {
            agent: "lead".into(),
            prompt: "Work".into(),
        });
        // The worker must finish its baseline before this test changes the checkout.
        while !journal.open.lock().unwrap().0["lead"]
            .snapshot
            .is_finished()
        {
            std::thread::yield_now();
        }
        std::fs::write(workspace.join("file"), "after").unwrap();
        std::fs::write(dir.path().join("outside"), "outside").unwrap();
        run_git(dir.path(), &["add", "."]);
        run_git(dir.path(), &["commit", "-qm", "Task changes"]);
        journal.event(&Event::Tool {
            agent: "lead".into(),
            name: "edit_file".into(),
            input: "tool-file".into(),
            output: "Edited".into(),
            ok: true,
        });
        let task = journal
            .event(&Event::Done {
                agent: "lead".into(),
                outcome: Ok("done".into()),
            })
            .unwrap();
        assert_eq!(task.files, ["tool-file", "file"]);
    }

    #[test]
    fn a_turn_is_kept_with_its_files_commands_and_costs() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::open(dir.path(), "github.com/org/repo", dir.path());
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
