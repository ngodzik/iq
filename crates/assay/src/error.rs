use std::path::PathBuf;

/// What can go wrong with the assay's folder, its settings and its runs.
#[derive(Debug, thiserror::Error)]
pub enum AssayError {
    /// A file or folder of the store could not be read or written.
    #[error("{path}")]
    Io {
        /// The file or folder.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },
    /// An exercise's settings, or the assay's, are not what it reads.
    #[error("{path} is not as the assay reads it")]
    Toml {
        /// The file.
        path: PathBuf,
        /// What is wrong.
        #[source]
        source: toml::de::Error,
    },
    /// A record could not be read or written as JSON.
    #[error("{path} is not a valid record")]
    Json {
        /// The file.
        path: PathBuf,
        /// What is wrong.
        #[source]
        source: serde_json::Error,
    },
    /// A model the assay's settings name cannot be one.
    #[error("{path}: `{key}` names no model")]
    Model {
        /// The settings' file.
        path: PathBuf,
        /// The setting.
        key: &'static str,
        /// Why.
        #[source]
        source: iq_core::CoreError,
    },
    /// No exercise of that name.
    #[error("no exercise is named `{0}`")]
    NoExercise(String),
    /// An exercise that is not ready for what was asked, and why.
    #[error("{0}")]
    NotReady(String),
    /// The gate of a run's folder could not be made.
    #[error("the gate of {path} could not be made")]
    Gate {
        /// The folder.
        path: PathBuf,
        /// Why.
        #[source]
        source: iq_gate::GateError,
    },
}

impl AssayError {
    pub(crate) fn io(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { path, source }
    }
}
