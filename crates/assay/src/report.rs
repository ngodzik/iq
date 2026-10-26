//! The results, summed by iq from the records: for each exercise, each
//! candidate's runs of its latest version, how many passed, what the
//! judges gave them, and what a success costs; across the exercises of a
//! kind of task, the same summed; and the cheapest candidate that does as
//! well as the reference.

use iq_core::Usd;

use crate::candidate::Kind;
use crate::exercise::Exercise;
use crate::record::Record;

/// How close to the reference's pass rate a candidate must come to be
/// said to do as well.
const AS_WELL_RATE: f32 = 0.05;

/// How close to the reference's score.
const AS_WELL_SCORE: f32 = 1.0;

/// Without a reference, the least pass rate of a candidate said to do the
/// work.
const FIT_RATE: f32 = 0.8;

/// One candidate's results.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The candidate, as its grade is kept: the model's name for a model,
    /// whoever served it, else the candidate as written.
    pub candidate: String,
    /// Its kind.
    pub kind: Kind,
    /// Who served it, when it is a model.
    pub providers: Vec<String>,
    /// Its runs.
    pub runs: usize,
    /// Those that passed.
    pub passed: usize,
    /// The mean share of tests passed, when the checks said.
    pub tests: Option<f32>,
    /// The judges' mean score.
    pub score: Option<f32>,
    /// What a run was billed, on average.
    pub billed: Usd,
    /// What a success cost: all that was billed over the runs that passed.
    pub per_success: Option<Usd>,
    /// How long a run took, on average, in seconds.
    pub seconds: u64,
}

impl Row {
    /// The share of its runs that passed.
    #[must_use]
    pub fn rate(&self) -> f32 {
        if self.runs == 0 {
            0.0
        } else {
            self.passed as f32 / self.runs as f32
        }
    }
}

/// An exercise's results.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// The exercise.
    pub exercise: String,
    /// Its title.
    pub title: String,
    /// The version the runs are of: its latest that has runs.
    pub version: Option<u32>,
    /// Its kinds of task.
    pub categories: Vec<String>,
    /// One row per candidate, by kind, the best first.
    pub rows: Vec<Row>,
}

/// The rows of `records`, one per candidate, by kind, the best first: a
/// model's runs together whoever served it.
#[must_use]
pub fn rows(records: &[&Record]) -> Vec<Row> {
    rows_by(records, false)
}

/// What runs are summed under: the candidate as its grade is kept, its
/// kind, and its provider when the providers are kept apart.
type Key = (String, Kind, Option<String>);

/// The rows of `records`, one per candidate, and per provider when
/// `providers`: a provider may serve a model weaker than another does.
#[must_use]
pub fn rows_by(records: &[&Record], providers: bool) -> Vec<Row> {
    let mut groups: Vec<(Key, Vec<&Record>)> = Vec::new();
    for record in records {
        let name = match record.kind {
            Kind::Model => record.model.clone(),
            Kind::Team => record.candidate.to_string(),
        };
        let key = (
            name,
            record.kind,
            providers.then(|| record.provider.clone()).flatten(),
        );
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, runs)) => runs.push(record),
            None => groups.push((key, vec![record])),
        }
    }
    let mut rows: Vec<Row> = groups
        .into_iter()
        .map(|((candidate, kind, _), runs)| {
            let n = runs.len();
            let passed = runs.iter().filter(|r| r.passed()).count();
            let shares: Vec<f32> = runs.iter().filter_map(|r| r.share()).collect();
            let scores: Vec<f32> = runs.iter().filter_map(|r| r.score()).collect();
            let billed: f64 = runs.iter().map(|r| r.spent.billed.0).sum();
            let mut providers: Vec<String> =
                runs.iter().filter_map(|r| r.provider.clone()).collect();
            providers.sort();
            providers.dedup();
            Row {
                candidate,
                kind,
                providers,
                runs: n,
                passed,
                tests: mean(&shares),
                score: mean(&scores),
                billed: Usd(billed / n as f64),
                per_success: (passed > 0).then(|| Usd(billed / passed as f64)),
                seconds: runs.iter().map(|r| r.seconds).sum::<u64>() / n as u64,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then(b.rate().total_cmp(&a.rate()))
            .then(b.score.unwrap_or(-1.0).total_cmp(&a.score.unwrap_or(-1.0)))
            .then(a.billed.0.total_cmp(&b.billed.0))
    });
    rows
}

