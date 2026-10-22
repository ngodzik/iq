use std::path::PathBuf;

/// What can go wrong in the gate or the sandbox. Each fails closed: nothing
/// runs when the gate cannot say what it may do.
#[derive(Debug, thiserror::Error)]
pub enum GateError {
    /// A rules file could not be read as rules.
    #[error("the rules are not valid TOML")]
    Rules(#[source] toml::de::Error),

    /// A rule's own example does not behave as the rule says.
    #[error("rule {prefix:?}: {example:?} {problem}")]
    Example {
        /// The rule's prefix, as written.
        prefix: String,
        /// The example that failed.
        example: String,
        /// What went wrong with it.
        problem: &'static str,
    },

    /// The workspace does not exist or cannot be resolved.
    #[error("the workspace {path} cannot be resolved")]
    Workspace {
        /// The folder given.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },

    /// A reviewer was asked to approve what only the person may: more than
    /// the sandbox holds.
    #[error("only the person may approve what widens the sandbox")]
    Widens,

    /// The permit given to the sandbox is not for a command.
    #[error("the sandbox runs commands only")]
    NotACommand,

    /// The permit is not for the file operation asked: reading takes a
    /// permit to read or write, writing one to write.
    #[error("this permit does not allow it")]
    NotAFile,

    /// A link now stands where the file was judged, or its folder resolves
    /// elsewhere: what would be touched is not what was judged.
    #[error("{0} moved since it was judged")]
    Moved(PathBuf),

    /// A file is too large to be read whole.
    #[error("{path} is larger than {largest} bytes")]
    TooLarge {
        /// The file.
        path: PathBuf,
        /// The most read.
        largest: u64,
    },

    /// A file could not be read or written.
    #[error("{path} could not be read or written")]
    File {
        /// The file.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },

    /// The operating system cannot hold a command in a sandbox, so it is
    /// not run.
    #[error("no sandbox on this system: {0}")]
    Unsupported(String),

    /// The sandbox could not be set up.
    #[error("the sandbox could not be set up: {0}")]
    Confine(String),

    /// The command could not be started or waited on.
    #[error("the command could not be run")]
    Spawn(#[source] std::io::Error),

    /// The audit log could not be written, so the action is not taken.
    #[error("the audit log {path} cannot be written")]
    Audit {
        /// The log's file.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },
}
