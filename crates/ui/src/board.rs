//! The window's state: the page shown, the crew, the costs and the
//! subscriptions, as what the program reads of them arrives.

use iq_agent::Event;
use iq_core::SubscriptionCall;

use crate::costs::{Costs, Payer, Spent};
use crate::crew::CrewView;
use crate::subscriptions::{Seen, Subscriptions};

/// The pages of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    /// The agents at work, and what they ask.
    #[default]
    Agents,
    /// Where the money goes.
    Costs,
    /// What is left of the subscriptions' windows.
    Subscriptions,
}

impl Page {
    /// Every page, in the order Tab goes through them.
    pub const ALL: [Page; 3] = [Page::Agents, Page::Costs, Page::Subscriptions];

    /// Its name, as its tab shows it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Page::Agents => "Agents",
            Page::Costs => "Costs",
            Page::Subscriptions => "Subscriptions",
        }
    }
}

/// What the program sends the window.
#[derive(Debug, Clone)]
pub enum Update {
    /// The agents' files, as just read.
    Read(Read),
    /// What the crew just did.
    Crew(Event),
    /// What iq kept of earlier sessions: the calls billed to the
    /// provider, and the quotas heard.
    Kept {
        /// The calls billed, oldest first.
        provider: Vec<Spent>,
        /// The quotas.
        quotas: Seen,
    },
    /// How the crew starts: the lead's model, the reviewer's, whether
    /// the reviewer answers for the person, and whether every command has
    /// the internet.
    Started {
        /// The lead's model.
        lead: String,
        /// The reviewer's.
        reviewer: String,
        /// Whether the autopilot is on.
        autopilot: bool,
        /// Whether the internet is open to every command.
        internet: bool,
    },
}

/// How far back the costs are summed, in seconds, as the window offers it.
pub const PERIODS: [u64; 4] = [3600, 6 * 3600, 24 * 3600, 7 * 24 * 3600];

/// What the agents' files hold, as the loop just read them: each call, and
/// what it would cost at list prices.
#[derive(Debug, Clone, Default)]
pub struct Read {
    /// The calls, oldest first.
    pub calls: Vec<SubscriptionCall>,
    /// What each call would cost, in the same order.
    pub spent: Vec<Spent>,
}

/// The window's state.
#[derive(Debug, Clone)]
pub struct Board {
    page: Page,
    crew: CrewView,
    /// How far back the costs are summed, one of [`PERIODS`].
    period: u64,
    costs: Costs,
    subscriptions: Subscriptions,
    /// Whether the agents' files were read once.
    read: bool,
}

impl Default for Board {
    fn default() -> Self {
        Self {
            page: Page::default(),
            crew: CrewView::default(),
            period: PERIODS[2],
            costs: Costs::default(),
            subscriptions: Subscriptions::default(),
            read: false,
        }
    }
}

impl Board {
    /// How far back the costs are summed, in seconds.
    #[must_use]
    pub fn period(&self) -> u64 {
        self.period
    }

    /// Sums the costs over `period`, one of [`PERIODS`].
    pub fn sum_over(&mut self, period: u64) {
        if PERIODS.contains(&period) {
            self.period = period;
        }
    }

    /// The page shown.
    #[must_use]
    pub fn page(&self) -> Page {
        self.page
    }

    /// Shows `page`.
    pub fn show(&mut self, page: Page) {
        self.page = page;
    }

    /// Shows the next page, as Tab does.
    pub fn next_page(&mut self) {
        let at = Page::ALL.iter().position(|p| *p == self.page).unwrap_or(0);
        self.page = Page::ALL[(at + 1) % Page::ALL.len()];
    }

    /// The costs.
    #[must_use]
    pub fn costs(&self) -> &Costs {
        &self.costs
    }

    /// The subscriptions.
    #[must_use]
    pub fn subscriptions(&self) -> &Subscriptions {
        &self.subscriptions
    }

    /// Whether the agents' files were read once.
    #[must_use]
    pub fn was_read(&self) -> bool {
        self.read
    }

    /// The crew.
    #[must_use]
    pub fn crew(&self) -> &CrewView {
        &self.crew
    }

    /// The crew, for what the person does with it.
    pub fn crew_mut(&mut self) -> &mut CrewView {
        &mut self.crew
    }

    /// Takes what the program sends.
    pub fn update(&mut self, update: Update) {
        match update {
            Update::Read(read) => {
                self.subscriptions.calls_read(read.calls);
                self.costs.subscriptions_read(read.spent);
                self.read = true;
            }
            Update::Crew(event) => {
                match &event {
                    // A subscription's call is read from its agent's files.
                    Event::Spent {
                        model,
                        cost,
                        billed: true,
                        ..
                    } => self.costs.add(Spent {
                        at: crate::now(),
                        payer: Payer::Provider,
                        model: model.clone(),
                        cost: *cost,
                    }),
                    Event::Quota(quota) => self.subscriptions.add(quota.clone()),
                    _ => {}
                }
                self.crew.event(event);
            }
            Update::Kept { provider, quotas } => {
                for spent in provider {
                    self.costs.add(spent);
                }
                self.subscriptions.loaded(quotas);
            }
            Update::Started {
                lead,
                reviewer,
                autopilot,
                internet,
            } => self.crew.started(lead, reviewer, autopilot, internet),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_goes_through_the_pages_and_periods_are_those_offered() {
        let mut board = Board::default();
        assert_eq!(board.page(), Page::Agents);
        board.next_page();
        assert_eq!(board.page(), Page::Costs);
        board.next_page();
        assert_eq!(board.page(), Page::Subscriptions);
        board.next_page();
        assert_eq!(board.page(), Page::Agents);
        assert_eq!(board.period(), 24 * 3600);
        board.sum_over(3600);
        board.sum_over(5);
        assert_eq!(board.period(), 3600);
    }

    #[test]
    fn calls_billed_to_the_provider_are_counted_as_they_come() {
        let mut board = Board::default();
        board.update(Update::Crew(Event::Spent {
            agent: "lead".into(),
            model: "openai/gpt-5-mini".into(),
            usage: iq_core::Usage::default(),
            cost: Some(iq_core::Usd(0.25)),
            billed: true,
        }));
        // A subscription's call is read from its agent's files instead.
        board.update(Update::Crew(Event::Spent {
            agent: "coder".into(),
            model: "gpt-5.5".into(),
            usage: iq_core::Usage::default(),
            cost: Some(iq_core::Usd(4.0)),
            billed: false,
        }));
        let totals = board.costs().totals(0);
        assert_eq!(totals.len(), 1);
        assert_eq!(totals[0].payer, Payer::Provider);
        assert_eq!(totals[0].cost, iq_core::Usd(0.25));
    }
}