fn mean(values: &[f32]) -> Option<f32> {
    (!values.is_empty()).then(|| values.iter().sum::<f32>() / values.len() as f32)
}

/// `exercise`'s results, from its latest version that has runs.
#[must_use]
pub fn table(exercise: &Exercise, records: &[Record]) -> Table {
    let version = records
        .iter()
        .filter(|r| exercise.versions.iter().any(|v| v.version == r.version))
        .map(|r| r.version)
        .max();
    let kept: Vec<&Record> = records
        .iter()
        .filter(|r| Some(r.version) == version)
        .collect();
    Table {
        exercise: exercise.id.clone(),
        title: exercise.title.clone(),
        version,
        categories: exercise.categories.clone(),
        rows: rows(&kept),
    }
}

/// The results of every exercise of `category`, each at its latest
/// version, summed by candidate.
#[must_use]
pub fn category(category: &str, exercises: &[(Exercise, Vec<Record>)]) -> Vec<Row> {
    let mut kept: Vec<&Record> = Vec::new();
    for (exercise, records) in exercises {
        if !exercise
            .categories
            .iter()
            .any(|c| c.eq_ignore_ascii_case(category))
        {
            continue;
        }
        let version = table(exercise, records).version;
        kept.extend(records.iter().filter(|r| Some(r.version) == version));
    }
    rows(&kept)
}

/// The cheapest of `rows` that does as well as `reference`, a model or a
/// candidate as written: as many of its runs passed, give or take a few,
/// and its judges gave it about as much. Without the reference among them,
/// the cheapest that passes most of its runs.
#[must_use]
pub fn cheapest_fit<'a>(rows: &'a [Row], reference: &str) -> Option<&'a Row> {
    let reference = rows.iter().find(|r| r.candidate == reference);
    rows.iter()
        .filter(|row| row.per_success.is_some())
        .filter(|row| match reference {
            Some(best) => {
                row.rate() + AS_WELL_RATE >= best.rate()
                    && match (row.score, best.score) {
                        (Some(score), Some(best)) => score + AS_WELL_SCORE >= best,
                        _ => true,
                    }
            }
            None => row.rate() >= FIT_RATE,
        })
        .min_by(|a, b| {
            let cost = |row: &Row| row.per_success.map_or(f64::MAX, |c| c.0);
            cost(a).total_cmp(&cost(b))
        })
}

/// `rows` as a table of text, the kinds apart.
#[must_use]
pub fn text(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "No runs yet.\n".into();
    }
    let mut out = String::new();
    for kind in Kind::ALL {
        let of: Vec<&Row> = rows.iter().filter(|r| r.kind == kind).collect();
        if of.is_empty() {
            continue;
        }
        out.push_str(&format!("{}:\n", capital(kind.plural())));
        for row in of {
            let tests = row
                .tests
                .map(|t| format!(", {:.0}% of tests", t * 100.0))
                .unwrap_or_default();
            let score = row
                .score
                .map_or("not judged".to_owned(), |s| format!("score {s:.1}"));
            let cost = if row.billed.0 > 0.0 {
                format!("{} a run", row.billed)
            } else {
                "nothing billed".to_owned()
            };
            let per = row
                .per_success
                .filter(|_| row.billed.0 > 0.0)
                .map(|c| format!(", {c} a success"))
                .unwrap_or_default();
            let providers = if row.providers.is_empty() {
                String::new()
            } else {
                format!(" [{}]", row.providers.join(", "))
            };
            out.push_str(&format!(
                "- {}{providers}: {}/{} passed{tests}, {score}, {cost}{per}, {} s a run\n",
                row.candidate, row.passed, row.runs, row.seconds
            ));
        }
    }
    out
}

