//! A place of its own for each chat the person starts, when they want it:
//! a worktree of the project's repository, on a branch of its own from
//! what is checked out, or from the commit the settings name, kept in iq's
//! folder of data. The person's checkout is left as it is while the agents
//! work, and their work is a branch to merge, or to drop. The commands the
//! settings list are run in the worktree as it is made, on the machine, to
//! give it the project's environment. A worktree stays until the person
//! removes it, or until iq finds it untouched for as long as the settings
//! say: it is then removed when nothing in it is uncommitted, and its
//! branch with it when its work was merged.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use iq_agent::{Placed, Placer};

use crate::journal::git_command;

/// The branches made are named under this.
const BRANCHES: &str = "iq/";

/// How many names are tried for one chat before giving up: the chat's,
/// then numbered.
const MOST_NAMES: u32 = 50;

/// How much of a set-up command's output is kept when it fails.
const SAID_CHARS: usize = 300;

/// Makes worktrees of one repository.
pub(crate) struct Worktrees {
    /// The repository's top, the person's checkout.
    top: PathBuf,
    /// The repository's own folder, where worktrees keep their index and
    /// their commits go.
    common: PathBuf,
    /// Where this repository's worktrees are made.
    folder: PathBuf,
    /// What a worktree starts from: `HEAD` unless the settings name a
    /// commit, a branch, a tag.
    from: String,
    /// The commands run in a worktree as it is made, on the machine.
    setup: Vec<String>,
}

/// A worktree iq made, as found again.
struct Found {
    folder: PathBuf,
    branch: Option<String>,
}

impl Worktrees {
    /// Worktrees of the repository `workspace` is in, made under
    /// `folder`, each from `from` (`HEAD` when empty) and set up with
    /// `setup`.
    ///
    /// # Errors
    ///
    /// `workspace` is not in a repository.
    pub(crate) fn open(
        workspace: &Path,
        folder: &Path,
        from: &str,
        setup: Vec<String>,
    ) -> Result<Self, String> {
        let top = PathBuf::from(git(workspace, &["rev-parse", "--show-toplevel"])?);
        let top = top
            .canonicalize()
            .map_err(|e| format!("{}: {e}", top.display()))?;
        let common = top.join(git(&top, &["rev-parse", "--git-common-dir"])?);
        let common = common
            .canonicalize()
            .map_err(|e| format!("{}: {e}", common.display()))?;
        let name = top
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repository".to_owned());
        let from = from.trim();
        Ok(Self {
            top,
            common,
            folder: folder.join(name),
            from: if from.is_empty() { "HEAD" } else { from }.to_owned(),
            setup,
        })
    }

    /// The repository's own folder, which commands in a worktree may
    /// write.
    pub(crate) fn common(&self) -> &Path {
        &self.common
    }

    /// Removes the worktrees iq made that nobody touched for `keep`, but
    /// those of `kept`, the chats still on the board: those with nothing
    /// uncommitted, and their branch with them when its work is merged into
    /// what the person has checked out. Says what it did, and what it left
    /// and why.
    pub(crate) fn prune(&self, keep: Duration, kept: &[PathBuf]) -> Vec<String> {
        let _ = git(&self.top, &["worktree", "prune"]);
        let now = SystemTime::now();
        let mut said = Vec::new();
        for found in self.found() {
            if kept.contains(&found.folder) {
                continue;
            }
            let Some(touched) = self.touched(&found) else {
                continue;
            };
            let idle = now.duration_since(touched).unwrap_or_default();
            if idle < keep {
                continue;
            }
            let days = idle.as_secs() / (24 * 3600);
            let name = found.folder.display();
            let clean = git(&found.folder, &["status", "--porcelain"]).is_ok_and(|s| s.is_empty());
            if !clean {
                said.push(format!(
                    "the worktree {name} has not been touched for {days} days, and holds \
                     changes not committed: it is kept"
                ));
                continue;
            }
            let folder = found.folder.to_string_lossy().into_owned();
            if let Err(error) = git(&self.top, &["worktree", "remove", &folder]) {
                said.push(format!("the worktree {name} could not be removed: {error}"));
                continue;
            }
            let Some(branch) = found.branch else {
                said.push(format!(
                    "the worktree {name}, untouched for {days} days, was removed"
                ));
                continue;
            };
            // Deleted only when merged: `-d` refuses otherwise.
            match git(&self.top, &["branch", "-d", &branch]) {
                Ok(_) => said.push(format!(
                    "the worktree {name}, untouched for {days} days, was removed with its branch \
                     {branch}, merged"
                )),
                Err(_) => said.push(format!(
                    "the worktree {name}, untouched for {days} days, was removed; its branch \
                     {branch} is kept, not merged: `git branch -D {branch}` drops it"
                )),
            }
        }
        said
    }

