//! A Markdown table, held until its last row, then drawn at the width it is
//! written at: its columns as wide as their widest cell, aligned as its rule
//! says, the widest made narrower when the terminal is, each cell's text
//! folding within its column.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::wrap::{spans_width, wrap};

/// How the cells of a column stand within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Center,
    Right,
}

/// What a column is made no narrower than, unless its cells are.
const NARROWEST: usize = 6;

/// Between two columns.
const BAR: &str = " │ ";

/// A table: its head when it has one, its rows, how each column aligns,
/// and the style of its bars.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Table {
    head: Option<Vec<Vec<Span<'static>>>>,
    rows: Vec<Vec<Vec<Span<'static>>>>,
    align: Vec<Align>,
    bars: Style,
}

/// Whether `line` is a row of a table.
pub(crate) fn is_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// The cells of a row: what stands between its bars, a bar inside code or
/// written `\|` left in its cell.
fn cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut code = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cell.push('|');
                chars.next();
            }
            '|' if !code => cells.push(std::mem::take(&mut cell)),
            _ => {
                if c == '`' {
                    code = !code;
                }
                cell.push(c);
            }
        }
    }
    // What follows the last bar is no cell.
    if cells.is_empty() || !cell.trim().is_empty() {
        cells.push(cell);
    }
    cells
        .into_iter()
        .map(|cell| cell.trim().to_owned())
        .collect()
}

/// How a column aligns, from its cell of the rule: `:--` left, `:-:`
/// centred, `--:` right; none when the cell is no rule's.
fn rule(cell: &str) -> Option<Align> {
    let dashes = cell.trim_matches(':');
    if dashes.is_empty() || !dashes.chars().all(|c| c == '-') {
        return None;
    }
    Some(match (cell.starts_with(':'), cell.ends_with(':')) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    })
}