fn capital(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::Candidate;
    use crate::record::{Checked, Ending, Spent, Verdict};

    fn record(candidate: &str, model: &str, passed: bool, billed: f64, score: f32) -> Record {
        let candidate: Candidate = candidate.parse().unwrap();
        Record {
            exercise: "x".into(),
            version: 1,
            digest: "d".into(),
            batch: "b".into(),
            label: "A".into(),
            kind: candidate.kind(),
            candidate,
            model: model.into(),
            provider: Some("Requesty".into()),
            started: 0,
            seconds: 60,
            ending: Ending::Answered,
            answer: String::new(),
            tools: 3,
            tool_errors: 0,
            refused: 0,
            spent: Spent {
                billed: Usd(billed),
                ..Spent::default()
            },
            changed: (1, 2, 0),
            checked: Some(Checked {
                line: "pytest".into(),
                passed,
                tally: Some((if passed { 4 } else { 2 }, 4)),
                output: String::new(),
                refused: None,
            }),
            verdicts: vec![Verdict {
                judge: "j".into(),
                model: String::new(),
                score,
                notes: String::new(),
                at: 0,
            }],
        }
    }

    #[test]
    fn the_cheapest_that_does_as_well_as_the_lead_is_found() {
        let records = [
            record(
                "anthropic/claude-sonnet-4-5",
                "claudesonnet45",
                true,
                0.30,
                9.0,
            ),
            record(
                "anthropic/claude-sonnet-4-5",
                "claudesonnet45",
                true,
                0.34,
                8.6,
            ),
            record("z-ai/glm-5", "glm5", true, 0.04, 8.2),
            record("z-ai/glm-5", "glm5", true, 0.06, 8.0),
            record("other/glm-5", "glm5", false, 0.05, 5.0),
            record("tiny/model", "tinymodel", true, 0.01, 6.0),
            record("tiny/model", "tinymodel", false, 0.01, 3.0),
        ];
        let refs: Vec<&Record> = records.iter().collect();
        let rows = rows(&refs);
        assert_eq!(rows[0].candidate, "claudesonnet45");
        let glm = rows.iter().find(|r| r.candidate == "glm5").unwrap();
        // One model, whoever served it.
        assert_eq!((glm.runs, glm.passed), (3, 2));
        assert!((glm.per_success.unwrap().0 - 0.075).abs() < 1e-9);
        // glm passes 2 of 3 against the lead's 2 of 2: not as well.
        assert_eq!(
            cheapest_fit(&rows, "claudesonnet45").unwrap().candidate,
            "claudesonnet45"
        );
        // Apart by provider, the other glm stands alone.
        let apart = rows_by(&refs, true);
        let glms: Vec<&Row> = apart.iter().filter(|r| r.candidate == "glm5").collect();
        assert_eq!(glms.len(), 1, "both providers are Requesty here");
        let mut elsewhere = records.clone();
        elsewhere[4].provider = Some("Other".into());
        let refs: Vec<&Record> = elsewhere.iter().collect();
        let apart = rows_by(&refs, true);
        assert_eq!(apart.iter().filter(|r| r.candidate == "glm5").count(), 2);
        let no_other_glm: Vec<&Record> = records
            .iter()
            .filter(|r| r.candidate.to_string() != "other/glm-5")
            .collect();
        let rows = super::rows(&no_other_glm);
        assert_eq!(
            cheapest_fit(&rows, "claudesonnet45").unwrap().candidate,
            "glm5"
        );
        let shown = text(&rows);
        assert!(shown.contains(
            "Models:\n- claudesonnet45 [Requesty]: 2/2 passed, 100% of tests, score 8.8"
        ));
    }
}
