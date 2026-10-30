//! What the plugin keeps of the worktrees it keeps apart, the machine's
//! alone: in `$XDG_STATE_HOME/iq-ports`, else `~/.local/state/iq-ports`,
//! out of what iq syncs. Each worktree's compose project and the ports of
//! its own its services were given, so that they stay the same from one
//! start to the next, the files that move them, the worktree served, and
//! the folder docker is pointed to in a worktree.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::PortsError;

/// A port of a service moved to one of the worktree's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moved {
    /// The service.
    pub service: String,
    /// Its port in the container.
    pub target: u16,
    /// `tcp` or `udp`.
    pub protocol: String,
    /// The machine's port the project's files publish it on.
    pub usual: u16,
    /// The worktree's own, on localhost.
    pub hidden: u16,
}

/// A worktree kept apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    /// Its compose project.
    pub project: String,
    /// Its services' ports moved.
    pub moved: Vec<Moved>,
}

/// All the plugin keeps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kept {
    /// The worktrees, by their folder.
    #[serde(default)]
    pub places: BTreeMap<PathBuf, Place>,
    /// The one the usual ports lead to.
    #[serde(default)]
    pub served: Option<PathBuf>,
}

impl Kept {
    /// Keeps `place` for `folder`, the ports moved before that it does
    /// not move kept beside its own: compose run there with other files
    /// than the usual ones moved them, and their containers hold them.
    pub fn keep(&mut self, folder: PathBuf, place: Place) {
        let before = self.places.remove(&folder);
        let mut place = place;
        let kept = before.into_iter().flat_map(|before| before.moved);
        for moved in kept {
            let known = place.moved.iter().any(|m| m.same_port(&moved));
            let held = place.moved.iter().any(|m| m.hidden == moved.hidden);
            if !known && !held {
                place.moved.push(moved);
            }
        }
        self.places.insert(folder, place);
    }

    /// The place whose compose project is `project`.
    #[must_use]
    pub fn of_project(&self, project: &str) -> Option<(&PathBuf, &Place)> {
        self.places
            .iter()
            .find(|(_, place)| place.project == project)
    }
}

impl Moved {
    /// Whether it is the same port of the same service as `other`.
    #[must_use]
    pub fn same_port(&self, other: &Self) -> bool {
        self.service == other.service
            && self.target == other.target
            && self.protocol == other.protocol
    }
}

/// Where it is kept.
#[derive(Debug, Clone)]
pub struct Store {
    folder: PathBuf,
}

/// The file in the store that holds [`Kept`].
const KEPT: &str = "kept.json";

impl Store {
    /// A store in `folder`.
    #[must_use]
    pub fn at(folder: impl Into<PathBuf>) -> Self {
        Self {
            folder: folder.into(),
        }
    }

