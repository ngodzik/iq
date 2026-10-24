//! The judges: models that read the candidates' work blind, each a letter
//! to them, after the hidden checks said what they could, and grade it by
//! the exercise's rubric. Their answer is read as JSON and checked before
//! it is kept: a grade for every letter, within the scale.

use std::path::Path;

use iq_agent::{Event, Report};
use iq_core::{
    ChatModel, ChatRequest, Delegate, DelegateEvent, DelegateRequest, Effort, Message, ModelId,
};
use serde::Deserialize;

use crate::exercise::Exercise;
use crate::record::Record;

/// How much of a candidate's changes a judge reads, in bytes: past it,
/// the diff is cut and the judge told.
const PATCH_MOST: usize = 60_000;

/// How much of what a candidate said last.
const ANSWER_MOST: usize = 4_000;

/// How much of the checks' output.
const CHECKS_MOST: usize = 2_500;

/// What a judge is, standing.
pub(crate) const INSTRUCTIONS: &str = "You judge the work of coding agents, each given the same \
task in the same starting folder. You do not know who did which: each is a letter. Judge only \
what is in front of you: their changes, what they said, and how the hidden checks went. iq ran \
the checks once each candidate was done, so a candidate that fails them did not do the task, \
whatever it says. Grade from 0 to 10 what the work is worth to the person who asked: 10 is what \
you would merge as it is, 7 works and needs small fixes, 4 works in part, 0 is nothing usable. \
Keep to the rubric when there is one. Be fair to short work that does the task: length is no \
merit. Answer with the JSON asked for, nothing after it.";

/// One letter's grade, as a judge gave it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Grade {
    /// The candidate's letter.
    pub label: String,
    /// From 0 to 10.
    pub score: f32,
    /// Why, in a sentence or two.
    #[serde(default)]
    pub notes: String,
}

#[derive(Deserialize)]
struct Answer {
    grades: Vec<Grade>,
}

/// The letters of `count` candidates, in an order of their own: the
/// candidates' order in the request is not their letters', so that the
/// judge who asked for them cannot tell who is who. `seed` makes the order
/// the same each time for the same batch.
#[must_use]
pub fn labels(count: usize, seed: &str) -> Vec<String> {
    let mut letters: Vec<String> = (0..count).map(letter).collect();
    let mut state = seed.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    for at in (1..letters.len()).rev() {
        // A linear congruential step: enough to shuffle a few letters.
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let other = (state >> 33) as usize % (at + 1);
        letters.swap(at, other);
    }
    letters
}

/// `A` to `Z`, then `AA`, `AB`…
fn letter(at: usize) -> String {
    let alphabet = |n: usize| char::from(b'A' + (n % 26) as u8);
    if at < 26 {
        alphabet(at).to_string()
    } else {
        format!("{}{}", alphabet(at / 26 - 1), alphabet(at))
    }
}

/// What the judges read of one run.
pub struct Work<'a> {
    /// Its record.
    pub record: &'a Record,
    /// What it changed, as a unified diff.
    pub patch: String,
}

