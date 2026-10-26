//! What agents remember of a project between sessions: notes, each a
//! Markdown file with its name and a line saying what it holds, kept in a
//! folder of the project's own, named after its repository's address so
//! that the same project on another machine finds them, wherever it was
//! cloned. Every agent of the crew is told which notes there are, reads
//! one with `recall`, and keeps what it learnt with `remember`.
//!
//! ```text
//! memory/github.com/org/repo/
//!   run-the-tests.md
//!   ---
//!   name: Run the tests
//!   description: the database must be up first, with `make db`
//!   ---
//!   The integration tests talk to Postgres on 5433...
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::path::{Path, PathBuf};

use iq_agent::{Called, Crew, Extension};
use iq_core::ToolSpec;
use serde_json::{Value, json};

/// The longest note kept, in bytes.
const NOTE_MOST: usize = 20_000;

/// How many notes the agents are told of by name; the others are counted.
const LISTED: usize = 80;

/// What a secret looks like at its start: keys of the common providers,
/// private keys, tokens. A note holding one is refused.
const SECRET_SIGNS: &[&str] = &[
    "-----BEGIN",
    "sk-",
    "sk_live_",
    "ghp_",
    "gho_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "AKIA",
    "AIza",
];

/// A project's memory.
#[derive(Debug, Clone)]
pub struct Memory {
    folder: PathBuf,
    project: String,
}

/// One note, as listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// Its name.
    pub name: String,
    /// What it holds, in a line.
    pub description: String,
    /// Its file.
    pub file: PathBuf,
}

impl Memory {
    /// The memory of the project in `workspace`, kept under `root`.
    #[must_use]
    pub fn open(root: &Path, workspace: &Path) -> Self {
        let project = project(workspace);
        Self {
            folder: root.join(&project),
            project,
        }
    }

    /// The project's name, as its memory is kept under: its repository's
    /// address, else its folder's name.
    #[must_use]
    pub fn project(&self) -> &str {
        &self.project
    }

    /// The notes, by name.
    #[must_use]
    pub fn notes(&self) -> Vec<Note> {
        let Ok(entries) = std::fs::read_dir(&self.folder) else {
            return Vec::new();
        };
        let mut notes: Vec<Note> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|e| e == "md"))
            .filter_map(|file| {
                let text = std::fs::read_to_string(&file).ok()?;
                let (name, description, _) = parse(&text);
                let fallback = file.file_stem()?.to_string_lossy().into_owned();
                Some(Note {
                    name: name.unwrap_or(fallback),
                    description: description.unwrap_or_default(),
                    file,
                })
            })
            .collect();
        notes.sort_by_key(|note| note.name.to_lowercase());
        notes
    }

    /// Keeps a note, replacing the one of the same name.
    ///
    /// # Errors
    ///
    /// What is wrong with it, for the agent to read: no name, too long, a
    /// secret in it, or the file could not be written.
    pub fn remember(&self, name: &str, description: &str, text: &str) -> Result<String, String> {
        let name = name.trim();
        let slug = slug(name);
        if slug.is_empty() {
            return Err("give the note a name of a few words".into());
        }
        if text.len() > NOTE_MOST {
            return Err(format!(
                "the note is {} bytes: keep it under {NOTE_MOST}, the essentials only",
                text.len()
            ));
        }
        if let Some(sign) = secret(text).or_else(|| secret(description)) {
            return Err(format!(
                "it holds what looks like a secret (`{sign}…`): a secret is never kept in memory"
            ));
        }
        let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let body = format!(
            "---\nname: {}\ndescription: {}\n---\n\n{}\n",
            one_line(name),
            one_line(description),
            text.trim()
        );
        std::fs::create_dir_all(&self.folder)
            .map_err(|e| format!("{}: {e}", self.folder.display()))?;
        let file = self.folder.join(format!("{slug}.md"));
        let replaced = file.exists();
        std::fs::write(&file, body).map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(if replaced {
            format!("The note \"{name}\" is replaced.")
        } else {
            format!("The note \"{name}\" is kept.")
        })
    }

    /// A note's text, by its name.
    ///
    /// # Errors
    ///
    /// There is no such note.
    pub fn recall(&self, name: &str) -> Result<String, String> {
        let note = self.find(name)?;
        let text = std::fs::read_to_string(&note.file)
            .map_err(|e| format!("{}: {e}", note.file.display()))?;
        let (_, _, body) = parse(&text);
        Ok(format!(
            "{}: {}\n\n{}",
            note.name,
            note.description,
            body.trim()
        ))
    }

    /// Removes a note.
    ///
    /// # Errors
    ///
    /// There is no such note, or it could not be removed.
    pub fn forget(&self, name: &str) -> Result<String, String> {
        let note = self.find(name)?;
        std::fs::remove_file(&note.file).map_err(|e| format!("{}: {e}", note.file.display()))?;
        Ok(format!("The note \"{}\" is gone.", note.name))
    }

    fn find(&self, name: &str) -> Result<Note, String> {
        let wanted = slug(name);
        let notes = self.notes();
        notes
            .iter()
            .find(|note| {
                slug(&note.name) == wanted
                    || note
                        .file
                        .file_stem()
                        .is_some_and(|s| s.to_string_lossy() == wanted)
            })
            .cloned()
            .ok_or_else(|| {
                let names: Vec<&str> = notes.iter().map(|n| n.name.as_str()).collect();
                if names.is_empty() {
                    "there is no note yet".to_owned()
                } else {
                    format!(
                        "no note is named \"{name}\": the notes are {}",
                        names.join(", ")
                    )
                }
            })
    }

    /// What the agents are told of the memory.
    #[must_use]
    pub fn told(&self) -> String {
        let notes = self.notes();
        let mut out = format!(
            "Your memory of this project ({}), kept between sessions and machines: notes you \
             and the agents before you wrote. Read one with recall before you rely on it. Keep \
             with remember what you learnt that the project's files do not say and that will \
             help next time: how to build, run or test it, a decision and its reason, what the \
             person prefers or asked not to do. Replace a note rather than add one that \
             contradicts it, and forget one that turns out wrong. Never a secret, nor what the \
             code or git already records.\n",
            self.project
        );
        if notes.is_empty() {
            out.push_str("There is no note yet.");
            return out;
        }
        out.push_str("The notes:\n");
        for note in notes.iter().take(LISTED) {
            out.push_str(&format!("- {}: {}\n", note.name, note.description));
        }
        if notes.len() > LISTED {
            out.push_str(&format!("… and {} more.\n", notes.len() - LISTED));
        }
        out
    }
}

