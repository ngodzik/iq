//! Each worktree's docker compose stack apart from the others', and the
//! usual ports of localhost led to the one the person chooses.
//!
//! Several worktrees of one project run the same stack: the same services,
//! publishing the same ports, often under the same names. Started side by
//! side as they are, the second fails on a port the first holds, or takes
//! its containers and its data. Nothing of the project is changed to keep
//! them apart: what compose reads besides its files does it.
//!
//! - Each worktree is a compose project of its own, named after the
//!   repository and the worktree: its containers, its networks and its
//!   volumes (its database among them) are its own, kept while it is.
//! - A file of the plugin's, outside the project, moves the ports each
//!   service publishes to ports of the worktree's own on localhost, and
//!   takes off the names a service fixes for its container. Compose reads
//!   it after the project's files, as `COMPOSE_FILE` lists them.
//! - The usual ports (8000, 3000, as the project's files say) are held by
//!   the plugin, which leads each connection to the worktree being served,
//!   so that the browser goes on using `localhost:8000` and what the
//!   front end says of its back end still holds. Switching is instant:
//!   every stack goes on running.
//!
//! The variables that make a worktree's compose its own are given to every
//! command run there: the agents' through iq, the person's through the
//! line they paste in a terminal (`eval "$(iq-ports env)"`). The plugin's
//! own files are kept in `~/.local/state/iq-ports`, the machine's alone.
//!
//! What compose itself cannot be told from outside is said, not hidden: a
//! service that shares the machine's network keeps its ports, a range of
//! ports is not moved, and a `make` target that hands compose its files
//! with `-f` reads the project's alone.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod compose;
pub mod config;
pub mod docker;
pub mod plugin;
pub mod relay;
pub mod stack;
pub mod store;

use std::path::PathBuf;

/// What can go wrong keeping a worktree's stack apart.
#[derive(Debug, thiserror::Error)]
pub enum PortsError {
    /// The folder holds no compose file compose would read.
    #[error("{0} holds no compose file (compose.yaml, docker-compose.yml…)")]
    NoCompose(PathBuf),

    /// A command could not be run, or failed.
    #[error("`{command}` failed: {said}")]
    Command {
        /// The command.
        command: String,
        /// What it said, its end.
        said: String,
    },

    /// What compose resolved could not be read.
    #[error("what `docker compose config` printed could not be read")]
    Config(#[source] serde_json::Error),

    /// A file of the plugin's could not be read or written.
    #[error("{path} could not be read or written")]
    File {
        /// The file.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },

    /// The settings could not be read.
    #[error("{path} is not valid")]
    Settings {
        /// The file.
        path: PathBuf,
        /// Why.
        #[source]
        source: toml::de::Error,
    },

    /// No port is free for a service.
    #[error("no port is free for {0}")]
    NoPort(String),
}