    /// The worktrees iq made for this repository, as git lists them.
    fn found(&self) -> Vec<Found> {
        let Ok(listed) = git(&self.top, &["worktree", "list", "--porcelain"]) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        let mut current: Option<Found> = None;
        for line in listed.lines().chain(std::iter::once("")) {
            if let Some(path) = line.strip_prefix("worktree ") {
                current = Some(Found {
                    folder: PathBuf::from(path),
                    branch: None,
                });
            } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                if let Some(current) = &mut current {
                    current.branch = Some(branch.to_owned());
                }
            } else if line.is_empty()
                && let Some(done) = current.take()
                && done.folder.starts_with(&self.folder)
            {
                found.push(done);
            }
        }
        found
    }

    /// When `found` was last touched: its branch's last commit, or git's
    /// last look at it, whichever is later; `None` when nothing says.
    fn touched(&self, found: &Found) -> Option<SystemTime> {
        let name = found.folder.file_name()?;
        let git_dir = self.common.join("worktrees").join(name);
        let mut latest: Option<SystemTime> = None;
        let mut note = |time: Option<SystemTime>| {
            if let Some(time) = time {
                latest = Some(latest.map_or(time, |kept| kept.max(time)));
            }
        };
        for file in ["index", "HEAD", "logs/HEAD"] {
            note(
                std::fs::metadata(git_dir.join(file))
                    .and_then(|m| m.modified())
                    .ok(),
            );
        }
        note(
            std::fs::metadata(&found.folder)
                .and_then(|m| m.modified())
                .ok(),
        );
        if let Some(branch) = &found.branch
            && let Ok(seconds) = git(&self.top, &["log", "-1", "--format=%ct", branch])
            && let Ok(seconds) = seconds.parse::<u64>()
        {
            note(Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)));
        }
        latest
    }

    /// Runs the set-up commands in `folder`, on the machine, as the
    /// person would: with their environment, less iq's own variables.
    /// Says how each went.
    fn set_up(&self, folder: &Path) -> String {
        let mut said = String::new();
        for line in &self.setup {
            let output = std::process::Command::new("/bin/sh")
                .args(["-c", &format!("exec 2>&1; {line}")])
                .current_dir(folder)
                .env_clear()
                .envs(std::env::vars_os().filter(|(name, _)| {
                    name.to_str().is_none_or(|name| {
                        !name.starts_with("IQ_") && !name.starts_with("REQUESTY_")
                    })
                }))
                .stdin(std::process::Stdio::null())
                .output();
            match output {
                Ok(output) if output.status.success() => {
                    said.push_str(&format!(" `{line}` ran."));
                }
                Ok(output) => {
                    let printed = String::from_utf8_lossy(&output.stdout);
                    let tail: String = printed
                        .trim()
                        .chars()
                        .rev()
                        .take(SAID_CHARS)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    said.push_str(&format!(
                        " `{line}` failed ({}): {}",
                        output
                            .status
                            .code()
                            .map_or("ended by a signal".to_owned(), |c| format!(
                                "exit status {c}"
                            )),
                        tail.replace('\n', " ")
                    ));
                }
                Err(error) => said.push_str(&format!(" `{line}` could not start: {error}.")),
            }
        }
        said
    }
}

