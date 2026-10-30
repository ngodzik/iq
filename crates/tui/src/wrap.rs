//! Styled text folded to the terminal's width at the spaces between words,
//! each line after the first under a prefix of its own, so that a block
//! keeps its indent as it wraps. iq folds what it writes in the
//! conversation itself: a line the terminal folded would leave its colours
//! and indent behind.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// How wide `text` shows.
pub(crate) fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// How wide `spans` show together.
pub(crate) fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| width(&s.content)).sum()
}

/// `text` cut to `most` columns, with `…` where it was cut.
pub(crate) fn cut(text: &str, most: usize) -> String {
    if width(text) <= most {
        return text.to_owned();
    }
    let mut kept = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > most {
            break;
        }
        kept.push(c);
        used += w;
    }
    kept.push('…');
    kept
}

/// One piece of the text: a word or the spaces after it, in its style.
struct Piece {
    text: String,
    style: Style,
    space: bool,
}

/// The pieces of `spans`, a new line its own piece.
fn pieces(spans: &[Span<'_>]) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for span in spans {
        for c in span.content.chars() {
            let space = c == ' ' || c == '\t';
            let c = if c == '\t' { ' ' } else { c };
            match out.last_mut() {
                Some(last)
                    if c != '\n'
                        && !last.text.ends_with('\n')
                        && last.space == space
                        && last.style == span.style =>
                {
                    last.text.push(c);
                }
                // A word goes on across a change of style.
                Some(last) if c != '\n' && !space && !last.space && !last.text.ends_with('\n') => {
                    out.push(Piece {
                        text: c.to_string(),
                        style: span.style,
                        space: false,
                    });
                }
                _ => out.push(Piece {
                    text: c.to_string(),
                    style: span.style,
                    space,
                }),
            }
        }
    }
    out
}

/// `body` folded within `width` columns: its first line after `first`,
/// the others after `rest`. Spaces where a line folds are dropped.
pub(crate) fn wrap(
    body: &[Span<'_>],
    width: usize,
    first: &[Span<'static>],
    rest: &[Span<'static>],
) -> Vec<Line<'static>> {
    let rest_width = spans_width(rest);
    let mut lines: Vec<Vec<Span<'static>>> = vec![first.to_vec()];
    let mut used = spans_width(first);
    // Whether nothing but the prefix is on the line yet.
    let mut fresh = true;
    let mut spaces: Option<Piece> = None;
    let mut all = pieces(body).into_iter().peekable();
    while let Some(piece) = all.next() {
        if piece.text == "\n" {
            lines.push(rest.to_vec());
            used = rest_width;
            fresh = true;
            spaces = None;
            continue;
        }
        if piece.space {
            if !fresh {
                spaces = Some(piece);
            }
            continue;
        }
        // A word whose style changes within it moves as one.
        let mut word = vec![piece];
        while all
            .peek()
            .is_some_and(|next| !next.space && next.text != "\n")
        {
            word.extend(all.next());
        }
        let word_width: usize = word.iter().map(|p| self::width(&p.text)).sum();
        let gap = spaces.as_ref().map_or(0, |s| self::width(&s.text));
        if !fresh && used + gap + word_width > width {
            lines.push(rest.to_vec());
            used = rest_width;
            spaces = None;
        } else if let Some(gap) = spaces.take() {
            used += self::width(&gap.text);
            push(lines.last_mut(), gap.text, gap.style);
        }
        for piece in word {
            for c in piece.text.chars() {
                let w = c.width().unwrap_or(0);
                // A word longer than a line is broken, once something
                // stands on the line.
                if used + w > width && used > rest_width.min(width.saturating_sub(1)) {
                    lines.push(rest.to_vec());
                    used = rest_width;
                }
                used += w;
                push(lines.last_mut(), c.to_string(), piece.style);
            }
        }
        fresh = false;
    }
    lines.into_iter().map(Line::from).collect()
}

/// Adds `text` to `line`, joined to its last span when they share a style.
fn push(line: Option<&mut Vec<Span<'static>>>, text: String, style: Style) {
    let Some(line) = line else {
        return;
    };
    match line.last_mut() {
        Some(last) if last.style == style => last.content.to_mut().push_str(&text),
        _ => line.push(Span::styled(text, style)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn words_fold_under_their_prefix_and_long_ones_are_broken() {
        let first = [Span::raw("› ")];
        let rest = [Span::raw("  ")];
        let lines = wrap(&[Span::raw("one two three")], 9, &first, &rest);
        assert_eq!(text(&lines), ["› one two", "  three"]);
        let lines = wrap(&[Span::raw("abcdefghijkl")], 6, &first, &rest);
        assert_eq!(text(&lines), ["› abcd", "  efgh", "  ijkl"]);
        let lines = wrap(&[Span::raw("a\nb  c")], 20, &first, &rest);
        assert_eq!(text(&lines), ["› a", "  b  c"]);
    }

    #[test]
    fn styles_stay_with_their_words_across_a_fold() {
        let bold = Style::new().bold();
        let body = [
            Span::raw("plain "),
            Span::styled("bold", bold),
            Span::raw("ed words"),
        ];
        let lines = wrap(&body, 8, &[], &[]);
        assert_eq!(text(&lines), ["plain", "bolded", "words"]);
        assert_eq!(lines[1].spans[0].style, bold);
        assert_eq!(cut("abcdef", 4), "abc…");
        assert_eq!(cut("abc", 4), "abc");
    }
}