/// The name a project's memory is kept under: the address of its
/// repository's `origin`, as `github.com/org/repo`, the same from any
/// machine; else `local/` and its folder's name.
#[must_use]
pub fn project(workspace: &Path) -> String {
    let address = std::process::Command::new("git")
        .args(["-c", "core.fsmonitor=", "-C"])
        .arg(workspace)
        .args(["remote", "get-url", "origin"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .and_then(|url| from_address(&url));
    address.unwrap_or_else(|| {
        let name = workspace
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!(
            "local/{}",
            segment(&name).unwrap_or_else(|| "project".into())
        )
    })
}

/// A repository's address as a path: `git@github.com:org/repo.git` and
/// `https://github.com/org/repo` both give `github.com/org/repo`.
///
/// # Examples
///
/// ```
/// use iq_memory::from_address;
///
/// assert_eq!(from_address("git@github.com:Org/repo.git").as_deref(), Some("github.com/Org/repo"));
/// assert_eq!(from_address("https://token@GitHub.com/org/repo/").as_deref(), Some("github.com/org/repo"));
/// assert_eq!(from_address(""), None);
/// ```
#[must_use]
pub fn from_address(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = match url.split_once("://") {
        Some((_, rest)) => rest.to_owned(),
        // `user@host:path`, as scp writes it.
        None => url.replacen(':', "/", 1),
    };
    let rest = rest.rsplit_once('@').map_or(rest.as_str(), |(_, r)| r);
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = rest.split('/');
    let host = parts.next()?.split(':').next()?.to_lowercase();
    let mut path = vec![segment(&host)?];
    path.extend(parts.filter_map(segment));
    (path.len() > 1).then(|| path.join("/"))
}

/// A part of a path made safe: letters, digits, dots, dashes and
/// underscores, never `.` or `..`.
fn segment(part: &str) -> Option<String> {
    let kept: String = part
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    (!kept.is_empty() && kept != "." && kept != "..").then_some(kept)
}

/// A note's file name, from its name.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    out.chars().take(60).collect()
}

/// A note's name, description and body.
fn parse(text: &str) -> (Option<String>, Option<String>, &str) {
    let Some(rest) = text.strip_prefix("---\n") else {
        return (None, None, text);
    };
    let Some((head, body)) = rest.split_once("\n---") else {
        return (None, None, text);
    };
    let field = |key: &str| {
        head.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
            .map(|value| value.trim().to_owned())
    };
    (
        field("name"),
        field("description"),
        body.trim_start_matches('\n'),
    )
}

/// The start of what looks like a secret in `text`, if any.
fn secret(text: &str) -> Option<&'static str> {
    SECRET_SIGNS.iter().copied().find(|sign| {
        text.match_indices(sign).any(|(at, _)| {
            let after = &text[at + sign.len()..];
            // A sign followed by a long run of token characters, not a word
            // that happens to start the same.
            sign.starts_with("-----")
                || after
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
                    .count()
                    >= 16
        })
    })
}

