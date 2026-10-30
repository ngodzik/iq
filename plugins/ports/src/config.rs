//! The person's settings for the plugin, kept beside iq's in
//! `~/.iq/ports.toml`: how a worktree's stack starts, for every repository
//! or for one.
//!
//! ```toml
//! up = "docker compose up -d"   # how a worktree's stack starts, run in it
//!
//! [repository.shop]             # one repository's, by its folder's name
//! up = "make up"
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::PortsError;

/// How a stack starts when the settings do not say.
pub const UP: &str = "docker compose up -d";

/// The settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    up: Option<String>,
    #[serde(default)]
    repository: BTreeMap<String, Repository>,
}

/// One repository's settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Repository {
    up: Option<String>,
}

impl Config {
    /// The settings in `home`'s `.iq/ports.toml`; none when there is no
    /// such file.
    ///
    /// # Errors
    ///
    /// The file is there but cannot be read, or is not valid.
    pub fn load(home: &Path) -> Result<Self, PortsError> {
        let path = file(home);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).map_err(|source| PortsError::Settings { path, source })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(PortsError::File { path, source }),
        }
    }

    /// How a stack of `repository` starts.
    #[must_use]
    pub fn up(&self, repository: &str) -> &str {
        self.repository
            .get(repository)
            .and_then(|r| r.up.as_deref())
            .or(self.up.as_deref())
            .map(str::trim)
            .filter(|up| !up.is_empty())
            .unwrap_or(UP)
    }
}

/// Where the settings are.
#[must_use]
pub fn file(home: &Path) -> PathBuf {
    home.join(".iq/ports.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stack_starts_as_its_repository_says_else_as_all_do() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(Config::load(home.path()).unwrap().up("shop"), UP);
        std::fs::create_dir_all(home.path().join(".iq")).unwrap();
        std::fs::write(
            file(home.path()),
            "up = \"docker compose up -d --wait\"\n[repository.shop]\nup = \"make up\"\n",
        )
        .unwrap();
        let config = Config::load(home.path()).unwrap();
        assert_eq!(config.up("shop"), "make up");
        assert_eq!(config.up("other"), "docker compose up -d --wait");
        std::fs::write(file(home.path()), "upp = 1").unwrap();
        assert!(Config::load(home.path()).is_err());
    }
}
