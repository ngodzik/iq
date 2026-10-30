//! Charts drawn with characters, so that any terminal shows them: a curve
//! in Braille dots, two by four to a cell, the area under it lit fainter;
//! bars stacked in half blocks, two colours to a cell; gauges filled by
//! eighths of a cell. Each is a few lines of text, scrolled with the page
//! that holds it.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::theme::{Palette, Rgb};
use crate::wrap::width as text_width;

/// The eighths a cell is filled by, from the left.
const EIGHTHS: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

/// A gauge `width` cells long, filled to `share` in `fill`, with a mark at
/// `mark` when given: where an even pace would be.
pub(crate) fn gauge(
    share: f64,
    mark: Option<f64>,
    width: usize,
    fill: Rgb,
    p: &Palette,
) -> Vec<Span<'static>> {
    let eighths = (share.clamp(0.0, 1.0) * width as f64 * 8.0).round() as usize;
    let full = eighths / 8;
    let mark_at =
        mark.map(|m| ((m.clamp(0.0, 1.0) * width as f64) as usize).min(width.saturating_sub(1)));
    let mut spans = Vec::new();
    for cell in 0..width {
        let (symbol, style) = if Some(cell) == mark_at {
            ("│".to_owned(), p.fg(p.text))
        } else if cell < full {
            ("█".to_owned(), p.fg(fill))
        } else if cell == full && !eighths.is_multiple_of(8) {
            (EIGHTHS[eighths % 8].to_owned(), p.fg(fill))
        } else {
            ("─".to_owned(), p.fg(p.faint))
        };
        match spans.last_mut() {
            Some(Span {
                content,
                style: last,
                ..
            }) if *last == style => content.to_mut().push_str(&symbol),
            _ => spans.push(Span::styled(symbol, style)),
        }
    }
    spans
}

/// The values as a line of bars an eighth high each, the highest full.
pub(crate) fn sparkline(values: &[f64], width: usize, tint: Rgb, p: &Palette) -> Span<'static> {
    const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let shown = &values[values.len().saturating_sub(width)..];
    let top = shown.iter().copied().fold(0.0, f64::max);
    let line: String = shown
        .iter()
        .map(|v| {
            if top <= 0.0 {
                LEVELS[0]
            } else {
                LEVELS[((v / top) * 7.0).round().clamp(0.0, 7.0) as usize]
            }
        })
        .collect();
    Span::styled(line, p.fg(tint))
}

/// A curve through `points`, over `x` and `y`, `width` cells by `height`
/// rows, its values said on the left by `label` and its ends below by
/// `ends`.
pub(crate) struct Curve<'a> {
    pub(crate) points: &'a [(f64, f64)],
    pub(crate) x: (f64, f64),
    pub(crate) y: (f64, f64),
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) tint: Rgb,
    pub(crate) label: &'a dyn Fn(f64) -> String,
    pub(crate) ends: (String, String),
}

impl Curve<'_> {
    /// The curve's lines.
    pub(crate) fn lines(&self, p: &Palette) -> Vec<Line<'static>> {
        let labels = [(self.label)(self.y.1), (self.label)(self.y.0)];
        let gutter = labels.iter().map(|l| text_width(l)).max().unwrap_or(0) + 1;
        let cells = self.width.saturating_sub(gutter + 1).max(4);
        let (dots_x, dots_y) = (cells * 2, self.height * 4);
        let mut line = vec![false; dots_x * dots_y];
        let mut under = vec![false; dots_x * dots_y];
        let span_x = (self.x.1 - self.x.0).max(f64::EPSILON);
        let span_y = (self.y.1 - self.y.0).max(f64::EPSILON);
        let at = |(x, y): (f64, f64)| -> (i64, i64) {
            let dx = ((x - self.x.0) / span_x * (dots_x - 1) as f64).round() as i64;
            let dy = ((self.y.1 - y) / span_y * (dots_y - 1) as f64).round() as i64;
            (
                dx.clamp(0, dots_x as i64 - 1),
                dy.clamp(0, dots_y as i64 - 1),
            )
        };
        let mut set = |x: i64, y: i64| {
            if x >= 0 && y >= 0 && (x as usize) < dots_x && (y as usize) < dots_y {
                line[y as usize * dots_x + x as usize] = true;
            }
        };
        for pair in self.points.windows(2) {
            let (a, b) = (at(pair[0]), at(pair[1]));
            let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
            for step in 0..=steps {
                let x = a.0 + (b.0 - a.0) * step / steps;
                let y = a.1 + (b.1 - a.1) * step / steps;
                set(x, y);
            }
        }
        if let [only] = self.points {
            let (x, y) = at(*only);
            set(x, y);
        }
        // The area under the curve: below its highest dot in each column.
        for x in 0..dots_x {
            if let Some(top) = (0..dots_y).find(|y| line[y * dots_x + x]) {
                for y in top + 1..dots_y {
                    under[y * dots_x + x] = true;
                }
            }
        }
        let faint = p.tint_under(self.tint);
        let mut lines = Vec::new();
        for row in 0..self.height {
            let label = match row {
                0 => labels[0].clone(),
                r if r + 1 == self.height => labels[1].clone(),
                _ => String::new(),
            };
            let mut spans = vec![
                Span::styled(
                    format!("{label:>gutter$}", gutter = gutter - 1),
                    p.fg(p.faint),
                ),
                Span::styled(" ", p.fg(p.faint)),
            ];
            for cell in 0..cells {
                let mut bits = 0u32;
                let mut on_line = false;
                for (dx, dy, bit) in BRAILLE {
                    let (x, y) = (cell * 2 + dx, row * 4 + dy);
                    let i = y * dots_x + x;
                    if line[i] {
                        bits |= bit;
                        on_line = true;
                    } else if under[i] {
                        bits |= bit;
                    }
                }
                let symbol = char::from_u32(0x2800 + bits).unwrap_or(' ').to_string();
                let style = if on_line {
                    p.fg(self.tint)
                } else {
                    p.fg(faint)
                };
                push(&mut spans, symbol, style);
            }
            lines.push(Line::from(spans));
        }
        let (start, end) = &self.ends;
        let room = cells.saturating_sub(text_width(start) + text_width(end));
        lines.push(Line::from(vec![Span::styled(
            format!("{:gutter$}{start}{:room$}{end}", "", ""),
            p.fg(p.faint),
        )]));
        lines
    }
}

