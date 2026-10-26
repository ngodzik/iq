//! The assay's page: who judges and what is tested, the runs at work,
//! each kind of task with the cheapest candidate that does it as well as
//! the lead, the exercises, and each one's results, the kinds of
//! candidates apart.

use iq_core::TokenCount;
use iq_ui::assay::{JUDGE, Kind, Phase, Row, State};
use ratatui::text::{Line, Span};

use super::Seen;
use crate::history::seconds;

/// The page's lines.
pub(super) fn lines(seen: &Seen<'_>) -> Vec<Line<'static>> {
    let p = seen.p;
    let board = seen.board;
    let view = board.assay();
    let overview = view.overview();
    let mut lines = vec![Line::default()];
    seen.said(
        &mut lines,
        "The same frozen exercises for every candidate, checked by hidden tests, then judged blind: for each kind of task, the cheapest agent that does it as well as your lead.",
        p.dim,
    );
    if !view.told() {
        seen.said(&mut lines, "The assay's folder is being read…", p.dim);
        return lines;
    }
    if let Some(trouble) = &overview.trouble {
        seen.said(&mut lines, trouble, p.bad);
    }
    for notice in &overview.notices {
        seen.said(&mut lines, notice, p.warn);
    }
    lines.push(Line::default());
    for (name, value) in [
        ("Judges", overview.judges.join(", ")),
        ("Reference", overview.reference.clone()),
        ("Tested", overview.allowed.clone()),
        ("Folder", overview.folder.display().to_string()),
    ] {
        lines.push(Line::from(vec![
            Span::styled(format!("  {name:<11}"), p.fg(p.dim)),
            Span::styled(value, p.fg(p.text)),
        ]));
    }
    lines.push(Line::default());
    let talking = board.crew().agent(JUDGE).is_some();
    lines.push(Line::from(vec![
        Span::styled("  t ", p.bold(p.accent)),
        Span::styled(
            if talking { "write to the judge" } else { "talk with the judge" },
            p.fg(p.accent),
        ),
        Span::styled(
            ": it proposes kinds of task and exercises with you, and asks before anything that costs",
            p.fg(p.faint),
        ),
    ]));
    if !view.running().is_empty() {
        seen.heading(&mut lines, "At work", "s stops them");
        for progress in view.running() {
            let (said, tint) = match progress.phase {
                Phase::Waiting => ("waiting", p.faint),
                Phase::Working => ("working", p.accent),
                Phase::Checking => ("checking", p.info),
                Phase::Judging => ("being judged", p.info),
                Phase::Done(true) => ("passed", p.ok),
                Phase::Done(false) => ("failed", p.bad),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  ● {:<4}", progress.label), p.fg(tint)),
                Span::styled(format!("{}  ", progress.candidate), p.fg(p.text)),
                Span::styled(format!("{said:<14}"), p.fg(tint)),
                Span::styled(
                    format!(
                        "{} tools · {} · {} tokens · {}",
                        progress.tools,
                        progress.billed,
                        TokenCount(progress.tokens),
                        seconds(seen.now.saturating_sub(progress.since))
                    ),
                    p.fg(p.faint),
                ),
            ]));
        }
    }
    if !overview.categories.is_empty() {
        seen.heading(&mut lines, "By kind of task", "");
        for (name, rows, fit) in &overview.categories {
            lines.push(Line::from(Span::styled(
                format!("  {name}"),
                p.bold(p.text),
            )));
            match fit {
                Some(fit) => seen.said(&mut lines, &format!(" {fit}"), p.ok),
                None => seen.said(
                    &mut lines,
                    " No candidate billed by the provider does as well as the reference yet.",
                    p.faint,
                ),
            }
            results(seen, rows, &mut lines);
        }
    }
    seen.heading(&mut lines, "Exercises", "");
    if overview.exercises.is_empty() {
        seen.said(
            &mut lines,
            "None yet. Talk with the judge to write the first ones, best from commits of your own work.",
            p.dim,
        );
    }
    for summary in &overview.exercises {
        let (state, tint) = match &summary.state {
            State::Draft => ("draft".to_owned(), p.dim),
            State::Frozen(v) => (format!("frozen · v{v}"), p.ok),
            State::Changed(v) => (format!("changed since v{v}"), p.warn),
            State::Broken(_) => ("broken".to_owned(), p.bad),
        };
        let mut spans = vec![
            Span::styled(format!("  {}  ", summary.id), p.bold(p.text)),
            Span::styled(format!("{state}  "), p.fg(tint)),
            Span::styled(summary.title.clone(), p.fg(p.text)),
        ];
        for category in &summary.categories {
            spans.push(Span::styled(format!("  #{category}"), p.fg(p.info)));
        }
        spans.push(Span::styled(
            format!(
                "  {} run{}",
                summary.runs,
                if summary.runs == 1 { "" } else { "s" }
            ),
            p.fg(p.faint),
        ));
        lines.push(Line::from(spans));
        if let Some(problem) = summary.problems.first() {
            let more = summary.problems.len() - 1;
            let text = if more > 0 {
                format!("   {problem} (and {more} more)")
            } else {
                format!("   {problem}")
            };
            seen.said(&mut lines, &text, p.warn);
        }
    }
    for table in &overview.tables {
        let version = table
            .version
            .map(|v| format!(", version {v}"))
            .unwrap_or_default();
        seen.heading(
            &mut lines,
            &format!("{} · {}{version}", table.exercise, table.title),
            "",
        );
        results(seen, &table.rows, &mut lines);
    }
    lines
}

