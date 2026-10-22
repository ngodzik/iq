//! What a model or an agent asks to do, and the gate's answer.
//!
//! A [`Permit`] can only be made by the gate, so that whatever takes one,
//! the sandbox first, is sure the action was judged: the compiler refuses
//! a path around the gate.

use std::path::PathBuf;

use serde::Serialize;

/// Something a model or an agent asks to do on the machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Reading a file.
    Read {
        /// The file.
        path: PathBuf,
    },
    /// Writing a file, made if it is not there.
    Write {
        /// The file.
        path: PathBuf,
    },
    /// Running a command line, as a shell reads it.
    Run {
        /// The line.
        line: String,
        /// The folder it runs in.
        folder: PathBuf,
        /// Whether it asks to reach the network, which the sandbox withholds
        /// otherwise.
        network: bool,
    },
    /// Reaching the network outside a command: fetching a page, searching
    /// the web. Always asked of the person: what leaves cannot be taken
    /// back.
    Fetch {
        /// What is fetched or searched for, as the agent wrote it.
        target: String,
    },
}

/// The gate's answer to an action.
#[derive(Debug)]
#[must_use]
pub enum Judged {
    /// It may be done as it is.
    Allowed(Permit),
    /// It waits for the person, or for a reviewer acting for them.
    Asked {
        /// Why it is not allowed as it is.
        reason: String,
        /// What approving it gives.
        pending: Pending,
    },
    /// It is not done, whoever asks.
    Denied {
        /// Why.
        reason: String,
    },
}

/// Leave to do one action, made only by the gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permit {
    pub(crate) action: Action,
}

impl Permit {
    /// The action it allows.
    pub fn action(&self) -> &Action {
        &self.action
    }
}

/// An action waiting for approval: [`crate::Gate::approve`] turns it into a
/// [`Permit`], and writes down who approved it.
#[derive(Debug)]
#[must_use]
pub struct Pending {
    pub(crate) permit: Permit,
    pub(crate) widens: bool,
}

impl Pending {
    /// The action waiting.
    pub fn action(&self) -> &Action {
        &self.permit.action
    }

    /// Whether approving it gives more than the sandbox holds (the network,
    /// a file outside the workspace, where code runs later): only the person
    /// may approve that.
    pub fn widens(&self) -> bool {
        self.widens
    }
}

/// Who approved an action that was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Approver {
    /// The person.
    Person,
    /// A model reviewing for them, which may approve only what stays within
    /// the sandbox.
    Reviewer,
}
