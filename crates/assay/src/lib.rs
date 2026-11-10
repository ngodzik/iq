//! The assay: frozen exercises given to candidate models, each run in a
//! folder of its own, checked by hidden tests a deterministic command
//! runs, then graded blind by one grader, always the same so that grades
//! compare; summed into what each candidate does and costs. The person
//! uses it to try a new model on their own kind of work.
//!
//! It is iq's own, built into the `iq` command with the `assay` feature,
//! but it keeps a folder and settings of its own (see [`folder`] and
//! [`Settings`]), never iq's. The person writes the exercises; a freeze
//! runs an exercise's check first, which must fail on the start and pass
//! on the reference; a run or a grading says what it should cost and is
//! asked of the person before anything is spent.
//!
//! The assay knows no provider: it is generic over [`iq_core::ChatModel`],
//! and its candidates are the provider's models, alone or in teams.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod assay;
mod candidate;
mod error;
mod estimate;
mod exercise;
pub mod judge;
mod record;
pub mod report;
mod roster;
mod run;
mod settings;
mod store;
mod tally;
mod tree;

pub use assay::{Assay, GradePlan, Overview, Plan, Planned, RunPlan, Summary};
pub use candidate::{Candidate, Kind};
pub use error::AssayError;
pub use estimate::{Guess, guess};
pub use exercise::{Exercise, Frozen, MOST_MINUTES, State, valid_id};
pub use record::{Checked, Ending, Record, Spent, Verdict};
pub use report::{Row, Table, cheapest_fit, rows, table};
pub use roster::{Allowed, Offer, Roster, plain};
pub use run::{Phase, Progress, Runner, Watch};
pub use settings::{FILE, Settings, VARIABLE, folder};
pub use store::{Store, batch_name};
pub use tally::tally;
pub use tree::Changed;

/// Seconds since 1970.
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
