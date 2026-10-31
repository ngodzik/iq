//! What iq's work went to, over a day, a week, a month, a year or all of
//! it, in the person's time zone: its tasks summed by category, model,
//! project or agent, spread over the period, the dearest among them, and
//! the period before to compare with.

use jiff::civil::Weekday;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

use crate::costs::TaskSpent;

/// How long a period looked at is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Span {
    /// A day, hour by hour.
    Day,
    /// A week from Monday, day by day.
    #[default]
    Week,
    /// A calendar month, day by day.
    Month,
    /// A year, month by month.
    Year,
    /// Everything the journal keeps, month by month.
    All,
}

impl Span {
    /// Every span, the shortest first.
    pub const ALL: [Span; 5] = [Span::Day, Span::Week, Span::Month, Span::Year, Span::All];

    /// Its name, as iq shows it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Span::Day => "Day",
            Span::Week => "Week",
            Span::Month => "Month",
            Span::Year => "Year",
            Span::All => "All",
        }
    }
}

/// What tasks are summed by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum By {
    /// The person's categories of work.
    #[default]
    Category,
    /// The model the task ran on.
    Model,
    /// The project it worked in.
    Project,
    /// The agent.
    Agent,
}

impl By {
    /// Every way, in the order offered.
    pub const ALL: [By; 4] = [By::Category, By::Model, By::Project, By::Agent];

    /// Its name, as iq shows it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            By::Category => "category",
            By::Model => "model",
            By::Project => "project",
            By::Agent => "agent",
        }
    }
}

/// How the work is looked at: over which span, how many back from the
/// present one, summed by what.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Lens {
    /// The span.
    pub span: Span,
    /// How many spans back: 0 for the present one.
    pub back: u32,
    /// What the tasks are summed by.
    pub by: By,
}

/// What a group of tasks spent.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Group {
    /// The group, as the lens names it.
    pub name: String,
    /// Its tasks.
    pub tasks: usize,
    /// Those that ended on an error.
    pub failed: usize,
    /// What the provider billed them, in dollars.
    pub billed: f64,
    /// The tokens they read and wrote.
    pub tokens: u64,
    /// How long they worked, in seconds.
    pub seconds: u64,
}

impl Group {
    /// What they cost, in dollars.
    #[must_use]
    pub fn cost(&self) -> f64 {
        self.billed
    }

    fn add(&mut self, task: &TaskSpent) {
        self.tasks += 1;
        self.failed += usize::from(!task.ok);
        self.billed += task.billed.0;
        self.tokens += task.tokens;
        self.seconds += task.seconds;
    }
}

/// One part of the period: an hour, a day or a month.
#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
    /// When it starts, in seconds since 1970.
    pub start: u64,
    /// Its name, short.
    pub name: String,
    /// What each group spent in it, by the groups' order.
    pub costs: Vec<f64>,
}

/// The work over a period, as a lens shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Seen {
    /// When the period starts and ends, in seconds since 1970.
    pub from: u64,
    /// When it ends.
    pub to: u64,
    /// Its name: `Fri 24 Oct 2026`, `Week of 20 Oct 2026`, `October 2026`.
    pub title: String,
    /// All its tasks.
    pub total: Group,
    /// The groups, the dearest first.
    pub groups: Vec<Group>,
    /// The period, part by part.
    pub slices: Vec<Slice>,
    /// The dearest tasks.
    pub top: Vec<TaskSpent>,
    /// What the period before was worth, to compare with.
    pub before: Option<f64>,
    /// Whether no task of the period is in a category, as when the person
    /// named none.
    pub unplaced: bool,
}

/// How many of the dearest tasks are shown.
const TOP: usize = 8;

/// What a task with no category is called.
pub const UNPLACED: &str = "not placed yet";

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn weekday(day: Weekday) -> &'static str {
    match day {
        Weekday::Monday => "Mon",
        Weekday::Tuesday => "Tue",
        Weekday::Wednesday => "Wed",
        Weekday::Thursday => "Thu",
        Weekday::Friday => "Fri",
        Weekday::Saturday => "Sat",
        Weekday::Sunday => "Sun",
    }
}

fn month(zoned: &Zoned) -> &'static str {
    MONTHS[(zoned.month() - 1) as usize]
}

fn seconds(zoned: &Zoned) -> u64 {
    u64::try_from(zoned.timestamp().as_second()).unwrap_or(0)
}

