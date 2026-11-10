//! What models write, styled as the Markdown it is, a line at a time: a
//! line is written to the conversation once it is whole, while the model
//! still writes the next, so that nothing waits for the end of a long
//! answer. Fenced code needs what came before, and it is kept; a table is
//! held until its last row, and a Mermaid diagram until its fence closes,
//! to be drawn whole.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::table::{self, Table};
use crate::theme::Palette;

/// One line, styled: what goes before its first fold and before the
/// others, then the line.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Styled {
    /// Before its first line.
    pub(crate) first: Vec<Span<'static>>,
    /// Before the lines it folds into.
    pub(crate) rest: Vec<Span<'static>>,
    /// The line.
    pub(crate) body: Vec<Span<'static>>,
    /// Whether it is code, or a mark above or below it: written as it is,
    /// at the line's start and never folded by iq, so that a person's
    /// selection copies it as the model wrote it, the terminal folding it
    /// where it must.
    pub(crate) whole: bool,
    /// The kind of block it belongs to.
    pub(crate) shape: Shape,
}

/// The kind of block a line belongs to, which says whether an empty row
/// stands between it and the line before: one between two blocks, none
/// within one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// An empty line: what the model wrote between two blocks.
    Empty,
    /// A paragraph's line.
    Text,
    /// A list's item, or a line of one, indented under it.
    Item,
    /// A heading.
    Heading,
    /// A quote's line.
    Quote,
    /// A line across, between two parts.
    Rule,
    /// The mark above a block of code.
    Opening,
    /// A line of code, or the mark below it.
    Code,
    /// A table or a diagram, whole.
    Whole,
}

impl Shape {
    /// Whether an empty row stands between a line of `self` and one of
    /// `next`, `parted` when the model left an empty line between them: one
    /// between two blocks, whatever the model left, and none within a list
    /// or a block of code; two lines of text or of a quote are one
    /// paragraph unless parted.
    fn parts(self, next: Shape, parted: bool) -> bool {
        match (self, next) {
            (_, Shape::Opening) => true,
            (Shape::Opening | Shape::Code, Shape::Code) | (Shape::Item, Shape::Item) => false,
            (Shape::Heading, _) | (_, Shape::Heading) => true,
            (before, next) => before != next || parted,
        }
    }
}

impl Styled {
    fn plain(body: Vec<Span<'static>>) -> Styled {
        Styled {
            first: Vec::new(),
            rest: Vec::new(),
            body,
            whole: false,
            shape: Shape::Text,
        }
    }

    fn shaped(self, shape: Shape) -> Styled {
        Styled { shape, ..self }
    }

    fn under(prefix: Span<'static>, hang: Span<'static>, body: Vec<Span<'static>>) -> Styled {
        Styled {
            first: vec![prefix],
            rest: vec![hang],
            body,
            whole: false,
            shape: Shape::Quote,
        }
    }

    fn whole(body: Vec<Span<'static>>) -> Styled {
        Styled {
            whole: true,
            shape: Shape::Code,
            ..Styled::plain(body)
        }
    }

    /// An empty row between two blocks.
    fn gap() -> Styled {
        Styled::plain(Vec::new()).shaped(Shape::Empty)
    }

    /// Whether it is an empty line of text, not of code.
    pub(crate) fn is_empty(&self) -> bool {
        !self.whole && self.first.is_empty() && self.body.iter().all(|s| s.content.is_empty())
    }
}

/// What a line, or the end of a text, gives to write.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Out {
    /// A line, styled.
    Line(Styled),
    /// A table, whole.
    Table(Table),
    /// The source of a Mermaid diagram, whole.
    Diagram(String),
}

/// How the lines of one text are styled, as they come.
#[derive(Debug, Clone, Default)]
pub(crate) struct Markdown {
    /// The fence of the code block the lines are in, if they are in one.
    fence: Option<String>,
    /// Whether a link is written as its text alone, a hyperlink.
    links: bool,
    /// The rows of the table being written.
    table: Vec<String>,
    /// The lines of the Mermaid diagram being written, while its fence is
    /// open.
    diagram: Option<Vec<String>>,
    /// The kind of block of the last line given, none at the text's start.
    last: Option<Shape>,
    /// Whether the model left an empty line since.
    parted: bool,
    /// Whether the last line given was within a list, for the lines
    /// indented under its item.
    listing: bool,
}

