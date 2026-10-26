//! The routing: for each kind of task, the agents to give it, computed by
//! iq from the assay's records alone, never by a model. Quality first: an
//! agent is routed to a kind only if every run of it passed the checks of
//! every frozen exercise of that kind, and its grade comes within a margin
//! of the best; among those, the cheapest leads, the others following in
//! cost, for when it cannot be used. An agent never tried on a kind is
//! never routed to it, and a kind no agent was tried on routes nothing:
//! the chat's own agent does the work.
//!
//! The routing is written as a file in the assay's folder, made again
//! whenever the records change, and read as it is when iq starts: two
//! machines with the same records make the same file. The person's own
//! choices (agents never to route, an agent pinned to a kind) are their
//! settings', applied as the file is read, never written into it.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use iq_agent::{Called, Crew, Extension};
use iq_core::{ChatModel, Delegate, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::candidate::Candidate;
use crate::exercise::{Exercise, State};
use crate::record::Record;

/// The routing's file, in the assay's folder.
pub const FILE: &str = "routing.toml";

/// How far below the best grade a routed agent may be, out of ten,
/// unless the settings say.
pub const MARGIN: f32 = 0.5;

/// What the file says first, for whoever opens it.
const HEADER: &str = "# Made by iq from the assay's records, made again as they change: do not \
edit it. Your own choices go in [route] of ~/.iq/settings.toml.\n\n";

/// An agent the routing gives a kind of task, with its evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    /// The candidate, as it is written to be run: a model, an agent, a
    /// team.
    pub candidate: String,
    /// Its runs on the kind's exercises.
    pub runs: usize,
    /// Those that passed.
    pub passed: usize,
    /// The grader's mean grade, when it graded them all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<f32>,
    /// What a run cost, billed and at list prices under a subscription.
    pub cost: f64,
    /// How long a run took, on average.
    pub seconds: u64,
    /// When it last ran, in seconds since 1970.
    pub last: u64,
}

impl Choice {
    /// Its evidence, in a few words: `6/6 passed, graded 9.1, $0.012 a run`.
    #[must_use]
    pub fn said(&self) -> String {
        let mut said = format!("{}/{} passed", self.passed, self.runs);
        if let Some(grade) = self.grade {
            said.push_str(&format!(", graded {grade:.1}"));
        }
        said.push_str(&format!(", {} a run", iq_core::Usd(self.cost)));
        said
    }
}

/// A kind of task's route: the agents to give it, the first first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    /// The kind, as the person's categories name it.
    pub category: String,
    /// Its frozen exercises the evidence comes from.
    pub exercises: Vec<String>,
    /// The agents routed, the cheapest first.
    #[serde(default, rename = "choice")]
    pub ladder: Vec<Choice>,
}

/// The routing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Routing {
    /// Whose grades it weighs: the model that answered as the grader.
    pub grader: String,
    /// When its newest record ran, in seconds since 1970.
    pub made: u64,
    /// Each kind of task's route.
    #[serde(default, rename = "route")]
    pub routes: Vec<Route>,
}

/// The person's own choices, from their settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Choices {
    /// No routing at all: every chat on the model it was started on.
    pub off: bool,
    /// Agents never routed, each with why.
    pub vetoed: BTreeMap<String, String>,
    /// An agent pinned to a kind of task, whatever the records say.
    pub pinned: BTreeMap<String, String>,
}

impl Routing {
    /// The routing of `exercises`, each with its records graded by
    /// `grader` alone, an agent routed within `margin` of the best grade.
    #[must_use]
    pub fn compute(exercises: &[(Exercise, Vec<Record>)], grader: &str, margin: f32) -> Routing {
        let mut kinds: Vec<String> = Vec::new();
        for (exercise, _) in exercises {
            if !matches!(exercise.state, State::Frozen(_)) {
                continue;
            }
            for category in &exercise.categories {
                if !kinds.iter().any(|k| k.eq_ignore_ascii_case(category)) {
                    kinds.push(category.clone());
                }
            }
        }
        kinds.sort();
        let mut made = 0;
        let routes = kinds
            .into_iter()
            .map(|category| {
                let of_kind: Vec<&(Exercise, Vec<Record>)> = exercises
                    .iter()
                    .filter(|(exercise, _)| {
                        matches!(exercise.state, State::Frozen(_))
                            && exercise
                                .categories
                                .iter()
                                .any(|c| c.eq_ignore_ascii_case(&category))
                    })
                    .collect();
                let mut by: BTreeMap<String, Vec<&Record>> = BTreeMap::new();
                for (exercise, records) in &of_kind {
                    let version = exercise.version();
                    for record in records.iter().filter(|r| Some(r.version) == version) {
                        made = made.max(record.started);
                        by.entry(record.candidate.to_string())
                            .or_default()
                            .push(record);
                    }
                }
                let ladder = ladder(&of_kind, &by, margin);
                Route {
                    category,
                    exercises: of_kind.iter().map(|(e, _)| e.id.clone()).collect(),
                    ladder,
                }
            })
            .collect();
        Routing {
            grader: grader.to_owned(),
            made,
            routes,
        }
    }