impl Table {
    /// The table of `lines`, each cell styled by `style`, its head in bold,
    /// its bars in `bars`.
    pub(crate) fn new(
        lines: &[String],
        style: impl Fn(&str) -> Vec<Span<'static>>,
        bars: Style,
    ) -> Table {
        let rows: Vec<Vec<String>> = lines.iter().map(|line| cells(line)).collect();
        let aligned: Option<Vec<Align>> = rows
            .get(1)
            .and_then(|second| second.iter().map(|cell| rule(cell)).collect());
        let styled = |row: &[String]| -> Vec<Vec<Span<'static>>> {
            row.iter().map(|cell| style(cell)).collect()
        };
        let (head, body, align) = match aligned {
            Some(align) if rows.len() >= 2 => {
                let head: Vec<Vec<Span<'static>>> = styled(&rows[0])
                    .into_iter()
                    .map(|cell| {
                        cell.into_iter()
                            .map(|span| {
                                let style = span.style.add_modifier(Modifier::BOLD);
                                span.style(style)
                            })
                            .collect()
                    })
                    .collect();
                (
                    Some(head),
                    rows[2..].iter().map(|r| styled(r)).collect(),
                    align,
                )
            }
            _ => (None, rows.iter().map(|r| styled(r)).collect(), Vec::new()),
        };
        Table {
            head,
            rows: body,
            align,
            bars,
        }
    }

    /// How many columns it has.
    fn columns(&self) -> usize {
        self.head
            .iter()
            .chain(&self.rows)
            .map(Vec::len)
            .max()
            .unwrap_or(0)
    }

    /// Its lines within `width` columns, each after `indent`.
    pub(crate) fn lines(&self, width: usize, indent: &[Span<'static>]) -> Vec<Line<'static>> {
        let columns = self.columns();
        if columns == 0 {
            return Vec::new();
        }
        let mut widths = vec![1; columns];
        for row in self.head.iter().chain(&self.rows) {
            for (at, cell) in row.iter().enumerate() {
                widths[at] = widths[at].max(spans_width(cell));
            }
        }
        let room = width
            .saturating_sub(spans_width(indent))
            .saturating_sub(crate::wrap::width(BAR) * (columns - 1));
        // The widest made narrower, one column at a time, until the table
        // fits or none can be.
        while widths.iter().sum::<usize>() > room {
            let Some((at, _)) = widths
                .iter()
                .enumerate()
                .filter(|(_, w)| **w > NARROWEST)
                .max_by_key(|(_, w)| **w)
            else {
                break;
            };
            widths[at] -= 1;
        }
        let mut lines = Vec::new();
        if let Some(head) = &self.head {
            lines.extend(self.row(head, &widths, indent));
            let mut rule = indent.to_vec();
            let drawn: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
            rule.push(Span::styled(drawn.join("─┼─"), self.bars));
            lines.push(Line::from(rule));
        }
        for row in &self.rows {
            lines.extend(self.row(row, &widths, indent));
        }
        lines
    }

    /// The lines of one row, its cells folded within their columns.
    fn row(
        &self,
        cells: &[Vec<Span<'static>>],
        widths: &[usize],
        indent: &[Span<'static>],
    ) -> Vec<Line<'static>> {
        let folded: Vec<Vec<Line<'static>>> = widths
            .iter()
            .enumerate()
            .map(|(at, width)| match cells.get(at) {
                Some(cell) if !cell.is_empty() => wrap(cell, *width, &[], &[]),
                _ => vec![Line::default()],
            })
            .collect();
        let height = folded.iter().map(Vec::len).max().unwrap_or(1);
        (0..height)
            .map(|y| {
                let mut spans = indent.to_vec();
                for (at, width) in widths.iter().enumerate() {
                    if at > 0 {
                        spans.push(Span::styled(BAR, self.bars));
                    }
                    let line = folded[at].get(y).cloned().unwrap_or_default();
                    let gap = width.saturating_sub(line.width());
                    let (before, after) = match self.align.get(at) {
                        Some(Align::Right) => (gap, 0),
                        Some(Align::Center) => (gap / 2, gap - gap / 2),
                        _ => (0, gap),
                    };
                    // The last column needs no space after it.
                    let after = if at + 1 == widths.len() { 0 } else { after };
                    if before > 0 {
                        spans.push(Span::raw(" ".repeat(before)));
                    }
                    spans.extend(line.spans);
                    if after > 0 {
                        spans.push(Span::raw(" ".repeat(after)));
                    }
                }
                Line::from(spans)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(lines: &[&str]) -> Table {
        let lines: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
        Table::new(
            &lines,
            |cell| vec![Span::raw(cell.to_owned())],
            Style::new(),
        )
    }

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn columns_line_up_with_their_widest_cell_aligned_as_the_rule_says() {
        let drawn = table(&[
            "| Issue | State | Points |",
            "|-------|:-----:|-------:|",
            "| A-1 | open | 3 |",
            "| A-12 `a|b` | in review | 13 |",
        ])
        .lines(80, &[Span::raw("  ")]);
        assert_eq!(
            text(&drawn),
            [
                "  Issue      │   State   │ Points",
                "  ───────────┼───────────┼───────",
                "  A-1        │   open    │      3",
                "  A-12 `a|b` │ in review │     13",
            ]
        );
        assert!(
            drawn[0].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn a_table_too_wide_folds_its_widest_cells_within_their_columns() {
        let drawn = table(&[
            "| Name | What it does |",
            "|---|---|",
            "| fetch | reads one page of the documentation and gives its text back |",
        ])
        .lines(30, &[]);
        let shown = text(&drawn);
        assert!(
            shown.iter().all(|line| line.chars().count() <= 30),
            "{shown:#?}"
        );
        assert_eq!(shown[2], "fetch │ reads one page of the");
        assert_eq!(shown[3], "      │ documentation and");
        assert_eq!(shown[4], "      │ gives its text back");
    }

    #[test]
    fn rows_without_a_rule_are_a_table_without_a_head() {
        let drawn = table(&["| a | b |", "| ccc | d |"]).lines(40, &[]);
        assert_eq!(text(&drawn), ["a   │ b", "ccc │ d"]);
        assert!(is_row("  | x |") && !is_row("x | y"));
        assert_eq!(cells(r"| a \| b | c |"), ["a | b", "c"]);
    }
}
