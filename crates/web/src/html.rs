//! A page of the web as a model reads it: its main content as Markdown,
//! as a reader mode shows it. Scripts, styles, menus, headers, footers,
//! forms and what the page hides are dropped; headings, paragraphs, lists,
//! links, tables, quotes and code kept. In plain Rust, bounded by what it is
//! given: no script of the page runs.

use std::collections::HashMap;

/// One piece of a page's markup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Token {
    Text(String),
    Open {
        name: String,
        attrs: HashMap<String, String>,
    },
    Close(String),
}

/// What is dropped whole, its content with it.
const DROPPED: &[&str] = &[
    "script", "style", "noscript", "template", "svg", "head", "nav", "footer", "header", "aside",
    "form", "button", "iframe", "select", "canvas", "object", "dialog",
];

/// Elements that take no closing tag.
const VOID: &[&str] = &[
    "br", "hr", "img", "input", "meta", "link", "area", "base", "col", "embed", "source", "track",
    "wbr",
];

/// The tokens of `html`: its tags, lower-cased, with their attributes, and
/// its text, entities read.
pub(crate) fn tokens(html: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let bytes = html.as_bytes();
    let mut at = 0;
    let mut text = String::new();
    let flush = |text: &mut String, out: &mut Vec<Token>| {
        if !text.is_empty() {
            out.push(Token::Text(entities(text)));
            text.clear();
        }
    };
    while at < html.len() {
        if bytes[at] != b'<' {
            let next = html[at..].find('<').map_or(html.len(), |n| at + n);
            text.push_str(&html[at..next]);
            at = next;
            continue;
        }
        let rest = &html[at..];
        if let Some(comment) = rest.strip_prefix("<!--") {
            at += 4 + comment.find("-->").map_or(comment.len(), |n| n + 3);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            at += rest.find('>').map_or(rest.len(), |n| n + 1);
            continue;
        }
        let Some(end) = tag_end(rest) else {
            text.push('<');
            at += 1;
            continue;
        };
        let inner = &rest[1..end];
        at += end + 1;
        let closing = inner.starts_with('/');
        let inner = inner.trim_start_matches('/');
        let name: String = inner
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if name.is_empty() {
            text.push('<');
            text.push_str(inner);
            text.push('>');
            continue;
        }
        flush(&mut text, &mut out);
        if closing {
            out.push(Token::Close(name));
            continue;
        }
        let raw = matches!(name.as_str(), "script" | "style");
        out.push(Token::Open {
            name: name.clone(),
            attrs: attributes(&inner[name.len()..]),
        });
        // A script's or a style's content is no markup: skipped to its end.
        if raw {
            let close = format!("</{name}");
            let lower = html[at..].to_ascii_lowercase();
            at += lower.find(&close).unwrap_or(lower.len());
        }
    }
    flush(&mut text, &mut out);
    out
}

/// Where the tag starting `rest` ends, quotes in its attributes skipped.
fn tag_end(rest: &str) -> Option<usize> {
    let mut quote = None;
    for (at, c) in rest.char_indices().skip(1) {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(at),
            (None, '<') if at > 1 => return None,
            _ => {}
        }
    }
    None
}

/// The attributes written in a tag, names lower-cased, values unquoted.
fn attributes(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut chars = text.trim_end_matches('/').chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == '/') {
            chars.next();
        }
        let name: String =
            std::iter::from_fn(|| chars.next_if(|c| !c.is_whitespace() && *c != '=' && *c != '/'))
                .collect();
        if name.is_empty() {
            return out;
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut value = String::new();
        if chars.next_if_eq(&'=').is_some() {
            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
            match chars.peek().copied() {
                Some(q @ ('"' | '\'')) => {
                    chars.next();
                    value = std::iter::from_fn(|| chars.next_if(|c| *c != q)).collect();
                    chars.next();
                }
                _ => {
                    value = std::iter::from_fn(|| chars.next_if(|c| !c.is_whitespace())).collect();
                }
            }
        }
        out.insert(name.to_ascii_lowercase(), entities(&value));
    }
}

