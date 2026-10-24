//! The steward: what the crew spends, held against the limits the person
//! set. Watching costs nothing: what each payer has used, how fast, and
//! when, at that pace, it reaches its limit, are counted from what iq
//! already knows. Only when the pace is too fast does it write, to the
//! agent that spends the most, which answers with its own model.
//!
//! A subscription is held to the share of its windows iq may use, the
//! provider to what it may bill in a month. What the person spends outside
//! iq counts in a window too, as external: iq cannot hold it back, but
//! leaves room for it.

use std::collections::BTreeMap;

use iq_core::{Agent, TokenCount, Usd};

use crate::costs::{Costs, Payer};
use crate::subscriptions::{Subscriptions, WindowView, local_time, percent, short, span};

/// How long the steward waits before writing again to an agent it wrote
/// to, unless things got worse, in seconds.
const QUIET: u64 = 10 * 60;

/// How far back an agent's pace is read, in seconds.
const RECENT: u64 = 10 * 60;

/// How far back a window's pace is read, at the least, in seconds: reports
/// closer together say too little.
const PACE: u64 = 10 * 60;

/// How much ahead of an even pace a payer may be, as a share of its limit,
/// before the steward watches it.
const AHEAD: f64 = 0.2;

/// How soon a limit reached at the current pace makes it an alert, in
/// seconds.
const SOON: u64 = 30 * 60;

/// The limits the person set.
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    /// The most the provider may bill in a calendar month; `None` for no
    /// limit.
    pub provider_month: Option<Usd>,
    /// For each agent, the share of its short window, Claude Code's five
    /// hours, iq may use, in percent.
    pub short: BTreeMap<Agent, f64>,
    /// The same for its week.
    pub week: BTreeMap<Agent, f64>,
}

impl Default for Limits {
    fn default() -> Self {
        let each = |share: f64| Agent::ALL.into_iter().map(|a| (a, share)).collect();
        Self {
            provider_month: None,
            short: each(80.0),
            week: each(70.0),
        }
    }
}

/// How a payer stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Well within its limit.
    Calm,
    /// Ahead of an even pace, or headed for its limit before its period
    /// ends.
    Watch,
    /// Headed for its limit within half an hour.
    Alert,
    /// At its limit: nothing new starts on it.
    Reached,
}

/// One limit, as it stands.
#[derive(Debug, Clone, PartialEq)]
pub struct Gauge {
    /// Who pays.
    pub payer: Payer,
    /// The period: `5h`, `7d`, `month`.
    pub period: String,
    /// What is used: percent of the window, or dollars.
    pub used: f64,
    /// Of that, what the person spent outside iq.
    pub external: f64,
    /// The limit, in the same unit.
    pub limit: f64,
    /// Whether the unit is dollars, else percent.
    pub dollars: bool,
    /// How much of the period has gone, from 0 to 1.
    pub elapsed: f64,
    /// When the limit is reached at the current pace, in seconds since
    /// 1970; `None` when it is not headed there.
    pub full_at: Option<u64>,
    /// When the period ends.
    pub ends_at: Option<u64>,
    /// How it stands.
    pub level: Level,
}

impl Gauge {
    /// Its name: `Claude · 5h`, `Provider · month`.
    #[must_use]
    pub fn name(&self) -> String {
        let who = match self.payer {
            Payer::Provider => "Provider",
            Payer::Subscription(agent) => short(agent),
        };
        format!("{who} · {}", self.period)
    }

    /// An amount in its unit: `42%`, `$3.20`.
    #[must_use]
    pub fn amount(&self, value: f64) -> String {
        if self.dollars {
            Usd(value).to_string()
        } else {
            percent(value)
        }
    }

    /// What it says in a line: what is used, of what limit, and when it is
    /// full at this pace.
    #[must_use]
    pub fn said(&self) -> String {
        let mut said = format!(
            "{} used, of a {} limit",
            self.amount(self.used),
            self.amount(self.limit)
        );
        if self.external > 0.05 {
            said.push_str(&format!(
                " ({} of it spent outside iq)",
                self.amount(self.external)
            ));
        }
        match (self.level, self.full_at) {
            (Level::Reached, _) => said.push_str(": reached"),
            (_, Some(at)) => said.push_str(&format!(", full {} at this pace", local_time(at))),
            _ => {}
        }
        said
    }
}