/// Candidates' results, a heading per kind when there are several.
fn results(seen: &Seen<'_>, rows: &[Row], lines: &mut Vec<Line<'static>>) {
    let p = seen.p;
    if rows.is_empty() {
        seen.said(lines, " No runs yet.", p.faint);
        return;
    }
    let kinds: Vec<Kind> = Kind::ALL
        .into_iter()
        .filter(|kind| rows.iter().any(|r| r.kind == *kind))
        .collect();
    let name_width = rows
        .iter()
        .map(|r| r.candidate.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    for kind in &kinds {
        if kinds.len() > 1 {
            lines.push(Line::from(Span::styled(
                format!("   {}", kind.plural()),
                p.fg(p.faint),
            )));
        }
        lines.push(Line::from(Span::styled(
            format!(
                "   {:<name_width$}{:<8}{:<7}{:<7}{:<12}{:<12}{}",
                "", "passed", "tests", "score", "a run", "a success", "time"
            ),
            p.fg(p.faint),
        )));
        for row in rows.iter().filter(|r| r.kind == *kind) {
            let rate = row.rate();
            let tint = if rate >= 0.8 {
                p.ok
            } else if rate >= 0.5 {
                p.warn
            } else {
                p.bad
            };
            let per_run = match (row.billed.0 > 0.0, row.subscription_tokens > 0) {
                (true, _) => row.billed.to_string(),
                (false, true) => format!("{} tok", TokenCount(row.subscription_tokens)),
                (false, false) => "·".to_owned(),
            };
            let per_success = row
                .per_success
                .filter(|_| row.billed.0 > 0.0)
                .map_or("·".to_owned(), |c| c.to_string());
            let mut spans = vec![
                Span::styled(format!("   {:<name_width$}", row.candidate), p.fg(p.text)),
                Span::styled(
                    format!("{:<8}", format!("{}/{}", row.passed, row.runs)),
                    p.fg(tint),
                ),
                Span::styled(
                    format!(
                        "{:<7}",
                        row.tests
                            .map_or("·".to_owned(), |t| format!("{:.0}%", t * 100.0))
                    ),
                    p.fg(p.dim),
                ),
                Span::styled(
                    format!(
                        "{:<7}",
                        row.score.map_or("·".to_owned(), |s| format!("{s:.1}"))
                    ),
                    p.fg(p.text),
                ),
                Span::styled(format!("{per_run:<12}"), p.fg(p.text)),
                Span::styled(format!("{per_success:<12}"), p.fg(p.accent)),
                Span::styled(seconds(row.seconds), p.fg(p.dim)),
            ];
            if !row.providers.is_empty() {
                spans.push(Span::styled(
                    format!("  served by {}", row.providers.join(", ")),
                    p.fg(p.faint),
                ));
            }
            lines.push(Line::from(spans));
        }
    }
}
