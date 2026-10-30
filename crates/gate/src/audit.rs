//! The audit log: one line of JSON for each action judged, approved and
//! run, written before the action is taken, so that nothing is done that
//! the log does not show.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::error::GateError;

/// Where the log is written.
#[derive(Debug, Clone)]
pub(crate) struct Audit {
    file: PathBuf,
}

impl Audit {
    pub(crate) fn new(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into() }
    }

    pub(crate) fn file(&self) -> &Path {
        &self.file
    }

    /// Appends `event` with the time, readable by its owner only.
    pub(crate) fn write(&self, event: &impl Serialize) -> Result<(), GateError> {
        let failed = |source| GateError::Audit {
            path: self.file.clone(),
            source,
        };
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut line = serde_json::to_value(event).map_err(|e| failed(std::io::Error::other(e)))?;
        if let Some(fields) = line.as_object_mut() {
            fields.insert("at".to_owned(), at.into());
        }
        let mut text = line.to_string();
        text.push('\n');
        if let Some(folder) = self.file.parent() {
            std::fs::create_dir_all(folder).map_err(failed)?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&self.file)
            .and_then(|mut file| file.write_all(text.as_bytes()))
            .map_err(failed)
    }
}