/// The work of `tasks` as `lens` shows it, `now` being now, in `zone`.
#[must_use]
pub fn see(tasks: &[TaskSpent], lens: Lens, now: u64, zone: &TimeZone) -> Seen {
    let Some((from, to, title, slices)) = period(tasks, lens.span, lens.back, now, zone) else {
        return Seen::default();
    };
    let within: Vec<&TaskSpent> = tasks.iter().filter(|t| t.at >= from && t.at < to).collect();
    let name = |task: &TaskSpent| -> String {
        let named = match lens.by {
            By::Category => task.kind.clone().unwrap_or_else(|| UNPLACED.to_owned()),
            By::Model => task.model.clone(),
            By::Project => task.project.clone(),
            By::Agent => task.agent.clone(),
        };
        if named.trim().is_empty() {
            "(none)".to_owned()
        } else {
            named
        }
    };
    let mut total = Group {
        name: "all".into(),
        ..Group::default()
    };
    let mut groups: Vec<Group> = Vec::new();
    for task in &within {
        total.add(task);
        let named = name(task);
        match groups.iter_mut().find(|g| g.name == named) {
            Some(group) => group.add(task),
            None => {
                let mut group = Group {
                    name: named,
                    ..Group::default()
                };
                group.add(task);
                groups.push(group);
            }
        }
    }
    groups.sort_by(|a, b| {
        b.cost()
            .total_cmp(&a.cost())
            .then(b.tasks.cmp(&a.tasks))
            .then(a.name.cmp(&b.name))
    });
    let slices: Vec<Slice> = slices
        .into_iter()
        .map(|(start, end, slice_name)| Slice {
            start,
            name: slice_name,
            costs: groups
                .iter()
                .map(|group| {
                    within
                        .iter()
                        .filter(|t| t.at >= start && t.at < end && name(t) == group.name)
                        .map(|t| t.billed.0)
                        .sum()
                })
                .collect(),
        })
        .collect();
    let mut top: Vec<TaskSpent> = within.iter().map(|t| (*t).clone()).collect();
    top.sort_by(|a, b| b.billed.0.total_cmp(&a.billed.0));
    top.truncate(TOP);
    let before = (lens.span != Span::All)
        .then(|| period(tasks, lens.span, lens.back + 1, now, zone))
        .flatten()
        .map(|(from, to, _, _)| {
            tasks
                .iter()
                .filter(|t| t.at >= from && t.at < to)
                .map(|t| t.billed.0)
                .sum()
        });
    Seen {
        from,
        to,
        title,
        unplaced: !within.is_empty() && within.iter().all(|t| t.kind.is_none()),
        total,
        groups,
        slices,
        top,
        before,
    }
}

