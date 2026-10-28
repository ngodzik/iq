//! A lock beside one of iq's files, held by whatever adds to the file or
//! writes it again, in this process or another: two iq running at once
//! take turns, so that a line one adds is never lost to the other's
//! rewrite.

use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Runs `act` holding the lock of `file`. Where the lock cannot be had, as
/// on a file system without locks, `act` runs all the same: a line at risk
/// is better than none kept.
pub(crate) fn holding<T>(file: &Path, act: impl FnOnce() -> T) -> T {
    let held = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(lock_of(file))
        .and_then(|lock| {
            lock.lock()?;
            Ok(lock)
        });
    if let Err(error) = &held {
        tracing::warn!("could not lock {}: {error}", file.display());
    }
    let done = act();
    // Dropping the file lets the lock go.
    drop(held);
    done
}

/// The lock's file: `billed.jsonl` is locked by `billed.jsonl.lock`.
fn lock_of(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}
