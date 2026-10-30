//! Secrets taken out of what commands print, before a model reads it.
//!
//! A sandboxed command gets none of the person's secret variables, but it
//! may read the workspace's files of variables, which the sandbox cannot
//! keep from it within the workspace.

use std::ffi::OsString;
use std::path::Path;

use crate::paths;

/// The shortest value redacted wherever it stands: shorter ones are too
/// common to be told from ordinary words.
const SHORTEST: usize = 8;

/// How deep the workspace is searched for files of variables.
const DEPTH: usize = 4;

/// Folders not searched for them: built or fetched, never the project's own.
const NOT_SEARCHED: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "dist",
    "build",
    ".venv",
    "venv",
];

/// The values to take out of a command's output, each with its name.
#[derive(Clone, Default)]
pub(crate) struct Redactor {
    secrets: Vec<(String, String)>,
}

impl std::fmt::Debug for Redactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Written by hand so that no value reaches a log line.
        f.debug_struct("Redactor")
            .field("secrets", &self.secrets.len())
            .finish()
    }
}

impl Redactor {
    /// The secrets of `env` and of the files of variables of `workspace`.
    pub(crate) fn new(
        env: impl IntoIterator<Item = (OsString, OsString)>,
        workspace: &Path,
    ) -> Self {
        let mut secrets: Vec<(String, String)> = env
            .into_iter()
            .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
            .filter(|(name, value)| secret_name(name) && value.len() >= SHORTEST)
            .collect();
        let mut files = Vec::new();
        variables_files(workspace, DEPTH, &mut files);
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            secrets.extend(
                assignments(&text)
                    .filter(|(name, value)| {
                        value.len() >= SHORTEST && (secret_name(name) || looks_secret(value))
                    })
                    .map(|(name, value)| (name.to_owned(), value.to_owned())),
            );
        }
        // The longest first, so that a part of one is not left over.
        secrets.sort_by_key(|(_, value)| std::cmp::Reverse(value.len()));
        secrets.dedup();
        Self { secrets }
    }

    /// `text` with each secret replaced by its name.
    pub(crate) fn redact(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for (name, value) in &self.secrets {
            if out.contains(value.as_str()) {
                out = out.replace(value.as_str(), &format!("<redacted:{name}>"));
            }
        }
        out
    }
}

/// Whether a variable's name says it holds a secret: one of its words,
/// between underscores, is a token, a key, a password, a session or a
/// cookie. Sockets and the desktop's own variables are not.
pub(crate) fn secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if upper.ends_with("_SOCK") || upper.starts_with("XDG_") || upper.starts_with("DBUS_") {
        return false;
    }
    upper.split('_').any(|word| {
        ["TOKEN", "SECRET", "PASSWORD"]
            .iter()
            .any(|w| word.contains(w))
            || matches!(
                word,
                "KEY"
                    | "APIKEY"
                    | "PASSWD"
                    | "CREDENTIAL"
                    | "CREDENTIALS"
                    | "SESSION"
                    | "COOKIE"
                    | "AUTH"
                    | "DSN"
            )
    })
}

/// Whether a value looks like a secret whatever its name: a password in an
/// address, or a long run of letters and digits.
fn looks_secret(value: &str) -> bool {
    let password_in_address = value.split_once("://").is_some_and(|(_, rest)| {
        rest.split('/')
            .next()
            .and_then(|authority| authority.split_once('@'))
            .is_some_and(|(user, _)| user.contains(':'))
    });
    let token = value.len() >= 16
        && !value.contains(char::is_whitespace)
        && value.chars().any(|c| c.is_ascii_digit())
        && value.chars().any(|c| c.is_ascii_alphabetic());
    password_in_address || token
}

/// The `NAME=value` lines of a file of variables, `export` and quotes taken
/// off.
fn assignments(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (name, value) = line.split_once('=')?;
        let name = name.trim();
        if name.is_empty() || name.starts_with('#') || name.contains(char::is_whitespace) {
            return None;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        Some((name, value))
    })
}

/// The files of variables under `folder`, `depth` levels down.
fn variables_files(folder: &Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if depth > 0 && !NOT_SEARCHED.contains(&name.as_str()) {
                variables_files(&entry.path(), depth - 1, out);
            }
        } else if paths::variables(&name) {
            out.push(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_of_the_environment_and_of_files_are_taken_out() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("api")).unwrap();
        std::fs::write(
            dir.path().join("api/.env.local"),
            "# local\nexport STRIPE_KEY=\"sk_live_0123456789abcdef\"\nHOST=localhost\nDB=postgres://app:hunter22@db:5432/app\n",
        )
        .unwrap();
        let env = [
            (
                OsString::from("GITHUB_TOKEN"),
                OsString::from("ghp_aaaaaaaaaaaaaaaa"),
            ),
            (
                OsString::from("XDG_SESSION_ID"),
                OsString::from("c2-session-long"),
            ),
        ];
        let redactor = Redactor::new(env, dir.path());
        let out = redactor.redact(
            "token ghp_aaaaaaaaaaaaaaaa, key sk_live_0123456789abcdef, host localhost, \
             url postgres://app:hunter22@db:5432/app, c2-session-long",
        );
        assert_eq!(
            out,
            "token <redacted:GITHUB_TOKEN>, key <redacted:STRIPE_KEY>, host localhost, \
             url <redacted:DB>, c2-session-long"
        );
        assert!(!format!("{redactor:?}").contains("ghp_"));
    }
}
