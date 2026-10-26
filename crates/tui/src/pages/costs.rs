//! Where the money goes: what the provider billed over the period, the
//! bill as it grew, each model's calls and each kind of task's; then the
//! context each agent's calls read.

use iq_core::{TokenCount, Usd};
use iq_ui::PERIODS;
use iq_ui::costs::Total;
use ratatui::text::{Line, Span};

use super::Seen;
use crate::chart::{Curve, gauge, sparkline};
use crate::wrap::cut;

/// A period's name: `1h`, `24h`, `7d`.
fn period_name(seconds: u64) -> String {
    if seconds >= 2 * 24 * 3600 {
        format!("{}d", seconds / (24 * 3600))
    } else {
        format!("{}h", seconds / 3600)
    }
}

/// The page's lines.
pub(super) fn lines(seen: &Seen<'_>) -> Vec<Line<'static>> {
    let p = seen.p;
    let board = seen.board;
    let now = seen.now;
    let since = now.saturating_sub(board.period());
    let mut lines = Vec::new();
    let mut periods = vec![Span::styled(" Over the last ", p.fg(p.dim))];
    for period in PERIODS {
        let on = board.period() == period;
        periods.push(Span::styled(
            format!(" {} ", period_name(period)),
            if on { p.bold(p.accent) } else { p.fg(p.faint) },
        ));
    }
    lines.push(Line::default());
    lines.push(Line::from(periods));
    let totals = board.costs().totals(since);
    if totals.is_empty() {
        lines.push(Line::default());
        seen.said(
            &mut lines,
            &format!(
                "iq made no call to a model in the last {}.",
                period_name(board.period())
            ),
            p.dim,
        );
    } else {
        billed(seen, &totals, &mut lines);
        running(seen, since, &mut lines);
        by_model(seen, &totals, &mut lines);
    }
    by_kind(seen, since, &mut lines);
    contexts(seen, &mut lines);
    lines
}

/// The provider's bill over the period, in large.
fn billed(seen: &Seen<'_>, totals: &[Total], lines: &mut Vec<Line<'static>>) {
    let p = seen.p;
    let calls: usize = totals.iter().map(|t| t.calls).sum();
    let mut cost = Usd(0.0);
    for total in totals {
        cost += total.cost;
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("  Billed      ", p.fg(p.dim)),
        Span::styled(format!("{:<12}", cost.to_string()), p.bold(p.ok)),
        Span::styled(format!("{calls} calls"), p.fg(p.faint)),
    ]));
}

/// What the provider billed, summed as time went.
fn running(seen: &Seen<'_>, since: u64, lines: &mut Vec<Line<'static>>) {
    let p = seen.p;
    let board = seen.board;
    let now = seen.now;
    let points = board.costs().running(since);
    let mut points: Vec<(f64, f64)> = points
        .iter()
        .map(|(at, sum)| (-(now.saturating_sub(*at) as f64) / 3600.0, *sum))
        .collect();
    if let Some(&(_, last)) = points.last() {
        points.push((0.0, last));
    }
    let top = points.iter().map(|(_, v)| *v).fold(0.0, f64::max).max(0.01);
    let hours = board.period() as f64 / 3600.0;
    seen.heading(lines, "The provider's bill", "as it grew over the period");
    let label = |v: f64| format!("${v:.2}");
    lines.extend(
        Curve {
            points: &points,
            x: (-hours, 0.0),
            y: (0.0, top),
            width: seen.width.saturating_sub(2).min(100),
            height: 5,
            tint: p.ok,
            label: &label,
            ends: (
                format!("{} ago", period_name(board.period())),
                "now".to_owned(),
            ),
        }
        .lines(p)
        .into_iter()
        .map(|line| {
            let mut spans = vec![Span::raw(" ")];
            spans.extend(line.spans);
            Line::from(spans)
        }),
    );
}

/// The calls by model.
fn by_model(seen: &Seen<'_>, totals: &[Total], lines: &mut Vec<Line<'static>>) {
    let p = seen.p;
    seen.heading(lines, "By model", "");
    let model_width = totals
        .iter()
        .map(|t| t.model.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    for total in totals {
        let tint = if total.unpriced > 0 { p.warn } else { p.text };
        let mut said = total.cost.to_string();
        if total.unpriced > 0 {
            said.push_str(&format!(" ({} without a known price)", total.unpriced));
        }
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<model_width$}", total.model), p.fg(p.text)),
            Span::styled(format!("{:>6} calls  ", total.calls), p.fg(p.faint)),
            Span::styled(format!("{:<12}", said), p.fg(tint)),
            Span::styled(format!("{} tokens", TokenCount(total.tokens)), p.fg(p.dim)),
        ]));
    }
}

/// What each kind of task spent over the period.
fn by_kind(seen: &Seen<'_>, since: u64, lines: &mut Vec<Line<'static>>) {
    let p = seen.p;
    let kinds = seen.board.costs().by_kind(since);
    if kinds.is_empty() {
        return;
    }
    seen.heading(
        lines,
        "By kind of task",
        "as your categories place iq's tasks",
    );
    let most = kinds.iter().map(|k| k.billed.0).fold(0.0, f64::max);
    let name_width = kinds
        .iter()
        .map(|k| k.kind.as_deref().unwrap_or("no kind").chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    for kind in &kinds {
        let name = kind.kind.as_deref().unwrap_or("no kind");
        let share = if most > 0.0 {
            kind.billed.0 / most
        } else {
            0.0
        };
        let mut spans = vec![
            Span::styled(
                format!("  {name:<name_width$}"),
                p.fg(if kind.kind.is_some() { p.text } else { p.dim }),
            ),
            Span::styled(format!("{:>4} tasks  ", kind.tasks), p.fg(p.faint)),
            Span::styled(format!("{:<10}", kind.billed.to_string()), p.fg(p.text)),
            Span::styled(
                format!("{:<12}", TokenCount(kind.tokens).to_string()),
                p.fg(p.dim),
            ),
        ];
        spans.extend(gauge(share, None, 16, p.accent, p));
        lines.push(Line::from(spans));
    }
}

/// The context each agent's calls read, as it grew: what its model reads
/// again at each call, and so what each call costs.
fn contexts(seen: &Seen<'_>, lines: &mut Vec<Line<'static>>) {
    let p = seen.p;
    let agents: Vec<_> = seen
        .board
        .crew()
        .agents()
        .iter()
        .filter(|a| !a.reads.is_empty())
        .collect();
    if agents.is_empty() {
        return;
    }
    seen.heading(lines, "Context", "what each call of each agent read");
    for agent in agents {
        let values: Vec<f64> = agent.reads.iter().map(|t| t.0 as f64).collect();
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "  {:<44}",
                    cut(
                        &format!(
                            "{} · {} · {} calls",
                            agent.name,
                            agent.model,
                            agent.reads.len()
                        ),
                        43
                    )
                ),
                p.fg(p.text),
            ),
            sparkline(&values, 32, p.accent, p),
            Span::styled(format!("  {}", agent.context()), p.fg(p.faint)),
        ]));
    }
}
