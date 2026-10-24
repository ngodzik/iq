//! Where the money goes: each call to a model with what it cost, or what
//! it would cost at the API's list prices when a subscription paid for it.

use std::collections::BTreeMap;

use iq_core::{Agent, Usd};

/// Who paid for a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Payer {
    /// The provider's key: the cost is billed.
    Provider,
    /// An agent's subscription: the cost is what the API would charge, an
    /// estimate rather than a bill.
    Subscription(Agent),
}

impl Payer {
    /// Its name, as the window shows it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Payer::Provider => "Provider",
            Payer::Subscription(Agent::ClaudeCode) => "Claude Code",
            Payer::Subscription(Agent::Codex) => "Codex",
        }
    }
}

/// One call to a model.
#[derive(Debug, Clone, PartialEq)]
pub struct Spent {
    /// When it ended, in seconds since 1970.
    pub at: u64,
    /// Who paid.
    pub payer: Payer,
    /// The model it called.
    pub model: String,
    /// What it cost, `None` when the model's price is unknown: never
    /// guessed.
    pub cost: Option<Usd>,
    /// The tokens it read and wrote, its cache's included.
    pub tokens: u64,
    /// Whether iq made it: else the person did, in a session of Claude
    /// Code or Codex of their own, which their agent's files also keep.
    pub by_iq: bool,
}

/// What one payer's calls to one model cost over a period.
#[derive(Debug, Clone, PartialEq)]
pub struct Total {
    /// Who paid.
    pub payer: Payer,
    /// The model.
    pub model: String,
    /// The calls.
    pub calls: usize,
    /// What those of a known price cost.
    pub cost: Usd,
    /// The calls whose price is unknown, left out of `cost`.
    pub unpriced: usize,
    /// The tokens they read and wrote.
    pub tokens: u64,
}

/// Every call known, oldest first.
#[derive(Debug, Clone, Default)]
pub struct Costs {
    provider: Vec<Spent>,
    subscriptions: Vec<Spent>,
}

impl Costs {
    /// The subscriptions' calls, as just read from the agents' files.
    pub fn subscriptions_read(&mut self, mut spent: Vec<Spent>) {
        spent.sort_by_key(|s| s.at);
        self.subscriptions = spent;
    }

    /// A call iq made with the provider's key.
    pub fn add(&mut self, spent: Spent) {
        self.provider.push(spent);
    }

    fn since(&self, since: u64, everyone: bool) -> impl Iterator<Item = &Spent> {
        self.provider
            .iter()
            .chain(&self.subscriptions)
            .filter(move |s| s.at >= since && (everyone || s.by_iq))
    }

    /// What was spent since `since`, by payer and model, the dearest
    /// first: by iq, or by `everyone`, the person's own sessions too.
    #[must_use]
    pub fn totals(&self, since: u64, everyone: bool) -> Vec<Total> {
        let mut by: BTreeMap<(Payer, String), Total> = BTreeMap::new();
        for spent in self.since(since, everyone) {
            let total = by
                .entry((spent.payer, spent.model.clone()))
                .or_insert_with(|| Total {
                    payer: spent.payer,
                    model: spent.model.clone(),
                    calls: 0,
                    cost: Usd(0.0),
                    unpriced: 0,
                    tokens: 0,
                });
            total.calls += 1;
            total.tokens += spent.tokens;
            match spent.cost {
                Some(cost) => total.cost += cost,
                None => total.unpriced += 1,
            }
        }
        let mut totals: Vec<Total> = by.into_values().collect();
        totals.sort_by(|a, b| b.cost.0.total_cmp(&a.cost.0));
        totals
    }

    /// What each payer spent since `since`, summed as time goes: one line
    /// of points, when and how much by then, per payer; by iq, or by
    /// `everyone`.
    #[must_use]
    pub fn running(&self, since: u64, everyone: bool) -> Vec<(Payer, Vec<(u64, f64)>)> {
        let mut spent: Vec<&Spent> = self.since(since, everyone).collect();
        spent.sort_by_key(|s| s.at);
        let mut lines: BTreeMap<Payer, Vec<(u64, f64)>> = BTreeMap::new();
        for s in spent {
            let line = lines.entry(s.payer).or_insert_with(|| vec![(since, 0.0)]);
            let before = line.last().map_or(0.0, |(_, sum)| *sum);
            line.push((s.at, before + s.cost.map_or(0.0, |c| c.0)));
        }
        lines.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spent(at: u64, payer: Payer, model: &str, cost: Option<f64>) -> Spent {
        Spent {
            at,
            payer,
            model: model.to_owned(),
            cost: cost.map(Usd),
            tokens: 100,
            by_iq: true,
        }
    }

    #[test]
    fn the_persons_own_sessions_count_only_when_asked() {
        let mut costs = Costs::default();
        let claude = Payer::Subscription(Agent::ClaudeCode);
        let mut theirs = spent(10, claude, "opus", Some(40.0));
        theirs.by_iq = false;
        costs.subscriptions_read(vec![theirs, spent(12, claude, "haiku", Some(0.01))]);
        let cost = |everyone| -> f64 { costs.totals(0, everyone).iter().map(|t| t.cost.0).sum() };
        assert!((cost(false) - 0.01).abs() < 1e-9);
        assert!((cost(true) - 40.01).abs() < 1e-9);
        assert_eq!(costs.running(0, false)[0].1.len(), 2);
    }

    #[test]
    fn totals_go_by_payer_and_model_and_leave_unknown_prices_out() {
        let mut costs = Costs::default();
        let claude = Payer::Subscription(Agent::ClaudeCode);
        costs.subscriptions_read(vec![
            spent(20, claude, "opus", Some(0.5)),
            spent(10, claude, "opus", Some(0.25)),
            spent(30, claude, "haiku", None),
            spent(1, claude, "opus", Some(9.0)),
        ]);
        costs.add(spent(15, Payer::Provider, "glm", Some(0.01)));
        let totals = costs.totals(5, true);
        assert_eq!(totals.len(), 3);
        assert_eq!((totals[0].model.as_str(), totals[0].calls), ("opus", 2));
        assert!((totals[0].cost.0 - 0.75).abs() < 1e-9);
        assert_eq!((totals[2].model.as_str(), totals[2].unpriced), ("haiku", 1));
        let running = costs.running(5, true);
        let (_, line) = running.iter().find(|(payer, _)| *payer == claude).unwrap();
        assert_eq!(line, &[(5, 0.0), (10, 0.25), (20, 0.75), (30, 0.75)]);
    }
}
