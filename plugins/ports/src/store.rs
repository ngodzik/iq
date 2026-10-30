//! What the plugin keeps of the worktrees it keeps apart, the machine's
//! alone: in `$XDG_STATE_HOME/iq-ports`, else `~/.local/state/iq-ports`,
//! out of what iq syncs. Each worktree's compose project and the ports of
//! its own its services were given, so that they stay the same from one
//! start to the next, the file that moves them, and the worktree served.

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
        let failed = |path: &Path| {
            let path = path.to_path_buf();
            move |source| PortsError::File { path, source }
        };
        std::fs::create_dir_all(&self.folder).map_err(failed(&self.folder))?;
        let file = self.folder.join(KEPT);
        let next = self.folder.join(format!("{KEPT}.{}", std::process::id()));
        let text = serde_json::to_string_pretty(kept).unwrap_or_default();
        std::fs::write(&next, text).map_err(failed(&next))?;
        std::fs::rename(&next, &file).map_err(failed(&file))
    }

    /// Its folder.
    #[must_use]
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The file that moves the ports of the compose project `project`.
    #[must_use]
    pub fn ports_file(&self, project: &str) -> PathBuf {
        self.folder.join(format!("{project}.yaml"))
    }

    /// Writes that file.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    pub fn write_ports_file(&self, project: &str, text: &str) -> Result<PathBuf, PortsError> {
        let file = self.ports_file(project);
        std::fs::create_dir_all(&self.folder).map_err(|source| PortsError::File {
            path: self.folder.clone(),
            source,
        })?;
        std::fs::write(&file, text).map_err(|source| PortsError::File {
            path: file.clone(),
            source,
        })?;
        Ok(file)
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
    }
}
