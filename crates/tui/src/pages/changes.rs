//! What a sync would send, line by line: the diff git makes of it, each
//! file named as the repository names it, lines added and removed in their
//! colours, for the person to read before they say whether it goes. A line
//! wider than the screen folds onto the next rows, marked, in its colour,
//! as Markdown writes a whole paragraph on one line; a line changed shows
//! the words that changed in it, as `git diff --word-diff` does, since one
//! word changed marks the whole paragraph.

use iq_ui::keys::Action;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::Seen;
use crate::theme::Palette;
use crate::wrap::wrap;

/// The most words of a line compared with the line it replaced: past them,
/// the line is shown whole in its colour.
const WORDS_MOST: usize = 1500;

/// The page's lines.
pub(super) fn lines(seen: &Seen<'_>) -> Vec<Line<'static>> {
    let p = seen.p;
    let mut lines = Vec::new();
    if let Some(ask) = seen.board.sync_ask() {
        seen.heading(&mut lines, &format!("To {}", ask.remote), "");
        let files = match ask.files.len() {
            1 => ask.said.clone(),
            n => format!("{n} files: {}", ask.said),
        };
        seen.said(&mut lines, &files, p.dim);
    }
    lines.push(Line::default());
    match seen.board.changes() {
        None => seen.said(&mut lines, "reading what changed…", p.dim),
        Some("") => seen.said(
            &mut lines,
            "nothing changed: a sync would send nothing",
            p.dim,
        ),
        Some(diff) => lines.extend(drawn(diff, seen.width.saturating_sub(1), p)),
    }
    lines
}

/// The style of a line of the diff that is no change of a line.
fn style_of(line: &str, p: &Palette) -> Style {
    if line.starts_with("diff --git ") {
        p.bold(p.text)
    } else if line.starts_with("+++ ") || line.starts_with("--- ") {
        p.bold(p.dim)
    } else if line.starts_with("@@") {
        p.fg(p.info)
    } else if line.starts_with('+') {
        p.fg(p.ok)
    } else if line.starts_with('-') {
        p.fg(p.bad)
    } else if line.starts_with(' ') {
        p.fg(p.text)
    } else {
        // `index`, a mode, `Binary files`, the lines left out.
        p.fg(p.faint)
    }
}

/// Whether `line` is a line removed, or added: not a file's name.
fn removed(line: &str) -> bool {
    line.starts_with('-') && !line.starts_with("--- ")
}

fn added(line: &str) -> bool {
    line.starts_with('+') && !line.starts_with("+++ ")
}

/// The diff's lines within `width` columns: each line folded, the lines
/// removed then added in a run paired one by one, the words that changed
/// between the two of a pair marked.
fn drawn(diff: &str, width: usize, p: &Palette) -> Vec<Line<'static>> {
    let all: Vec<String> = diff.lines().map(|l| l.replace('\t', "    ")).collect();
    let mut styled: Vec<Vec<Span<'static>>> = Vec::new();
    let mut at = 0;
    while at < all.len() {
        let gone = all[at..].iter().take_while(|l| removed(l)).count();
        let came = all[at + gone..].iter().take_while(|l| added(l)).count();
        if gone > 0 && came > 0 {
            let pairs = gone.min(came);
            let (old, new) = (&all[at..at + gone], &all[at + gone..at + gone + came]);
            let mut olds = Vec::new();
            let mut news = Vec::new();
            for (o, n) in old.iter().zip(new).take(pairs) {
                let (o, n) = words_changed(o, n, p);
                olds.push(o);
                news.push(n);
            }
            olds.extend(
                old[pairs..]
                    .iter()
                    .map(|l| vec![Span::styled(l.clone(), p.fg(p.bad))]),
            );
            news.extend(
                new[pairs..]
                    .iter()
                    .map(|l| vec![Span::styled(l.clone(), p.fg(p.ok))]),
            );
            styled.extend(olds);
            styled.extend(news);
            at += gone + came;
            continue;
        }
        let line = &all[at];
        styled.push(vec![Span::styled(line.clone(), style_of(line, p))]);
        at += 1;
    }
    let mark = [Span::styled(" ↳ ", p.fg(p.faint))];
    styled
        .into_iter()
        .flat_map(|mut spans| {
            // The spaces a line starts with, a context line's mark and the
            // code's indent, go before it whole: folding would drop them.
            let mut first = vec![Span::raw(" ")];
            if let Some(head) = spans.first_mut() {
                let indent = head.content.len() - head.content.trim_start_matches(' ').len();
                if indent > 0 {
                    first.push(Span::raw(" ".repeat(indent)));
                    head.content = head.content[indent..].to_owned().into();
                }
            }
            let folded = wrap(&spans, width, &first, &mark);
            if folded.is_empty() {
                vec![Line::default()]
            } else {
                folded
            }
        })
        .collect()
}

