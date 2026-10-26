//! The assay's page: who grades and what is tested, the runs at work,
//! each kind of task with the cheapest candidate that does it as well as
//! the lead, the exercises, and each one's results, the kinds of
//! candidates apart.

use iq_core::TokenCount;
use iq_ui::assay::{CHAT, Kind, Phase, Row, State};
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
        "The same frozen exercises for every candidate, checked by hidden tests, then graded blind by one grader, always the same: for each kind of task, the cheapest agent that does it as well as your lead.",
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
    let grader = match &overview.graded_by {
        Some(model) => format!("{} (lately {model})", overview.grader),
        None => overview.grader.clone(),
    };
    for (name, value) in [
        ("Grader", grader),
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
    let chatting = board.crew().agent(CHAT).is_some();
    lines.push(Line::from(vec![
        Span::styled("  t ", p.bold(p.accent)),
        Span::styled(
            if chatting { "write in the assay's chat" } else { "a chat in the assay" },
            p.fg(p.accent),
        ),
        Span::styled(
            ": ask it to test a model, write or adjust an exercise, read the results; what costs is asked first",
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
                Phase::Judging => ("being graded", p.info),
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
    routing(seen, &mut lines);
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
            "None yet. Ask the assay's chat (t) to write the first ones, best from commits of your own work.",
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
        if let Some(grade) = summary.reference_grade {
            let tint = if grade < 8.0 { p.warn } else { p.faint };
            seen.said(
                &mut lines,
                &format!("   the grader gave its reference {grade:.1}"),
                tint,
            );
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

/// The routing: for each kind of task, the agents new tasks of it start on,
/// the first first, as the records route them and the person's choices
/// leave them.
fn routing(seen: &Seen<'_>, lines: &mut Vec<Line<'static>>) {
    let p = seen.p;
    let book = seen.board.routes();
    if book.routing.routes.is_empty() {
        return;
    }
    let what = if book.choices.off {
        "off, as your settings say: every chat on the model it starts on".to_owned()
    } else {
        format!(
            "graded by {}; a new task of a kind starts on its first",
            book.routing.grader
        )
    };
    seen.heading(lines, "Routing", &what);
    for route in &book.routing.routes {
        let ladder = book.routing.route(&route.category, &book.choices);
        let mut spans = vec![Span::styled(
            format!("  {:<14}", route.category),
            p.fg(p.text),
        )];
        match ladder.first() {
            Some(first) => {
                spans.push(Span::styled(first.candidate.clone(), p.bold(p.accent)));
                spans.push(Span::styled(format!("  {}", first.said()), p.fg(p.dim)));
                if ladder.len() > 1 {
                    let then: Vec<&str> =
                        ladder[1..].iter().map(|c| c.candidate.as_str()).collect();
                    spans.push(Span::styled(
                        format!("  then {}", then.join(", ")),
                        p.fg(p.faint),
                    ));
                }
            }
            None => spans.push(Span::styled(
                "nobody passes every check yet: the chat's own agent works",
                p.fg(p.faint),
            )),
        }
        lines.push(Line::from(spans));
    }
    for (agent, why) in &book.choices.vetoed {
        lines.push(Line::from(vec![
            Span::styled(format!("  never {agent}"), p.fg(p.bad)),
            Span::styled(format!("  {why}"), p.fg(p.faint)),
        ]));
    }
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
