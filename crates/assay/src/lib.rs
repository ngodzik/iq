//! The assay: the same frozen exercises given to agents, each in a folder
//! of its own, checked by hidden tests a deterministic command runs, then
//! graded blind by judges; summed by iq into what each candidate does and
//! costs, to find, for each kind of task, the cheapest agent that does it
//! as well as the person's lead.
//!
//! A judge, one of the strongest models (a subscription's, as it costs
//! less), talks with the person through the crew: it proposes the kinds of
//! task and the exercises, writes them in the store, and asks for runs
//! through its tools, which ask the person, with what they should cost,
//! before anything is spent. The store's files are the reference: plain
//! files, which a copy elsewhere is made from.
//!
//! The assay knows no provider: it is generic over [`iq_core::ChatModel`]
//! and [`iq_core::Delegate`], and its candidates are any of them.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod assay;
mod candidate;
mod commit;
mod error;
mod estimate;
mod exercise;
pub mod judge;
mod prompt;
mod record;
pub mod report;
mod roster;
mod run;
mod store;
mod tally;
mod tree;

pub use assay::{Assay, JUDGE, Overview, Settings, Summary, Told};
pub use candidate::{Candidate, Kind};
pub use error::AssayError;
pub use estimate::{Guess, guess};
pub use exercise::{Exercise, Frozen, MOST_MINUTES, State, valid_id};
pub use record::{Checked, Ending, Record, Spent, Verdict};
pub use report::{Row, Table, cheapest_fit, rows, table};
pub use roster::{Allowed, Offer, Roster, plain};
pub use run::{GateFor, Order, Phase, Progress, Reach, Runner};
pub use store::{Store, batch_name};
pub use tally::tally;
pub use tree::Changed;

/// Seconds since 1970.
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
