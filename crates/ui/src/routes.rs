//! The routing as the terminal holds it: the person's kinds of task, the
//! routes the assay's records make, and the person's own choices, to start
//! a new chat on the agent its kind of task is routed to.

use iq_assay::{Candidate, Categories, Choices, Routing};

use crate::costs::Payer;

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

/// Where a task is routed.
#[derive(Debug, Clone, PartialEq)]
pub enum Routed {
    /// Nowhere: no kind of task fits it, or none of its kind was tried. The
    /// chat's own agent takes it.
    Nowhere,
    /// To this candidate, for this kind of task, with its evidence.
    To {
        /// The kind of task.
        category: String,
        /// The agent routed.
        candidate: Candidate,
        /// Its evidence, in a few words.
        said: String,
    },
    /// Every agent routed for this kind is at a limit the person set: the
    /// person chooses, rather than a weaker one quietly.
    Closed(String),
}

impl RouteBook {
    /// Where a new task given as `text` goes: the first agent of its kind's
    /// route whose payer the person's limits leave open.
    #[must_use]
    pub fn route(&self, text: &str, closed: &[Payer]) -> Routed {
        let Some(category) = self.categories.of_text(text) else {
            return Routed::Nowhere;
        };
        let ladder = self.routing.route(category, &self.choices);
        if ladder.is_empty() {
            return Routed::Nowhere;
        }
        let open = |candidate: &Candidate| {
            candidate.models.iter().all(|model| {
                let payer = model
                    .delegate()
                    .map_or(Payer::Provider, |(agent, _)| Payer::Subscription(agent));
                !closed.contains(&payer)
            })
        };
        ladder
            .iter()
            .find_map(|choice| {
                let candidate: Candidate = choice.candidate.parse().ok()?;
                open(&candidate).then(|| Routed::To {
                    category: category.to_owned(),
                    candidate,
                    said: choice.said(),
                })
            })
            .unwrap_or_else(|| Routed::Closed(category.to_owned()))
    }
}
