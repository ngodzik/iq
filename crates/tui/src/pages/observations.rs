//! What iq's work went to: over a day, a week, a month, a year or all of
//! it, back as far as the person goes, summed by category, model, project
//! or agent; a chart part by part, each group stacked, a table of the
//! groups, and the dearest tasks.

use iq_core::{TokenCount, Usd};
use iq_ui::observations::{By, Span as Period, UNPLACED};
use ratatui::text::{Line, Span};

use super::Seen;
use crate::chart::{gauge, stacked};
use crate::history::seconds;
use crate::theme::{Palette, Rgb};
use crate::wrap::cut;

/// How many groups the chart shows apart; the rest are summed as one.
const CHARTED: usize = 6;

/// A colour for the group at `at`.
fn colour(p: &Palette, at: usize) -> Rgb {
    let colours = [p.accent, p.info, p.ok, p.bad, p.agent(3), p.warn, p.dim];
    colours[at.min(CHARTED) % colours.len()]
}

/// The page's lines.
pub(super) fn lines(seen: &Seen<'_>) -> Vec<Line<'static>> {
    let p = seen.p;
    let board = seen.board;
    let lens = board.lens();
    let period = board.seen();
    let mut lines = vec![Line::default()];
    let mut choices = vec![Span::styled(" ", p.fg(p.faint))];
    for span in Period::ALL {
        let on = lens.span == span;
        choices.push(Span::styled(
            format!(" {} ", span.name()),
            if on { p.bold(p.accent) } else { p.fg(p.faint) },
        ));
    }
    choices.push(Span::styled("   by ", p.fg(p.faint)));
    for by in By::ALL {
        let on = lens.by == by;
        choices.push(Span::styled(
            format!(" {} ", by.name()),
            if on { p.bold(p.accent) } else { p.fg(p.faint) },
        ));
    }
    lines.push(Line::from(choices));
    lines.push(Line::default());
    let total = &period.total;
    let mut said = format!(
        "{} task{} · {} billed · {}",
        total.tasks,
        if total.tasks == 1 { "" } else { "s" },
        Usd(total.billed),
        seconds(total.seconds)
    );
    if total.failed > 0 {
        said.push_str(&format!(" · {} failed", total.failed));
    }
    let mut head = vec![
        Span::styled(format!(" {}", period.title), p.bold(p.text)),
        Span::styled(format!("   {said}"), p.fg(p.dim)),
    ];
    if let Some(before) = period.before.filter(|b| *b > 0.0) {
        let change = (total.cost() - before) / before * 100.0;
        head.push(Span::styled(
            format!(
                "   {change:+.0}% against the period before ({})",
                Usd(before)
            ),
            p.fg(if change > 0.0 { p.warn } else { p.ok }),
        ));
    }
    lines.push(Line::from(head));
    if total.tasks == 0 {
        lines.push(Line::default());
        seen.said(&mut lines, "No task of iq's in this period.", p.dim);
        return lines;
    }
    if period.unplaced && lens.by == By::Category {
        seen.said(
            &mut lines,
            "No task is in a category: name yours in ~/.iq/settings.toml, [[observe.category]], and a cheap model places the work in them as it ends.",
            p.warn,
        );
    }
    // The chart: each part of the period, its groups stacked.
    let charted = period.groups.len().min(CHARTED);
    let bars: Vec<(String, Vec<(f64, Rgb)>)> = period
        .slices
        .iter()
        .map(|slice| {
            let mut parts: Vec<(f64, Rgb)> = (0..charted)
                .map(|at| (slice.costs.get(at).copied().unwrap_or(0.0), colour(p, at)))
                .collect();
            if period.groups.len() > CHARTED {
                parts.push((slice.costs.iter().skip(CHARTED).sum(), colour(p, CHARTED)));
            }
            (slice.name.clone(), parts)
        })
        .collect();
    lines.push(Line::default());
    let top = period
        .slices
        .iter()
        .map(|s| s.costs.iter().sum::<f64>())
        .fold(0.0, f64::max);
    lines.push(Line::from(Span::styled(
        format!(" {}", Usd(top)),
        p.fg(p.faint),
    )));
    for line in stacked(&bars, seen.width.saturating_sub(4), 7, p) {
        let mut spans = vec![Span::raw("  ")];
        spans.extend(line.spans);
        lines.push(Line::from(spans));
    }
    // The groups, the dearest first.
    lines.push(Line::default());
    let most = period.groups.first().map_or(0.0, |g| g.cost());
    let name_width = period
        .groups
        .iter()
        .map(|g| g.name.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 28)
        + 2;
    lines.push(Line::from(Span::styled(
        format!(
            "   {:<name_width$}{:>6}{:>8}{:>11}{:>11}{:>9}{:>9}",
            "", "tasks", "failed", "billed", "a success", "tokens", "time"
        ),
        p.fg(p.faint),
    )));
    for (at, group) in period.groups.iter().enumerate() {
        let failed = if group.failed > 0 {
            group.failed.to_string()
        } else {
            "·".to_owned()
        };
        let mut spans = vec![
            Span::styled(" ● ", p.fg(colour(p, at))),
            Span::styled(
                format!("{:<name_width$}", cut(&group.name, name_width - 1)),
                p.fg(p.text),
            ),
            Span::styled(format!("{:>6}", group.tasks), p.fg(p.text)),
            Span::styled(
                format!("{failed:>8}"),
                p.fg(if group.failed > 0 { p.bad } else { p.faint }),
            ),
            Span::styled(
                format!("{:>11}", Usd(group.billed).to_string()),
                p.fg(p.text),
            ),
            Span::styled(
                format!("{:>9}", TokenCount(group.tokens).to_string()),
                p.fg(p.dim),
            ),
            Span::styled(format!("{:>9}  ", seconds(group.seconds)), p.fg(p.dim)),
        ];
        let share = if most > 0.0 { group.cost() / most } else { 0.0 };
        spans.extend(gauge(share, None, 12, p.accent, p));
        lines.push(Line::from(spans));
    }
    seen.heading(&mut lines, "The dearest tasks", "");
    for task in &period.top {
        let mut spans = vec![
            Span::styled(format!("  {:<10}", task.billed.to_string()), p.fg(p.accent)),
            Span::styled(
                format!("{:<18}", task.kind.as_deref().unwrap_or(UNPLACED)),
                p.fg(p.info),
            ),
            Span::styled(format!("{:<26}", cut(&task.model, 25)), p.fg(p.dim)),
        ];
        if !task.ok {
            spans.push(Span::styled("failed  ", p.fg(p.bad)));
        }
        let prompt: String = task.prompt.split_whitespace().collect::<Vec<_>>().join(" ");
        let room = seen.width.saturating_sub(58);
        spans.push(Span::styled(cut(&prompt, room), p.fg(p.text)));
        lines.push(Line::from(spans));
    }
    lines
}
