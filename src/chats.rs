//! The chats kept between sessions, one file each under iq's own folder,
//! `~/.iq/chats/<project>/<chat>.json`, the project named as its memory
//! is: an agent and its conversation as they last stood, written at each
//! turn's end, readable by their owner only. When iq starts, the chats of
//! the project come back on the board, resting, and those untouched for as
//! long as the settings say are forgotten. An agent another started that
//! is done with its task is retired: its file goes to `retired/`, read
//! back never on the board, only shown with its chat, and forgotten with
//! it.

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
        Self::read(&self.folder)
    }

    /// The chats kept in `folder`, oldest first.
    fn read(folder: &Path) -> Vec<Found> {
        let Ok(entries) = std::fs::read_dir(folder) else {
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

    /// The agents retired, oldest first: what they did, to be shown with
    /// their chats.
    pub(crate) fn retired(&self) -> Vec<Found> {
        Self::read(&self.folder.join(RETIRED))
    }

    /// Forgets the chats untouched for `keep`, and says which; and the
    /// agents retired whose chat is forgotten, or as old.
    pub(crate) fn prune(&self, keep: Duration) -> Vec<String> {
        let said = self.prune_chats(keep);
        let kept: Vec<String> = self
            .list()
            .into_iter()
            .map(|found| found.chat.name)
            .collect();
        let now = now();
        if let Ok(entries) = std::fs::read_dir(self.folder.join(RETIRED)) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let chat = name.split(['+', '@', '.']).next().unwrap_or_default();
                let old = entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
                    .is_some_and(|at| now.saturating_sub(at.as_secs()) >= keep.as_secs());
                if old || !kept.iter().any(|kept| kept == chat) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        said
    }

    /// Forgets the chats untouched for `keep`, and says which.
    fn prune_chats(&self, keep: Duration) -> Vec<String> {
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

    fn retire(&self, name: &str) {
        let folder = self.folder.join(RETIRED);
        if std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&folder)
            .is_err()
        {
            return;
        }
        // Apart from the next to take its name: when it retired.
        let file = folder.join(format!("{}@{}.json", name.replace('/', "+"), now_nanos()));
        let _ = std::fs::rename(self.file(name), file);
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

/// Moves `chat` to `lead`, the settings' lead, at `at`, when it follows
/// the settings and runs on another: the person changed their lead since.
/// Returns what the person is told of it.
pub(crate) fn on_the_lead(chat: &mut Saved, lead: &str, at: u64) -> Option<String> {
    if !chat.follows_settings() || chat.model == lead {
        return None;
    }
    let was = chat.model.clone();
    chat.move_to(lead, iq_agent::PERSON, iq_agent::SETTINGS_MODEL, at);
    Some(format!(
        "{} answers on {lead}, the settings' lead, from its next turn (it was on {was})",
        chat.name
    ))
}

/// Where the agents retired are kept, under a project's chats.
const RETIRED: &str = "retired";

/// Nanoseconds since 1970: apart for two agents retired in one second.
fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
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
            models_chosen: vec![iq_agent::ModelChoice {
                model: "lab/model".to_owned(),
                by: "person".to_owned(),
                how: "/new".to_owned(),
                at: 1,
            }],
            calls: 3,
            spent: iq_core::Usd(0.25),
            aside: vec![iq_agent::Aside {
                at: 2,
                turn: 1,
                after: 2,
                shown: iq_agent::Shown::Question {
                    id: 4,
                    question: "which one?".to_owned(),
                    choices: vec!["a".to_owned(), "b".to_owned()],
                    answered: true,
                    answer: Some("b".to_owned()),
                },
            }],
            set_apart: vec!["the whole of an old result".to_owned()],
            messages: vec![Message::system("be brief"), Message::user("fix it")],
            compacted_away: Vec::new(),
            compacted_head: 0,
        }
    }

    #[test]
    fn a_chat_on_the_settings_model_follows_them_and_one_chosen_stays() {
        let mut chat = saved("fix");
        chat.models_chosen[0].how = iq_agent::SETTINGS_MODEL.to_owned();
        chat.messages.push(Message::Assistant {
            content: Some("Fixed.".into()),
            tool_calls: Vec::new(),
            reasoning_signature: Some(serde_json::json!("c2lnbmVk")),
        });
        assert_eq!(
            on_the_lead(&mut chat, "lab/haiku", 9).as_deref(),
            Some(
                "fix answers on lab/haiku, the settings' lead, from its next turn (it was on lab/model)"
            )
        );
        assert_eq!(chat.model, "lab/haiku");
        let last = chat.models_chosen.last().unwrap();
        assert_eq!((last.how.as_str(), last.at), (iq_agent::SETTINGS_MODEL, 9));
        // Its answers go on without the reasoning another model signed.
        assert!(chat.messages.iter().all(|message| !matches!(
            message,
            Message::Assistant {
                reasoning_signature: Some(_),
                ..
            }
        )));
        // On the lead, and once it was moved, it stays.
        assert_eq!(on_the_lead(&mut chat, "lab/haiku", 10), None);
        assert_eq!(chat.models_chosen.len(), 2);
        // A model the person chose for it stays, as does an agent's.
        let mut chosen = saved("chosen");
        chosen.models_chosen[0].how = "/model".to_owned();
        assert_eq!(on_the_lead(&mut chosen, "lab/haiku", 9), None);
        let mut started = saved("lead/tests");
        started.by = "lead".to_owned();
        started.models_chosen.clear();
        assert_eq!(on_the_lead(&mut started, "lab/haiku", 9), None);
        // One kept before choices were, the settings'.
        let mut older = saved("older");
        older.models_chosen.clear();
        assert!(on_the_lead(&mut older, "lab/haiku", 9).is_some());
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
        // A retired agent, kept apart, never listed among the chats.
        chats.retire("lead/helper");
        assert_eq!(chats.list().len(), 1);
        let retired = chats.retired();
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].chat.name, "lead/helper");
        chats.keep(&saved("lead/helper"));
        // Nothing is old yet; everything is, given no time at all.
        assert!(chats.prune(Duration::from_secs(60)).is_empty());
        assert_eq!(chats.retired().len(), 1);
        let said = chats.prune(Duration::ZERO);
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].contains("is forgotten"));
        assert!(chats.list().is_empty());
        assert!(!file.exists());
        // Gone with its chat.
        assert!(chats.retired().is_empty());
    }
}