/// The words of `text`: runs of letters and digits, each other character
/// alone, spaces kept.
fn words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if c.is_alphanumeric() || c == '_' {
            while chars
                .peek()
                .is_some_and(|(_, next)| next.is_alphanumeric() || *next == '_')
            {
                chars.next();
            }
        } else if c == ' ' {
            while chars.peek().is_some_and(|(_, next)| *next == ' ') {
                chars.next();
            }
        }
        let end = chars.peek().map_or(text.len(), |(next, _)| *next);
        out.push(&text[start.max(at)..end]);
        start = end;
    }
    out
}

/// The line removed `old` and the line added `new` that replaced it, the
/// words they share dimmed and those that changed in their colour, bold and
/// underlined.
fn words_changed(old: &str, new: &str, p: &Palette) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    // Past their mark, which says which they are.
    let (a, b) = (words(&old[1..]), words(&new[1..]));
    if a.len() > WORDS_MOST || b.len() > WORDS_MOST {
        return (
            vec![Span::styled(old.to_owned(), p.fg(p.bad))],
            vec![Span::styled(new.to_owned(), p.fg(p.ok))],
        );
    }
    // The longest run of words they share, kept in order.
    let mut longest = vec![vec![0u16; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            longest[i][j] = if a[i] == b[j] {
                longest[i + 1][j + 1] + 1
            } else {
                longest[i + 1][j].max(longest[i][j + 1])
            };
        }
    }
    let changed = |colour| {
        p.fg(colour)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    };
    let mut olds = vec![Span::styled("-", p.fg(p.bad))];
    let mut news = vec![Span::styled("+", p.fg(p.ok))];
    let (mut i, mut j) = (0, 0);
    let push = |spans: &mut Vec<Span<'static>>, word: &str, style: Style| match spans.last_mut() {
        Some(last) if last.style == style => last.content.to_mut().push_str(word),
        _ => spans.push(Span::styled(word.to_owned(), style)),
    };
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            push(&mut olds, a[i], p.fg(p.dim));
            push(&mut news, b[j], p.fg(p.dim));
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || longest[i][j + 1] >= longest[i + 1][j]) {
            push(&mut news, b[j], changed(p.ok));
            j += 1;
        } else {
            push(&mut olds, a[i], changed(p.bad));
            i += 1;
        }
    }
    (olds, news)
}

/// The keys the page takes: those that answer, while a sync asks.
pub(super) fn foot(seen: &Seen<'_>) -> String {
    if seen.board.sync_ask().is_some() {
        format!(
            "{} send · {} not now · ↑↓ scroll",
            seen.key(Action::Allow),
            seen.key(Action::Refuse)
        )
    } else {
        "↑↓ scroll · /sync sends it".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Depth;
    use iq_ui::look::Theme;

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_long_line_folds_in_its_colour_under_a_mark() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let diff = "@@ -1 +1 @@\n+one two three four five six seven eight nine ten";
        let lines = drawn(diff, 24, &p);
        assert_eq!(
            text(&lines),
            [
                " @@ -1 +1 @@",
                " +one two three four",
                " ↳ five six seven eight",
                " ↳ nine ten"
            ]
        );
        assert_eq!(lines[2].spans[1].style, p.fg(p.ok));
    }

    #[test]
    fn a_line_changed_marks_the_words_that_changed() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let diff = "-The reviewer answers what stays within the sandbox.\n+The reviewer answers what stays inside the sandbox.\n context";
        let lines = drawn(diff, 200, &p);
        let marked = |line: &Line<'_>| -> Vec<String> {
            line.spans
                .iter()
                .filter(|s| s.style.add_modifier.contains(Modifier::UNDERLINED))
                .map(|s| s.content.to_string())
                .collect()
        };
        assert_eq!(marked(&lines[0]), ["within"]);
        assert_eq!(marked(&lines[1]), ["inside"]);
        assert_eq!(
            text(&lines),
            [
                " -The reviewer answers what stays within the sandbox.",
                " +The reviewer answers what stays inside the sandbox.",
                "  context"
            ]
        );
        assert_eq!(words("a  b_c.d"), ["a", "  ", "b_c", ".", "d"]);
    }
}