/// The fence that opens a block of code in `trimmed`, with its language.
fn opening(trimmed: &str) -> Option<(String, &str)> {
    ["```", "~~~"].into_iter().find_map(|fence| {
        let language = trimmed.strip_prefix(fence)?;
        let marker = format!(
            "{fence}{}",
            language
                .chars()
                .take_while(|c| *c == fence.as_bytes()[0] as char)
                .collect::<String>()
        );
        let language = language.trim_matches(|c: char| c == '`' || c == '~' || c.is_whitespace());
        Some((marker, language))
    })
}

/// The lines of a block of code in `language`, as a fenced block is styled.
pub(crate) fn code(language: &str, source: &str, p: &Palette) -> Vec<Styled> {
    std::iter::once(above(language, p))
        .chain(source.lines().map(|line| code_line(line, p)))
        .chain(std::iter::once(below(p)))
        .collect()
}

/// The mark above a block of code, with its language when it has one.
/// Marks stand above and below the code rather than before each of its
/// lines, which a selection would take with them.
fn above(language: &str, p: &Palette) -> Styled {
    let mark = if language.is_empty() {
        "╭".to_owned()
    } else {
        format!("╭ {language}")
    };
    Styled::whole(vec![Span::styled(mark, p.fg(p.faint))]).shaped(Shape::Opening)
}

/// The mark below a block of code.
fn below(p: &Palette) -> Styled {
    Styled::whole(vec![Span::styled("╰", p.fg(p.faint))])
}

/// A line of code, as it is written.
fn code_line(line: &str, p: &Palette) -> Styled {
    Styled::whole(vec![Span::styled(line.to_owned(), p.fg(p.code))])
}

impl Markdown {
    /// Styling that writes a link as its text alone, a hyperlink, when
    /// `links`, else with its address after it.
    pub(crate) fn linking(links: bool) -> Markdown {
        Markdown {
            links,
            ..Markdown::default()
        }
    }

    /// What `line` gives to write: itself styled, unless it only opens or
    /// closes a block of code; a table it ends; nothing while a table or a
    /// diagram is held. An empty row comes before a line that starts a
    /// block, one whatever the model left, and none within a block.
    pub(crate) fn line(&mut self, line: &str, p: &Palette) -> Vec<Out> {
        let out = self.read(line, p);
        self.laid_out(out)
    }

    /// The same, not yet laid out in blocks.
    fn read(&mut self, line: &str, p: &Palette) -> Vec<Out> {
        let trimmed = line.trim_start();
        if let Some(lines) = &mut self.diagram {
            if self.fence.as_ref().is_some_and(|fence| {
                trimmed.starts_with(fence.as_str()) && trimmed.trim_end() == fence
            }) {
                let source = lines.join("\n");
                self.diagram = None;
                self.fence = None;
                return vec![Out::Diagram(source)];
            }
            lines.push(line.to_owned());
            return Vec::new();
        }
        if self.fence.is_none() && table::is_row(line) {
            self.table.push(line.to_owned());
            return Vec::new();
        }
        let mut out = self.table_out(p);
        if self.fence.is_none()
            && let Some((marker, language)) = opening(trimmed)
            && language.eq_ignore_ascii_case("mermaid")
        {
            self.fence = Some(marker);
            self.diagram = Some(Vec::new());
            return out;
        }
        out.extend(self.styled(line, p).map(Out::Line));
        out
    }

    /// What is held as a text ends: a table, a diagram whose fence never
    /// closed, written as the code it is, and the mark below a block of
    /// code left open.
    pub(crate) fn flush(&mut self, p: &Palette) -> Vec<Out> {
        let mut out = self.table_out(p);
        if let Some(lines) = self.diagram.take() {
            self.fence = None;
            out.extend(
                code("mermaid", &lines.join("\n"), p)
                    .into_iter()
                    .map(Out::Line),
            );
        } else if self.fence.take().is_some() {
            out.push(Out::Line(below(p)));
        }
        let out = self.laid_out(out);
        // The next text starts afresh, under its own name or steps.
        self.last = None;
        self.parted = false;
        self.listing = false;
        out
    }

    /// `out` laid out in blocks: an empty line of the model's kept only as
    /// a mark that the next block is parted from the last, and an empty
    /// row put before each line that starts a block.
    fn laid_out(&mut self, out: Vec<Out>) -> Vec<Out> {
        let mut laid = Vec::with_capacity(out.len() + 1);
        for out in out {
            let shape = match &out {
                Out::Line(styled) if styled.is_empty() => {
                    self.parted = self.last.is_some();
                    continue;
                }
                Out::Line(styled) => styled.shape,
                Out::Table(_) | Out::Diagram(_) => Shape::Whole,
            };
            if let Some(last) = self.last
                && last.parts(shape, self.parted)
            {
                laid.push(Out::Line(Styled::gap()));
            }
            self.last = Some(shape);
            self.parted = false;
            laid.push(out);
        }
        laid
    }

