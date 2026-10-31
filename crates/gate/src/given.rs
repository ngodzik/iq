//! Variables the person's own programs give the commands of one place: a
//! plugin of iq's that keeps a worktree's containers apart from the
//! others' names them, so that whatever runs there, an agent's command or
//! the person's, reaches that worktree's and no other.
//!
//! They are the person's choice, as their program makes it, and widen
//! nothing: a command in the sandbox is held as before. What they may not
//! name is what the gate's own workings rest on (where programs and the
//! shell's start come from, the sandbox's folders, git's own variables,
//! which would lead a command to another repository's hooks, and iq's),
//! and a secret, which reaches a command only as the person allows it.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::redact::secret_name;

/// What decides where a command's programs, libraries and shell come
/// from, and where the sandbox puts it.
const KEPT: &[&str] = &[
    "PATH",
    "HOME",
    "SHELL",
    "TMPDIR",
    "ENV",
    "BASH_ENV",
    "IFS",
    "PS4",
    "SHELLOPTS",
    "BASHOPTS",
    "PROMPT_COMMAND",
    "CDPATH",
];

/// The same, by their start: the loaders', git's, iq's, its provider's,
/// and the functions a shell takes from its variables.
const KEPT_PREFIXES: &[&str] = &["LD_", "DYLD_", "GIT_", "IQ_", "REQUESTY_", "BASH_FUNC_"];

/// Why `name` may not be given to the commands of a place, if it may not.
pub(crate) fn refused(name: &str) -> Option<&'static str> {
    let word = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !word {
        return Some("it is not a variable's name");
    }
    if KEPT.contains(&name) || KEPT_PREFIXES.iter().any(|start| name.starts_with(start)) {
        return Some("the sandbox and the shell rest on it");
    }
    if secret_name(name) {
        return Some("it names a secret, which reaches a command only as the person allows it");
    }
    None
}

/// The variables given to each place, by its folder, resolved: shared by
/// every gate made from one, as the places' are.
#[derive(Debug, Clone, Default)]
pub(crate) struct Given(Arc<Mutex<HashMap<PathBuf, BTreeMap<String, String>>>>);

impl Given {
    /// Gives `vars` to the commands of `folder`, in place of what was
    /// given before; none takes them back.
    pub(crate) fn set(&self, folder: PathBuf, vars: BTreeMap<String, String>) {
        if let Ok(mut given) = self.0.lock() {
            if vars.is_empty() {
                given.remove(&folder);
            } else {
                given.insert(folder, vars);
            }
        }
    }

    /// What the commands of the place `workspace` are given.
    pub(crate) fn of(&self, workspace: &Path) -> BTreeMap<String, String> {
        self.0
            .lock()
            .ok()
            .and_then(|given| given.get(workspace).cloned())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_sandbox_rests_on_and_secrets_are_not_given() {
        for name in ["COMPOSE_PROJECT_NAME", "COMPOSE_FILE", "_PORTS", "API_PORT"] {
            assert_eq!(refused(name), None, "{name}");
        }
        for name in [
            "PATH",
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "GIT_DIR",
            "IQ_SANDBOX",
            "BASH_ENV",
            "BASH_FUNC_ls%%",
            "DATABASE_PASSWORD",
            "STRIPE_API_KEY",
            "",
            "1PORT",
            "A-B",
            "A=B",
        ] {
            assert!(refused(name).is_some(), "{name}");
        }
    }

    #[test]
    fn a_place_keeps_what_it_was_given_last_and_nothing_takes_it_back() {
        let given = Given::default();
        let lead = PathBuf::from("/w/lead");
        let vars =
            |value: &str| BTreeMap::from([("COMPOSE_PROJECT_NAME".to_owned(), value.into())]);
        given.set(lead.clone(), vars("shop-lead"));
        given.set(lead.clone(), vars("shop-lead-2"));
        assert_eq!(given.of(&lead), vars("shop-lead-2"));
        assert!(given.of(Path::new("/w/other")).is_empty());
        given.set(lead.clone(), BTreeMap::new());
        assert!(given.of(&lead).is_empty());
    }
}
