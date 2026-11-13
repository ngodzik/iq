//! What an agent's compactions replaced, found again: the messages they
//! took out of its conversation, one entry each, numbered, by the words
//! they hold or on from a number, cut short so that reading them back
//! never swells the conversation for long.

use iq_core::Message;

/// The most characters of one entry given.
const ENTRY_MOST: usize = 4_000;

/// The most characters given in all.
const ALL_MOST: usize = 24_000;

/// `message` as one entry: what it was said to the agent, what the agent
/// said and called, what a tool returned. Its standing instructions are
/// none.
fn entry(message: &Message) -> Option<String> {
    Some(match message {
        Message::System(_) => return None,
        Message::User(text) => format!("to you: {text}"),
        Message::Assistant {
            content,
            tool_calls,
            ..
        } => {
            let mut said = format!("you: {}", content.as_deref().unwrap_or("").trim());
            for call in tool_calls {
                said.push_str(&format!(
                    "\n(you called {} {})",
                    call.name,
                    crate::tools::tool_input(&call.arguments)
                ));
            }
            said
        }
        Message::Tool { content, .. } => format!("result: {content}"),
        _ => return None,
    })
}

/// What `look_back` answers from `away`, the messages compactions
/// replaced: the entries holding all the words of `query`, or those on
/// from the number `from`, or both, each cut past [`ENTRY_MOST`]
/// characters and [`ALL_MOST`] in all, saying what was left out and how
/// to ask for it.
pub(crate) fn found(away: &[Message], query: Option<&str>, from: Option<usize>) -> String {
    let entries: Vec<String> = away.iter().filter_map(entry).collect();
    if entries.is_empty() {
        return "Nothing of your conversation was compacted away: all of it is still there.".into();
    }
    let words: Vec<String> = query
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let from = from.unwrap_or(1).max(1);
    let chosen: Vec<(usize, &String)> = entries
        .iter()
        .enumerate()
        .map(|(at, text)| (at + 1, text))
        .filter(|(number, _)| *number >= from)
        .filter(|(_, text)| {
            let lower = text.to_lowercase();
            words.iter().all(|word| lower.contains(word.as_str()))
        })
        .collect();
    let asked = match query {
        Some(query) if !words.is_empty() => format!(" holding all of \"{}\"", query.trim()),
        _ => String::new(),
    };
    if chosen.is_empty() {
        return format!(
            "No entry{asked} from {from} on, of the {} your compactions replaced: `from` reads \
             them on from a number, `query` finds them by their words.",
            entries.len()
        );
    }
    let mut out = format!(
        "{} entries{asked} of the {} your compactions replaced, oldest first:",
        chosen.len(),
        entries.len()
    );
    for (given, (number, text)) in chosen.iter().enumerate() {
        let cut = cut(text);
        if given > 0 && out.chars().count() + cut.chars().count() > ALL_MOST {
            let again = if asked.is_empty() {
                format!("`look_back` with from {number}")
            } else {
                format!("`look_back` with the same query and from {number}")
            };
            out.push_str(&format!(
                "\n\n({} more left out: {again} reads on.)",
                chosen.len() - given
            ));
            return out;
        }
        out.push_str(&format!("\n\n#{number} {cut}"));
    }
    out
}

/// `text`, or its first [`ENTRY_MOST`] characters and how many it leaves.
fn cut(text: &str) -> String {
    match text.char_indices().nth(ENTRY_MOST) {
        Some((at, _)) => format!(
            "{}… ({} more characters)",
            &text[..at],
            text[at..].chars().count()
        ),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iq_core::ToolCall;

    fn away() -> Vec<Message> {
        vec![
            Message::system("be brief"),
            Message::user("rename the parser"),
            Message::Assistant {
                content: Some("Reading it.".into()),
                tool_calls: vec![ToolCall {
                    id: "1".into(),
                    name: "read_file".into(),
                    arguments: r#"{"path": "src/parser.rs"}"#.into(),
                }],
                reasoning_signature: None,
            },
            Message::Tool {
                call_id: "1".into(),
                content: "fn parse() {}".into(),
            },
            Message::Assistant {
                content: Some("Renamed the Parser to Reader.".into()),
                tool_calls: Vec::new(),
                reasoning_signature: None,
            },
        ]
    }

    #[test]
    fn entries_are_found_by_their_words_or_read_on_from_a_number() {
        let said = found(&away(), Some("parser RENAMED"), None);
        assert_eq!(
            said,
            "1 entries holding all of \"parser RENAMED\" of the 4 your compactions replaced, \
             oldest first:\n\n#4 you: Renamed the Parser to Reader."
        );
        let said = found(&away(), None, Some(2));
        assert!(
            said.contains("#2 you: Reading it.\n(you called read_file src/parser.rs)"),
            "{said}"
        );
        assert!(said.contains("#3 result: fn parse() {}"), "{said}");
        assert!(!said.contains("#1 "), "{said}");
        assert!(found(&away(), Some("lexer"), None).starts_with("No entry holding all of"));
        assert!(found(&[], None, None).starts_with("Nothing of your conversation"));
    }

    #[test]
    fn a_long_entry_is_cut_and_what_passes_the_most_is_left_for_later() {
        let long: Vec<Message> = (0..12)
            .map(|n| Message::user(format!("{n} {}", "x".repeat(ENTRY_MOST * 2))))
            .collect();
        let said = found(&long, None, None);
        // "to you: 0 " and twice the most, less the most.
        assert!(said.contains(&format!("… ({} more characters)", ENTRY_MOST + 10)));
        assert!(said.chars().count() <= ALL_MOST + 200, "{}", said.len());
        assert!(
            said.ends_with("more left out: `look_back` with from 6 reads on.)"),
            "{}",
            &said[said.len() - 80..]
        );
    }
}