    /// The table held, when one is.
    fn table_out(&mut self, p: &Palette) -> Vec<Out> {
        if self.table.is_empty() {
            return Vec::new();
        }
        let rows = std::mem::take(&mut self.table);
        let links = self.links;
        vec![Out::Table(Table::new(
            &rows,
            |cell| inline(cell, p.fg(p.text), p, links),
            p.fg(p.faint),
        ))]
    }

    /// `line` styled: a fence as the mark above or below its code.
    fn styled(&mut self, line: &str, p: &Palette) -> Option<Styled> {
        let trimmed = line.trim_start();
        if let Some(fence) = &self.fence {
            if trimmed.starts_with(fence.as_str()) && trimmed.trim_end() == fence.as_str() {
                self.fence = None;
                return Some(below(p));
            }
            return Some(code_line(line, p));
        }
        if let Some((marker, language)) = opening(trimmed) {
            self.fence = Some(marker);
            return Some(above(language, p));
        }
        let indent: String = line.chars().take_while(|c| *c == ' ').collect();
        if trimmed.is_empty() {
            return Some(Styled::plain(Vec::new()).shaped(Shape::Empty));
        }
        let listing = std::mem::take(&mut self.listing);
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            let style = if hashes <= 2 {
                p.bold(p.accent)
            } else {
                p.bold(p.text)
            };
            return Some(
                Styled::plain(inline(trimmed[hashes..].trim(), style, p, self.links))
                    .shaped(Shape::Heading),
            );
        }
        if trimmed.chars().all(|c| c == '-' || c == ' ') && trimmed.matches('-').count() >= 3
            || trimmed.chars().all(|c| c == '*' || c == ' ') && trimmed.matches('*').count() >= 3
            || trimmed.chars().all(|c| c == '_') && trimmed.len() >= 3
        {
            return Some(
                Styled::plain(vec![Span::styled("─".repeat(32), p.fg(p.faint))])
                    .shaped(Shape::Rule),
            );
        }
        for bullet in ["- ", "* ", "+ "] {
            if let Some(item) = trimmed.strip_prefix(bullet) {
                let (mark, item) = match item.strip_prefix("[ ] ") {
                    Some(item) => ("☐ ", item),
                    None => match item
                        .strip_prefix("[x] ")
                        .or_else(|| item.strip_prefix("[X] "))
                    {
                        Some(item) => ("☑ ", item),
                        None => ("• ", item),
                    },
                };
                self.listing = true;
                return Some(Styled {
                    first: vec![
                        Span::raw(indent.clone()),
                        Span::styled(mark, p.fg(p.accent)),
                    ],
                    rest: vec![Span::raw(format!("{indent}  "))],
                    body: inline(item, p.fg(p.text), p, self.links),
                    whole: false,
                    shape: Shape::Item,
                });
            }
        }
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        if digits > 0
            && let Some(item) = trimmed[digits..].strip_prefix(". ")
        {
            let number = &trimmed[..digits + 1];
            self.listing = true;
            return Some(Styled {
                first: vec![
                    Span::raw(indent.clone()),
                    Span::styled(format!("{number} "), p.fg(p.accent)),
                ],
                rest: vec![Span::raw(format!("{indent}{}", " ".repeat(digits + 2)))],
                body: inline(item, p.fg(p.text), p, self.links),
                whole: false,
                shape: Shape::Item,
            });
        }
        if let Some(quote) = trimmed.strip_prefix('>') {
            let style = p.fg(p.dim).add_modifier(Modifier::ITALIC);
            return Some(Styled::under(
                Span::styled("▎ ", p.fg(p.faint)),
                Span::styled("▎ ", p.fg(p.faint)),
                inline(quote.trim_start(), style, p, self.links),
            ));
        }
        // A line indented under a list's item is the item's.
        if listing && indent.len() >= 2 {
            self.listing = true;
            return Some(
                Styled::plain(inline(line, p.fg(p.text), p, self.links)).shaped(Shape::Item),
            );
        }
        Some(Styled::plain(inline(line, p.fg(p.text), p, self.links)))
    }
}