/// Where the share of a pace reaches `limit`: `used` now, `rate` a second.
fn full_at(used: f64, limit: f64, rate: f64, now: u64) -> Option<u64> {
    if used >= limit || rate <= 0.0 {
        return None;
    }
    let seconds = (limit - used) / rate;
    (seconds.is_finite() && seconds < 1e9).then(|| now + seconds as u64)
}

/// How a payer stands, from what it used, its limit, how much of its
/// period has gone, and when it is full at its pace.
fn level(
    used: f64,
    limit: f64,
    elapsed: f64,
    full: Option<u64>,
    ends: Option<u64>,
    now: u64,
) -> Level {
    if used >= limit {
        return Level::Reached;
    }
    let before_end = |at: u64| ends.is_none_or(|end| at < end);
    match full {
        Some(at) if before_end(at) && at.saturating_sub(now) <= SOON => Level::Alert,
        Some(at) if before_end(at) => Level::Watch,
        _ if used / limit > elapsed + AHEAD => Level::Watch,
        _ => Level::Calm,
    }
}

/// A subscription window held against its limit.
fn window_gauge(
    view: &WindowView,
    limits: &Limits,
    subscriptions: &Subscriptions,
    now: u64,
) -> Option<Gauge> {
    let ends = view.resets_at?;
    let length = view.minutes * 60;
    let start = view.started_at()?;
    let short_window = view.minutes < 2 * 24 * 60;
    let limit = *if short_window {
        limits.short.get(&view.agent)
    } else {
        limits.week.get(&view.agent)
    }?;
    let used = view.estimate.unwrap_or(view.reported);
    let iq = subscriptions.iq_part(view);
    let external = iq.map_or(0.0, |iq| (used - iq).max(0.0));
    // The pace since the report the furthest back within the last while,
    // else since the period started.
    let (from_at, from_used) = view
        .reports
        .iter()
        .rev()
        .find(|(at, _)| now.saturating_sub(*at) >= PACE)
        .copied()
        .unwrap_or((start, 0.0));
    let rate = (used - from_used) / now.saturating_sub(from_at).max(1) as f64;
    let elapsed = (now.saturating_sub(start) as f64 / length.max(1) as f64).clamp(0.0, 1.0);
    let full = full_at(used, limit, rate, now);
    Some(Gauge {
        payer: Payer::Subscription(view.agent),
        period: span(view.minutes),
        used,
        external,
        limit,
        dollars: false,
        elapsed,
        full_at: full,
        ends_at: Some(ends),
        level: level(used, limit, elapsed, full, Some(ends), now),
    })
}

/// The calendar month around `now`, in the person's time zone: when it
/// started and when it ends, in seconds since 1970.
#[must_use]
pub fn month(now: u64) -> Option<(u64, u64)> {
    let stamp = jiff::Timestamp::from_second(i64::try_from(now).ok()?).ok()?;
    let here = stamp.to_zoned(jiff::tz::TimeZone::system());
    let start = here.first_of_month().ok()?.start_of_day().ok()?;
    let end = start.checked_add(jiff::Span::new().months(1)).ok()?;
    Some((
        u64::try_from(start.timestamp().as_second()).ok()?,
        u64::try_from(end.timestamp().as_second()).ok()?,
    ))
}