/// What a judge is asked: the task, the rubric, then each candidate's
/// work by its letter, in the letters' order.
#[must_use]
pub fn prompt(exercise: &Exercise, task: &str, works: &[Work<'_>]) -> String {
    let mut out = format!(
        "# The exercise: {}\n\n## The task, as each was given it\n\n",
        exercise.title
    );
    out.push_str(task.trim());
    out.push_str("\n\n## What to judge beyond the checks\n\n");
    let rubric = exercise.rubric();
    if rubric.trim().is_empty() {
        out.push_str(
            "No rubric: judge whether it does the task well, as a careful reviewer would.",
        );
    } else {
        out.push_str(rubric.trim());
    }
    match &exercise.check {
        Some(line) => out.push_str(&format!(
            "\n\nThe hidden checks: `{line}`, run by iq in each candidate's folder once it was done.\n"
        )),
        None => out.push_str("\n\nThis exercise has no hidden checks.\n"),
    }
    let mut ordered: Vec<&Work<'_>> = works.iter().collect();
    ordered.sort_by(|a, b| {
        (a.record.label.len(), &a.record.label).cmp(&(b.record.label.len(), &b.record.label))
    });
    for work in &ordered {
        let record = work.record;
        out.push_str(&format!("\n## Candidate {}\n\n", record.label));
        out.push_str(&format!(
            "It worked {} and ended: {}. It called {} tools, {} of which failed.\n",
            minutes(record.seconds),
            record.ending.said(),
            record.tools,
            record.tool_errors
        ));
        if let Some(checked) = &record.checked {
            let tally = checked
                .tally
                .map(|(passed, total)| format!(", {passed} of {total} tests"))
                .unwrap_or_default();
            match &checked.refused {
                Some(why) => out.push_str(&format!("The checks could not run: {why}.\n")),
                None => out.push_str(&format!(
                    "The checks {}{tally}. Their output ends:\n\n```\n{}\n```\n",
                    if checked.passed { "passed" } else { "failed" },
                    crate::run::tail(checked.output.trim(), CHECKS_MOST)
                )),
            }
        }
        if !record.answer.trim().is_empty() {
            out.push_str(&format!(
                "\nWhat it said last:\n\n{}\n",
                cut(record.answer.trim(), ANSWER_MOST)
            ));
        }
        let patch = if work.patch.trim().is_empty() {
            "(it changed nothing)".to_owned()
        } else {
            cut(&work.patch, PATCH_MOST)
        };
        out.push_str(&format!(
            "\nIts changes:\n\n```diff\n{}\n```\n",
            patch.trim_end()
        ));
    }
    let letters: Vec<&str> = ordered.iter().map(|w| w.record.label.as_str()).collect();
    out.push_str(&format!(
        "\n## Your answer\n\nOne JSON object, nothing after it, with a grade for each of {}:\n\n\
         {{\"grades\": [{{\"label\": \"{}\", \"score\": 7, \"notes\": \"a sentence or two: what \
         works, what does not\"}}]}}\n",
        letters.join(", "),
        letters.first().copied().unwrap_or("A")
    ));
    out
}

fn minutes(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds} s")
    } else {
        format!("{} min {} s", seconds / 60, seconds % 60)
    }
}

/// The first `most` bytes of `text`, cut at a character and said to be.
fn cut(text: &str, most: usize) -> String {
    if text.len() <= most {
        return text.to_owned();
    }
    let mut at = most;
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    format!("{}\n[… cut: {} more bytes]", &text[..at], text.len() - at)
}

/// The grades in a judge's answer, one for each of `labels`.
///
/// # Errors
///
/// What is wrong with the answer, to tell the judge.
///
/// # Examples
///
/// ```
/// use iq_assay::judge::read;
///
/// let answer = "Here.\n{\"grades\": [{\"label\": \"A\", \"score\": 8, \"notes\": \"good\"}]}";
/// assert_eq!(read(answer, &["A".to_owned()]).unwrap()[0].score, 8.0);
/// assert!(read("{\"grades\": []}", &["A".to_owned()]).is_err());
/// ```
pub fn read(answer: &str, labels: &[String]) -> Result<Vec<Grade>, String> {
    let json = last_object(answer).ok_or("no JSON object in the answer")?;
    let parsed: Answer =
        serde_json::from_str(json).map_err(|e| format!("the JSON is not as asked: {e}"))?;
    let mut grades = Vec::new();
    for label in labels {
        let grade = parsed
            .grades
            .iter()
            .find(|g| g.label.trim() == label)
            .ok_or_else(|| format!("no grade for {label}"))?;
        if !(0.0..=10.0).contains(&grade.score) || grade.score.is_nan() {
            return Err(format!(
                "{label}'s score {} is not from 0 to 10",
                grade.score
            ));
        }
        grades.push(Grade {
            label: label.clone(),
            score: grade.score,
            notes: grade.notes.trim().to_owned(),
        });
    }
    Ok(grades)
}

/// The last balanced `{…}` of `text`, strings read as JSON writes them.
fn last_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut end = text.rfind('}')?;
    loop {
        // Walk back to the `{` that opens the object closed at `end`.
        let mut depth = 0i32;
        let mut start = None;
        let mut at = end + 1;
        let mut in_string = false;
        while at > 0 {
            at -= 1;
            let byte = bytes[at];
            if in_string {
                if byte == b'"' && !escaped(bytes, at) {
                    in_string = false;
                }
                continue;
            }
            match byte {
                b'"' => in_string = true,
                b'}' => depth += 1,
                b'{' => {
                    depth -= 1;
                    if depth == 0 {
                        start = Some(at);
                        break;
                    }
                }
                _ => {}
            }
        }
        if let Some(start) = start {
            let candidate = &text[start..=end];
            if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                return Some(candidate);
            }
        }
        end = text[..end].rfind('}')?;
    }
}

