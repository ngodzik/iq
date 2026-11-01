//! The models the person names in what they write, as they would say it
//! to anyone: "with opus", "avec haiku", "sur glm-5". A name counts only
//! after such a word, so that a haiku asked for stays a poem. Read here,
//! by matching words, never by a model: what the person names is what
//! works.

/// The words after which a model is named, in English and French.
const BEFORE: [&str; 9] = [
    "with", "using", "via", "on", "by", "avec", "sur", "par", "en",
];

/// The models `text` names, in the order it names them, among `known`,
/// the provider's, in the order it lists them: by their whole name, the
/// part after the provider's, or one word of it, as people say `opus` for
/// `anthropic/claude-opus-5-5`. Of several a word names, the newest, as
/// their numbers say, then one served without a region or a variant
/// (`@eu`, `:flex`), then the first listed. A name with a slash the
/// provider does not list is no model: `src/main.rs` is a path.
///
/// # Examples
///
/// ```
/// use iq_ui::named::named;
///
/// let known = [
///     "anthropic/claude-opus-4-1".to_owned(),
///     "vertex/claude-opus-5-5@eu".to_owned(),
///     "anthropic/claude-opus-5-5".to_owned(),
///     "vertex/claude-opus-5-5".to_owned(),
///     "z-ai/glm-5.3-flash".to_owned(),
/// ];
/// assert_eq!(
///     named("Fais la conception avec Opus, et le code avec glm-5.3-flash.", &known),
///     ["anthropic/claude-opus-5-5", "z-ai/glm-5.3-flash"]
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

/// The model of `known` that `word` names, as [`named`] reads a word
/// after "with": its whole name, the part after the provider's, or one
/// word of it. None when it names none.
///
/// # Examples
///
/// ```
/// use iq_ui::named::model;
///
/// let known = ["lab/sage-5-5".to_owned()];
/// assert_eq!(model("Sage", &known).as_deref(), Some("lab/sage-5-5"));
/// assert_eq!(model("chat2", &known), None);
/// ```
#[must_use]
pub fn model(word: &str, known: &[String]) -> Option<String> {
    resolve(&word.trim().to_lowercase(), known)
}

/// The model `word` names, if it names one.
fn resolve(word: &str, known: &[String]) -> Option<String> {
    if word.is_empty() {
        return None;
    }
    let last = |model: &str| model.rsplit('/').next().unwrap_or(model).to_owned();
    if let Some(model) = known
        .iter()
        .find(|m| m.eq_ignore_ascii_case(word) || last(m).eq_ignore_ascii_case(word))
    {
        return Some(model.clone());
    }
    // A word of a model's name, never a number alone: `opus`, `glm`.
    if !word.chars().any(char::is_alphabetic) || word.contains('/') {
        return None;
    }
    let mut best: Option<(&String, Rank)> = None;
    for model in known {
        let name = last(model);
        if !name
            .split(['-', '.', '_', ':', '@'])
            .any(|part| part.eq_ignore_ascii_case(word))
        {
            continue;
        }
        let rank = rank(&name);
        // Strictly better only: of equals, the first listed stays.
        if best.as_ref().is_none_or(|(_, kept)| rank > *kept) {
            best = Some((model, rank));
        }
    }
    best.map(|(model, _)| model.clone())
}

/// How a model's name ranks among those a word names: its numbers as a
/// version, then whether it is served without a region or a variant.
type Rank = (Vec<u64>, bool);

/// The rank of `name`, a model's name without its provider's.
fn rank(name: &str) -> Rank {
    let base = name.split(['@', ':']).next().unwrap_or(name);
    let numbers = base
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter_map(|part| part.parse().ok())
        .collect();
    (numbers, base.len() == name.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_counts_only_after_a_word_that_names_it() {
        let known = vec![
            "lab/model-7-quill".to_owned(),
            "z-ai/glm-5".to_owned(),
            "anthropic/claude-sonnet-5-9".to_owned(),
            "anthropic/claude-sonnet-5-10".to_owned(),
        ];
        assert_eq!(
            named("réfléchis à ça avec sonnet", &known),
            ["anthropic/claude-sonnet-5-10"]
        );
        assert_eq!(
            named("run it on model-7-quill.", &known),
            ["lab/model-7-quill"]
        );
        assert_eq!(
            named("with z-ai/glm-5, then with z-ai/glm-5", &known),
            ["z-ai/glm-5"]
        );
        // A haiku, a word after "with" that names nothing, a path, a
        // number.
        assert!(named("write a haiku with care", &known).is_empty());
        assert!(named("work on src/main.rs", &known).is_empty());
        assert!(named("do it in 5 steps, on 5", &known).is_empty());
        assert_eq!(
            named("look at src/main.rs with quill", &known),
            ["lab/model-7-quill"]
        );
    }
}
