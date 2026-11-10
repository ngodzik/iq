//! Folders as the assay handles them: copied for a candidate to work in,
//! told apart by a digest, and compared file by file once it is done.

use std::path::{Path, PathBuf};

use iq_core::Change;

use crate::error::AssayError;

/// What a project's tools fill and its authors do not write: kept out of
/// what a candidate is said to have changed.
const GENERATED: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".gradle",
    ".next",
    ".dart_tool",
    ".turbo",
];

/// The largest file compared line by line; a larger one is said to have
/// changed, no more.
const COMPARED_BYTES: u64 = 1 << 20;

/// Every file under `root`, by its path relative to it, in order: links
/// as links, never followed; what a project's tools fill left out when
/// `all` is false.
pub(crate) fn files(root: &Path, all: bool) -> Result<Vec<PathBuf>, AssayError> {
    let mut found = Vec::new();
    if !root.exists() {
        return Ok(found);
    }
    let mut folders = vec![PathBuf::new()];
    while let Some(folder) = folders.pop() {
        let at = root.join(&folder);
        let entries = std::fs::read_dir(&at).map_err(AssayError::io(&at))?;
        for entry in entries {
            let entry = entry.map_err(AssayError::io(&at))?;
            let relative = folder.join(entry.file_name());
            let kind = entry.file_type().map_err(AssayError::io(entry.path()))?;
            if kind.is_dir() {
                let generated = GENERATED.iter().any(|g| entry.file_name() == *g);
                if all || !generated {
                    folders.push(relative);
                }
            } else {
                found.push(relative);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Copies the folder `from` into `to`, made if it is not there: files with
/// their permissions, links as links.
pub(crate) fn copy(from: &Path, to: &Path) -> Result<(), AssayError> {
    std::fs::create_dir_all(to).map_err(AssayError::io(to))?;
    for relative in files(from, true)? {
        let source = from.join(&relative);
        let target = to.join(&relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(AssayError::io(parent))?;
        }
        let meta = std::fs::symlink_metadata(&source).map_err(AssayError::io(&source))?;
        if meta.file_type().is_symlink() {
            let link = std::fs::read_link(&source).map_err(AssayError::io(&source))?;
            let _ = std::fs::remove_file(&target);
            std::os::unix::fs::symlink(&link, &target).map_err(AssayError::io(&target))?;
        } else {
            std::fs::copy(&source, &target).map_err(AssayError::io(&target))?;
        }
    }
    Ok(())
}

/// A digest of what `paths` hold, each a file or a folder under `root`:
/// FNV-1a over every file's path and bytes, in order. It tells that an
/// exercise changed, not who changed it: it is no seal.
pub(crate) fn digest(root: &Path, paths: &[&str]) -> Result<String, AssayError> {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    let mut eat = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
    };
    for path in paths {
        let at = root.join(path);
        let listed: Vec<PathBuf> = if at.is_dir() {
            files(&at, true)?
                .into_iter()
                .map(|file| Path::new(path).join(file))
                .collect()
        } else if at.exists() {
            vec![PathBuf::from(path)]
        } else {
            Vec::new()
        };
        for file in listed {
            let full = root.join(&file);
            eat(file.to_string_lossy().as_bytes());
            eat(&[0]);
            let meta = std::fs::symlink_metadata(&full).map_err(AssayError::io(&full))?;
            let bytes = if meta.file_type().is_symlink() {
                std::fs::read_link(&full)
                    .map_err(AssayError::io(&full))?
                    .to_string_lossy()
                    .into_owned()
                    .into_bytes()
            } else {
                std::fs::read(&full).map_err(AssayError::io(&full))?
            };
            eat(&(bytes.len() as u64).to_le_bytes());
            eat(&bytes);
        }
    }
    Ok(format!("{hash:016x}"))
}

/// What a candidate changed: the text files compared line by line, and the
/// others, too large or not text, named.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changed {
    /// The text files changed, made or removed.
    pub changes: Vec<Change>,
    /// The other files changed, made or removed.
    pub others: Vec<String>,
}

impl Changed {
    /// The lines added, in every text file.
    #[must_use]
    pub fn added(&self) -> usize {
        self.changes.iter().map(Change::added).sum()
    }

    /// The lines removed.
    #[must_use]
    pub fn removed(&self) -> usize {
        self.changes.iter().map(Change::removed).sum()
    }

    /// How many files changed.
    #[must_use]
    pub fn files(&self) -> usize {
        self.changes.len() + self.others.len()
    }

    /// The changes as one unified diff, the other files named after it.
    #[must_use]
    pub fn patch(&self) -> String {
        let mut out: String = self.changes.iter().map(Change::patch).collect();
        if !self.others.is_empty() {
            out.push_str(&format!(
                "\nOther files changed (not text, or too large to compare): {}\n",
                self.others.join(", ")
            ));
        }
        out
    }
}

/// What changed from the folder `before` to the folder `after`, what a
/// project's tools fill left out.
pub(crate) fn compare(before: &Path, after: &Path) -> Result<Changed, AssayError> {
    let old = files(before, false)?;
    let new = files(after, false)?;
    let mut all: Vec<&PathBuf> = old.iter().chain(&new).collect();
    all.sort();
    all.dedup();
    let mut changed = Changed::default();
    for relative in all {
        let a = read(&before.join(relative));
        let b = read(&after.join(relative));
        if a == b {
            continue;
        }
        let name = relative.to_string_lossy().into_owned();
        match (text(&a), text(&b)) {
            (Some(a), Some(b)) => changed.changes.push(Change::between(name, a, b, true)),
            _ => changed.others.push(name),
        }
    }
    Ok(changed)
}

/// What a file holds, for comparing: `None` when it is not there, and its
/// size alone, as a mark, when it is too large to compare.
#[derive(PartialEq)]
enum Held {
    Bytes(Vec<u8>),
    Large(u64),
}

fn read(path: &Path) -> Option<Held> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        let link = std::fs::read_link(path).ok()?;
        return Some(Held::Bytes(link.to_string_lossy().as_bytes().to_vec()));
    }
    if meta.len() > COMPARED_BYTES {
        return Some(Held::Large(meta.len()));
    }
    std::fs::read(path).ok().map(Held::Bytes)
}

