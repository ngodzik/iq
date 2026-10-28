//! The rules agents follow: the person's own, kept in files of their home
//! such as `~/.claude/CLAUDE.md`, and the project's, in its
//! `AGENTS.md`, its `CLAUDE.md` and the files of its `.claude/rules`. Only
//! the agents the person starts read them: they give the agents they start
//! what their part needs. A rule of `.claude/rules` whose head names
//! `paths` is for those files alone: any agent is given it as it first
//! reads or writes one of them.

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
/// file, as `read` gives it, the way `CLAUDE.md` files import others;
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

/// A rule of the project's for some of its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scoped {
    /// Its file, in the workspace: `.claude/rules/api.md`.
    pub(crate) file: String,
    /// The patterns of the files it is for.
    pub(crate) paths: Vec<String>,
    /// What it says.
    pub(crate) text: String,
}

/// A rules file's `paths`, as its head names them, and its body: no paths
/// when its head names none, as a rule for the whole project's does. The
/// paths are a list, one `- pattern` a line, or on the line itself,
/// written with commas or in brackets.
pub(crate) fn scope(text: &str) -> (Vec<String>, &str) {
    let Some(rest) = text.strip_prefix("---\n") else {
        return (Vec::new(), text);
    };
    let Some((head, body)) = rest.split_once("\n---") else {
        return (Vec::new(), text);
    };
    let body = body.split_once('\n').map_or("", |(_, body)| body);
    let mut paths = Vec::new();
    let mut lines = head.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(value) = line.trim_start().strip_prefix("paths:") else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            while let Some(item) = lines
                .peek()
                .and_then(|line| line.trim_start().strip_prefix('-'))
            {
                paths.push(unquoted(item));
                lines.next();
            }
        } else {
            let value = value.strip_prefix('[').unwrap_or(value);
            let value = value.strip_suffix(']').unwrap_or(value);
            paths.extend(listed(value));
        }
    }
    paths.retain(|path| !path.is_empty());
    (paths, body)
}

/// The items of a list written with commas, those within braces or quotes
/// kept whole.
fn listed(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let (mut item, mut braces, mut quote) = (String::new(), 0usize, None);
    for c in text.chars() {
        match c {
            '"' | '\'' if quote == Some(c) => quote = None,
            '"' | '\'' if quote.is_none() => quote = Some(c),
            '{' if quote.is_none() => braces += 1,
            '}' if quote.is_none() => braces = braces.saturating_sub(1),
            ',' if quote.is_none() && braces == 0 => {
                items.push(unquoted(&item));
                item.clear();
                continue;
            }
            _ => {}
        }
        item.push(c);
    }
    items.push(unquoted(&item));
    items
}

/// `text` trimmed of its spaces and quotes.
fn unquoted(text: &str) -> String {
    text.trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .trim()
        .to_owned()
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

    #[test]
    fn a_rule_for_some_files_names_them_in_its_head() {
        let (paths, body) =
            scope("---\npaths:\n  - \"src/api/**\"\n  - lib/*.ts\n---\nValidate every input.\n");
        assert_eq!(paths, ["src/api/**", "lib/*.ts"]);
        assert_eq!(body, "Validate every input.\n");
        let (paths, _) =
            scope("---\ndescription: x\npaths: \"src/**/*.{ts,tsx}\", docs/**\n---\nShort.\n");
        assert_eq!(paths, ["src/**/*.{ts,tsx}", "docs/**"]);
        let (paths, _) = scope("---\npaths: [a/**, 'b/*.md']\n---\nx\n");
        assert_eq!(paths, ["a/**", "b/*.md"]);
        // For the whole project: no head, or none naming paths.
        assert_eq!(scope("Tests first.\n"), (Vec::new(), "Tests first.\n"));
        assert!(scope("---\nname: style\n---\nShort lines.\n").0.is_empty());
    }
}
