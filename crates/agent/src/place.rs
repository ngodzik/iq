//! A place of its own for each agent the person starts, when they want
//! one: a folder apart from their checkout, as a worktree of the project's
//! repository is, so that what they are doing is left as it is while the
//! agents work. The crew asks for it as the agent joins, and the agents it
//! starts in turn work there with it.

use std::path::PathBuf;

/// Gives each agent the person starts a place of its own.
pub trait Placer: Send + Sync {
    /// A place for the agent `name`, made now.
    ///
    /// # Errors
    ///
    /// Why no place could be made: the agent is not started, and the
    /// person is told.
    fn place(&self, name: &str) -> Result<Placed, String>;
}

/// A place made for an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// Its folder, the workspace its actions are judged in.
    pub folder: PathBuf,
    /// Folders beside it that commands may write too: the repository's own
    /// folder, for a worktree, which keeps its index there and takes its
    /// commits to it.
    pub writable: Vec<PathBuf>,
    /// What the person is told of it: where it is, and how to take its
    /// work.
    pub said: String,
}
