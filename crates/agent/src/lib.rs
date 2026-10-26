//! iq's agents.
//!
//! A [`Crew`] holds the agents at work in one workspace: models of the
//! provider, which iq runs with its tools. Every action any of them takes
//! goes through the gate, files and commands alike; what the gate asks
//! waits for the person, or for the reviewer when the autopilot is on,
//! which may answer only what stays within the sandbox. The agents write
//! to each other, and to the person, on a board.
//!
//! The crew is generic over [`iq_core::ChatModel`], so that it is tested
//! against scripted models, with no network.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod crew;
mod event;
mod extension;
mod once;
mod review;
mod team;
mod tools;

pub use crew::{ALL, Crew, Listed, Posted, Report, Settings};
pub use event::{Event, PERSON};
pub use extension::{Called, Extension};
pub use once::{Answer, Asked, ask_once, ask_once_by};
pub use team::Pattern;
pub use tools::board as board_tools;
