//! iq's own files kept in a git repository the person chose, a private one
//! of theirs: the projects' memory, a copy of the settings. The
//! folder they live in is a git repository; a sync commits what changed
//! here, takes what changed elsewhere, and sends the rest. Git decides,
//! never a model, with three rules:
//!
//! - a machine new to the repository takes what is there before it sends
//!   anything, so that it never overwrites the others with its emptiness;
//! - what changed on one side only is taken or sent;
//! - where both sides changed the same lines, this machine's win, the
//!   others' staying in the history.
//!
//! Git reaches the repository with what the machine already has: its own
//! credentials, or GitHub's command line when it is signed in. Nothing
//! asks: a sync that cannot reach the repository fails and says why.
//!
//! Beside iq's own files, the person may keep any file or folder of their
//! home there, as their settings list them: a new machine, taking the
//! settings with the rest, puts each file back in its place. What holds
//! secrets is never kept, as the gate tells it; in a folder of credentials
//! of the home, such as `~/.claude`, only the Markdown files are: the
//! rules and notes written there. Nor what is too large, nor what is
//! rebuilt. A sync never removes a file of the home: what another machine
//! removed stays in the history.
//!
//! What a sync would send is listed before, for the person to say whether
//! it goes; a machine new to the repository takes what is there first, and
//! then is asked as any other. What the folder's `.gitignore` leaves out is
//! not kept, nor what was sent of it before: the store of the assay iq
//! once had, among it.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;
use tokio::sync::Mutex;

/// The branch the files are kept on unless the person says another, as
/// when their organisation holds its repositories' default branches to
/// pull requests.
pub const BRANCH: &str = "main";

/// What a sync leaves out: what only this machine needs, or what is built
/// again from the rest; and the store of the assay iq once had, which it
/// reads no more.
const IGNORED: &str = "# What only this machine needs, or what is built again from the rest.
*.before-sync
# The worktrees the agents work in, which their repositories keep.
worktrees/
# The assay iq once had, which it reads no more.
assay/
";

/// What the `.gitignore` once said, which the store's own line now covers.
const IGNORED_ONCE: [&str; 2] = ["assay/runs/*/*/*/work/", "assay/judging/"];

/// Where they are kept in the folder.
const KEPT: &str = "home";

/// The largest file kept, in bytes: a repository of notes and settings,
/// not of archives.
const KEPT_MOST: u64 = 5 << 20;

/// What is left out of a folder kept: what tools rebuild or fetch.
const LEFT_OUT: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
];

/// How many times a sync takes what was sent meanwhile and tries again,
/// when another machine sent between its fetch and its push.
const ATTEMPTS: usize = 3;

/// The most lines of what a sync would send that are shown.
pub const DIFF_LINES: usize = 4000;

/// One change a sync would send.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Change {
    /// A file copied in from where it lives, which differs from its copy:
    /// changed there, new there, or gone from there.
    Copied {
        /// Its name, as the repository names it.
        name: String,
        /// Where it lives.
        outside: PathBuf,
        /// Its copy in the folder.
        inside: PathBuf,
    },
    /// A file of the folder the repository holds, changed or removed.
    Own {
        /// Its name.
        name: String,
    },
    /// A file of the folder the repository does not hold yet.
    New {
        /// Its name.
        name: String,
    },
    /// A file sent before that the folder's `.gitignore` now leaves out,
    /// which goes from the repository.
    LeftOut {
        /// Its name.
        name: String,
    },
}

impl Change {
    /// Its name, as the repository names it.
    fn name(&self) -> &str {
        match self {
            Change::Copied { name, .. }
            | Change::Own { name }
            | Change::New { name }
            | Change::LeftOut { name } => name,
        }
    }
}

