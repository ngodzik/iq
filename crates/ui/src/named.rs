//! The models the person names in what they write, as they would say it
//! to anyone: "with opus", "avec haiku", "sur glm-5". A name counts only
//! after such a word, so that a haiku asked for stays a poem. Read here,
//! by matching words, never by a model: what the person names is what
//! works.

/// The words after which a model is named, in English and French.
const BEFORE: [&str; 9] = [
    "with", "using", "via", "on", "by", "avec", "sur", "par", "en",
];

/// The short names of the agents' models, as people say them.
const ALIASES: [(&str, &str); 6] = [
    ("opus", "claude-code/opus"),
    ("sonnet", "claude-code/sonnet"),
    ("haiku", "claude-code/haiku"),
    ("claude", "claude-code"),
    ("claude-code", "claude-code"),
    ("codex", "codex"),
];

/// The models `text` names, in the order it names them: short names of
/// the agents' models, an agent's model written whole (`claude-code/opus`,
/// `codex/gpt-6.1-sol`), and models of `known`, the provider's, by their
/// whole name or the part after the provider's. A name with a slash the
/// provider does not list is no model: `src/main.rs` is a path.
///
/// # Examples
///
/// ```
/// use iq_ui::named::named;
///
/// let known = ["z-ai/glm-5.3-flash".to_owned()];
/// assert_eq!(
///     named("Fais la conception avec Opus, et le code avec glm-5.3-flash.", &known),
///     ["claude-code/opus", "z-ai/glm-5.3-flash"]
/// );
/// assert!(named("Write a haiku about rust", &known).is_empty());
/// ```
#[must_use]
pub fn named(text: &str, known: &[String]) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut found: Vec<String> = Vec::new();
    for pair in words.windows(2) {
        let before = pair[0]
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if !BEFORE.contains(&before.as_str()) {
            continue;
        }
        let word = pair[1]
            .trim_matches(|c: char| !(c.is_alphanumeric() || "/-._:".contains(c)))
            .trim_end_matches(['.', ':'])
            .to_lowercase();
        if let Some(model) = resolve(&word, known)
            && !found.contains(&model)
        {
            found.push(model);
        }
    }
    found
}

/// The model `word` names, if it names one.
fn resolve(word: &str, known: &[String]) -> Option<String> {
    if word.is_empty() {
        return None;
    }
    if let Some((_, model)) = ALIASES.iter().find(|(alias, _)| *alias == word) {
        return Some((*model).to_owned());
    }
    if let Some(model) = known.iter().find(|m| {
        m.eq_ignore_ascii_case(word)
            || m.rsplit('/')
                .next()
                .is_some_and(|last| last.eq_ignore_ascii_case(word))
    }) {
        return Some(model.clone());
    }
    ["claude-code/", "codex/"]
        .iter()
        .find_map(|agent| word.strip_prefix(agent))
        .filter(|model| {
            !model.is_empty()
                && model
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-._".contains(c))
        })
        .map(|_| word.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_counts_only_after_a_word_that_names_it() {
        let known = vec!["openai/gpt-6-luna".to_owned(), "z-ai/glm-5".to_owned()];
        assert_eq!(
            named("réfléchis à ça avec opus", &known),
            ["claude-code/opus"]
        );
        assert_eq!(
            named("run it on gpt-6-luna.", &known),
            ["openai/gpt-6-luna"]
        );
        assert_eq!(
            named("with z-ai/glm-5, then with z-ai/glm-5", &known),
            ["z-ai/glm-5"]
        );
        assert_eq!(
            named("using codex/gpt-6.1-sol", &known),
            ["codex/gpt-6.1-sol"]
        );
        // A haiku, a word after "with" that names nothing, a path.
        assert!(named("write a haiku with care", &known).is_empty());
        assert!(named("work on src/main.rs", &known).is_empty());
        assert_eq!(
            named("look at src/main.rs with sonnet", &known),
            ["claude-code/sonnet"]
        );
    }
}
