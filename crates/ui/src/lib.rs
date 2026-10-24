//! The window's state: what it shows and what the person did with it.
//! Draws nothing, so that it is tested without a window.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod assay;
pub mod board;
pub mod costs;
pub mod crew;
pub mod error;
pub mod keys;
pub mod layout;
pub mod look;
pub mod steward;
pub mod subscriptions;

pub use board::{Board, PERIODS, Post, Read, Synced, Update};
pub use error::UiError;
pub use iq_agent::Pattern;

/// Seconds since 1970.
#[must_use]
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