/// Each dot of a Braille cell: its column, its row, and its bit.
const BRAILLE: [(usize, usize, u32); 8] = [
    (0, 0, 0x01),
    (0, 1, 0x02),
    (0, 2, 0x04),
    (1, 0, 0x08),
    (1, 1, 0x10),
    (1, 2, 0x20),
    (0, 3, 0x40),
    (1, 3, 0x80),
];

/// Bars side by side, each a stack of parts in their colours, as high as
/// its sum against the highest: `height` rows of two halves each, and
/// each bar's name below it, cut to its width.
pub(crate) fn stacked(
    bars: &[(String, Vec<(f64, Rgb)>)],
    width: usize,
    height: usize,
    p: &Palette,
) -> Vec<Line<'static>> {
    if bars.is_empty() {
        return Vec::new();
    }
    let top = bars
        .iter()
        .map(|(_, parts)| parts.iter().map(|(v, _)| v).sum::<f64>())
        .fold(0.0, f64::max);
    let slot = (width / bars.len()).clamp(2, 8);
    let bar = slot.saturating_sub(1).max(1);
    let halves = height * 2;
    // Each bar's colour in each half, from the bottom.
    let columns: Vec<Vec<Option<Rgb>>> = bars
        .iter()
        .map(|(_, parts)| {
            let mut filled = vec![None; halves];
            let mut from = 0.0;
            for (value, tint) in parts {
                let start = if top > 0.0 {
                    from / top * halves as f64
                } else {
                    0.0
                };
                from += value;
                let end = if top > 0.0 {
                    from / top * halves as f64
                } else {
                    0.0
                };
                let (from_half, to_half) =
                    (start.round() as usize, (end.round() as usize).min(halves));
                for half in filled.iter_mut().take(to_half).skip(from_half) {
                    *half = Some(*tint);
                }
            }
            // A part too small for a half still shows on an empty bar.
            if filled.iter().all(Option::is_none)
                && let Some((v, tint)) = parts.iter().find(|(v, _)| *v > 0.0)
                && *v > 0.0
            {
                filled[0] = Some(*tint);
            }
            filled
        })
        .collect();
    let mut lines = Vec::new();
    for row in (0..height).rev() {
        let mut spans = Vec::new();
        for column in &columns {
            let (lower, upper) = (column[row * 2], column[row * 2 + 1]);
            let (symbol, style) = match (lower, upper) {
                (None, None) => (" ", Style::new()),
                (Some(l), None) => ("▄", p.fg(l)),
                (None, Some(u)) => ("▀", p.fg(u)),
                (Some(l), Some(u)) if l == u => ("█", p.fg(l)),
                (Some(l), Some(u)) => ("▀", p.fg(u).bg(p.color(l))),
            };
            push(&mut spans, symbol.repeat(bar), style);
            push(&mut spans, " ".repeat(slot - bar), Style::new());
        }
        lines.push(Line::from(spans));
    }
    let names: String = bars
        .iter()
        .map(|(name, _)| {
            let name: String = name.chars().take(slot.saturating_sub(1)).collect();
            format!("{name:<slot$}")
        })
        .collect();
    lines.push(Line::from(Span::styled(names, p.fg(p.faint))));
    lines
}

/// Adds `text` to `spans`, joined to the last when they share a style.
fn push(spans: &mut Vec<Span<'static>>, text: String, style: Style) {
    match spans.last_mut() {
        Some(last) if last.style == style => last.content.to_mut().push_str(&text),
        _ => spans.push(Span::styled(text, style)),
    }
}

impl Palette {
    /// The fainter colour of the area under a curve in `tint`.
    pub(crate) fn tint_under(&self, tint: Rgb) -> Rgb {
        tint.mix(self.ground, 0.62)
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
    fn gauges_fill_by_eighths_and_mark_the_pace() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let drawn: String = gauge(0.55, Some(0.8), 10, p.ok, &p)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(drawn, "█████▌──│─");
        assert_eq!(sparkline(&[0.0, 1.0, 2.0], 8, p.ok, &p).content, "▁▅█");
    }

    #[test]
    fn a_curve_and_bars_are_drawn_in_characters() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let label = |v: f64| format!("{v:.0}");
        let curve = Curve {
            points: &[(0.0, 0.0), (1.0, 1.0)],
            x: (0.0, 1.0),
            y: (0.0, 1.0),
            width: 8,
            height: 2,
            tint: p.ok,
            label: &label,
            ends: ("a".into(), "b".into()),
        };
        let lines = text(&curve.lines(&p));
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("1 "));
        assert!(lines[2].ends_with('b'));
        let bars = stacked(
            &[
                ("mon".into(), vec![(1.0, p.ok), (1.0, p.bad)]),
                ("tue".into(), vec![(1.0, p.ok)]),
            ],
            8,
            1,
            &p,
        );
        assert_eq!(text(&bars), ["▀▀▀ ▄▄▄ ", "mon tue "]);
    }
}
