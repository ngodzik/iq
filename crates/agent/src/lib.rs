//! iq's agents.
//!
//! A [`Crew`] holds the agents at work in one workspace: models of the
//! provider, which iq runs with its tools. Every action any of them takes
//! goes through the gate, files and commands alike; what the gate asks
//! waits for the person, or for the reviewer when the autopilot is on,
//! which may answer only what stays within the sandbox. The agents write
//! to each other, and to the person, on a board. When the person wants it,
//! each agent they start is given a place of its own by a [`Placer`], a
//! worktree of the project, and the agents it starts work there with it.
//! Each chat is given to a [`Keeper`] at every turn's end, and resumed
//! from what it kept when iq starts again.
//!
//! The crew is generic over [`iq_core::ChatModel`], so that it is tested
//! against scripted models, with no network.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod crew;
mod event;
mod extension;
mod keep;
mod make;
mod once;
mod place;
mod review;
mod rules;
mod team;
mod tools;

pub use crew::{
    ALL, BUSY_MOST, BUSY_TRIES, BUSY_WAIT, COMMAND_QUIET, COMMAND_TIMEOUT, CONTEXT_EVERY, Crew,
    Listed, Posted, REFUSALS, Report, Settings,
};
pub use event::{Event, PERSON};
pub use extension::{Called, Extension};
pub use keep::{Keeper, Saved};
pub use once::{Asked, ask_once};
pub use place::{Placed, Placer};
pub use rules::{RulesFile, with_imports};
pub use team::Pattern;
pub use tools::board as board_tools;
pub use tools::tool_input;