/// When the period `back` spans before the present one starts and ends,
/// its name, and its parts.
#[allow(clippy::type_complexity)]
fn period(
    tasks: &[TaskSpent],
    span: Span,
    back: u32,
    now: u64,
    zone: &TimeZone,
) -> Option<(u64, u64, String, Vec<(u64, u64, String)>)> {
    let here = Timestamp::from_second(i64::try_from(now).ok()?)
        .ok()?
        .to_zoned(zone.clone());
    let back = i64::from(back);
    let today = here.start_of_day().ok()?;
    let (start, end, title) = match span {
        Span::Day => {
            let start = today.checked_sub(jiff::Span::new().days(back)).ok()?;
            let end = start.checked_add(jiff::Span::new().days(1)).ok()?;
            let title = format!(
                "{} {} {} {}",
                weekday(start.weekday()),
                start.day(),
                month(&start),
                start.year()
            );
            (start, end, title)
        }
        Span::Week => {
            let monday = today
                .checked_sub(
                    jiff::Span::new().days(i64::from(today.weekday().to_monday_zero_offset())),
                )
                .ok()?;
            let start = monday.checked_sub(jiff::Span::new().weeks(back)).ok()?;
            let end = start.checked_add(jiff::Span::new().weeks(1)).ok()?;
            let title = format!("Week of {} {} {}", start.day(), month(&start), start.year());
            (start, end, title)
        }
        Span::Month => {
            let first = today.first_of_month().ok()?.start_of_day().ok()?;
            let start = first.checked_sub(jiff::Span::new().months(back)).ok()?;
            let end = start.checked_add(jiff::Span::new().months(1)).ok()?;
            let title = format!(
                "{} {}",
                MONTH_NAMES[(start.month() - 1) as usize],
                start.year()
            );
            (start, end, title)
        }
        Span::Year => {
            let first = today.first_of_year().ok()?.start_of_day().ok()?;
            let start = first.checked_sub(jiff::Span::new().years(back)).ok()?;
            let end = start.checked_add(jiff::Span::new().years(1)).ok()?;
            let title = start.year().to_string();
            (start, end, title)
        }
        Span::All => {
            let first = tasks.iter().map(|t| t.at).min().unwrap_or(now);
            let first = Timestamp::from_second(i64::try_from(first).ok()?)
                .ok()?
                .to_zoned(zone.clone())
                .first_of_month()
                .ok()?
                .start_of_day()
                .ok()?;
            let end = here
                .first_of_month()
                .ok()?
                .start_of_day()
                .ok()?
                .checked_add(jiff::Span::new().months(1))
                .ok()?;
            (first, end, "Everything".to_owned())
        }
    };
    // Its parts: hours of a day, days of a week or a month, months beyond.
    let step = match span {
        Span::Day => jiff::Span::new().hours(1),
        Span::Week | Span::Month => jiff::Span::new().days(1),
        Span::Year | Span::All => jiff::Span::new().months(1),
    };
    let mut parts = Vec::new();
    let mut at = start.clone();
    while at < end {
        let next = at.checked_add(step).ok()?;
        let name = match span {
            Span::Day => format!("{:02}h", at.hour()),
            Span::Week => format!("{} {}", weekday(at.weekday()), at.day()),
            Span::Month => at.day().to_string(),
            Span::Year => month(&at).to_owned(),
            Span::All => format!("{} {}", month(&at), at.year() % 100),
        };
        parts.push((seconds(&at), seconds(&next), name));
        at = next;
    }
    Some((seconds(&start), seconds(&end), title, parts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iq_core::Usd;

    fn task(at: u64, kind: Option<&str>, model: &str, billed: f64, ok: bool) -> TaskSpent {
        TaskSpent {
            at,
            kind: kind.map(Into::into),
            billed: Usd(billed),
            tokens: 100,
            model: model.into(),
            agent: "lead".into(),
            project: "github.com/org/repo".into(),
            seconds: 60,
            ok,
            prompt: "do it".into(),
        }
    }

    /// Saturday 24 October 2026, 15:00 UTC.
    const NOW: u64 = 1_792_854_000;

    #[test]
    fn a_week_is_summed_by_category_and_spread_day_by_day() {
        let day = 86_400;
        let tasks = [
            task(NOW - 3600, Some("Python"), "a/m", 0.30, true),
            task(NOW - day, Some("Python"), "a/m", 0.20, false),
            task(NOW - 2 * day, Some("Reviews"), "b/m", 0.10, true),
            task(NOW - 2 * day, None, "b/m", 0.05, true),
            // The week before.
            task(NOW - 7 * day, Some("Python"), "a/m", 1.0, true),
        ];
        let seen = see(&tasks, Lens::default(), NOW, &TimeZone::UTC);
        assert_eq!(seen.title, "Week of 19 Oct 2026");
        assert_eq!(seen.total.tasks, 4);
        let names: Vec<&str> = seen.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Python", "Reviews", UNPLACED]);
        assert_eq!((seen.groups[0].tasks, seen.groups[0].failed), (2, 1));
        assert_eq!(seen.slices.len(), 7);
        assert_eq!(seen.slices[5].name, "Sat 24");
        assert!((seen.slices[5].costs[0] - 0.30).abs() < 1e-9);
        assert!((seen.before.unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(seen.top[0].billed, Usd(0.30));
        assert!(!seen.unplaced);
        // A week back, by model.
        let back = see(
            &tasks,
            Lens {
                back: 1,
                by: By::Model,
                ..Lens::default()
            },
            NOW,
            &TimeZone::UTC,
        );
        assert_eq!(back.title, "Week of 12 Oct 2026");
        assert_eq!(back.groups[0].name, "a/m");
    }

    #[test]
    fn each_span_has_its_parts() {
        let tasks = [task(NOW - 400 * 86_400, None, "m", 0.1, true)];
        let parts = |span| {
            see(
                &tasks,
                Lens {
                    span,
                    ..Lens::default()
                },
                NOW,
                &TimeZone::UTC,
            )
        };
        let day = parts(Span::Day);
        assert_eq!(
            (day.title.as_str(), day.slices.len()),
            ("Sat 24 Oct 2026", 24)
        );
        assert_eq!(parts(Span::Month).title, "October 2026");
        assert_eq!(parts(Span::Month).slices.len(), 31);
        assert_eq!(parts(Span::Year).slices.len(), 12);
        let all = parts(Span::All);
        assert_eq!(all.title, "Everything");
        assert_eq!(all.slices.first().unwrap().name, "Sep 25");
        assert_eq!(all.total.tasks, 1);
        assert!(all.unplaced);
        assert_eq!(all.before, None);
    }
}
