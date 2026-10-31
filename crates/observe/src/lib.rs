//! What iq's work goes to. iq keeps a journal of its agents' tasks: what
//! each was given, the files it wrote, the commands it ran, what it cost
//! and how long it took. The person names the categories of their work in
//! their settings, with rules if they wish; as each task ends, the rules
//! place it if they can, and a cheap model places the others in the
//! background, kept to the categories and to how tasks were placed
//! before. All of it stays on the machine.
//!
//! The code knows none of the categories: they are the person's.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod categories;
mod labels;
mod observer;
mod place;
mod task;

pub use categories::{Categories, Category, OTHER};
pub use labels::{Label, Labels, category_of};
pub use observer::{Observer, Settings};
pub use task::{Task, read};
