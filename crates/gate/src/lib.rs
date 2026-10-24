//! The one gate every action of iq's models and agents goes through, and the
//! sandbox the commands it allows run in.
//!
//! An [`Action`] is judged by [`Gate::judge`]: allowed, asked or denied, by
//! what it reaches and by the [`Rules`], and written to the audit log before
//! anything is done. Only the gate makes a [`Permit`], so whatever does an
//! action, the [`Sandbox`] first, takes one: the compiler refuses a way
//! around the gate.
//!
//! The sandbox is the boundary. A command reads the system, the toolchains
//! and the workspace, writes only the workspace, a folder of its own and the
//! toolchains' caches, and reaches no network unless its permit says; it
//! gets none of the person's secrets, and what it prints is cleared of
//! those its workspace's files hold. The rules only decide what needs
//! asking among what the sandbox lets through and cannot take back, such
//! as `git push` or `git reset --hard`; a reviewing model may approve only
//! what stays within it.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod action;
mod audit;
mod confine;
mod error;
mod files;
mod gate;
mod paths;
mod redact;
mod rules;
mod sandbox;
#[cfg(any(target_os = "macos", test))]
mod seatbelt;
mod shell;
mod watch;

pub use action::{Action, Approver, Judged, Pending, Permit, Scope};
pub use error::GateError;
pub use gate::{Gate, GateBuilder};
pub use rules::{Decision, Rules};
pub use sandbox::{
    HELPER_FLAG, HOLD_FLAG, Helper, Ran, Sandbox, helper_failure, hold_network, sandboxed,
};
