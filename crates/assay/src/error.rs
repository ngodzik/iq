use std::path::PathBuf;

/// What can go wrong with the assay's store and runs.
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
    /// An exercise's settings are not what the store expects.
    #[error("{path} is not a valid exercise file")]
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
    /// No exercise of that name.
    #[error("no exercise is named `{0}`")]
    NoExercise(String),
    /// An exercise that is not ready for what was asked, and why.
    #[error("{0}")]
    NotReady(String),
    /// The gate refused something the assay needed.
    #[error("the gate refused it: {0}")]
    Gate(String),
}

impl AssayError {
    pub(crate) fn io(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { path, source }
    }
}
