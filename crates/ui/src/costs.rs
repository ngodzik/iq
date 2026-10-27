//! Where the money goes: each call to a model of the provider with what it
//! cost, and iq's tasks by the person's kinds of task.

use std::collections::BTreeMap;

use iq_core::Usd;

/// One call to a model.
#[derive(Debug, Clone, PartialEq)]
pub struct Spent {
    /// When it ended, in seconds since 1970.
    pub at: u64,
    /// The model it called.
    pub model: String,
    /// What it cost, `None` when the model's price is unknown: never
    /// guessed.
    pub cost: Option<Usd>,
    /// The tokens it read and wrote, its cache's included.
    pub tokens: u64,
}

/// What the calls to one model cost over a period.
#[derive(Debug, Clone, PartialEq)]
pub struct Total {
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

/// One task of iq's agents, by the kind of task the person's rules place
/// it in: the kinds are the person's, named in their settings.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskSpent {
    /// When it started, in seconds since 1970.
    pub at: u64,
    /// Its kind, `None` when no rule places it.
    pub kind: Option<String>,
    /// What the provider billed it.
    pub billed: Usd,
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
    /// The tokens they read and wrote.
    pub tokens: u64,
}

/// Every call known, oldest first, and iq's tasks.
#[derive(Debug, Clone, Default)]
pub struct Costs {
    spent: Vec<Spent>,
    tasks: Vec<TaskSpent>,
}

impl Costs {
    /// A call billed to the provider's key.
    pub fn add(&mut self, spent: Spent) {
        self.spent.push(spent);
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
    /// those of no kind last.
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
                        tokens: 0,
                    });
                    totals.len() - 1
                }
            };
            let total = &mut totals[at];
            total.tasks += 1;
            total.billed += task.billed;
            total.tokens += task.tokens;
        }
        totals.sort_by(|a, b| {
            a.kind
                .is_none()
                .cmp(&b.kind.is_none())
                .then(b.billed.0.total_cmp(&a.billed.0))
        });
        totals
    }

    fn since(&self, since: u64) -> impl Iterator<Item = &Spent> {
        self.spent.iter().filter(move |s| s.at >= since)
    }

    /// What was spent since `since`, by model, the dearest first.
    #[must_use]
    pub fn totals(&self, since: u64) -> Vec<Total> {
        let mut by: BTreeMap<String, Total> = BTreeMap::new();
        for spent in self.since(since) {
            let total = by.entry(spent.model.clone()).or_insert_with(|| Total {
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

    /// What was spent since `since`, summed as time goes: when, and how
    /// much by then.
    #[must_use]
    pub fn running(&self, since: u64) -> Vec<(u64, f64)> {
        let mut spent: Vec<&Spent> = self.since(since).collect();
        spent.sort_by_key(|s| s.at);
        let mut line = vec![(since, 0.0)];
        for s in spent {
            let before = line.last().map_or(0.0, |(_, sum)| *sum);
            line.push((s.at, before + s.cost.map_or(0.0, |c| c.0)));
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spent(at: u64, model: &str, cost: Option<f64>) -> Spent {
        Spent {
            at,
            model: model.to_owned(),
            cost: cost.map(Usd),
            tokens: 100,
        }
    }

    #[test]
    fn kinds_of_task_are_summed_over_the_period_the_dearest_first() {
        let mut costs = Costs::default();
        let task = |at, kind: Option<&str>, billed| TaskSpent {
            at,
            kind: kind.map(Into::into),
            billed: Usd(billed),
            tokens: 10,
            model: "m".into(),
            agent: "lead".into(),
            project: "p".into(),
            seconds: 5,
            ok: true,
            prompt: String::new(),
        };
        costs.tasks_read(vec![
            task(1, Some("Rebase"), 5.0),
            task(10, None, 9.0),
            task(11, Some("Python"), 0.7),
            task(12, Some("Rebase"), 0.1),
            task(13, Some("Python"), 0.1),
        ]);
        let kinds = costs.by_kind(5);
        let names: Vec<Option<&str>> = kinds.iter().map(|k| k.kind.as_deref()).collect();
        assert_eq!(names, [Some("Python"), Some("Rebase"), None]);
        assert_eq!(kinds[0].tasks, 2);
        assert!((kinds[0].billed.0 - 0.8).abs() < 1e-9);
    }

    #[test]
    fn totals_go_by_model_and_leave_unknown_prices_out() {
        let mut costs = Costs::default();
        for s in [
            spent(1, "opus", Some(9.0)),
            spent(10, "opus", Some(0.25)),
            spent(20, "opus", Some(0.5)),
            spent(30, "haiku", None),
        ] {
            costs.add(s);
        }
        let totals = costs.totals(5);
        assert_eq!(totals.len(), 2);
        assert_eq!((totals[0].model.as_str(), totals[0].calls), ("opus", 2));
        assert!((totals[0].cost.0 - 0.75).abs() < 1e-9);
        assert_eq!((totals[1].model.as_str(), totals[1].unpriced), ("haiku", 1));
        assert_eq!(
            costs.running(5),
            [(5, 0.0), (10, 0.25), (20, 0.75), (30, 0.75)]
        );
    }
}
