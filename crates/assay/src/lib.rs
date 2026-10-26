//! The assay: the same frozen exercises given to agents, each in a folder
//! of its own, checked by hidden tests a deterministic command runs, then
//! graded blind by one grader, always the same so that grades compare;
//! summed by iq into what each candidate does and costs, to find, for each
//! kind of task, the cheapest agent that does it as well as the person's
//! lead.
//!
//! The person works on it in a chat like any other, in the assay's
//! folder, whose agent has the assay's tools: it writes exercises, and
//! asks for freezes and runs, which ask the person, with what they should
//! cost, before anything is spent. A freeze runs the check first, which
//! must fail on the start and pass on the reference. The store's files are
//! the reference: plain files, which a copy elsewhere is made from.
//!
//! The assay knows no provider: it is generic over [`iq_core::ChatModel`],
//! and its candidates are the provider's models, alone or in teams.

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
pub mod route;
mod run;
mod store;
mod tally;
mod tree;

pub use assay::{Assay, CHAT, Overview, Plan, Planned, RunPlan, Settings, Summary, Told};
pub use candidate::{Candidate, Kind};
pub use error::AssayError;
pub use estimate::{Guess, guess};
pub use exercise::{Exercise, Frozen, MOST_MINUTES, State, valid_id};
pub use iq_observe::{Categories, Category};
pub use record::{Checked, Ending, Record, Spent, Verdict};
pub use report::{Row, Table, cheapest_fit, rows, table};
pub use roster::{Allowed, Offer, Roster, plain};
pub use route::{Card, Choice, Choices, Route, Routing};
pub use run::{GateFor, Order, Phase, Progress, Runner};
pub use store::{Store, batch_name};
pub use tally::tally;
pub use tree::Changed;

/// Seconds since 1970.
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