/// The provider's month held against its limit, when there is one.
fn provider_gauge(costs: &Costs, limits: &Limits, now: u64) -> Option<Gauge> {
    let limit = limits.provider_month?.0;
    let (start, end) = month(now)?;
    let billed = |since: u64| -> f64 {
        costs
            .totals(since, true)
            .iter()
            .filter(|t| t.payer == Payer::Provider)
            .map(|t| t.cost.0)
            .sum()
    };
    let used = billed(start);
    let rate = billed(now.saturating_sub(3600).max(start)) / 3600.0;
    let elapsed = (now.saturating_sub(start) as f64 / end.saturating_sub(start).max(1) as f64)
        .clamp(0.0, 1.0);
    let full = full_at(used, limit, rate, now);
    Some(Gauge {
        payer: Payer::Provider,
        period: "month".to_owned(),
        used,
        external: 0.0,
        limit,
        dollars: true,
        elapsed,
        full_at: full,
        ends_at: Some(end),
        level: level(used, limit, elapsed, full, Some(end), now),
    })
}

/// Every limit set, as it stands at `now`.
#[must_use]
pub fn gauges(
    costs: &Costs,
    subscriptions: &Subscriptions,
    limits: &Limits,
    now: u64,
) -> Vec<Gauge> {
    let mut gauges: Vec<Gauge> = provider_gauge(costs, limits, now).into_iter().collect();
    gauges.extend(
        subscriptions
            .windows(now)
            .iter()
            .filter_map(|view| window_gauge(view, limits, subscriptions, now)),
    );
    gauges
}

/// What an agent spent lately, and on whom.
#[derive(Debug, Clone, PartialEq)]
pub struct Pace {
    /// The agent.
    pub agent: String,
    /// Who pays for it.
    pub payer: Payer,
    /// What it spent in the last ten minutes: tokens under a subscription,
    /// dollars on the provider.
    pub recent: f64,
}

/// A message from the steward to an agent.
#[derive(Debug, Clone, PartialEq)]
pub struct Advice {
    /// The agent.
    pub to: String,
    /// What it says.
    pub text: String,
}

/// The steward's memory: whether it acts, to whom it wrote and how bad
/// things were then.
#[derive(Debug, Clone, Default)]
pub struct Steward {
    on: bool,
    said: BTreeMap<String, (u64, Level)>,
}

impl Steward {
    /// The steward, acting or only watching.
    #[must_use]
    pub fn new(on: bool) -> Self {
        Self {
            on,
            said: BTreeMap::new(),
        }
    }

    /// Whether it writes to the agents, beside watching.
    #[must_use]
    pub fn on(&self) -> bool {
        self.on
    }

    /// Lets it write to the agents, or not.
    pub fn set_on(&mut self, on: bool) {
        self.on = on;
    }

    /// What it writes, at `now`: for each payer past an even pace, to the
    /// agent at work that spent the most on it lately, once a while, or
    /// again when things got worse.
    pub fn advise(&mut self, gauges: &[Gauge], paces: &[Pace], now: u64) -> Vec<Advice> {
        if !self.on {
            return Vec::new();
        }
        let mut advice = Vec::new();
        for gauge in gauges.iter().filter(|g| g.level >= Level::Watch) {
            let Some(pace) = paces
                .iter()
                .filter(|p| p.payer == gauge.payer && p.recent > 0.0)
                .max_by(|a, b| a.recent.total_cmp(&b.recent))
            else {
                continue;
            };
            if advice.iter().any(|a: &Advice| a.to == pace.agent) {
                continue;
            }
            let worse = self
                .said
                .get(&pace.agent)
                .is_none_or(|(at, level)| now.saturating_sub(*at) >= QUIET || gauge.level > *level);
            if !worse {
                continue;
            }
            self.said.insert(pace.agent.clone(), (now, gauge.level));
            advice.push(Advice {
                to: pace.agent.clone(),
                text: letter(gauge, pace),
            });
        }
        advice
    }
}

/// What the steward writes to the agent that spends the most on `gauge`.
fn letter(gauge: &Gauge, pace: &Pace) -> String {
    let took = if gauge.dollars {
        Usd(pace.recent).to_string()
    } else {
        format!("{} tokens", TokenCount(pace.recent as u64))
    };
    let ask = if gauge.level == Level::Reached {
        "No new agent starts on it now. Finish what you do with the least you can, and tell the \
         person what is left."
    } else {
        "Answer me in one line, with send_message to steward: why so much, and how you will go \
         leaner: read less, keep answers short, hand long reading to an agent on a cheaper model. \
         Then go on."
    };
    format!(
        "Steward: {} has {}. You took {took} in the last ten minutes, the most of the crew. {ask}",
        gauge.name(),
        gauge.said()
    )
}

