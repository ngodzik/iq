//! The journal of iq's tasks, kept in its folder: each turn of an agent's,
//! the files it wrote, the commands it ran and what it cost, one line of
//! JSON each, for the observations to say what each kind of task spends. Only
//! what iq's agents did is kept, never what they read or were told beyond
//! the start of their task. In a repository, a task's files are also what
//! git saw change while it worked, so that the journal shows what the
//! person will find, whatever wrote it: a tool, a command, a commit, another
//! branch checked out; in the place the agent works in, when it was given
//! one of its own.

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
    /// Where each agent works, when not in the workspace.
    places: Mutex<HashMap<String, PathBuf>>,
}

impl Journal {
    /// The journal of `folder`, its tasks done in `project`.
    pub(crate) fn open(folder: &Path, project: &str, workspace: &Path) -> Self {
        Self {
            file: folder.join("tasks.jsonl"),
            project: project.to_owned(),
            workspace: workspace.to_owned(),
            open: Mutex::new((HashMap::new(), HashMap::new())),
            places: Mutex::new(HashMap::new()),
        }
    }

    /// Where `agent` works: its own place, else the workspace.
    fn place_of(&self, agent: &str) -> PathBuf {
        self.places
            .lock()
            .ok()
            .and_then(|places| places.get(agent).cloned())
            .unwrap_or_else(|| self.workspace.clone())
    }

    /// Where it is kept.
    pub(crate) fn file(&self) -> &Path {
        &self.file
    }

