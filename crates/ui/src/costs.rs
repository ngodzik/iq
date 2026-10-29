//! Where the money goes: each call to a model of the provider with what it
//! cost, and iq's tasks by the person's kinds of task.

use std::collections::BTreeMap;

use iq_core::Usd;
use jiff::Timestamp;
use jiff::tz::TimeZone;

/// How far back the costs are summed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    /// The last so many seconds.
    Last(u64),
    /// Since midnight, in the person's time zone.
    Today,
}

impl Period {
    /// When it starts, in seconds since 1970, `now` being one too and the
    /// person's time zone `zone`.
    #[must_use]
    pub fn since(self, now: u64, zone: &TimeZone) -> u64 {
        match self {
            Period::Last(seconds) => now.saturating_sub(seconds),
            Period::Today => midnight(now, zone).unwrap_or(now.saturating_sub(now % (24 * 3600))),
        }
    }

    /// Its name, as the costs' page offers it: `1h`, `today`, `7d`.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Period::Last(seconds) if seconds >= 2 * 24 * 3600 => {
                format!("{}d", seconds / (24 * 3600))
            }
            Period::Last(seconds) => format!("{}h", seconds / 3600),
            Period::Today => "today".to_owned(),
        }
    }

    /// When it starts, in words: `24h ago`, `midnight`.
    #[must_use]
    pub fn start(self) -> String {
        match self {
            Period::Last(_) => format!("{} ago", self.name()),
            Period::Today => "midnight".to_owned(),
        }
    }

    /// It, in a sentence: `in the last 24h`, `today`.
    #[must_use]
    pub fn said(self) -> String {
        match self {
            Period::Last(_) => format!("in the last {}", self.name()),
            Period::Today => "today".to_owned(),
        }
    }
}

/// The last midnight before `now`, in `zone`.
fn midnight(now: u64, zone: &TimeZone) -> Option<u64> {
    let at = Timestamp::from_second(i64::try_from(now).ok()?).ok()?;
    let start = at.to_zoned(zone.clone()).start_of_day().ok()?;
    u64::try_from(start.timestamp().as_second()).ok()
}

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

    /// What the calls of a known price cost since `since`.
    #[must_use]
    pub fn billed(&self, since: u64) -> Usd {
        let mut sum = Usd(0.0);
        for cost in self.since(since).filter_map(|s| s.cost) {
            sum += cost;
        }
        sum
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

    #[test]
    fn today_starts_at_midnight_where_the_person_is() {
        // 2026-10-29 14:30 in UTC, 15:30 an hour east of it.
        let now = 1_793_284_200;
        assert_eq!(
            Period::Today.since(now, &TimeZone::UTC),
            1_793_232_000,
            "midnight in UTC"
        );
        let east = TimeZone::fixed(jiff::tz::offset(1));
        assert_eq!(Period::Today.since(now, &east), 1_793_228_400);
        assert_eq!(Period::Last(3600).since(now, &east), now - 3600);
        assert_eq!(Period::Last(7 * 24 * 3600).name(), "7d");
        assert_eq!(Period::Last(6 * 3600).said(), "in the last 6h");
        assert_eq!(Period::Today.start(), "midnight");
        let mut costs = Costs::default();
        costs.add(spent(1_793_220_000, "lab/a", Some(1.0)));
        costs.add(spent(1_793_240_000, "lab/a", Some(0.25)));
        costs.add(spent(1_793_250_000, "lab/b", None));
        assert_eq!(costs.billed(Period::Today.since(now, &east)), Usd(0.25));
    }

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
