//! Files read and written under a permit, where the gate judged them.
//!
//! A permit holds its path resolved, links followed. A command may have
//! put a link on the way since, as commands write the workspace: a file is
//! touched only when its folder still resolves to where it was judged, and
//! never through a link in its own place.

use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::action::{Action, Permit};
use crate::error::GateError;

/// The largest file read whole: what a model reads is far smaller, and a
/// file larger than this is not source.
const LARGEST: u64 = 8 << 20;

impl Permit {
    /// Reads the file this permit allows reading or writing: what may be
    /// written may be read.
    ///
    /// # Examples
    ///
    /// ```
    /// use iq_gate::{Action, Gate, Judged};
    ///
    /// let workspace = tempfile::tempdir()?;
    /// std::fs::write(workspace.path().join("notes.txt"), "hello")?;
    /// let gate = Gate::builder(workspace.path()).build()?;
    /// let Judged::Allowed(permit) = gate.judge(Action::Read { path: "notes.txt".into() })? else {
    ///     unreachable!()
    /// };
    /// assert_eq!(permit.read()?, b"hello");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn read(&self) -> Result<Vec<u8>, GateError> {
        let path = self.file()?;
        let failed = |source| GateError::File {
            path: path.to_path_buf(),
            source,
        };
        still_there(path)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(failed)?;
        if file.metadata().map_err(failed)?.len() > LARGEST {
            return Err(GateError::TooLarge {
                path: path.to_path_buf(),
                largest: LARGEST,
            });
        }
        let mut bytes = Vec::new();
        file.take(LARGEST + 1)
            .read_to_end(&mut bytes)
            .map_err(failed)?;
        Ok(bytes)
    }

    /// Writes `contents` to the file this permit allows writing, its
    /// folders made when they are not there. The file is replaced whole, a
    /// link in its place with it rather than what the link leads to, and
    /// keeps its permissions.
    pub fn write(&self, contents: &[u8]) -> Result<(), GateError> {
        let Action::Write { path } = &self.action else {
            return Err(GateError::NotAFile);
        };
        let failed = |source| GateError::File {
            path: path.clone(),
            source,
        };
        let parent = path
            .parent()
            .ok_or_else(|| GateError::Moved(path.clone()))?;
        std::fs::create_dir_all(parent).map_err(failed)?;
        still_there(path)?;
        let mode = std::fs::symlink_metadata(path)
            .ok()
            .filter(|meta| meta.is_file())
            .map(|meta| meta.permissions());
        let mut file = tempfile::Builder::new()
            .prefix(".iq-")
            .tempfile_in(parent)
            .map_err(failed)?;
        file.write_all(contents).map_err(failed)?;
        if let Some(mode) = mode {
            file.as_file().set_permissions(mode).map_err(failed)?;
        }
        file.persist(path).map_err(|e| failed(e.error))?;
        Ok(())
    }

    /// The file of a permit to read or write.
    fn file(&self) -> Result<&Path, GateError> {
        match &self.action {
            Action::Read { path } | Action::Write { path } => Ok(path),
            Action::Run { .. } | Action::Fetch { .. } | Action::Tool { .. } => {
                Err(GateError::NotAFile)
            }
        }
    }
}

/// Fails when `path`'s folder no longer resolves to where it was judged,
/// or when a link now stands in its place.
fn still_there(path: &Path) -> Result<(), GateError> {
    let moved = || GateError::Moved(PathBuf::from(path));
    let parent = path.parent().ok_or_else(moved)?;
    if parent.canonicalize().ok().as_deref() != Some(parent) {
        return Err(moved());
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(moved()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use crate::{Action, Gate, Judged};

    fn allowed(gate: &Gate, action: Action) -> crate::Permit {
        match gate.judge(action).unwrap() {
            Judged::Allowed(permit) => permit,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_file_is_written_whole_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let gate = Gate::builder(dir.path()).build().unwrap();
        let permit = allowed(
            &gate,
            Action::Write {
                path: "src/new/lib.rs".into(),
            },
        );
        permit.write(b"fn main() {}\n").unwrap();
        assert_eq!(permit.read().unwrap(), b"fn main() {}\n");
        let read = allowed(
            &gate,
            Action::Read {
                path: "src/new/lib.rs".into(),
            },
        );
        assert!(matches!(read.write(b"no"), Err(crate::GateError::NotAFile)));
    }

    #[test]
    fn a_link_put_on_the_way_after_judging_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let gate = Gate::builder(dir.path()).build().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        let permit = allowed(
            &gate,
            Action::Write {
                path: "src/lib.rs".into(),
            },
        );
        // Between judging and writing, a command swaps the folder for a
        // link out of the workspace.
        std::fs::remove_dir(dir.path().join("src")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("src")).unwrap();
        assert!(matches!(
            permit.write(b"x"),
            Err(crate::GateError::Moved(_))
        ));
        assert!(!outside.path().join("lib.rs").exists());
        // And a link in the file's own place.
        std::fs::remove_file(dir.path().join("src")).unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(outside.path().join("secret"), "s").unwrap();
        let read = allowed(
            &gate,
            Action::Read {
                path: "src/lib.rs".into(),
            },
        );
        std::os::unix::fs::symlink(outside.path().join("secret"), dir.path().join("src/lib.rs"))
            .unwrap();
        assert!(matches!(read.read(), Err(crate::GateError::Moved(_))));
    }
}