/// Whether the byte at `at` is escaped by the backslashes before it.
fn escaped(bytes: &[u8], at: usize) -> bool {
    let slashes = bytes[..at]
        .iter()
        .rev()
        .take_while(|b| **b == b'\\')
        .count();
    slashes % 2 == 1
}

/// Asks `judge` for its answer to `prompt`: an agent with tools of its
/// own reads only, in `directory`, keeping nothing of it; a model of the
/// provider answers without tools. What it costs goes to `costs`.
///
/// # Errors
///
/// The judge could not answer.
pub async fn ask<M, D>(
    provider: &M,
    agents: &D,
    judge: &ModelId,
    prompt: String,
    directory: &Path,
    costs: &Report,
) -> Result<String, String>
where
    M: ChatModel,
    D: Delegate,
{
    let shown = judge.to_string();
    match judge.delegate() {
        Some((agent, model)) => {
            let request = DelegateRequest {
                agent,
                read_only: true,
                effort: Some(Effort::High),
                model: model.to_owned(),
                prompt,
                instructions: INSTRUCTIONS.to_owned(),
                resume: None,
                fork: false,
                ephemeral: true,
                directory: directory.to_path_buf(),
                reach: None,
                network: false,
            };
            let costs = costs.clone();
            let mut on_event = move |event: DelegateEvent| {
                if let DelegateEvent::Usage {
                    usage,
                    cost,
                    billed,
                    ..
                } = event
                {
                    costs(Event::Spent {
                        agent: "assay judge".into(),
                        model: shown.clone(),
                        usage,
                        cost,
                        billed,
                    });
                }
            };
            agents
                .run(&request, &mut on_event)
                .await
                .map(|reply| reply.text)
                .map_err(|e| crate::exercise::error_text(&e))
        }
        None => {
            let request = ChatRequest {
                model: judge.clone(),
                messages: vec![Message::system(INSTRUCTIONS), Message::user(prompt)],
                tools: Vec::new(),
                effort: Some(Effort::High),
            };
            let response = provider
                .complete(&request)
                .await
                .map_err(|e| crate::exercise::error_text(&e))?;
            let cost = match response.cost {
                Some(cost) => Some(cost),
                None => provider
                    .pricing(judge)
                    .await
                    .map(|pricing| pricing.cost(&response.usage)),
            };
            costs(Event::Spent {
                agent: "assay judge".into(),
                model: shown,
                usage: response.usage,
                cost,
                billed: true,
            });
            Ok(response.content.unwrap_or_default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_are_shuffled_the_same_way_for_a_batch() {
        let a = labels(5, "20261024-191500");
        assert_eq!(a, labels(5, "20261024-191500"));
        let mut sorted = a.clone();
        sorted.sort();
        assert_eq!(sorted, ["A", "B", "C", "D", "E"]);
        assert_eq!(letter(26), "AA");
        assert_eq!(letter(27), "AB");
        // Some seed moves them: the order is not the request's.
        assert!((0..20).any(|n| labels(4, &n.to_string()) != ["A", "B", "C", "D"]));
    }

    #[test]
    fn an_answer_is_read_from_its_last_object_and_checked() {
        let labels = vec!["A".to_owned(), "B".to_owned()];
        let answer = r#"Thinking {not json}. Here:
{"grades": [{"label": "B", "score": 4, "notes": "half \"works\" {"},
            {"label": "A", "score": 9.5, "notes": "clean"}]}"#;
        let grades = read(answer, &labels).unwrap();
        assert_eq!((grades[0].label.as_str(), grades[0].score), ("A", 9.5));
        assert_eq!(grades[1].notes, "half \"works\" {");
        assert!(read(r#"{"grades": [{"label": "A", "score": 11}]}"#, &labels[..1]).is_err());
        assert!(
            read(r#"{"grades": [{"label": "A", "score": 3}]}"#, &labels)
                .unwrap_err()
                .contains("B")
        );
        assert!(read("no json", &labels).is_err());
    }
}