/// The payers whose limits are reached: nothing new starts on them, and
/// the provider bills no more.
#[must_use]
pub fn closed(gauges: &[Gauge]) -> Vec<Payer> {
    let mut closed: Vec<Payer> = gauges
        .iter()
        .filter(|g| g.level == Level::Reached)
        .map(|g| g.payer)
        .collect();
    closed.sort();
    closed.dedup();
    closed
}

/// How far back an agent's spending is kept, for its pace.
pub(crate) const KEPT: u64 = RECENT;

#[cfg(test)]
mod tests {
    use super::*;

    fn gauge(level: Level, payer: Payer) -> Gauge {
        Gauge {
            payer,
            period: "5h".into(),
            used: 60.0,
            external: 20.0,
            limit: 80.0,
            dollars: false,
            elapsed: 0.2,
            full_at: Some(1_000),
            ends_at: Some(5_000),
            level,
        }
    }

    #[test]
    fn a_pace_is_held_against_the_limit_and_the_period() {
        // Half the limit at a fifth of the period: ahead.
        assert_eq!(level(40.0, 80.0, 0.2, None, Some(10_000), 0), Level::Watch);
        assert_eq!(level(10.0, 80.0, 0.5, None, Some(10_000), 0), Level::Calm);
        // Full before the end, and soon.
        assert_eq!(
            level(70.0, 80.0, 0.9, Some(600), Some(10_000), 0),
            Level::Alert
        );
        // Full only after the window starts again: not a worry.
        assert_eq!(
            level(30.0, 80.0, 0.5, Some(20_000), Some(10_000), 0),
            Level::Calm
        );
        assert_eq!(level(80.0, 80.0, 0.1, None, None, 0), Level::Reached);
        assert_eq!(full_at(40.0, 80.0, 0.01, 100), Some(4_100));
        assert_eq!(full_at(40.0, 80.0, 0.0, 100), None);
    }

    #[test]
    fn the_steward_writes_to_who_spends_most_once_in_a_while() {
        let claude = Payer::Subscription(Agent::ClaudeCode);
        let paces = vec![
            Pace {
                agent: "lead".into(),
                payer: claude,
                recent: 12_000.0,
            },
            Pace {
                agent: "tests".into(),
                payer: claude,
                recent: 41_000.0,
            },
            Pace {
                agent: "cheap".into(),
                payer: Payer::Provider,
                recent: 0.4,
            },
        ];
        let mut steward = Steward::new(false);
        assert!(
            steward
                .advise(&[gauge(Level::Watch, claude)], &paces, 0)
                .is_empty()
        );
        steward.set_on(true);
        let advice = steward.advise(&[gauge(Level::Watch, claude)], &paces, 0);
        assert_eq!(advice.len(), 1);
        assert_eq!(advice[0].to, "tests");
        assert!(advice[0].text.contains("41.0k tokens"));
        assert!(advice[0].text.contains("outside iq"));
        // Not again so soon, unless things got worse.
        assert!(
            steward
                .advise(&[gauge(Level::Watch, claude)], &paces, 60)
                .is_empty()
        );
        assert_eq!(
            steward
                .advise(&[gauge(Level::Alert, claude)], &paces, 120)
                .len(),
            1
        );
        assert_eq!(
            steward
                .advise(&[gauge(Level::Watch, claude)], &paces, 121 + QUIET)
                .len(),
            1
        );
        assert_eq!(
            closed(&[
                gauge(Level::Reached, claude),
                gauge(Level::Calm, Payer::Provider)
            ]),
            vec![claude]
        );
    }

    #[test]
    fn the_month_runs_from_its_first_day() {
        let (start, end) = month(1_792_700_000).unwrap();
        assert!(start <= 1_792_700_000 && 1_792_700_000 < end);
        assert!((28 * 86_400..=31 * 86_400 + 3_600).contains(&(end - start)));
    }
}