/// `text` with its entities read: the common named ones and every number.
pub(crate) fn entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest[1..].find(';').map(|n| n + 1).filter(|n| *n <= 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let read = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('-'),
            "mdash" => Some('-'),
            "hellip" => Some('…'),
            "rsquo" | "lsquo" => Some('\''),
            "rdquo" | "ldquo" => Some('"'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|n| n.parse().ok()))
                .and_then(char::from_u32),
        };
        match read {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Whether an element is hidden by what it says of itself.
fn hidden(attrs: &HashMap<String, String>) -> bool {
    attrs.contains_key("hidden")
        || attrs.get("aria-hidden").is_some_and(|v| v == "true")
        || attrs
            .get("style")
            .is_some_and(|s| s.replace(' ', "").contains("display:none"))
}

/// The tokens of the page's main content: its first `main` or `article`,
/// else its `body`, else all of it.
fn main_content(tokens: &[Token]) -> &[Token] {
    for wanted in ["main", "article", "body"] {
        let Some(start) = tokens
            .iter()
            .position(|t| matches!(t, Token::Open { name, .. } if name == wanted))
        else {
            continue;
        };
        let mut depth = 0usize;
        for (at, token) in tokens.iter().enumerate().skip(start) {
            match token {
                Token::Open { name, .. } if name == wanted => depth += 1,
                Token::Close(name) if name == wanted => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return &tokens[start + 1..at];
                    }
                }
                _ => {}
            }
        }
        return &tokens[start + 1..];
    }
    tokens
}

/// The page's title, when it says one.
pub(crate) fn title(tokens: &[Token]) -> Option<String> {
    let at = tokens
        .iter()
        .position(|t| matches!(t, Token::Open { name, .. } if name == "title"))?;
    match tokens.get(at + 1) {
        Some(Token::Text(text)) => Some(squeezed(text)).filter(|t| !t.is_empty()),
        _ => None,
    }
}

/// `text` with its runs of white space made one space.
fn squeezed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The Markdown being written: its text, and where a link and a table
/// stand.
#[derive(Default)]
struct Writer {
    out: String,
    /// The text of the link being written, and its address.
    link: Option<(String, String)>,
    /// The rows of the table being written, each its cells.
    table: Option<Vec<Vec<String>>>,
    /// Whether the text goes in a block of code, kept as it is.
    pre: usize,
    /// How deep the lists are.
    lists: usize,
}

impl Writer {
    /// Ends the line, a blank one after it when `blank`.
    fn block(&mut self, blank: bool) {
        if let Some(rows) = &mut self.table
            && let Some(row) = rows.last_mut()
            && let Some(cell) = row.last_mut()
        {
            cell.push(' ');
            return;
        }
        let trimmed = self.out.trim_end_matches(' ').len();
        self.out.truncate(trimmed);
        if self.out.is_empty() {
            return;
        }
        let wanted = if blank { "\n\n" } else { "\n" };
        while !self.out.ends_with(wanted) {
            self.out.push('\n');
        }
    }

    /// Writes `text`, where it goes: a link's text, a table's cell, the
    /// page.
    fn text(&mut self, text: &str) {
        let text = if self.pre > 0 {
            text.to_owned()
        } else {
            let squeezed = squeezed(text);
            let spaced_before = text.starts_with(char::is_whitespace);
            let spaced_after = text.ends_with(char::is_whitespace);
            if squeezed.is_empty() {
                if spaced_before {
                    " ".to_owned()
                } else {
                    String::new()
                }
            } else {
                format!(
                    "{}{squeezed}{}",
                    if spaced_before { " " } else { "" },
                    if spaced_after { " " } else { "" }
                )
            }
        };
        if text.is_empty() {
            return;
        }
        if let Some((label, _)) = &mut self.link {
            label.push_str(&text);
        } else if let Some(rows) = &mut self.table {
            if let Some(cell) = rows.last_mut().and_then(|row| row.last_mut()) {
                cell.push_str(&text);
            }
        } else {
            let at_start = self.out.is_empty() || self.out.ends_with('\n');
            self.out
                .push_str(if at_start { text.trim_start() } else { &text });
        }
    }

