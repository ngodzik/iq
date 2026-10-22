//! iq's agents.
//!
//! A [`Crew`] holds the agents at work in one workspace: models of the
//! provider, which iq runs with its tools, and agents with tools of their
//! own (Claude Code, Codex), which reach iq back through its program. Every
//! action any of them takes goes through the gate, files and commands
//! alike; what the gate asks waits for the person, or for the reviewer when
//! the autopilot is on, which may answer only what stays within the
//! sandbox. The agents write to each other, and to the person, on a board.
//!
//! The crew is generic over [`iq_core::ChatModel`] and
//! [`iq_core::Delegate`], so that it is tested against scripted models,
//! with no network.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod bridge;
mod crew;
mod event;
mod hook;
mod mcp;
mod review;
mod tools;

pub use bridge::{exec_main, hook_main};
pub use crew::{
    AGENT_VAR, ALL, BRIDGE_VAR, Bridge, Crew, EXEC_FLAG, HOOK_FLAG, Listed, MCP_FLAG, Posted,
    Report, Settings,
};
pub use event::{Event, PERSON};
pub use mcp::mcp_main;
pub use tools::board as board_tools;
