//! Each chat's folder of scratch, outside the repository: where its agents
//! keep what outlives one command (a long output read in parts, a snapshot
//! to compare with, notes of their own), so that nothing of it lands among
//! the project's files. It lives in iq's cache, `~/.cache/iq/scratch/
//! <project>/<chat>`, never synced: what is there may go at any time, and
//! goes with its chat.

use std::path::{Path, PathBuf};

/// The folder of `project`'s chats, under `cache`, iq's: each chat's
/// folder under it by the chat's name, as the gate gives it to the
/// commands of the chat's agents.
pub(crate) fn folder(cache: &Path, project: &str) -> PathBuf {
    cache.join("scratch").join(project.replace('/', "-"))
}

/// Takes away the folders under `folder` of the chats not in `kept`, as
/// iq starts: their chats were forgotten or let go, and what they kept
/// with them. Returns how many went.
pub(crate) fn prune(folder: &Path, kept: &[String]) -> usize {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            !kept.contains(&name)
                && entry.file_type().is_ok_and(|kind| kind.is_dir())
                && std::fs::remove_dir_all(entry.path()).is_ok()
        })
        .count()
}

/// Moves the folder of the chat `from` under `folder` to `to`'s, as the
/// person renames the chat: what its agents kept follows it. Only a chat's
/// own name moves (an agent of a chat, `<chat>/<part>`, has no folder of
/// its own), and never onto a folder there already.
pub(crate) fn renamed(folder: &Path, from: &str, to: &str) {
    let plain = |name: &str| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    };
    if !plain(from) || !plain(to) {
        return;
    }
    let (old, new) = (folder.join(from), folder.join(to));
    if old.is_dir() && !new.exists() {
        let _ = std::fs::rename(old, new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_s_folder_follows_its_name_and_goes_with_it() {
        let cache = tempfile::tempdir().unwrap();
        let folder = folder(cache.path(), "github.com/someone/shop");
        assert!(folder.ends_with("scratch/github.com-someone-shop"));
        for chat in ["fix", "docs", "gone"] {
            std::fs::create_dir_all(folder.join(chat)).unwrap();
            std::fs::write(folder.join(chat).join("check.log"), "ok").unwrap();
        }
        renamed(&folder, "fix", "fix-build");
        assert!(folder.join("fix-build/check.log").is_file());
        // An agent's name, or one taken, moves nothing.
        renamed(&folder, "docs/tests", "x");
        renamed(&folder, "docs", "fix-build");
        renamed(&folder, "docs", "../out");
        assert!(folder.join("docs/check.log").is_file());
        assert_eq!(prune(&folder, &["fix-build".into(), "docs".into()]), 1);
        assert!(!folder.join("gone").exists());
        assert!(folder.join("docs").is_dir());
    }
}
