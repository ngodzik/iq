//! The chats kept between sessions, one file each under iq's own folder,
//! `~/.iq/chats/<project>/<chat>.json`, the project named as its memory
//! is: an agent and its conversation as they last stood, written at each
//! turn's end, readable by their owner only. When iq starts, the chats of
//! the project come back on the board, resting, and those untouched for as
//! long as the settings say are forgotten.

use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iq_agent::{Keeper, Saved};
use serde::{Deserialize, Serialize};

/// The chats of one project.
pub(crate) struct Chats {
    folder: PathBuf,
}

/// A chat as kept in its file: when, and what.
#[derive(Debug, Serialize, Deserialize)]
struct Kept {
    /// When it was kept, in seconds since 1970.
    at: u64,
    chat: Saved,
}

/// A chat found again.
#[derive(Debug)]
pub(crate) struct Found {
    /// When it was last kept, in seconds since 1970.
    pub(crate) at: u64,
    pub(crate) chat: Saved,
}

impl Chats {
    /// The chats of `project`, kept under `folder`, iq's own.
    pub(crate) fn open(folder: &Path, project: &str) -> Self {
        Self {
            folder: folder.join("chats").join(project.replace('/', "-")),
        }
    }

    /// The chats kept, oldest first.
    pub(crate) fn list(&self) -> Vec<Found> {
        let Ok(entries) = std::fs::read_dir(&self.folder) else {
            return Vec::new();
        };
        let mut found: Vec<Found> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ending| ending == "json"))
            .filter_map(|path| {
                let text = std::fs::read_to_string(&path).ok()?;
                let kept: Kept = serde_json::from_str(&text).ok()?;
                Some(Found {
                    at: kept.at,
                    chat: kept.chat,
                })
            })
            .collect();
        found.sort_by_key(|found| (found.at, found.chat.name.clone()));
        found
    }

    /// Forgets the chats untouched for `keep`, and says which.
    pub(crate) fn prune(&self, keep: Duration) -> Vec<String> {
        let now = now();
        let mut said = Vec::new();
        for found in self.list() {
            let idle = Duration::from_secs(now.saturating_sub(found.at));
            if idle < keep {
                continue;
            }
            let file = self.file(&found.chat.name);
            if std::fs::remove_file(&file).is_ok() {
                said.push(format!(
                    "the chat {}, untouched for {} days, is forgotten",
                    found.chat.name,
                    idle.as_secs() / (24 * 3600)
                ));
            }
        }
        said
    }

    /// The folders of the chats kept, where they have a place of their
    /// own.
    pub(crate) fn places(&self) -> Vec<PathBuf> {
        self.list()
            .into_iter()
            .filter_map(|found| found.chat.folder)
            .collect()
    }

    /// The file of the chat or agent `name`: an agent's name holds its
    /// chat's and a slash, `login/review`, which no file's may, and a name
    /// never holds a `+`.
    fn file(&self, name: &str) -> PathBuf {
        self.folder.join(format!("{}.json", name.replace('/', "+")))
    }
}

impl Keeper for Chats {
    fn forget(&self, name: &str) {
        let _ = std::fs::remove_file(self.file(name));
    }

    fn keep(&self, chat: &Saved) {
        let Ok(text) = serde_json::to_string(&Kept {
            at: now(),
            chat: chat.clone(),
        }) else {
            return;
        };
        // Readable by their owner only, as all of iq's own folder is.
        if std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.folder)
            .is_err()
        {
            return;
        }
        // Whole or not at all: the file is read by the next iq.
        let file = self.file(&chat.name);
        let part = self
            .folder
            .join(format!(".{}.part", chat.name.replace('/', "+")));
        let written = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&part)
            .and_then(|mut out| std::io::Write::write_all(&mut out, text.as_bytes()));
        if written.is_ok() {
            let _ = std::fs::rename(&part, &file);
        }
    }
}

/// Seconds since 1970.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use iq_core::Message;

    fn saved(name: &str) -> Saved {
        Saved {
            name: name.to_owned(),
            model: "lab/model".to_owned(),
            by: "person".to_owned(),
            role: None,
            task: "fix it".to_owned(),
            asked: Vec::new(),
            written: None,
            effort: None,
            folder: Some(PathBuf::from("/place/lead")),
            context: 1200,
            messages: vec![Message::system("be brief"), Message::user("fix it")],
        }
    }

    #[test]
    fn a_chat_is_kept_whole_listed_and_forgotten_once_old() {
        let dir = tempfile::tempdir().unwrap();
        let chats = Chats::open(dir.path(), "github.com/org/repo");
        chats.keep(&saved("lead"));
        // An agent of the chat, named after it.
        chats.keep(&saved("lead/helper"));
        let mut again = saved("lead");
        again.messages.push(Message::User("and the tests".into()));
        chats.keep(&again);
        let listed = chats.list();
        assert_eq!(listed.len(), 2);
        let lead = listed.iter().find(|f| f.chat.name == "lead").unwrap();
        assert_eq!(lead.chat, again);
        assert_eq!(
            chats.places(),
            [PathBuf::from("/place/lead"), PathBuf::from("/place/lead")]
        );
        let file = dir.path().join("chats/github.com-org-repo/lead.json");
        assert!(file.is_file());
        assert!(
            dir.path()
                .join("chats/github.com-org-repo/lead+helper.json")
                .is_file()
        );
        assert!(listed.iter().any(|f| f.chat.name == "lead/helper"));
        let mode = |path: &Path| {
            std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(path).unwrap().permissions())
                & 0o777
        };
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(file.parent().unwrap()), 0o700);
        assert!(
            !dir.path()
                .join("chats/github.com-org-repo/.lead.part")
                .exists()
        );
        // Nothing is old yet; everything is, given no time at all.
        assert!(chats.prune(Duration::from_secs(60)).is_empty());
        let said = chats.prune(Duration::ZERO);
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].contains("is forgotten"));
        assert!(chats.list().is_empty());
        assert!(!file.exists());
    }
}
