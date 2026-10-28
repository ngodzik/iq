//! A place of its own for each chat the person starts, when they want it:
//! a worktree of the project's repository, on a branch of its own from
//! what is checked out, kept in iq's folder of data. The person's checkout
//! is left as it is while the agents work, and their work is a branch to
//! merge, or to drop. The worktree stays until the person removes it: what
//! is not committed there is theirs to look at.

use std::path::{Path, PathBuf};

use iq_agent::{Placed, Placer};

use crate::journal::git_command;

/// The branches made are named under this.
const BRANCHES: &str = "iq/";

/// How many names are tried for one chat before giving up: the chat's,
/// then numbered.
const MOST_NAMES: u32 = 50;

/// Makes worktrees of one repository.
pub(crate) struct Worktrees {
    /// The repository's top, the person's checkout.
    top: PathBuf,
    /// The repository's own folder, where worktrees keep their index and
    /// their commits go.
    common: PathBuf,
    /// Where this repository's worktrees are made.
    folder: PathBuf,
}

impl Worktrees {
    /// Worktrees of the repository `workspace` is in, made under
    /// `folder`.
    ///
    /// # Errors
    ///
    /// `workspace` is not in a repository.
    pub(crate) fn open(workspace: &Path, folder: &Path) -> Result<Self, String> {
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
        Ok(Self {
            top,
            common,
            folder: folder.join(name),
        })
    }
}

impl Placer for Worktrees {
    fn place(&self, name: &str) -> Result<Placed, String> {
        let head = git(&self.top, &["rev-parse", "--short", "HEAD"])
            .map_err(|e| format!("the repository has no commit yet ({e})"))?;
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
                "HEAD",
            ],
        )?;
        Ok(Placed {
            folder: folder.clone(),
            writable: vec![self.common.clone()],
            said: format!(
                "works in a worktree of its own, {}, on the branch {branch} from {head}: your \
                 checkout is left as it is, and what is not committed there is not in the \
                 worktree. `git merge {branch}` takes its work; `git worktree remove {}` removes \
                 the worktree after.",
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
        let placer = Worktrees::open(&sub, worktrees.path()).unwrap();
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
    fn outside_a_repository_there_is_no_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let worktrees = tempfile::tempdir().unwrap();
        assert!(Worktrees::open(dir.path(), worktrees.path()).is_err());
    }
}
