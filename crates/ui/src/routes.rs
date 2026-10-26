//! The routing as the terminal holds it: the person's kinds of task, the
//! routes the assay's records make, and the person's own choices, to show
//! the person what the lead is told.

use iq_assay::{Categories, Choices, Routing};

/// The routing, and what it is read with.
#[derive(Debug, Clone, Default)]
pub struct RouteBook {
    /// The person's kinds of task, with their rules.
    pub categories: Categories,
    /// The routes, as the assay's records made them.
    pub routing: Routing,
    /// The person's own choices.
    pub choices: Choices,
}