impl Placer for Worktrees {
    fn place(&self, name: &str) -> Result<Placed, String> {
        let base = git(
            &self.top,
            &["rev-parse", "--short", &format!("{}^{{commit}}", self.from)],
        )
        .map_err(|e| format!("nothing to start from at {} ({e})", self.from))?;
        // A worktree the person removed by hand is still on git's list
        // until it is pruned, and its name taken.
        let _ = git(&self.top, &["worktree", "prune"]);
        let (branch, folder) = (1..=MOST_NAMES)
            .map(|n| {
                let leaf = if n == 1 {
                    name.to_owned()
                } else {
                    format!("{name}-{n}")
                };
                (format!("{BRANCHES}{leaf}"), self.folder.join(leaf))
            })
            .find(|(branch, folder)| {
                !folder.exists()
                    && git(
                        &self.top,
                        &[
                            "rev-parse",
                            "--verify",
                            "--quiet",
                            &format!("refs/heads/{branch}"),
                        ],
                    )
                    .is_err()
            })
            .ok_or_else(|| format!("{MOST_NAMES} worktrees are named after {name} already"))?;
        std::fs::create_dir_all(&self.folder)
            .map_err(|e| format!("making {}: {e}", self.folder.display()))?;
        let folder_arg = folder.to_string_lossy().into_owned();
        git(
            &self.top,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                &branch,
                &folder_arg,
                &self.from,
            ],
        )?;
        let from = if self.from == "HEAD" {
            format!("your checkout's commit {base}")
        } else {
            format!("{} ({base})", self.from)
        };
        let set_up = self.set_up(&folder);
        Ok(Placed {
            folder: folder.clone(),
            writable: vec![self.common.clone()],
            said: format!(
                "works in a worktree of its own, {}, on the branch {branch} from {from}: your \
                 checkout is left as it is, and what is not committed there is not in the \
                 worktree. `git merge {branch}` takes its work; `git worktree remove {}` removes \
                 the worktree after.{set_up}",
                folder.display(),
                folder.display()
            ),
        })
    }
}

