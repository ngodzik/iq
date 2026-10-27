//! The rules agents follow: the person's own, kept in files of their home
//! such as Claude Code's `~/.claude/CLAUDE.md`, and the project's, in its
//! `AGENTS.md` and `CLAUDE.md`. Only the agents the person starts read
//! them: they give the agents they start what their part needs.

/// The most of one file of rules an agent is given, in bytes.
pub(crate) const RULES_MOST: usize = 40_000;

/// A file of the person's rules, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesFile {
    /// Where it was read, as the person names it: `~/.claude/CLAUDE.md`.
    pub from: String,
    /// What it says, its imports read in.
    pub text: String,
}

/// `text` with each line that is `@` and a file name replaced by that
/// file, as `read` gives it, the way Claude Code's files import others;
/// a line `read` gives nothing for stays as it is.
///
/// # Examples
///
/// ```
/// let text = iq_agent::with_imports("Rules:\n@more.md\n@missing.md\n", |name| {
///     (name == "more.md").then(|| "Tests first.".to_owned())
/// });
/// assert_eq!(text, "Rules:\nTests first.\n@missing.md\n");
/// ```
pub fn with_imports(text: &str, read: impl Fn(&str) -> Option<String>) -> String {
    text.lines()
        .map(|line| {
            let imported = import(line).and_then(&read);
            imported.unwrap_or_else(|| line.to_owned()) + "\n"
        })
        .collect()
}

/// The file a line imports, if it is one: `@` and a name, nothing else.
pub(crate) fn import(line: &str) -> Option<&str> {
    line.trim()
        .strip_prefix('@')
        .filter(|name| !name.is_empty() && !name.contains(char::is_whitespace))
}

/// `text` trimmed, cut to [`RULES_MOST`] bytes with a word on what was
/// left out; `None` when nothing is left.
pub(crate) fn kept(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut cut = text.len().min(RULES_MOST);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let more = if cut < text.len() {
        "\n[the rest of the file is left out]"
    } else {
        ""
    };
    Some(format!("{}{more}", &text[..cut]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_are_cut_to_what_an_agent_is_given() {
        assert_eq!(kept("  \n"), None);
        assert_eq!(kept(" a \n").as_deref(), Some("a"));
        let long = "é".repeat(RULES_MOST);
        let cut = kept(&long).unwrap();
        assert!(cut.ends_with("[the rest of the file is left out]"));
        assert!(cut.len() < long.len());
        assert_eq!(import(" @docs/rules.md "), Some("docs/rules.md"));
        assert_eq!(import("@ two words"), None);
        assert_eq!(import("email me @ home"), None);
    }
}
