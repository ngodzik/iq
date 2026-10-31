//! iq's state in the terminal: what it shows and what the person did with
//! it. Draws nothing, so that it is tested without a terminal.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod board;
pub mod command;
pub mod costs;
pub mod crew;
pub mod error;
pub mod input;
pub mod keys;
pub mod look;
pub mod named;
pub mod observations;
pub mod plugins;
pub mod screen;
pub mod steps;

pub use board::{Board, PERIODS, Post, Recall, SyncAsk, Synced, Update};
pub use error::UiError;
pub use iq_agent::Pattern;

/// Seconds since 1970.
#[must_use]
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// How long ago `at` was at `now`, both in seconds since 1970, as the
/// person reads it beside a chat.
///
/// # Examples
///
/// ```
/// assert_eq!(iq_ui::ago(1000, 1030), "just now");
/// assert_eq!(iq_ui::ago(0, 3 * 3600), "3 h ago");
/// assert_eq!(iq_ui::ago(0, 30 * 3600), "yesterday");
/// assert_eq!(iq_ui::ago(0, 80 * 3600), "3 days ago");
/// ```
#[must_use]
pub fn ago(at: u64, now: u64) -> String {
    let seconds = now.saturating_sub(at);
    match seconds {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        86_400..172_800 => "yesterday".to_owned(),
        _ => format!("{} days ago", seconds / 86_400),
    }
}
