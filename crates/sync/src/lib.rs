//! iq's own files kept in a git repository the person chose, a private one
//! of theirs: the assay, the projects' memory, a copy of the settings. The
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
//! settings with the rest, puts each file back in its place. What holds secrets is never kept, as the
//! gate tells it; nor what is too large, nor what is rebuilt. A sync
//! never removes a file of the home: what another machine removed stays
//! in the history.

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
/// again from the rest.
const IGNORED: &str = "# What only this machine needs, or what is built again from the rest.
assay/runs/*/*/*/work/
assay/judging/
*.before-sync
";

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

/// A file kept outside the folder, copied in to be synced and out once
/// synced, as the settings are, which iq reads from its own folder.
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
    /// In a few words.
    #[must_use]
    pub fn said(&self) -> String {
        let mut parts = Vec::new();
        if !self.sent.is_empty() {
            parts.push(format!("{} sent", count(self.sent.len())));
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
        if iq_gate::holds_secrets(&outside, Some(home)) {
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

    /// Whether this machine has something to send: a file of the folder
    /// changed, or a mirrored file changed where it lives. Reads only, and
    /// reaches no repository.
    pub async fn pending(&self) -> bool {
        if !self.root.join(".git").exists() {
            return true;
        }
        let mirrored_changed = self.mirrored.iter().any(|file| {
            std::fs::read(&file.outside).ok().is_some_and(|here| {
                std::fs::read(self.root.join(&file.inside)).ok().as_deref() != Some(here.as_slice())
            })
        });
        let kept_changed = self.kept_places().iter().any(|(_, outside, inside)| {
            let here = files_kept(outside, self.home.as_deref());
            let copied = files_kept(inside, None);
            (outside.is_dir() && here != copied)
                || here.iter().any(|relative| {
                    let (from, to) = if relative.as_os_str().is_empty() {
                        (outside.clone(), inside.clone())
                    } else {
                        (outside.join(relative), inside.join(relative))
                    };
                    std::fs::read(&from).ok() != std::fs::read(&to).ok()
                })
        });
        if mirrored_changed || kept_changed {
            return true;
        }
        let git = Git::new(&self.root, &self.remote, &self.machine).await;
        git.run(&["status", "--porcelain"])
            .await
            .map_or(true, |status| !status.trim().is_empty())
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
        if !ignored.exists() {
            std::fs::write(&ignored, IGNORED).map_err(io(&ignored))?;
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

    /// Copies the mirrored files in, as they are here.
    fn copy_in(&self) -> Result<(), SyncError> {
        for file in &self.mirrored {
            let Ok(here) = std::fs::read(&file.outside) else {
                continue;
            };
            let inside = self.root.join(&file.inside);
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
        for file in &self.mirrored {
            let Ok(synced) = std::fs::read(self.root.join(&file.inside)) else {
                continue;
            };
            let here = std::fs::read(&file.outside).ok();
            if here.as_deref() == Some(synced.as_slice()) {
                continue;
            }
            if fresh && let Some(here) = &here {
                let mut beside = file.outside.clone().into_os_string();
                beside.push(".before-sync");
                let beside = PathBuf::from(beside);
                write_private(&beside, here)?;
            }
            write_private(&file.outside, &synced)?;
        }
        Ok(())
    }
}

/// The files of `place` kept, by their paths relative to it: the place
/// itself, empty, when it is a file. Links are not followed; what holds
/// secrets as `home` says, what is too large and what tools rebuild are
/// left out.
fn files_kept(place: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let keeps = |path: &Path| {
        !home.is_some_and(|home| iq_gate::holds_secrets(path, Some(home)))
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
                if !LEFT_OUT.iter().any(|left| entry.file_name() == *left)
                    && !home.is_some_and(|home| iq_gate::holds_secrets(&entry.path(), Some(home)))
                {
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
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.root);
        for setting in &self.settings {
            command.arg("-c").arg(setting);
        }
        let output = command
            .args(args)
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
        if output.status.success() {
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
            "1 file sent, 2 files taken, 1 file changed on both sides and merged, this machine's lines kept where they met"
        );
    }
}
