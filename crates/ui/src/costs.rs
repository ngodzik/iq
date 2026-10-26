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
    /// An agent's own API key, as Claude Code may run on one: billed, but
    /// to the agent's vendor, not to the provider.
    Key(Agent),
}

impl Payer {
    /// Its name, as iq shows it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Payer::Provider => "Provider",
            Payer::Subscription(Agent::ClaudeCode) => "Claude Code",
            Payer::Subscription(Agent::Codex) => "Codex",
            Payer::Key(Agent::ClaudeCode) => "Claude Code's key",
            Payer::Key(Agent::Codex) => "Codex's key",
        }
    }

    /// Who paid a call billed for `model`: the agent's own key when it is
    /// an agent with tools of its own, else the provider.
    #[must_use]
    pub fn billed_for(model: &str) -> Payer {
        iq_core::ModelId::new(model)
            .ok()
            .and_then(|model| model.delegate().map(|(agent, _)| agent))
            .map_or(Payer::Provider, Payer::Key)
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

/// One task of iq's agents, by the kind of task the assay's rules place
/// it in: the kinds are the person's, named in their settings.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskSpent {
    /// When it started, in seconds since 1970.
    pub at: u64,
    /// Its kind, `None` when no rule places it.
    pub kind: Option<String>,
    /// What the provider billed it.
    pub billed: Usd,
    /// What its calls under a subscription would cost at list prices.
    pub listed: Usd,
    /// The tokens it read and wrote.
    pub tokens: u64,
    /// The model it ran on.
    pub model: String,
    /// The agent.
    pub agent: String,
    /// The project it worked in.
    pub project: String,
    /// How long it worked, in seconds.
    pub seconds: u64,
    /// Whether it ended with an answer, rather than an error.
    pub ok: bool,
    /// The start of what it was given.
    pub prompt: String,
}

/// What one kind of task spent over a period.
#[derive(Debug, Clone, PartialEq)]
pub struct KindTotal {
    /// The kind, `None` for the tasks no rule places.
    pub kind: Option<String>,
    /// How many tasks.
    pub tasks: usize,
    /// What the provider billed them.
    pub billed: Usd,
    /// What their calls under a subscription would cost at list prices.
    pub listed: Usd,
    /// The tokens they read and wrote.
    pub tokens: u64,
}

/// Every call known, oldest first, and iq's tasks.
#[derive(Debug, Clone, Default)]
pub struct Costs {
    provider: Vec<Spent>,
    subscriptions: Vec<Spent>,
    tasks: Vec<TaskSpent>,
    /// The agents found billing iq's calls to a key of their own: their
    /// files keep those calls too, counted once, as billed.
    keyed: Vec<Agent>,
}

impl Costs {
    /// The subscriptions' calls, as just read from the agents' files.
    pub fn subscriptions_read(&mut self, mut spent: Vec<Spent>) {
        spent.sort_by_key(|s| s.at);
        self.subscriptions = spent;
    }

    /// A call billed: to the provider's key, or to an agent's own.
    pub fn add(&mut self, spent: Spent) {
        if let Payer::Key(agent) = spent.payer
            && !self.keyed.contains(&agent)
        {
            self.keyed.push(agent);
        }
        self.provider.push(spent);
    }

    /// iq's tasks, as just read from its journal and placed in the
    /// person's categories.
    pub fn tasks_read(&mut self, tasks: Vec<TaskSpent>) {
        self.tasks = tasks;
    }

    /// iq's tasks, oldest first.
    #[must_use]
    pub fn tasks(&self) -> &[TaskSpent] {
        &self.tasks
    }

    /// What each kind of task spent since `since`, the dearest first,
    /// those of no kind last: dollars billed and list prices under a
    /// subscription weigh alike, as both are what the work is worth.
    #[must_use]
    pub fn by_kind(&self, since: u64) -> Vec<KindTotal> {
        let mut totals: Vec<KindTotal> = Vec::new();
        for task in self.tasks.iter().filter(|t| t.at >= since) {
            let at = match totals.iter().position(|t| t.kind == task.kind) {
                Some(at) => at,
                None => {
                    totals.push(KindTotal {
                        kind: task.kind.clone(),
                        tasks: 0,
                        billed: Usd(0.0),
                        listed: Usd(0.0),
                        tokens: 0,
                    });
                    totals.len() - 1
                }
            };
            let total = &mut totals[at];
            total.tasks += 1;
            total.billed += task.billed;
            total.listed += task.listed;
            total.tokens += task.tokens;
        }
        totals.sort_by(|a, b| {
            a.kind
                .is_none()
                .cmp(&b.kind.is_none())
                .then((b.billed.0 + b.listed.0).total_cmp(&(a.billed.0 + a.listed.0)))
        });
        totals
    }

    fn since(&self, since: u64, everyone: bool) -> impl Iterator<Item = &Spent> {
        let keyed = |s: &Spent| match s.payer {
            Payer::Subscription(agent) => s.by_iq && self.keyed.contains(&agent),
            _ => false,
        };
        self.provider
            .iter()
            .chain(self.subscriptions.iter().filter(move |s| !keyed(s)))
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
    fn kinds_of_task_are_summed_over_the_period_the_dearest_first() {
        let mut costs = Costs::default();
        let task = |at, kind: Option<&str>, billed, listed| TaskSpent {
            at,
            kind: kind.map(Into::into),
            billed: Usd(billed),
            listed: Usd(listed),
            tokens: 10,
            model: "m".into(),
            agent: "lead".into(),
            project: "p".into(),
            seconds: 5,
            ok: true,
            prompt: String::new(),
        };
        costs.tasks_read(vec![
            task(1, Some("Rebase"), 5.0, 0.0),
            task(10, None, 9.0, 0.0),
            task(11, Some("Python"), 0.2, 0.5),
            task(12, Some("Rebase"), 0.1, 0.0),
            task(13, Some("Python"), 0.1, 0.0),
        ]);
        let kinds = costs.by_kind(5);
        let names: Vec<Option<&str>> = kinds.iter().map(|k| k.kind.as_deref()).collect();
        assert_eq!(names, [Some("Python"), Some("Rebase"), None]);
        assert_eq!(kinds[0].tasks, 2);
        assert!((kinds[0].listed.0 - 0.5).abs() < 1e-9);
    }

    #[test]
    fn an_agents_own_key_is_its_payer_and_its_calls_count_once() {
        assert_eq!(Payer::billed_for("z-ai/glm-5"), Payer::Provider);
        assert_eq!(
            Payer::billed_for("claude-code/opus"),
            Payer::Key(Agent::ClaudeCode)
        );
        let mut costs = Costs::default();
        let claude = Payer::Subscription(Agent::ClaudeCode);
        costs.subscriptions_read(vec![spent(10, claude, "opus", Some(0.5))]);
        costs.add(spent(10, Payer::Key(Agent::ClaudeCode), "opus", Some(0.5)));
        let totals = costs.totals(0, false);
        assert_eq!(totals.len(), 1);
        assert_eq!(totals[0].payer.name(), "Claude Code's key");
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