/// The memory's tools.
fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "remember".into(),
            description: "Keeps a note about this project for the agents after you, across sessions and machines: what you learnt that its files do not say. A note of the same name is replaced.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "A few words: run-the-tests, the-api-layout."},
                    "description": {"type": "string", "description": "One line saying what it holds, so that others know when to read it."},
                    "text": {"type": "string", "description": "The note itself, short."},
                },
                "required": ["name", "description", "text"],
            }),
        },
        ToolSpec {
            name: "recall".into(),
            description: "Reads a note of this project's memory, by its name.".into(),
            parameters: json!({
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"],
            }),
        },
        ToolSpec {
            name: "forget".into(),
            description: "Removes a note of this project's memory that turned out wrong or useless.".into(),
            parameters: json!({
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"],
            }),
        },
    ]
}

impl<M> Extension<M> for Memory
where
    M: Send + Sync + 'static,
{
    fn tools(&self, _agent: &str) -> Vec<ToolSpec> {
        tools()
    }

    fn instructions(&self, _agent: &str) -> Option<String> {
        Some(self.told())
    }

    fn call(&self, _crew: Crew<M>, _agent: String, name: String, args: Value) -> Called {
        let text = |key: &str| args[key].as_str().unwrap_or_default().to_owned();
        let result = match name.as_str() {
            "remember" => self.remember(&text("name"), &text("description"), &text("text")),
            "recall" => self.recall(&text("name")),
            "forget" => self.forget(&text("name")),
            _ => Err(format!("the memory has no tool named {name:?}")),
        };
        Box::pin(std::future::ready(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_is_known_by_its_repository_wherever_it_was_cloned() {
        for url in [
            "git@github.com:org/repo.git",
            "https://github.com/org/repo",
            "ssh://git@github.com/org/repo.git",
            "https://x-access-token:abc@github.com/org/repo.git/",
        ] {
            assert_eq!(
                from_address(url).as_deref(),
                Some("github.com/org/repo"),
                "{url}"
            );
        }
        assert_eq!(
            from_address("https://gitlab.example.com:8443/a/b/../c.git").as_deref(),
            Some("gitlab.example.com/a/b/c")
        );
        assert_eq!(from_address("nonsense"), None);
    }

    #[test]
    fn notes_are_kept_read_listed_and_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory {
            folder: dir.path().join("github.com/org/repo"),
            project: "github.com/org/repo".into(),
        };
        assert!(memory.told().contains("There is no note yet."));
        assert_eq!(
            memory
                .remember(
                    "Run the tests",
                    "the db must be up first",
                    "make db, then make test"
                )
                .unwrap(),
            "The note \"Run the tests\" is kept."
        );
        memory
            .remember("API layout", "where the routes live", "src/api/routes")
            .unwrap();
        let told = memory.told();
        assert!(
            told.contains(
                "- API layout: where the routes live\n- Run the tests: the db must be up first"
            ),
            "{told}"
        );
        assert!(
            memory
                .recall("run the tests")
                .unwrap()
                .ends_with("make db, then make test")
        );
        assert!(
            memory
                .remember("Run the tests", "now with docker", "docker compose up")
                .unwrap()
                .contains("replaced")
        );
        assert!(
            memory
                .recall("Run the tests")
                .unwrap()
                .contains("docker compose up")
        );
        assert_eq!(memory.notes().len(), 2);
        memory.forget("API layout").unwrap();
        assert!(
            memory
                .recall("API layout")
                .unwrap_err()
                .contains("Run the tests")
        );
        assert!(memory.remember("  ", "x", "y").is_err());
    }

    #[test]
    fn a_secret_is_never_kept() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory {
            folder: dir.path().to_path_buf(),
            project: "p".into(),
        };
        // Made here rather than written whole, so that no scanner takes
        // the source for a leak.
        let token = format!("{}{}", "gh", "p_0123456789abcdefghijklmnopqrstuvwx");
        let refused = memory
            .remember("deploy", "how", &format!("use the token {token}"))
            .unwrap_err();
        assert!(refused.contains("secret"), "{refused}");
        let key = format!("{} OPENSSH {} KEY-----", "-----BEGIN", "PRIVATE");
        assert!(memory.remember("key", "k", &key).is_err());
        // Words that only start the same are no secret.
        assert!(
            memory
                .remember("tasks", "t", "ask-the-person before pushing")
                .is_ok()
        );
    }
}