    /// Notes what `event` says of a task, and keeps the task once its turn
    /// ends, which it gives.
    pub(crate) fn event(&self, event: &Event) -> Option<Task> {
        if let Event::Done { agent, outcome } = event {
            // Git reads the checkout with the journal let go: the other
            // agents' events do not wait for it.
            let open = self.open.lock().ok()?.0.remove(agent)?;
            let mut task = open.task;
            let place = self.place_of(agent);
            if let Ok(Some(start)) = open.snapshot.join()
                && let Some(end) = Snapshot::take(&place)
            {
                for path in start.changed(&end, &place) {
                    if task.files.len() < MOST_KEPT && !task.files.contains(&path) {
                        task.files.push(path);
                    }
                }
            }
            task.seconds = iq_ui::now().saturating_sub(task.at);
            task.ok = outcome.is_ok();
            task.why = outcome
                .as_ref()
                .err()
                .map(|why| why.chars().take(PROMPT_CHARS).collect());
            self.keep(&task);
            return Some(task);
        }
        let Ok(mut open) = self.open.lock() else {
            return None;
        };
        let (tasks, models) = &mut *open;
        match event {
            Event::Joined { agent, model, .. } => {
                models.insert(agent.clone(), model.clone());
            }
            Event::Placed { agent, folder, .. } => {
                if let Ok(mut places) = self.places.lock() {
                    places.insert(agent.clone(), folder.clone());
                }
            }
            Event::Working { agent, prompt } => {
                tasks.entry(agent.clone()).or_insert_with(|| OpenTask {
                    snapshot: {
                        let place = self.place_of(agent);
                        std::thread::spawn(move || Snapshot::take(&place))
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

/// A task under way: what it did so far, and the state of the checkout
/// as it started, read on a thread of its own.
struct OpenTask {
    task: Task,
    snapshot: JoinHandle<Option<Snapshot>>,
}

/// The state of a checkout: each file git lists as changed, with its
/// status and what it holds, the commit checked out, and where the
/// workspace is within the repository.
struct Snapshot {
    entries: BTreeMap<String, (Vec<u8>, Vec<u8>)>,
    head: Option<String>,
    prefix: String,
}

/// A git command in `folder`, as git itself runs hooks: none of the
/// variables a hook is given, which would point it at another repository's
/// index, and no lock taken that the person's own git would wait for.
pub(crate) fn git_command(folder: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(folder)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    command
}

/// What `git args` prints in `workspace`, when it succeeds.
fn git(workspace: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = git_command(workspace)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// What each of `paths` holds, by git's hash, in one call however many
/// there are: a large checkout lists thousands. Paths that are not a file
/// are given an empty hash.
fn hashes(root: &Path, paths: &[&String]) -> Vec<Vec<u8>> {
    let files: Vec<&String> = paths
        .iter()
        .copied()
        .filter(|path| root.join(path).is_file())
        .collect();
    let mut hashed: HashMap<&String, Vec<u8>> = HashMap::new();
    if !files.is_empty() {
        // Quoted as git reads them: spaces, line ends and any byte kept.
        let input: String = files
            .iter()
            .map(|path| {
                let quoted: String = path.bytes().map(|byte| format!("\\{byte:03o}")).collect();
                format!("\"{quoted}\"\n")
            })
            .collect();
        let child = git_command(root)
            .args(["hash-object", "--no-filters", "--stdin-paths"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            // Written on a thread of its own: git answers as it reads, and
            // a pipe full both ways would hold them both.
            let writer = child
                .stdin
                .take()
                .map(|mut stdin| std::thread::spawn(move || stdin.write_all(input.as_bytes())));
            let output = child.wait_with_output();
            let written = writer.is_some_and(|writer| writer.join().is_ok_and(|w| w.is_ok()));
            if let Ok(output) = output
                && written
                && output.status.success()
            {
                let lines: Vec<&[u8]> = output
                    .stdout
                    .split(|byte| *byte == b'\n')
                    .filter(|line| !line.is_empty())
                    .collect();
                if lines.len() == files.len() {
                    hashed = files
                        .into_iter()
                        .zip(lines)
                        .map(|(path, hash)| (path, hash.to_vec()))
                        .collect();
                }
            }
        }
    }
    paths
        .iter()
        .map(|path| hashed.get(path).cloned().unwrap_or_default())
        .collect()
}

impl Snapshot {
    /// The checkout `workspace` is in, as it is now; `None` outside a
    /// repository.
    fn take(workspace: &Path) -> Option<Self> {
        let prefix = String::from_utf8(git(workspace, &["rev-parse", "--show-prefix"])?).ok()?;
        let prefix = prefix.trim_end_matches('\n').to_owned();
        let root = String::from_utf8(git(workspace, &["rev-parse", "--show-toplevel"])?).ok()?;
        let root = Path::new(root.trim_end_matches('\n'));
        let status = git(
            workspace,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?;
        let mut listed = Vec::new();
        let mut sources = Vec::new();
        let mut records = status
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty());
        while let Some(record) = records.next() {
            if record.len() < 4 {
                return None;
            }
            let path = String::from_utf8(record[3..].to_vec()).ok()?;
            listed.push((path, record[..2].to_vec()));
            if record[..2].iter().any(|byte| matches!(byte, b'R' | b'C')) {
                sources.push(String::from_utf8(records.next()?.to_vec()).ok()?);
            }
        }
        let paths: Vec<&String> = listed.iter().map(|(path, _)| path).collect();
        let hashed = hashes(root, &paths);
        let mut entries: BTreeMap<String, (Vec<u8>, Vec<u8>)> = listed
            .into_iter()
            .zip(hashed)
            .map(|((path, status), hash)| (path, (status, hash)))
            .collect();
        for source in sources {
            entries.insert(source, (b"D ".to_vec(), Vec::new()));
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

    /// The files that changed from this state to `end`, relative to the
    /// workspace: those whose status or content changed, and those the
    /// commits between them changed. Files outside the workspace are left
    /// out.
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
    fn an_agent_given_a_place_of_its_own_has_its_changes_read_there() {
        let dir = tempfile::tempdir().unwrap();
        let logs = tempfile::tempdir().unwrap();
        repository(dir.path());
        std::fs::write(dir.path().join("file"), "before").unwrap();
        run_git(dir.path(), &["add", "."]);
        run_git(dir.path(), &["commit", "-qm", "Initial files"]);
        let place = tempfile::tempdir().unwrap();
        let worktree = place.path().join("lead");
        run_git(
            dir.path(),
            &["worktree", "add", "-q", worktree.to_str().unwrap()],
        );
        let journal = Journal::open(logs.path(), "project", dir.path());
        journal.event(&Event::Placed {
            agent: "lead".into(),
            folder: worktree.clone(),
            said: Some("works apart".into()),
        });
        journal.event(&Event::Working {
            agent: "lead".into(),
            prompt: "Work".into(),
        });
        while !journal.open.lock().unwrap().0["lead"]
            .snapshot
            .is_finished()
        {
            std::thread::yield_now();
        }
        std::fs::write(worktree.join("file"), "after").unwrap();
        std::fs::write(worktree.join("new"), "new").unwrap();
        // The person's own checkout changes too, which is not the task's.
        std::fs::write(dir.path().join("theirs"), "theirs").unwrap();
        let task = journal
            .event(&Event::Done {
                agent: "lead".into(),
                outcome: Ok("done".into()),
            })
            .unwrap();
        assert_eq!(task.files, ["file", "new"]);
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
        assert!(task.why.is_none());
        assert!((task.billed - 0.01).abs() < 1e-12);
        journal.trim();
        assert_eq!(iq_observe::read(journal.file(), 0).len(), 1);
        // A turn that ends without an answer keeps why.
        journal.event(&Event::Working {
            agent: "lead".into(),
            prompt: "again".into(),
        });
        journal.event(&Event::Done {
            agent: "lead".into(),
            outcome: Err("the provider answered HTTP 500".into()),
        });
        let tasks = iq_observe::read(journal.file(), 0);
        assert_eq!(tasks.len(), 2);
        assert!(!tasks[1].ok);
        assert_eq!(
            tasks[1].why.as_deref(),
            Some("the provider answered HTTP 500")
        );
    }
}