/// The text of what is held, empty when the file is not there; `None`
/// when it is not text, or too large.
fn text(held: &Option<Held>) -> Option<&str> {
    match held {
        None => Some(""),
        Some(Held::Bytes(bytes)) => std::str::from_utf8(bytes).ok(),
        Some(Held::Large(_)) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, path: &str, text: &str) {
        let at = root.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, text).unwrap();
    }

    #[test]
    fn a_copy_compares_equal_and_a_change_is_told_line_by_line() {
        let dir = tempfile::tempdir().unwrap();
        let start = dir.path().join("start");
        write(&start, "calc.py", "def add(a, b):\n    return a + b\n");
        write(&start, "lib/notes.txt", "keep\n");
        let work = dir.path().join("work");
        copy(&start, &work).unwrap();
        assert_eq!(compare(&start, &work).unwrap(), Changed::default());
        write(
            &work,
            "calc.py",
            "def add(a, b):\n    return a + b\n\n\ndef mul(a, b):\n    return a * b\n",
        );
        write(&work, "test_calc.py", "from calc import mul\n");
        write(&work, "__pycache__/calc.pyc", "\0\0");
        std::fs::remove_file(work.join("lib/notes.txt")).unwrap();
        std::fs::write(work.join("blob.bin"), [0xff, 0xfe, 0x00]).unwrap();
        let changed = compare(&start, &work).unwrap();
        let names: Vec<&str> = changed.changes.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(names, ["calc.py", "lib/notes.txt", "test_calc.py"]);
        assert_eq!(changed.others, ["blob.bin"]);
        assert_eq!((changed.added(), changed.removed()), (5, 1));
        assert_eq!(changed.files(), 4);
        assert!(changed.patch().contains("+def mul(a, b):"));
        assert!(changed.patch().contains("Other files changed"));
    }

    #[test]
    fn the_digest_follows_what_the_files_hold() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "task.md", "Add mul.\n");
        write(root, "start/calc.py", "x\n");
        let first = digest(root, &["task.md", "start", "checks"]).unwrap();
        assert_eq!(
            first,
            digest(root, &["task.md", "start", "checks"]).unwrap()
        );
        write(root, "start/calc.py", "y\n");
        let second = digest(root, &["task.md", "start", "checks"]).unwrap();
        assert_ne!(first, second);
        // A file moved is a change too.
        std::fs::rename(root.join("start/calc.py"), root.join("start/other.py")).unwrap();
        assert_ne!(
            second,
            digest(root, &["task.md", "start", "checks"]).unwrap()
        );
    }
}