/// A diff of two files made with `--no-index`, which names them by their
/// paths, named `name` as the repository does: its header lines rewritten,
/// as far as its first hunk, `/dev/null` where a side is not there.
fn headed(text: &str, name: &str, from_there: bool, to_there: bool) -> String {
    let mut out = String::new();
    let mut header = false;
    for line in text.lines() {
        if line.starts_with("diff --git ") {
            header = true;
            out.push_str(&format!("diff --git a/{name} b/{name}\n"));
            continue;
        }
        if header && (line.starts_with("@@") || line.starts_with("Binary files ")) {
            header = false;
            if line.starts_with("Binary files ") {
                out.push_str(&format!("Binary files a/{name} and b/{name} differ\n"));
                continue;
            }
        }
        if header && line.starts_with("--- ") {
            let from = if from_there {
                format!("a/{name}")
            } else {
                "/dev/null".to_owned()
            };
            out.push_str(&format!("--- {from}\n"));
            continue;
        }
        if header && line.starts_with("+++ ") {
            let to = if to_there {
                format!("b/{name}")
            } else {
                "/dev/null".to_owned()
            };
            out.push_str(&format!("+++ {to}\n"));
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// What can go wrong with a sync.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// Git said no.
    #[error("git {command} failed: {said}")]
    Git {
        /// What was asked of git.
        command: String,
        /// What git said.
        said: String,
    },
    /// What was asked cannot be done.
    #[error("{0}")]
    Refused(String),
    /// A file could not be read or written.
    #[error("{path}")]
    Io {
        /// The file.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> SyncError + '_ {
    move |source| SyncError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// A file or a folder kept outside the folder, copied in to be synced and
/// out once synced, as the settings and the skills are, which iq reads from
/// its own folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mirrored {
    /// Where it lives.
    pub outside: PathBuf,
    /// Where its copy is kept in the folder, relative to it.
    pub inside: String,
}

/// What a sync did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Whether this machine was new to the repository, and took what was
    /// there.
    pub fresh: bool,
    /// The files changed here and sent.
    pub sent: Vec<String>,
    /// The files changed elsewhere and taken.
    pub taken: Vec<String>,
    /// The files changed on both sides and merged, this machine's lines
    /// kept where they met.
    pub kept_here: Vec<String>,
}

impl Outcome {
    /// In a few words, what was sent named.
    #[must_use]
    pub fn said(&self) -> String {
        let mut parts = Vec::new();
        if !self.sent.is_empty() {
            parts.push(format!(
                "{} sent ({})",
                count(self.sent.len()),
                named(&self.sent)
            ));
        }
        if !self.taken.is_empty() {
            parts.push(format!("{} taken", count(self.taken.len())));
        }
        if !self.kept_here.is_empty() {
            parts.push(format!(
                "{} changed on both sides and merged, this machine's lines kept where they met",
                count(self.kept_here.len())
            ));
        }
        if parts.is_empty() {
            "nothing changed".into()
        } else {
            parts.join(", ")
        }
    }
}

/// `files`, as the repository names them, said for the person to decide
/// on: a few by name; more, the person's own by name (what is kept from
/// the home, iq's settings, rules and skills) and the rest counted by
/// folder.
///
/// # Examples
///
/// ```
/// let files: Vec<String> = ["home/.claude/CLAUDE.md", "memory/a.md", "memory/b.md", "assay/x", "assay/y", "assay/z"]
///     .map(str::to_owned)
///     .to_vec();
/// assert_eq!(iq_sync::named(&files[..2]), "home/.claude/CLAUDE.md, memory/a.md");
/// assert_eq!(iq_sync::named(&files), "home/.claude/CLAUDE.md; assay/ 3, memory/ 2");
/// ```
#[must_use]
pub fn named(files: &[String]) -> String {
    /// So many files are named whole.
    const NAMED: usize = 3;
    /// So many of the person's own are named among more.
    const OWN: usize = 6;
    /// So many folders are counted.
    const FOLDERS: usize = 4;
    if files.len() <= NAMED {
        return files.join(", ");
    }
    let own: Vec<&String> = files
        .iter()
        .filter(|f| f.starts_with(&format!("{KEPT}/")) || f.starts_with("config/"))
        .collect();
    let mut folders: Vec<(String, usize)> = Vec::new();
    for file in files.iter().filter(|f| !own.contains(f)) {
        let folder = match file.split_once('/') {
            Some((folder, _)) => format!("{folder}/"),
            None => file.clone(),
        };
        match folders.iter_mut().find(|(f, _)| *f == folder) {
            Some((_, count)) => *count += 1,
            None => folders.push((folder, 1)),
        }
    }
    folders.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut parts = Vec::new();
    if !own.is_empty() {
        let mut said = own
            .iter()
            .take(OWN)
            .map(|f| f.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        if own.len() > OWN {
            said.push_str(&format!(" and {} more of yours", own.len() - OWN));
        }
        parts.push(said);
    }
    if !folders.is_empty() {
        let mut said = folders
            .iter()
            .take(FOLDERS)
            .map(|(folder, count)| format!("{folder} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        let others: usize = folders.iter().skip(FOLDERS).map(|(_, count)| count).sum();
        if others > 0 {
            said.push_str(&format!(" and {others} elsewhere"));
        }
        parts.push(said);
    }
    parts.join("; ")
}

fn count(files: usize) -> String {
    if files == 1 {
        "1 file".into()
    } else {
        format!("{files} files")
    }
}

/// A folder synced with a repository.
#[derive(Debug)]
pub struct Sync {
    root: PathBuf,
    remote: String,
    machine: String,
    mirrored: Vec<Mirrored>,
    branch: String,
    /// The home whose files are kept, when some are.
    home: Option<PathBuf>,
    /// Those kept, each `~/…`.
    keep: Vec<String>,
    /// One sync at a time.
    running: Mutex<()>,
}

impl Sync {
    /// The folder `root` synced with the repository at `remote`, this
    /// machine named `machine` in what it sends.
    pub fn new(
        root: impl Into<PathBuf>,
        remote: impl Into<String>,
        machine: impl Into<String>,
    ) -> Self {
        Self {
            root: root.into(),
            remote: remote.into(),
            machine: machine.into(),
            mirrored: Vec::new(),
            branch: BRANCH.to_owned(),
            home: None,
            keep: Vec::new(),
            running: Mutex::new(()),
        }
    }

    /// Syncs `outside` too, kept in the folder at `inside`.
    #[must_use]
    pub fn mirror(mut self, outside: impl Into<PathBuf>, inside: impl Into<String>) -> Self {
        self.mirrored.push(Mirrored {
            outside: outside.into(),
            inside: inside.into(),
        });
        self
    }

    /// Keeps `paths` of `home` too, each written `~/…`, a file or a
    /// folder: copied in the folder, and put back in their place on a new
    /// machine.
    #[must_use]
    pub fn keeping(mut self, home: impl Into<PathBuf>, paths: Vec<String>) -> Self {
        self.home = Some(home.into());
        self.keep = paths;
        self
    }

    /// The paths of the home kept, as the list writes them.
    #[must_use]
    pub fn kept(&self) -> Vec<String> {
        self.kept_places()
            .into_iter()
            .map(|(entry, _, _)| entry)
            .collect()
    }

    /// The paths of the list that are not kept, each with why.
    #[must_use]
    pub fn refused(&self) -> Vec<String> {
        self.keep
            .iter()
            .filter_map(|entry| self.place(entry).err())
            .collect()
    }

    /// Where `entry` lives and where its copy is, or why it is not kept.
    fn place(&self, entry: &str) -> Result<(PathBuf, PathBuf), String> {
        let home = self.home.as_ref().ok_or("no home is kept")?;
        let relative = Path::new(
            entry
                .trim()
                .strip_prefix("~/")
                .ok_or_else(|| format!("{entry}: write it as ~/ and its path in the home"))?,
        );
        // Never out of the home or the folder, whatever the list says.
        let plain = relative
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !plain || relative.as_os_str().is_empty() {
            return Err(format!(
                "{entry}: name a file or a folder in the home, plainly"
            ));
        }
        let outside = home.join(relative);
        if !outside.is_dir() && !keeps_file(&outside, Some(home)) {
            return Err(format!("{entry} holds secrets, which are never kept"));
        }
        if outside.starts_with(&self.root) || relative.starts_with(".iq") {
            return Err(format!("{entry} is iq's own, kept already"));
        }
        Ok((outside, self.root.join(KEPT).join(relative)))
    }

    /// Puts the files kept back in the home, as the folder holds them,
    /// what the home had kept beside: on a machine new to the repository,
    /// once the settings it took say what is kept.
    ///
    /// # Errors
    ///
    /// A file could not be read or written.
    pub fn put_back(&self) -> Result<(), SyncError> {
        self.keep_out(true)
    }

    /// Each path kept: as the list writes it, where it lives, and where its
    /// copy is.
    fn kept_places(&self) -> Vec<(String, PathBuf, PathBuf)> {
        self.keep
            .iter()
            .filter_map(|entry| {
                let (outside, inside) = self.place(entry).ok()?;
                Some((entry.trim().to_owned(), outside, inside))
            })
            .collect()
    }

    /// Copies the files kept in, as the home holds them: what it no longer
    /// holds goes from the copy, as long as the place itself is there.
    fn keep_in(&self) -> Result<(), SyncError> {
        let home = self.home.as_deref();
        for (_, outside, inside) in self.kept_places() {
            if !outside.exists() {
                continue;
            }
            let here = files_kept(&outside, home);
            for relative in &here {
                let from = if relative.as_os_str().is_empty() {
                    outside.clone()
                } else {
                    outside.join(relative)
                };
                let to = if relative.as_os_str().is_empty() {
                    inside.clone()
                } else {
                    inside.join(relative)
                };
                let bytes = std::fs::read(&from).map_err(io(&from))?;
                if std::fs::read(&to).ok().as_deref() != Some(bytes.as_slice()) {
                    if let Some(parent) = to.parent() {
                        std::fs::create_dir_all(parent).map_err(io(parent))?;
                    }
                    std::fs::write(&to, bytes).map_err(io(&to))?;
                }
            }
            if outside.is_dir() {
                for relative in files_kept(&inside, None) {
                    if !here.contains(&relative) {
                        let gone = inside.join(&relative);
                        std::fs::remove_file(&gone).map_err(io(&gone))?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Puts the files kept back in the home, as the sync left them; on a
    /// machine new to the repository, what it had is kept beside. Never
    /// removes one.
    fn keep_out(&self, fresh: bool) -> Result<(), SyncError> {
        for (_, outside, inside) in self.kept_places() {
            for relative in files_kept(&inside, None) {
                let (from, to) = if relative.as_os_str().is_empty() {
                    (inside.clone(), outside.clone())
                } else {
                    (inside.join(&relative), outside.join(&relative))
                };
                let synced = std::fs::read(&from).map_err(io(&from))?;
                let here = std::fs::read(&to).ok();
                if here.as_deref() == Some(synced.as_slice()) {
                    continue;
                }
                if fresh && let Some(here) = &here {
                    let mut beside = to.clone().into_os_string();
                    beside.push(".before-sync");
                    write_private(&PathBuf::from(beside), here)?;
                }
                write_private(&to, &synced)?;
            }
        }
        Ok(())
    }

    /// Keeps the files on `branch` rather than on [`BRANCH`].
    #[must_use]
    pub fn on_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = branch.into();
        self
    }

    /// The folder.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The repository.
    #[must_use]
    pub fn remote(&self) -> &str {
        &self.remote
    }

    /// Syncs once: commits what changed here, takes what changed
    /// elsewhere, sends the rest.
    ///
    /// # Errors
    ///
    /// Git could not be run, or said no, as when the repository cannot be
    /// reached; or a file could not be read or written.
    pub async fn run(&self) -> Result<Outcome, SyncError> {
        let _one = self.running.lock().await;
        let git = Git::new(&self.root, &self.remote, &self.machine).await;
        self.prepare(&git).await?;
        let fresh = !git.ok(&["rev-parse", "--verify", "-q", "HEAD"]).await;
        let mut outcome = Outcome {
            fresh,
            ..Outcome::default()
        };
        git.run(&["fetch", "-q", "origin"]).await?;
        let theirs = format!("refs/remotes/origin/{}", self.branch);
        let push = format!("HEAD:refs/heads/{}", self.branch);
        if fresh && git.ok(&["rev-parse", "--verify", "-q", &theirs]).await {
            // New here: what is there comes first.
            git.run(&["reset", "-q", &theirs]).await?;
            git.run(&["checkout", "-q", &theirs, "--", "."]).await?;
            outcome.taken = git.lines(&["ls-tree", "-r", "--name-only", "HEAD"]).await?;
            self.copy_out(true)?;
            self.keep_out(true)?;
        }
        self.copy_in()?;
        self.keep_in()?;
        git.run(&["add", "-A"]).await?;
        if !git.ok(&["diff", "--cached", "--quiet"]).await {
            outcome.sent = git.lines(&["diff", "--cached", "--name-only"]).await?;
            git.run(&[
                "commit",
                "-q",
                "-m",
                &format!("Changes on {}", self.machine),
            ])
            .await?;
        }
        for attempt in 1..=ATTEMPTS {
            if git.ok(&["rev-parse", "--verify", "-q", &theirs]).await {
                self.merge(&git, &theirs, &mut outcome).await?;
            }
            if !git.ok(&["rev-parse", "--verify", "-q", "HEAD"]).await {
                // Nothing here, nothing there.
                break;
            }
            match git.run(&["push", "-q", "origin", &push]).await {
                Ok(_) => break,
                Err(error) if attempt == ATTEMPTS => return Err(error),
                // Another machine sent meanwhile: take it, and try again.
                Err(_) => git.run(&["fetch", "-q", "origin"]).await.map(drop)?,
            }
        }
        self.copy_out(false)?;
        self.keep_out(false)?;
        outcome.taken.sort();
        outcome.taken.dedup();
        Ok(outcome)
    }

    /// Whether this machine has something to send, or never synced. Reads
    /// only, and reaches no repository.
    pub async fn pending(&self) -> bool {
        self.fresh() || !self.changes().await.is_empty()
    }

    /// Whether this machine never synced with the repository.
    #[must_use]
    pub fn fresh(&self) -> bool {
        !self.root.join(".git").exists()
    }

    /// Takes what the repository holds, on a machine that never synced,
    /// and sends nothing: what this machine has is then asked about as any
    /// change is. Where the repository holds nothing yet, the folder is
    /// only made one of it.
    ///
    /// # Errors
    ///
    /// Git could not be run, or said no, as when the repository cannot be
    /// reached; or a file could not be read or written.
    pub async fn take_first(&self) -> Result<Outcome, SyncError> {
        let _one = self.running.lock().await;
        let git = Git::new(&self.root, &self.remote, &self.machine).await;
        self.prepare(&git).await?;
        let mut outcome = Outcome {
            fresh: true,
            ..Outcome::default()
        };
        git.run(&["fetch", "-q", "origin"]).await?;
        let theirs = format!("refs/remotes/origin/{}", self.branch);
        if !git.ok(&["rev-parse", "--verify", "-q", "HEAD"]).await
            && git.ok(&["rev-parse", "--verify", "-q", &theirs]).await
        {
            git.run(&["reset", "-q", &theirs]).await?;
            git.run(&["checkout", "-q", &theirs, "--", "."]).await?;
            outcome.taken = git.lines(&["ls-tree", "-r", "--name-only", "HEAD"]).await?;
            self.copy_out(true)?;
            self.keep_out(true)?;
        }
        Ok(outcome)
    }

    /// What a sync would send, as the repository names it: the files of
    /// the folder changed, those mirrored or kept that changed where they
    /// live, and what was sent before that the folder's `.gitignore` now
    /// leaves out, which goes from the repository. Reads only, and
    /// reaches no repository; nothing on a machine that never synced,
    /// which takes first.
    pub async fn changes(&self) -> Vec<String> {
        self.changed()
            .await
            .into_iter()
            .map(|change| change.name().to_owned())
            .collect()
    }

    /// What a sync would send, line by line: the diff git makes of each
    /// change of [`Self::changes`], a file copied in against what it is
    /// where it lives, a file of the folder against what was sent of it,
    /// each named as the repository names it, as far as [`DIFF_LINES`]
    /// lines. Reads only, and reaches no repository; empty where nothing
    /// would be sent.
    pub async fn diff(&self) -> String {
        let git = Git::new(&self.root, &self.remote, &self.machine).await;
        let mut out = String::new();
        for change in self.changed().await {
            let text = match &change {
                Change::Copied {
                    name,
                    outside,
                    inside,
                } => git
                    .diff_no_index(inside, outside)
                    .await
                    .map(|text| headed(&text, name, inside.is_file(), outside.is_file())),
                Change::Own { name } => git.diffed(&["diff", "HEAD", "--", name]).await,
                Change::New { name } => {
                    let file = self.root.join(name);
                    git.diff_no_index(Path::new("/dev/null"), &file)
                        .await
                        .map(|text| headed(&text, name, false, true))
                }
                Change::LeftOut { name } => {
                    let file = self.root.join(name);
                    git.diff_no_index(&file, Path::new("/dev/null"))
                        .await
                        .map(|text| headed(&text, name, true, false))
                }
            };
            match text {
                Ok(text) => out.push_str(&text),
                Err(error) => out.push_str(&format!("{}: {error}\n", change.name())),
            }
        }
        let lines = out.lines().count();
        if lines > DIFF_LINES {
            let kept: Vec<&str> = out.lines().take(DIFF_LINES).collect();
            let mut cut = kept.join("\n");
            cut.push_str(&format!("\n[{} more lines]\n", lines - DIFF_LINES));
            return cut;
        }
        out
    }

    /// Each change a sync would send, by name, once each; nothing on a
    /// machine that never synced, which takes first.
    async fn changed(&self) -> Vec<Change> {
        if self.fresh() {
            return Vec::new();
        }
        let differs = |from: &Path, to: &Path| std::fs::read(from).ok() != std::fs::read(to).ok();
        let mut changed: Vec<Change> = Vec::new();
        let copied = |outside: &Path, inside: &Path| Change::Copied {
            name: self.named_inside(inside),
            outside: outside.to_path_buf(),
            inside: inside.to_path_buf(),
        };
        for (outside, inside) in self.mirrored_files() {
            let sent = if outside.exists() {
                differs(&outside, &inside)
            } else {
                self.removed_here(&outside)
            };
            if sent {
                changed.push(copied(&outside, &inside));
            }
        }
        for (_, outside, inside) in self.kept_places() {
            let here = files_kept(&outside, self.home.as_deref());
            let there = files_kept(&inside, None);
            let at = |place: &Path, relative: &PathBuf| {
                if relative.as_os_str().is_empty() {
                    place.to_path_buf()
                } else {
                    place.join(relative)
                }
            };
            for relative in &here {
                if differs(&at(&outside, relative), &at(&inside, relative)) {
                    changed.push(copied(&at(&outside, relative), &at(&inside, relative)));
                }
            }
            if outside.is_dir() {
                for relative in there.iter().filter(|r| !here.contains(r)) {
                    changed.push(copied(&at(&outside, relative), &at(&inside, relative)));
                }
            }
        }
        let git = Git::new(&self.root, &self.remote, &self.machine).await;
        if let Ok(status) = git.run(&["status", "--porcelain", "-uall"]).await {
            changed.extend(status.lines().filter_map(|line| {
                let name = line.get(3..)?.trim_matches('"').to_owned();
                Some(if line.starts_with("??") {
                    Change::New { name }
                } else {
                    Change::Own { name }
                })
            }));
        }
        if let Ok(left_out) = git.lines(&["ls-files", "-ci", "--exclude-standard"]).await {
            changed.extend(left_out.into_iter().map(|name| Change::LeftOut { name }));
        }
        changed.sort_by(|a, b| a.name().cmp(b.name()));
        changed.dedup_by(|a, b| a.name() == b.name());
        changed
    }

    /// A path of the folder, as the repository names it.
    fn named_inside(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }

    /// Whether `outside`, a file of a mirrored folder that is here, is no
    /// longer in it: its copy goes too.
    fn removed_here(&self, outside: &Path) -> bool {
        !outside.exists()
            && self.mirrored.iter().any(|m| {
                m.outside.is_dir() && outside.starts_with(&m.outside) && outside != m.outside
            })
    }

    /// Each mirrored file, where it lives and where its copy is: a file
    /// mirrored, or each of a folder's, on either side.
    fn mirrored_files(&self) -> Vec<(PathBuf, PathBuf)> {
        let mut pairs = Vec::new();
        for mirrored in &self.mirrored {
            let inside = self.root.join(&mirrored.inside);
            if !mirrored.outside.is_dir() && !inside.is_dir() {
                pairs.push((mirrored.outside.clone(), inside));
                continue;
            }
            let mut files = files_kept(&mirrored.outside, None);
            files.extend(files_kept(&inside, None));
            files.sort();
            files.dedup();
            pairs.extend(
                files
                    .into_iter()
                    .map(|relative| (mirrored.outside.join(&relative), inside.join(relative))),
            );
        }
        pairs
    }

    /// Takes what the repository holds beyond this machine's history.
    async fn merge(&self, git: &Git, theirs: &str, outcome: &mut Outcome) -> Result<(), SyncError> {
        if git
            .ok(&["merge-base", "--is-ancestor", theirs, "HEAD"])
            .await
        {
            return Ok(());
        }
        if git
            .ok(&["merge-base", "--is-ancestor", "HEAD", theirs])
            .await
        {
            let taken = git.lines(&["diff", "--name-only", "HEAD", theirs]).await?;
            git.run(&["merge", "-q", "--ff-only", theirs]).await?;
            outcome.taken.extend(taken);
            return Ok(());
        }
        let base = git.run(&["merge-base", "HEAD", theirs]).await.ok();
        let (here, there) = match base.as_deref().map(str::trim) {
            Some(base) => (
                git.lines(&["diff", "--name-only", base, "HEAD"]).await?,
                git.lines(&["diff", "--name-only", base, theirs]).await?,
            ),
            None => (
                git.lines(&["ls-tree", "-r", "--name-only", "HEAD"]).await?,
                git.lines(&["ls-tree", "-r", "--name-only", theirs]).await?,
            ),
        };
        let both: Vec<String> = there.iter().filter(|f| here.contains(f)).cloned().collect();
        let message = format!("Changes from elsewhere, merged on {}", self.machine);
        let merged = git
            .run(&[
                "merge",
                "-q",
                "--no-edit",
                "--allow-unrelated-histories",
                "-X",
                "ours",
                "-m",
                &message,
                theirs,
            ])
            .await;
        if merged.is_err() {
            // What lines cannot settle, such as a file changed here and
            // removed there: this machine's side stands.
            for path in git
                .lines(&["diff", "--name-only", "--diff-filter=U"])
                .await?
            {
                if git.ok(&["cat-file", "-e", &format!(":2:{path}")]).await {
                    git.run(&["checkout", "--ours", "--", &path]).await?;
                    git.run(&["add", "--", &path]).await?;
                } else {
                    git.run(&["rm", "-q", "--", &path]).await?;
                }
            }
            git.run(&["commit", "-q", "--no-edit"]).await?;
        }
        outcome
            .taken
            .extend(there.into_iter().filter(|f| !both.contains(f)));
        outcome.kept_here.extend(both);
        Ok(())
    }

    /// Makes the folder a repository of the remote, once.
    async fn prepare(&self, git: &Git) -> Result<(), SyncError> {
        std::fs::create_dir_all(&self.root).map_err(io(&self.root))?;
        if !self.root.join(".git").exists() {
            git.run(&["init", "-q", "-b", &self.branch]).await?;
        }
        let ignored = self.root.join(".gitignore");
        let before = std::fs::read_to_string(&ignored).unwrap_or_default();
        let now = ignore_lines(&before);
        if now != before {
            std::fs::write(&ignored, now).map_err(io(&ignored))?;
        }
        // What it now leaves out goes from the repository, as the assay's
        // store did once iq read it no more; its history keeps it.
        let left_out = git
            .lines(&["ls-files", "-ci", "--exclude-standard"])
            .await
            .unwrap_or_default();
        for some in left_out.chunks(200) {
            let mut args = vec!["rm", "-q", "--cached", "--ignore-unmatch", "--"];
            args.extend(some.iter().map(String::as_str));
            git.run(&args).await?;
        }
        match git.run(&["remote", "get-url", "origin"]).await {
            Ok(url) if url.trim() == self.remote => Ok(()),
            Ok(_) => git
                .run(&["remote", "set-url", "origin", &self.remote])
                .await
                .map(drop),
            Err(_) => git
                .run(&["remote", "add", "origin", &self.remote])
                .await
                .map(drop),
        }
    }

    /// Copies the mirrored files in, as they are here: of a folder that is
    /// here, what it no longer holds goes from the copy.
    fn copy_in(&self) -> Result<(), SyncError> {
        for (outside, inside) in self.mirrored_files() {
            let Ok(here) = std::fs::read(&outside) else {
                if self.removed_here(&outside) && inside.exists() {
                    std::fs::remove_file(&inside).map_err(io(&inside))?;
                }
                continue;
            };
            if std::fs::read(&inside).ok().as_deref() != Some(here.as_slice()) {
                if let Some(parent) = inside.parent() {
                    std::fs::create_dir_all(parent).map_err(io(parent))?;
                }
                std::fs::write(&inside, here).map_err(io(&inside))?;
            }
        }
        Ok(())
    }

    /// Copies the mirrored files out, as the sync left them. On a machine
    /// new to the repository, what it had is kept beside, as it was never
    /// sent.
    fn copy_out(&self, fresh: bool) -> Result<(), SyncError> {
        for (outside, inside) in self.mirrored_files() {
            let Ok(synced) = std::fs::read(&inside) else {
                continue;
            };
            let here = std::fs::read(&outside).ok();
            if here.as_deref() == Some(synced.as_slice()) {
                continue;
            }
            if fresh && let Some(here) = &here {
                let mut beside = outside.clone().into_os_string();
                beside.push(".before-sync");
                let beside = PathBuf::from(beside);
                write_private(&beside, here)?;
            }
            write_private(&outside, &synced)?;
        }
        Ok(())
    }
}

/// The folder's `.gitignore` as it should be, from what it says now: the
/// person's own lines kept, those iq wrote once and no longer needs
/// dropped, iq's own added where missing.
fn ignore_lines(now: &str) -> String {
    if now.trim().is_empty() {
        return IGNORED.to_owned();
    }
    let mut lines: Vec<&str> = now
        .lines()
        .filter(|line| !IGNORED_ONCE.contains(&line.trim()))
        .collect();
    // What is missing is added under the comment that says why, once.
    let mut comment = None;
    for line in IGNORED.lines() {
        if line.starts_with('#') {
            comment = Some(line);
            continue;
        }
        if line.trim().is_empty() || lines.iter().any(|kept| kept.trim() == line.trim()) {
            continue;
        }
        if let Some(comment) = comment.take() {
            lines.push(comment);
        }
        lines.push(line);
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Whether the file at `path` may be kept: not one whose name says it
/// holds secrets, nor, in a folder of credentials of `home`, one that is
/// not Markdown, as the rules and notes kept there are.
fn keeps_file(path: &Path, home: Option<&Path>) -> bool {
    if iq_gate::holds_secrets(path, None) {
        return false;
    }
    match home {
        Some(home) if iq_gate::holds_secrets(path, Some(home)) => {
            path.extension().is_some_and(|ending| ending == "md")
        }
        _ => true,
    }
}

/// The files of `place` kept, by their paths relative to it: the place
/// itself, empty, when it is a file. Links are not followed; what holds
/// secrets as `home` says, what is too large and what tools rebuild are
/// left out.
fn files_kept(place: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let keeps = |path: &Path| {
        home.is_none_or(|home| keeps_file(path, Some(home)))
            && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() <= KEPT_MOST)
    };
    let Ok(meta) = std::fs::symlink_metadata(place) else {
        return Vec::new();
    };
    if meta.is_file() {
        return if keeps(place) {
            vec![PathBuf::new()]
        } else {
            Vec::new()
        };
    }
    let mut found = Vec::new();
    let mut folders = vec![PathBuf::new()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(place.join(&folder)) else {
            continue;
        };
        for entry in entries.flatten() {
            let relative = folder.join(entry.file_name());
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                // A folder of credentials within is walked: only its
                // Markdown files are kept.
                if !LEFT_OUT.iter().any(|left| entry.file_name() == *left) {
                    folders.push(relative);
                }
            } else if kind.is_file() && keeps(&entry.path()) {
                found.push(relative);
            }
        }
    }
    found.sort();
    found
}

/// Writes `bytes` to `path`, readable by its owner only, as iq's files are.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), SyncError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io(parent))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(io(path))?;
    file.write_all(bytes).map_err(io(path))
}

/// Git, run in the folder with what reaches the repository.
struct Git {
    root: PathBuf,
    /// What every call is given beside its arguments: the credentials and
    /// the name to commit with, when the machine has none of its own.
    settings: Vec<String>,
}

impl Git {
    async fn new(root: &Path, remote: &str, machine: &str) -> Self {
        let mut settings = Vec::new();
        // GitHub's command line, when signed in, gives git its credentials
        // for GitHub: nothing else to set up on a machine that has it.
        if remote.starts_with("https://github.com/") && signed_in_to_github().await {
            settings.extend([
                "credential.https://github.com.helper=".to_owned(),
                "credential.https://github.com.helper=!gh auth git-credential".to_owned(),
            ]);
        }
        let named = Command::new("git")
            .args(["config", "--get", "user.email"])
            .current_dir(root.parent().unwrap_or(root))
            .stdin(Stdio::null())
            .output()
            .await
            .is_ok_and(|out| out.status.success() && !out.stdout.trim_ascii().is_empty());
        if !named {
            settings.extend([
                "user.name=iq".to_owned(),
                format!(
                    "user.email=iq@{}",
                    machine.replace(char::is_whitespace, "-")
                ),
            ]);
        }
        Self {
            root: root.to_path_buf(),
            settings,
        }
    }

    /// What git printed, once it succeeded.
    async fn run(&self, args: &[&str]) -> Result<String, SyncError> {
        self.done(args, |status| status.success()).await
    }

    /// What `git diff` printed: it ends with 1 where the sides differ,
    /// which is no failure.
    async fn diffed(&self, args: &[&str]) -> Result<String, SyncError> {
        self.done(args, |status| matches!(status.code(), Some(0 | 1)))
            .await
    }

    /// The diff of `from` to `to`, files anywhere, `/dev/null` for one not
    /// there.
    async fn diff_no_index(&self, from: &Path, to: &Path) -> Result<String, SyncError> {
        let side = |file: &Path| {
            if file.is_file() {
                file.to_string_lossy().into_owned()
            } else {
                "/dev/null".to_owned()
            }
        };
        self.diffed(&["diff", "--no-index", "--", &side(from), &side(to)])
            .await
    }

    /// What git printed, once it ended as `ok` says.
    async fn done(
        &self,
        args: &[&str],
        ok: impl Fn(std::process::ExitStatus) -> bool,
    ) -> Result<String, SyncError> {
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.root);
        for setting in &self.settings {
            command.arg("-c").arg(setting);
        }
        let output = command
            .args(args)
            // The folder's repository, whatever repository iq was started
            // from, as by a hook of another, which names its own.
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            // Nobody is there to answer a prompt: a sync fails rather
            // than waits.
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|error| SyncError::Git {
                command: args.join(" "),
                said: format!("git could not be run: {error}"),
            })?;
        if ok(output.status) {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(SyncError::Git {
                command: args.first().copied().unwrap_or_default().to_owned(),
                said: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }

    async fn ok(&self, args: &[&str]) -> bool {
        self.run(args).await.is_ok()
    }

    async fn lines(&self, args: &[&str]) -> Result<Vec<String>, SyncError> {
        Ok(self
            .run(args)
            .await?
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect())
    }
}

/// Whether GitHub's command line is installed and signed in.
async fn signed_in_to_github() -> bool {
    Command::new("gh")
        .args(["auth", "status"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outcome_says_what_moved() {
        assert_eq!(Outcome::default().said(), "nothing changed");
        let outcome = Outcome {
            fresh: false,
            sent: vec!["a".into()],
            taken: vec!["b".into(), "c".into()],
            kept_here: vec!["d".into()],
        };
        assert_eq!(
            outcome.said(),
            "1 file sent (a), 2 files taken, 1 file changed on both sides and merged, this machine's lines kept where they met"
        );
        let many: Vec<String> = ["a", "b", "c", "d", "e"].map(str::to_owned).to_vec();
        assert_eq!(named(&many), "a 1, b 1, c 1, d 1 and 1 elsewhere");
        let mut first: Vec<String> = (0..1500).map(|n| format!("assay/runs/{n}")).collect();
        first.extend((0..20).map(|n| format!("memory/{n}.md")));
        first.extend(
            [
                "config/settings.toml",
                "config/skills/teams/SKILL.md",
                ".gitignore",
            ]
            .map(str::to_owned),
        );
        assert_eq!(
            named(&first),
            "config/settings.toml, config/skills/teams/SKILL.md; assay/ 1500, memory/ 20, .gitignore 1"
        );
    }

    #[test]
    fn the_gitignore_keeps_the_persons_lines_and_drops_what_iq_wrote_once() {
        assert_eq!(ignore_lines(""), IGNORED);
        assert_eq!(ignore_lines(IGNORED), IGNORED);
        let once = "# What only this machine needs, or what is built again from the rest.\nassay/runs/*/*/*/work/\nassay/judging/\n*.before-sync\nnotes/drafts/\n";
        assert_eq!(
            ignore_lines(once),
            "# What only this machine needs, or what is built again from the rest.\n*.before-sync\nnotes/drafts/\n# The worktrees the agents work in, which their repositories keep.\nworktrees/\n# The assay iq once had, which it reads no more.\nassay/\n"
        );
    }
}