    /// Writes the table held, as Markdown's rows.
    fn table_out(&mut self) {
        let Some(rows) = self.table.take() else {
            return;
        };
        let rows: Vec<Vec<String>> = rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|c| squeezed(&c).replace('|', "\\|"))
                    .collect()
            })
            .filter(|row: &Vec<String>| row.iter().any(|c| !c.is_empty()))
            .collect();
        if rows.is_empty() {
            return;
        }
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        self.block(true);
        for (at, row) in rows.iter().enumerate() {
            let mut cells = row.clone();
            cells.resize(columns, String::new());
            self.out.push_str(&format!("| {} |\n", cells.join(" | ")));
            if at == 0 {
                self.out
                    .push_str(&format!("|{}\n", " --- |".repeat(columns)));
            }
        }
        self.out.push('\n');
    }
}

/// The page `html`, from `base`, as Markdown: its main content, its links
/// made whole from `base`.
pub(crate) fn markdown(html: &str, base: Option<&reqwest::Url>) -> String {
    let all = tokens(html);
    let content = main_content(&all);
    let mut w = Writer::default();
    // The depth within what is dropped, by element.
    let mut skipping: Vec<String> = Vec::new();
    let mut ordered: Vec<usize> = Vec::new();
    for token in content {
        match token {
            Token::Open { name, attrs } => {
                if !skipping.is_empty() {
                    if !VOID.contains(&name.as_str()) {
                        skipping.push(name.clone());
                    }
                    continue;
                }
                if DROPPED.contains(&name.as_str()) || hidden(attrs) {
                    if !VOID.contains(&name.as_str()) {
                        skipping.push(name.clone());
                    }
                    continue;
                }
                match name.as_str() {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        w.block(true);
                        let level = usize::from(name.as_bytes()[1] - b'0');
                        w.out.push_str(&format!("{} ", "#".repeat(level)));
                    }
                    "p" | "div" | "section" | "figure" | "figcaption" | "dl" | "dt" | "dd" => {
                        w.block(name == "p");
                    }
                    "br" => w.block(false),
                    "hr" => {
                        w.block(true);
                        w.out.push_str("---\n\n");
                    }
                    "ul" => {
                        w.lists += 1;
                        ordered.push(0);
                        w.block(false);
                    }
                    "ol" => {
                        w.lists += 1;
                        ordered.push(1);
                        w.block(false);
                    }
                    "li" => {
                        w.block(false);
                        let indent = "  ".repeat(w.lists.saturating_sub(1));
                        let mark = match ordered.last_mut() {
                            Some(n) if *n > 0 => {
                                let mark = format!("{n}. ");
                                *n += 1;
                                mark
                            }
                            _ => "- ".to_owned(),
                        };
                        w.out.push_str(&format!("{indent}{mark}"));
                    }
                    "blockquote" => {
                        w.block(true);
                        w.out.push_str("> ");
                    }
                    "pre" => {
                        w.block(true);
                        w.out.push_str("```\n");
                        w.pre += 1;
                    }
                    "code" if w.pre == 0 => w.text("`"),
                    "strong" | "b" => w.text("**"),
                    "em" | "i" => w.text("*"),
                    "a" => {
                        let href = attrs.get("href").cloned().unwrap_or_default();
                        let address = match base.and_then(|b| b.join(&href).ok()) {
                            Some(url) => url.to_string(),
                            None => href,
                        };
                        w.link = Some((String::new(), address));
                    }
                    "img" => {
                        if let Some(alt) = attrs.get("alt").filter(|a| !a.trim().is_empty()) {
                            w.text(&format!("[image: {}]", alt.trim()));
                        }
                    }
                    "table" => w.table = Some(Vec::new()),
                    "tr" => {
                        if let Some(rows) = &mut w.table {
                            rows.push(Vec::new());
                        }
                    }
                    "td" | "th" => {
                        if let Some(row) = w.table.as_mut().and_then(|rows| rows.last_mut()) {
                            row.push(String::new());
                        }
                    }
                    _ => {}
                }
            }
            Token::Close(name) => {
                if let Some(at) = skipping.iter().rposition(|n| n == name) {
                    skipping.truncate(at);
                    continue;
                }
                if !skipping.is_empty() {
                    continue;
                }
                match name.as_str() {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "blockquote" => w.block(true),
                    "div" | "section" | "li" | "dt" | "dd" | "figure" => w.block(false),
                    "ul" | "ol" => {
                        w.lists = w.lists.saturating_sub(1);
                        ordered.pop();
                        w.block(w.lists == 0);
                    }
                    "pre" => {
                        w.pre = w.pre.saturating_sub(1);
                        if !w.out.ends_with('\n') {
                            w.out.push('\n');
                        }
                        w.out.push_str("```\n\n");
                    }
                    "code" if w.pre == 0 => w.text("`"),
                    "strong" | "b" => w.text("**"),
                    "em" | "i" => w.text("*"),
                    "a" => {
                        if let Some((label, address)) = w.link.take() {
                            let label = squeezed(&label);
                            let written = match (label.is_empty(), address.starts_with("http")) {
                                (true, _) => String::new(),
                                (false, true) => format!("[{label}]({address})"),
                                (false, false) => label,
                            };
                            w.text(&written);
                        }
                    }
                    "table" => w.table_out(),
                    _ => {}
                }
            }
            Token::Text(text) => {
                if skipping.is_empty() {
                    w.text(text);
                }
            }
        }
    }
    w.table_out();
    let mut out = String::new();
    let mut blank = 0;
    for line in w.out.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_its_main_content_as_markdown_what_surrounds_it_dropped() {
        let html = r#"<!doctype html><html><head><title>The Guide</title>
            <style>body { color: red }</style><script>alert("<b>no</b>")</script></head>
            <body><nav><a href="/">Home</a> | <a href="/docs">Docs</a></nav>
            <main><h1>Getting started</h1>
            <p>Install it with <code>cargo add tool</code>, then read
            <a href="/docs/intro">the introduction</a>.</p>
            <ul><li>fast</li><li>small <b>and</b> safe</li></ul>
            <div style="display: none">hidden text</div>
            <table><tr><th>Name</th><th>Size</th></tr><tr><td>a</td><td>1&nbsp;KB</td></tr></table>
            <pre>fn main() {
    run();
}</pre>
            </main><footer>© a company</footer></body></html>"#;
        let base = reqwest::Url::parse("https://example.com/guide/").unwrap();
        let tokens = tokens(html);
        assert_eq!(title(&tokens).as_deref(), Some("The Guide"));
        assert_eq!(
            markdown(html, Some(&base)),
            "# Getting started\n\nInstall it with `cargo add tool`, then read [the introduction](https://example.com/docs/intro).\n\n- fast\n- small **and** safe\n\n| Name | Size |\n| --- | --- |\n| a | 1 KB |\n\n```\nfn main() {\n    run();\n}\n```"
        );
    }

    #[test]
    fn entities_and_attributes_are_read_and_broken_markup_kept_as_text() {
        assert_eq!(
            entities("a &amp; b &lt;c&gt; &#233;&#x2014; &bogus; &"),
            "a & b <c> é— &bogus; &"
        );
        assert_eq!(
            tokens(r#"<a href='x y' data-x=1 hidden>t</a>"#)[0],
            Token::Open {
                name: "a".into(),
                attrs: HashMap::from([
                    ("href".into(), "x y".into()),
                    ("data-x".into(), "1".into()),
                    ("hidden".into(), String::new()),
                ]),
            }
        );
        assert_eq!(markdown("2 < 3 and <unclosed", None), "2 < 3 and <unclosed");
        let ordered = markdown("<ol><li>one</li><li>two</li></ol>", None);
        assert_eq!(ordered, "1. one\n2. two");
    }
}