/// `text` with its emphasis, code and links styled, over `base`: a link as
/// its text alone, a hyperlink, when `links`, else with its address after
/// it. A marker with nothing to close it stays as it was written.
pub(crate) fn inline(text: &str, base: Style, p: &Palette, links: bool) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut plain = String::new();
    let mut bold = false;
    let mut italic = false;
    let style = |bold: bool, italic: bool| {
        let mut style = base;
        if bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if italic {
            style = style.add_modifier(Modifier::ITALIC);
        }
        style
    };
    let flush = |plain: &mut String, spans: &mut Vec<Span<'static>>, style: Style| {
        if !plain.is_empty() {
            spans.push(Span::styled(std::mem::take(plain), style));
        }
    };
    let closes = |from: usize, marker: &[char]| -> bool {
        (from..chars.len()).any(|at| chars[at..].starts_with(marker))
    };
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if c == '`' {
            let ticks = chars[at..].iter().take_while(|c| **c == '`').count();
            let marker = vec!['`'; ticks];
            if let Some(end) =
                (at + ticks..chars.len()).find(|end| chars[*end..].starts_with(&marker))
            {
                flush(&mut plain, &mut spans, style(bold, italic));
                let code: String = chars[at + ticks..end].iter().collect();
                spans.push(Span::styled(code.trim().to_owned(), p.fg(p.code)));
                at = end + ticks;
                continue;
            }
        }
        if (c == '*' || c == '_') && at + 1 < chars.len() && chars[at + 1] == c {
            let marker = [c, c];
            if bold || closes(at + 2, &marker) {
                flush(&mut plain, &mut spans, style(bold, italic));
                bold = !bold;
            } else {
                plain.extend(marker);
            }
            at += 2;
            continue;
        }
        if c == '*' || c == '_' {
            let before = at.checked_sub(1).map(|b| chars[b]);
            let after = chars.get(at + 1).copied();
            // snake_case and 2*3 are not emphasis.
            let inside_word = before.is_some_and(char::is_alphanumeric)
                && after.is_some_and(char::is_alphanumeric);
            let opens =
                !italic && after.is_some_and(|a| !a.is_whitespace()) && closes(at + 1, &[c]);
            if !inside_word && (opens || italic) {
                flush(&mut plain, &mut spans, style(bold, italic));
                italic = !italic;
                at += 1;
                continue;
            }
        }
        if c == '['
            && let Some(close) = (at + 1..chars.len()).find(|i| chars[*i] == ']')
            && chars.get(close + 1) == Some(&'(')
            && let Some(end) = (close + 2..chars.len()).find(|i| chars[*i] == ')')
        {
            flush(&mut plain, &mut spans, style(bold, italic));
            let label: String = chars[at + 1..close].iter().collect();
            let address: String = chars[close + 2..end].iter().collect();
            let style = p.fg(p.info).add_modifier(Modifier::UNDERLINED);
            match links.then(|| crate::link::mark(style, &address)).flatten() {
                Some(linked) => spans.push(Span::styled(label, linked)),
                None => {
                    spans.push(Span::styled(label.clone(), style));
                    if address != label {
                        spans.push(Span::styled(format!(" ({address})"), p.fg(p.faint)));
                    }
                }
            }
            at = end + 1;
            continue;
        }
        plain.push(c);
        at += 1;
    }
    flush(&mut plain, &mut spans, style(bold, italic));
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Depth;
    use iq_ui::look::Theme;

    fn text(styled: &Styled) -> String {
        styled
            .first
            .iter()
            .chain(&styled.body)
            .map(|s| s.content.as_ref())
            .collect()
    }

    /// `out` without the empty rows between its blocks.
    fn unparted(out: Vec<Out>) -> Vec<Out> {
        out.into_iter()
            .filter(|out| !matches!(out, Out::Line(styled) if styled.shape == Shape::Empty))
            .collect()
    }

    /// The one line `out` holds, as text, the empty rows between blocks
    /// aside; none when it holds none.
    fn one(out: Vec<Out>) -> Option<String> {
        match unparted(out).as_slice() {
            [Out::Line(styled)] => Some(text(styled)),
            [] => None,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn lines_are_styled_as_they_come_code_kept_whole() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut md = Markdown::default();
        let mut shown = |line: &str| one(md.line(line, &p));
        assert_eq!(shown("## Plan").as_deref(), Some("Plan"));
        assert_eq!(
            shown("- run `cargo test`").as_deref(),
            Some("• run cargo test")
        );
        assert_eq!(shown("12. then").as_deref(), Some("12. then"));
        assert_eq!(shown("```rust").as_deref(), Some("╭ rust"));
        // Code as it is written, nothing before it, whole.
        let [Out::Line(code)] = &unparted(md.line("    let x = *a * b;", &p))[..] else {
            panic!("one line of code")
        };
        assert_eq!(text(code), "    let x = *a * b;");
        assert!(code.whole && code.first.is_empty());
        let mut shown = |line: &str| one(md.line(line, &p));
        assert_eq!(shown("```").as_deref(), Some("╰"));
        assert_eq!(shown("a *b* c").as_deref(), Some("a b c"));
        assert_eq!(shown("snake_case_name").as_deref(), Some("snake_case_name"));
        assert_eq!(shown("2 * 3 = 6").as_deref(), Some("2 * 3 = 6"));
        assert_eq!(
            shown("see [the docs](https://x.y)").as_deref(),
            Some("see the docs (https://x.y)")
        );
        assert_eq!(shown("> said").as_deref(), Some("▎ said"));
    }

    #[test]
    fn emphasis_is_styled_and_an_unclosed_marker_left_as_written() {
        let p = Palette::new(Theme::Ink, Depth::True);
        let spans = inline("a **bold** and **open", p.fg(p.text), &p, false);
        let shown: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(shown, "a bold and **open");
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn a_table_is_held_until_its_last_row_and_a_diagram_until_its_fence_closes() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut md = Markdown::default();
        assert!(md.line("| a | b |", &p).is_empty());
        assert!(md.line("|---|---|", &p).is_empty());
        assert!(md.line("| 1 | 2 |", &p).is_empty());
        let out = unparted(md.line("after", &p));
        assert!(matches!(&out[..], [Out::Table(_), Out::Line(after)] if text(after) == "after"));
        assert!(md.line("```Mermaid", &p).is_empty());
        assert!(md.line("flowchart LR", &p).is_empty());
        assert!(md.line("  A --> B", &p).is_empty());
        assert_eq!(
            unparted(md.line("```", &p)),
            [Out::Diagram("flowchart LR\n  A --> B".into())]
        );
        // A row as the text ends, and a diagram never closed, written as
        // they stand: a table, and code.
        assert!(md.line("| x |", &p).is_empty());
        assert!(matches!(&md.flush(&p)[..], [Out::Table(_)]));
        assert!(md.line("~~~mermaid", &p).is_empty());
        assert!(md.line("graph TD", &p).is_empty());
        let shown: Vec<String> = md
            .flush(&p)
            .iter()
            .map(|out| match out {
                Out::Line(styled) => text(styled),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(shown, ["╭ mermaid", "graph TD", "╰"]);
        // A row within code is code, and a block left open is closed as
        // the text ends.
        assert_eq!(one(md.line("```", &p)).as_deref(), Some("╭"));
        assert_eq!(
            one(md.line("| in code |", &p)).as_deref(),
            Some("| in code |")
        );
        assert_eq!(one(md.flush(&p)).as_deref(), Some("╰"));
        assert_eq!(one(md.line("| out |", &p)), None);
    }

    #[test]
    fn a_link_is_its_text_alone_where_links_are_taken() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut md = Markdown::linking(true);
        let out = md.line("see [A-1](https://tracker.example/A-1)", &p);
        let [Out::Line(styled)] = &out[..] else {
            panic!("{out:?}")
        };
        assert_eq!(text(styled), "see A-1");
        let link = &styled.body[1];
        let (_, address, _) = crate::link::address_of(link.style).unwrap();
        assert_eq!(address, "https://tracker.example/A-1");
        // An address no link takes is written after its text.
        assert_eq!(
            one(md.line("[x](javascript:alert(1))", &p)).as_deref(),
            Some("x (javascript:alert(1))")
        );
    }

    #[test]
    fn blocks_are_parted_by_one_empty_row_and_a_list_by_none() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut md = Markdown::default();
        let source = "Two things:\n- one\n\n- two\n  still two\n\n\n\n### Next\nText\nmore text\n\n\nAnother\n```sh\nls\n\n\nls -a\n```\nAfter\n> a\n> b";
        let mut out: Vec<Out> = source.lines().flat_map(|line| md.line(line, &p)).collect();
        out.extend(md.flush(&p));
        let rows: Vec<String> = out
            .into_iter()
            .map(|out| match out {
                Out::Line(styled) => text(&styled),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            rows,
            [
                "Two things:",
                "",
                "• one",
                "• two",
                "  still two",
                "",
                "Next",
                "",
                "Text",
                "more text",
                "",
                "Another",
                "",
                "╭ sh",
                "ls",
                "",
                "",
                "ls -a",
                "╰",
                "",
                "After",
                "",
                "▎ a",
                "▎ b",
            ]
        );
    }
}