/// What `git args` prints in `folder`, trimmed; else what it said.
fn git(folder: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_command(folder)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
    }
    let said = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(if said.is_empty() {
        format!("git {} failed", args.join(" "))
    } else {
        said
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_git(dir: &Path, args: &[&str]) {
        let status = git_command(dir).args(args).status().unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn repository() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-q", "-b", "main"]);
        run_git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        run_git(dir.path(), &["config", "user.name", "Test"]);
        run_git(dir.path(), &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.path().join("file"), "kept\n").unwrap();
        run_git(dir.path(), &["add", "."]);
        run_git(dir.path(), &["commit", "-qm", "Initial file"]);
        dir
    }

    #[test]
    fn each_chat_gets_a_worktree_on_a_branch_of_its_own_and_the_checkout_stays() {
        let dir = repository();
        let worktrees = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        // Not committed: the worktree starts from the commit.
        std::fs::write(dir.path().join("draft"), "draft\n").unwrap();
        let placer = Worktrees::open(&sub, worktrees.path(), "", Vec::new()).unwrap();
        let first = placer.place("lead").unwrap();
        let second = placer.place("lead").unwrap();
        let top = dir.path().canonicalize().unwrap();
        let name = top.file_name().unwrap().to_str().unwrap().to_owned();
        assert_eq!(first.folder, worktrees.path().join(&name).join("lead"));
        assert_eq!(second.folder, worktrees.path().join(&name).join("lead-2"));
        assert_eq!(first.writable, [top.join(".git")]);
        assert!(
            first.said.contains("on the branch iq/lead from"),
            "{}",
            first.said
        );
        assert!(
            second.said.contains("`git merge iq/lead-2`"),
            "{}",
            second.said
        );
        assert_eq!(
            std::fs::read_to_string(first.folder.join("file")).unwrap(),
            "kept\n"
        );
        assert!(!first.folder.join("draft").exists());
        // Work there is a commit on its branch, and nothing in the checkout.
        std::fs::write(first.folder.join("file"), "changed\n").unwrap();
        run_git(&first.folder, &["commit", "-qam", "Change"]);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file")).unwrap(),
            "kept\n"
        );
        let branches = git(dir.path(), &["branch", "--list", "iq/*"]).unwrap();
        assert!(
            branches.contains("iq/lead") && branches.contains("iq/lead-2"),
            "{branches}"
        );
        run_git(dir.path(), &["merge", "-q", "iq/lead"]);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file")).unwrap(),
            "changed\n"
        );
        // Removed by hand, folder and branch, its name is free again.
        std::fs::remove_dir_all(&second.folder).unwrap();
        run_git(dir.path(), &["worktree", "prune"]);
        run_git(dir.path(), &["branch", "-q", "-D", "iq/lead-2"]);
        let third = placer.place("lead").unwrap();
        assert_eq!(third.folder, second.folder);
    }

    #[test]
    fn a_worktree_starts_from_what_the_settings_name_and_is_set_up_as_they_say() {
        let dir = repository();
        let worktrees = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["branch", "base"]);
        std::fs::write(dir.path().join("file"), "later\n").unwrap();
        run_git(dir.path(), &["commit", "-qam", "Later"]);
        let placer = Worktrees::open(
            dir.path(),
            worktrees.path(),
            "base",
            vec!["echo ready > made".to_owned(), "exit 3".to_owned()],
        )
        .unwrap();
        let placed = placer.place("lead").unwrap();
        assert_eq!(
            std::fs::read_to_string(placed.folder.join("file")).unwrap(),
            "kept\n"
        );
        assert!(placed.said.contains("from base ("), "{}", placed.said);
        assert_eq!(
            std::fs::read_to_string(placed.folder.join("made")).unwrap(),
            "ready\n"
        );
        assert!(
            placed
                .said
                .contains("`echo ready > made` ran. `exit 3` failed (exit status 3)"),
            "{}",
            placed.said
        );
        let nowhere = Worktrees::open(dir.path(), worktrees.path(), "no-such", Vec::new()).unwrap();
        assert!(nowhere.place("lead").is_err());
    }

    #[test]
    fn worktrees_untouched_for_long_are_removed_unless_they_hold_work() {
        let dir = repository();
        let worktrees = tempfile::tempdir().unwrap();
        let placer = Worktrees::open(dir.path(), worktrees.path(), "", Vec::new()).unwrap();
        let merged = placer.place("merged").unwrap();
        let dirty = placer.place("dirty").unwrap();
        let apart = placer.place("apart").unwrap();
        std::fs::write(merged.folder.join("file"), "merged\n").unwrap();
        run_git(&merged.folder, &["commit", "-qam", "Merged work"]);
        run_git(dir.path(), &["merge", "-q", "iq/merged"]);
        std::fs::write(dirty.folder.join("draft"), "draft\n").unwrap();
        std::fs::write(apart.folder.join("file"), "apart\n").unwrap();
        run_git(&apart.folder, &["commit", "-qam", "Work apart"]);
        // Nothing is old enough yet; a chat still on the board is kept.
        assert!(placer.prune(Duration::from_secs(3600), &[]).is_empty());
        assert!(
            placer
                .prune(
                    Duration::ZERO,
                    &[merged.folder.clone(), apart.folder.clone()]
                )
                .iter()
                .all(|s| s.contains("dirty")),
        );
        assert!(merged.folder.exists() && apart.folder.exists());
        let said = placer.prune(Duration::ZERO, &[]);
        assert_eq!(said.len(), 3, "{said:?}");
        assert!(
            said.iter()
                .any(|s| s.contains("removed with its branch iq/merged, merged")),
            "{said:?}"
        );
        assert!(
            said.iter()
                .any(|s| s.contains("holds changes not committed: it is kept")),
            "{said:?}"
        );
        assert!(
            said.iter()
                .any(|s| s.contains("its branch iq/apart is kept, not merged")),
            "{said:?}"
        );
        assert!(!merged.folder.exists());
        assert!(dirty.folder.exists());
        assert!(!apart.folder.exists());
        let branches = git(dir.path(), &["branch", "--list", "iq/*"]).unwrap();
        assert!(!branches.contains("iq/merged"), "{branches}");
        assert!(
            branches.contains("iq/dirty") && branches.contains("iq/apart"),
            "{branches}"
        );
    }

    #[test]
    fn outside_a_repository_there_is_no_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let worktrees = tempfile::tempdir().unwrap();
        assert!(Worktrees::open(dir.path(), worktrees.path(), "", Vec::new()).is_err());
    }
}