    /// The routing as its file says it, when there is one.
    #[must_use]
    pub fn load(folder: &Path) -> Option<Routing> {
        let text = std::fs::read_to_string(folder.join(FILE)).ok()?;
        toml::from_str(&text).ok()
    }

    /// Writes the routing's file in `folder`, unless it says this already:
    /// a file made again the same is not a change to sync.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn save(&self, folder: &Path) -> std::io::Result<()> {
        let text = format!(
            "{HEADER}{}",
            toml::to_string_pretty(self).map_err(std::io::Error::other)?
        );
        let file = folder.join(FILE);
        if std::fs::read_to_string(&file).ok().as_deref() == Some(text.as_str()) {
            return Ok(());
        }
        crate::store::write_whole(&file, &text)
    }

    /// The route of `category`, as the person's `choices` leave it: their
    /// pinned agent alone, else the records' without those vetoed.
    #[must_use]
    pub fn route(&self, category: &str, choices: &Choices) -> Vec<Choice> {
        if choices.off {
            return Vec::new();
        }
        if let Some(pinned) = choices
            .pinned
            .iter()
            .find(|(kind, _)| kind.eq_ignore_ascii_case(category))
            .map(|(_, agent)| agent)
        {
            let evidence = self
                .routes
                .iter()
                .find(|r| r.category.eq_ignore_ascii_case(category))
                .and_then(|r| r.ladder.iter().find(|c| &c.candidate == pinned));
            return vec![evidence.cloned().unwrap_or(Choice {
                candidate: pinned.clone(),
                runs: 0,
                passed: 0,
                grade: None,
                cost: 0.0,
                seconds: 0,
                last: 0,
            })];
        }
        self.routes
            .iter()
            .find(|r| r.category.eq_ignore_ascii_case(category))
            .map(|r| {
                r.ladder
                    .iter()
                    .filter(|c| !choices.vetoed.contains_key(&c.candidate))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// What an agent starting others is told of the routing, in a few
    /// lines: each kind's agents, those never to use, and `pace`, how the
    /// person's limits stand.
    #[must_use]
    pub fn card(&self, choices: &Choices, pace: Option<&str>) -> Option<String> {
        if choices.off {
            return None;
        }
        let mut lines = Vec::new();
        for route in &self.routes {
            let ladder = self.route(&route.category, choices);
            let Some(first) = ladder.first() else {
                continue;
            };
            let then: Vec<&str> = ladder[1..]
                .iter()
                .take(2)
                .map(|c| c.candidate.as_str())
                .collect();
            let mut line = format!(
                "- {}: {} ({})",
                route.category,
                first.candidate,
                first.said()
            );
            if !then.is_empty() {
                line.push_str(&format!("; then {}", then.join(", ")));
            }
            lines.push(line);
        }
        if lines.is_empty() && choices.vetoed.is_empty() {
            return None;
        }
        let mut card = format!(
            "The agents to start for each kind of task, from the assay's results (graded by {}): \
             when you start an agent for a task of one of these kinds, start it on the first, \
             else on one that follows. iq refuses another for these kinds, unless the person \
             named it.\n{}",
            self.grader,
            lines.join("\n")
        );
        for (agent, why) in &choices.vetoed {
            card.push_str(&format!("\nNever start an agent on {agent}: {why}."));
        }
        if let Some(pace) = pace {
            card.push_str(&format!("\n{pace}"));
        }
        Some(card)
    }

    /// Whether an agent may start another on `model` for a task of
    /// `category`: a model the person named, or one the route gives; on a
    /// kind no route covers, any but those vetoed. Else why not, with
    /// those it may.
    ///
    /// # Errors
    ///
    /// Why the model may not take the task, and which may.
    pub fn allows(
        &self,
        category: Option<&str>,
        model: &str,
        named: &[String],
        choices: &Choices,
    ) -> Result<(), String> {
        if choices.off || named.iter().any(|n| n == model) {
            return Ok(());
        }
        let routed: Vec<Choice> = category.map(|c| self.route(c, choices)).unwrap_or_default();
        if let Some(why) = choices.vetoed.get(model) {
            let instead = routed
                .first()
                .map_or(String::new(), |c| format!("; start it on {}", c.candidate));
            return Err(format!(
                "{model} is never to be used, as the person set it: {why}{instead}"
            ));
        }
        let (Some(category), false) = (category, routed.is_empty()) else {
            return Ok(());
        };
        let runs_it = |choice: &Choice| {
            choice.candidate == model
                || choice
                    .candidate
                    .parse::<Candidate>()
                    .is_ok_and(|c| c.models.iter().any(|m| m.as_str() == model))
        };
        if routed.iter().any(runs_it) {
            return Ok(());
        }
        Err(format!(
            "for a {category} task, the assay's results route {}: start it on one of them, unless \
             the person names {model}",
            routed
                .iter()
                .map(|c| format!("{} ({})", c.candidate, c.said()))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// The routing as agents of a crew are told it, each turn as it stands:
/// a few lines among their instructions, no tool. What they start is held
/// to it by the crew's guard, not by what they are told.
pub struct Card {
    /// The routing, as the records last made it.
    pub routing: Arc<Mutex<Routing>>,
    /// The person's own choices.
    pub choices: Choices,
    /// How the person's limits stand, in a line, when they set some.
    pub pace: Arc<dyn Fn() -> Option<String> + Send + Sync>,
}

impl<M, D> Extension<M, D> for Card
where
    M: ChatModel + Clone + 'static,
    D: Delegate + Clone + 'static,
{
    fn tools(&self, _: &str) -> Vec<ToolSpec> {
        Vec::new()
    }

    fn instructions(&self, _: &str) -> Option<String> {
        let routing = self.routing.lock().ok()?.clone();
        routing.card(&self.choices, (self.pace)().as_deref())
    }

    fn call(&self, _: Crew<M, D>, _: String, name: String, _: Value) -> Called {
        Box::pin(async move { Err(format!("the routing has no tool named {name:?}")) })
    }
}

/// The agents routed to a kind, from the runs of its exercises `of_kind`
/// by each candidate in `by`.
fn ladder(
    of_kind: &[&(Exercise, Vec<Record>)],
    by: &BTreeMap<String, Vec<&Record>>,
    margin: f32,
) -> Vec<Choice> {
    let mut passing: Vec<Choice> = Vec::new();
    for (candidate, runs) in by {
        // Every exercise of the kind tried, and every run of it passed.
        let tried_all = of_kind
            .iter()
            .all(|(exercise, _)| runs.iter().any(|r| r.exercise == exercise.id));
        if !tried_all || !runs.iter().all(|r| r.passed()) {
            continue;
        }
        let scores: Vec<f32> = runs.iter().filter_map(|r| r.score()).collect();
        let grade =
            (scores.len() == runs.len()).then(|| scores.iter().sum::<f32>() / scores.len() as f32);
        let n = runs.len();
        let cost: f64 = runs
            .iter()
            .map(|r| r.spent.billed.0 + r.spent.listed.0)
            .sum::<f64>()
            / n as f64;
        passing.push(Choice {
            candidate: candidate.clone(),
            runs: n,
            passed: n,
            grade,
            cost,
            seconds: runs.iter().map(|r| r.seconds).sum::<u64>() / n as u64,
            last: runs.iter().map(|r| r.started).max().unwrap_or(0),
        });
    }
    // When some were graded, those not are no evidence of quality.
    if passing.iter().any(|c| c.grade.is_some()) {
        passing.retain(|c| c.grade.is_some());
    }
    let best = passing
        .iter()
        .filter_map(|c| c.grade)
        .fold(f32::MIN, f32::max);
    passing.retain(|c| c.grade.is_none_or(|g| g >= best - margin));
    passing.sort_by(|a, b| {
        a.cost
            .total_cmp(&b.cost)
            .then(b.grade.unwrap_or(0.0).total_cmp(&a.grade.unwrap_or(0.0)))
    });
    passing
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exercise::Frozen;
    use crate::record::{Checked, Ending, Spent, Verdict};

    fn exercise(id: &str, category: &str) -> Exercise {
        Exercise {
            id: id.into(),
            title: id.into(),
            categories: vec![category.into()],
            check: Some("true".into()),
            minutes: 5,
            dollars: None,
            source: None,
            versions: vec![Frozen {
                version: 1,
                digest: "d".into(),
                at: 0,
            }],
            state: State::Frozen(1),
            digest: "d".into(),
            folder: std::path::PathBuf::new(),
        }
    }

    fn run(exercise: &str, candidate: &str, passed: bool, grade: f32, cost: f64) -> Record {
        Record {
            exercise: exercise.into(),
            version: 1,
            digest: "d".into(),
            batch: "b".into(),
            label: "A".into(),
            candidate: candidate.parse().unwrap(),
            kind: crate::Kind::Model,
            model: candidate.into(),
            provider: None,
            harness: "iq".into(),
            started: 100,
            seconds: 60,
            ending: Ending::Answered,
            answer: String::new(),
            tools: 3,
            tool_errors: 0,
            refused: 0,
            spent: Spent {
                billed: iq_core::Usd(cost),
                ..Spent::default()
            },
            changed: (1, 1, 0),
            checked: Some(Checked {
                line: "true".into(),
                passed,
                tally: None,
                output: String::new(),
                refused: None,
            }),
            verdicts: vec![Verdict {
                judge: "g".into(),
                model: String::new(),
                score: grade,
                notes: String::new(),
                at: 0,
            }],
        }
    }

    fn routing() -> Routing {
        let exercises = vec![
            (
                exercise("bug", "python"),
                vec![
                    run("bug", "a/cheap", true, 8.8, 0.01),
                    run("bug", "a/dear", true, 9.2, 0.40),
                    run("bug", "a/weak", true, 6.0, 0.001),
                    run("bug", "a/flaky", false, 9.9, 0.002),
                ],
            ),
            (
                exercise("algo", "python"),
                vec![
                    run("algo", "a/cheap", true, 9.0, 0.02),
                    run("algo", "a/dear", true, 9.4, 0.50),
                    run("algo", "a/weak", true, 6.0, 0.001),
                ],
            ),
            (
                exercise("design", "design"),
                vec![run("design", "a/dear", true, 9.0, 0.6)],
            ),
        ];
        Routing::compute(&exercises, "g", MARGIN)
    }

    #[test]
    fn the_cheapest_that_passes_all_and_grades_near_the_best_leads() {
        let routing = routing();
        let python = routing.route("python", &Choices::default());
        let names: Vec<&str> = python.iter().map(|c| c.candidate.as_str()).collect();
        // The weak one grades too low, the flaky one failed a run and never
        // tried the other exercise.
        assert_eq!(names, ["a/cheap", "a/dear"]);
        assert!(python[0].said().starts_with("2/2 passed, graded 8.9"));
        assert_eq!(routing.route("cooking", &Choices::default()), Vec::new());
        // The person's choices: a veto, a pin, or none at all.
        let mut choices = Choices::default();
        choices
            .vetoed
            .insert("a/cheap".into(), "it holds requests".into());
        assert_eq!(routing.route("python", &choices)[0].candidate, "a/dear");
        choices.pinned.insert("python".into(), "a/weak".into());
        assert_eq!(routing.route("python", &choices)[0].candidate, "a/weak");
        let off = Choices {
            off: true,
            ..Choices::default()
        };
        assert!(routing.route("python", &off).is_empty());
    }

    #[test]
    fn agents_are_started_as_routed_unless_the_person_named_another() {
        let routing = routing();
        let none = Choices::default();
        assert!(
            routing
                .allows(Some("python"), "a/cheap", &[], &none)
                .is_ok()
        );
        let refused = routing
            .allows(Some("python"), "a/weak", &[], &none)
            .unwrap_err();
        assert!(refused.contains("route a/cheap"), "{refused}");
        assert!(
            routing
                .allows(Some("python"), "a/weak", &["a/weak".into()], &none)
                .is_ok()
        );
        // A kind with no route, or none at all, is the agents' to choose.
        assert!(
            routing
                .allows(Some("cooking"), "a/weak", &[], &none)
                .is_ok()
        );
        assert!(routing.allows(None, "a/weak", &[], &none).is_ok());
        let mut vetoed = Choices::default();
        vetoed
            .vetoed
            .insert("a/weak".into(), "it never answers".into());
        assert!(routing.allows(None, "a/weak", &[], &vetoed).is_err());
        let card = routing
            .card(&vetoed, Some("The provider's month: 34% of its limit."))
            .unwrap();
        assert!(card.contains("- python: a/cheap (2/2 passed"), "{card}");
        assert!(card.contains("Never start an agent on a/weak"), "{card}");
    }

    #[test]
    fn the_file_is_written_once_for_the_same_routing() {
        let dir = tempfile::tempdir().unwrap();
        let routing = routing();
        routing.save(dir.path()).unwrap();
        let first = std::fs::metadata(dir.path().join(FILE))
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        routing.save(dir.path()).unwrap();
        let second = std::fs::metadata(dir.path().join(FILE))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(Routing::load(dir.path()), Some(routing));
    }
}
