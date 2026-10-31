//! What a model is asked to place tasks no rule places, and how its answer
//! is read: one of the person's categories for each, or "other", checked
//! before it is kept.

use std::collections::HashMap;

use crate::categories::{Categories, OTHER};
use crate::labels::Label;
use crate::task::Task;

/// What the model is, standing.
pub(crate) const INSTRUCTIONS: &str = "You place the tasks of a coding agent among the \
categories of work the person named. Each goes to the one category that fits it best, by what \
the agent was given and what it did; to \"other\" only when none fits. Stay consistent with the \
tasks placed before: the same kind of work goes to the same category. Answer with the JSON \
asked for, nothing after it.";

/// How much of a task's prompt the model reads.
const PROMPT_CHARS: usize = 240;

/// How many tasks placed before are shown of each category, to keep to.
const EXAMPLES: usize = 2;

/// What the model is asked: the categories, examples placed before, and
/// `tasks`, each by a short number.
pub(crate) fn prompt(categories: &Categories, before: &[(Task, String)], tasks: &[Task]) -> String {
    let mut out = String::from("## The categories\n\n");
    for category in &categories.categories {
        if category.description.trim().is_empty() {
            out.push_str(&format!("- {}\n", category.name));
        } else {
            out.push_str(&format!(
                "- {}: {}\n",
                category.name,
                category.description.trim()
            ));
        }
    }
    out.push_str(&format!("- {OTHER}: none of these\n"));
    let examples: Vec<String> = categories
        .categories
        .iter()
        .flat_map(|category| {
            before
                .iter()
                .filter(|(_, name)| *name == category.name)
                .take(EXAMPLES)
                .map(|(task, name)| format!("- \"{}\" → {name}", short(&task.prompt, 100)))
        })
        .collect();
    if !examples.is_empty() {
        out.push_str("\n## Tasks placed before\n\n");
        out.push_str(&examples.join("\n"));
        out.push('\n');
    }
    out.push_str("\n## The tasks to place\n\n");
    for (at, task) in tasks.iter().enumerate() {
        out.push_str(&format!(
            "T{}: given \"{}\"",
            at + 1,
            short(&task.prompt, PROMPT_CHARS)
        ));
        let files = summary(&task.files);
        if !files.is_empty() {
            out.push_str(&format!("; files written: {files}"));
        }
        let commands = summary(&task.commands);
        if !commands.is_empty() {
            out.push_str(&format!("; commands run: {commands}"));
        }
        out.push('\n');
    }
    out.push_str(
        "\n## Your answer\n\nOne JSON object, nothing after it, a category for each task by its number:\n\n\
         {\"categories\": {\"T1\": \"a category's name\"}}\n",
    );
    out
}

/// `text` cut to `most` characters, on one line.
fn short(text: &str, most: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= most {
        line
    } else {
        let cut: String = line.chars().take(most).collect();
        format!("{cut}…")
    }
}

/// A few of `items`, the rest counted.
fn summary(items: &[String]) -> String {
    let mut shown: Vec<&str> = items.iter().map(String::as_str).take(8).collect();
    if items.len() > shown.len() {
        shown.push("…");
    }
    shown.join(", ")
}

/// The labels in the model's `answer` for `tasks`, as `model` gave them:
/// a name the categories do not know is taken as [`OTHER`], a task left
/// out is left unplaced.
pub(crate) fn read(
    answer: &str,
    tasks: &[Task],
    categories: &Categories,
    model: &str,
    now: u64,
) -> Vec<Label> {
    let Some(json) = object(answer) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let given: HashMap<String, String> = parsed["categories"]
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    Some((key.trim().to_owned(), value.as_str()?.trim().to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    tasks
        .iter()
        .enumerate()
        .filter_map(|(at, task)| {
            let name = given.get(&format!("T{}", at + 1))?;
            let category = if categories.knows(name) {
                name.clone()
            } else {
                OTHER.to_owned()
            };
            Some(Label {
                task: task.id(),
                category,
                by: model.to_owned(),
                at: now,
            })
        })
        .collect()
}

/// The outermost `{…}` of `text`.
fn object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| &text[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categories::Category;

    fn categories() -> Categories {
        Categories::new(vec![
            Category {
                name: "Reviews".into(),
                description: "reading others' changes".into(),
                ..Category::default()
            },
            Category {
                name: "Python".into(),
                files: vec!["*.py".into()],
                ..Category::default()
            },
        ])
    }

    fn task(at: u64, prompt: &str) -> Task {
        Task {
            at,
            agent: "lead".into(),
            prompt: prompt.into(),
            files: vec!["src/a.rs".into()],
            ..Task::default()
        }
    }

    #[test]
    fn the_model_is_shown_the_categories_and_what_was_placed_before() {
        let before = vec![(task(1, "review the auth PR"), "Reviews".to_owned())];
        let asked = prompt(
            &categories(),
            &before,
            &[task(2, "review   the\nnew API"), task(3, "x")],
        );
        assert!(
            asked.contains("- Reviews: reading others' changes\n- Python\n- other: none of these"),
            "{asked}"
        );
        assert!(
            asked.contains("\"review the auth PR\" → Reviews"),
            "{asked}"
        );
        assert!(
            asked.contains("T1: given \"review the new API\"; files written: src/a.rs"),
            "{asked}"
        );
        assert!(asked.contains("T2: given \"x\""));
    }

    #[test]
    fn an_answer_is_read_against_the_categories() {
        let tasks = [task(2, "a"), task(3, "b"), task(4, "c")];
        let answer = "Sure.\n{\"categories\": {\"T1\": \"Reviews\", \"T2\": \"Gardening\"}}";
        let labels = read(answer, &tasks, &categories(), "cheap/model", 9);
        assert_eq!(labels.len(), 2, "the third was left out");
        assert_eq!(
            (labels[0].task.as_str(), labels[0].category.as_str()),
            ("2-lead", "Reviews")
        );
        assert_eq!(labels[1].category, OTHER);
        assert_eq!(labels[1].by, "cheap/model");
        assert!(read("no json", &tasks, &categories(), "m", 0).is_empty());
    }
}