    /// The machine's store, for the person whose home is `home`.
    #[must_use]
    pub fn of_machine(home: &Path) -> Self {
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|state| state.is_absolute())
            .unwrap_or_else(|| home.join(".local/state"));
        Self::at(state.join("iq-ports"))
    }

    /// What is kept; nothing when nothing is, or it cannot be read.
    #[must_use]
    pub fn load(&self) -> Kept {
        std::fs::read_to_string(self.folder.join(KEPT))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Keeps `kept`, whole: written beside, then put in place, so that a
    /// reader never finds half of it.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    pub fn save(&self, kept: &Kept) -> Result<(), PortsError> {
        let text = serde_json::to_string_pretty(kept).unwrap_or_default();
        self.write(&self.folder.join(KEPT), &text)
    }

    /// Its folder.
    #[must_use]
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The file that moves the ports of the compose project `project`, as
    /// its usual files are read.
    #[must_use]
    pub fn ports_file(&self, project: &str) -> PathBuf {
        self.folder.join(format!("{project}.yaml"))
    }

    /// The one that moves them as the files `key` names them are read: a
    /// `make` target handing compose files of its own.
    #[must_use]
    pub fn ports_file_of(&self, project: &str, key: u64) -> PathBuf {
        self.folder.join(format!("{project}.{key:016x}.yaml"))
    }

    /// Writes `text` to `file` of the store, whole: written beside, then
    /// put in place, so that compose never reads half of it. The store is
    /// its owner's alone.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    pub fn write(&self, file: &Path, text: &str) -> Result<(), PortsError> {
        self.put(file, text, 0o600)
    }

    /// Writes the program `text` to `file` of the store, as [`Store::write`]
    /// does.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    pub fn write_program(&self, file: &Path, text: &str) -> Result<(), PortsError> {
        self.put(file, text, 0o700)
    }

    fn put(&self, file: &Path, text: &str, mode: u32) -> Result<(), PortsError> {
        use std::io::Write;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let failed = |path: &Path| {
            let path = path.to_path_buf();
            move |source| PortsError::File { path, source }
        };
        let folder = file.parent().unwrap_or(&self.folder);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(folder)
            .map_err(failed(folder))?;
        let mut next = file.as_os_str().to_owned();
        next.push(format!(".{}", std::process::id()));
        let next = PathBuf::from(next);
        let _ = std::fs::remove_file(&next);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&next)
            .and_then(|mut written| written.write_all(text.as_bytes()))
            .map_err(failed(&next))?;
        std::fs::rename(&next, file).map_err(failed(file))
    }

    /// Writes the file that moves the ports of `project`.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    pub fn write_ports_file(&self, project: &str, text: &str) -> Result<PathBuf, PortsError> {
        let file = self.ports_file(project);
        self.write(&file, text)?;
        Ok(file)
    }

    /// Removes every file that moves the ports of `project`.
    pub fn forget(&self, project: &str) {
        let _ = std::fs::remove_file(self.ports_file(project));
        let Ok(entries) = std::fs::read_dir(&self.folder) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let of_files = name
                .strip_prefix(project)
                .and_then(|rest| rest.strip_prefix('.'))
                .and_then(|rest| rest.strip_suffix(".yaml"))
                .is_some_and(|key| key.len() == 16 && key.chars().all(|c| c.is_ascii_hexdigit()));
            if of_files {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    /// The folder docker is pointed to in a worktree, `DOCKER_CONFIG`:
    /// the person's own settings, and the plugin's compose in place of
    /// docker's.
    #[must_use]
    pub fn docker(&self) -> PathBuf {
        self.folder.join("docker")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_kept_is_read_back_whole() {
        let folder = tempfile::tempdir().unwrap();
        let store = Store::at(folder.path().join("iq-ports"));
        assert_eq!(store.load(), Kept::default());
        let kept = Kept {
            places: BTreeMap::from([(
                PathBuf::from("/w/shop/lead"),
                Place {
                    project: "shop-lead".into(),
                    moved: vec![Moved {
                        service: "api".into(),
                        target: 8000,
                        protocol: "tcp".into(),
                        usual: 8000,
                        hidden: 20001,
                    }],
                },
            )]),
            served: Some("/w/shop/lead".into()),
        };
        store.save(&kept).unwrap();
        assert_eq!(store.load(), kept);
        let file = store
            .write_ports_file("shop-lead", "services: {}\n")
            .unwrap();
        assert_eq!(file, store.ports_file("shop-lead"));
        assert_eq!(std::fs::read_to_string(file).unwrap(), "services: {}\n");
        let of_files = store.ports_file_of("shop-lead", 0xab);
        store.write(&of_files, "services: {}\n").unwrap();
        store
            .write_ports_file("shop-lead-2", "services: {}\n")
            .unwrap();
        store.forget("shop-lead");
        assert!(!store.ports_file("shop-lead").exists());
        assert!(!of_files.exists());
        assert!(store.ports_file("shop-lead-2").exists());
    }

    #[test]
    fn a_place_kept_again_keeps_the_ports_it_does_not_move_itself() {
        let moved = |service: &str, target: u16, hidden: u16| Moved {
            service: service.into(),
            target,
            protocol: "tcp".into(),
            usual: target,
            hidden,
        };
        let folder = PathBuf::from("/w/shop/lead");
        let mut kept = Kept::default();
        kept.keep(
            folder.clone(),
            Place {
                project: "shop-lead".into(),
                moved: vec![moved("api", 8000, 20001), moved("db", 5432, 20002)],
            },
        );
        // Kept again without the database, moved by other files: it
        // stays, the API's port as now moved.
        kept.keep(
            folder.clone(),
            Place {
                project: "shop-lead".into(),
                moved: vec![moved("api", 8000, 20005), moved("web", 3000, 20002)],
            },
        );
        assert_eq!(
            kept.places[&folder].moved,
            [moved("api", 8000, 20005), moved("web", 3000, 20002)]
        );
        kept.keep(
            folder.clone(),
            Place {
                project: "shop-lead".into(),
                moved: vec![moved("api", 8000, 20005)],
            },
        );
        assert_eq!(
            kept.places[&folder].moved,
            [moved("api", 8000, 20005), moved("web", 3000, 20002)]
        );
        assert_eq!(kept.of_project("shop-lead").map(|(f, _)| f), Some(&folder));
        assert!(kept.of_project("shop").is_none());
    }
}
