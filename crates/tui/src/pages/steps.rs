//! Every step the agents took and what they wrote to each other, the
//! latest last, as the conversation's counts and messages number them:
//! each fold under its number and its agent, a line for each step; each
//! message under its number, its writer and its reader, summed up. The
//! entry chosen shows whole as Enter says, and folds again: a fold's
//! steps whole (the whole command, the whole reason, the whole question,
//! the whole notice) and what its tools returned, a message as it was
//! written.

use iq_ui::steps::{Entry, Step};
use ratatui::text::{Line, Span};

use super::Seen;
use crate::history::{agent_tint, short, step_line};
use crate::wrap::{cut, wrap};

/// How many lines of a failed tool's output show while its fold is folded:
/// what went wrong.
const FAILED_LINES: usize = 3;

/// How many lines of each tool's output show once its fold is unfolded.
const OUTPUT_LINES: usize = 40;

/// How many lines of a message show, folded, when no model summed it up.
const MESSAGE_LINES: usize = 3;

/// The page's lines, and the line where the entry chosen begins.
pub(super) fn lines(seen: &Seen<'_>) -> (Vec<Line<'static>>, Option<usize>) {
    let p = seen.p;
    let width = seen.width.saturating_sub(1);
    let screen = seen.board.screen();
    let entries = seen.board.steps().entries();
    let chosen = screen.step().or_else(|| entries.last().map(Entry::id));
    let mut lines = Vec::new();
    let mut focus = None;
    for entry in &entries {
        let on = Some(entry.id()) == chosen;
        let whole = screen.unfolded(entry.id());
        lines.push(Line::default());
        if on {
            focus = Some(lines.len());
        }
        let mut head = vec![
            Span::styled(if on { "▸" } else { " " }, p.bold(p.accent)),
            Span::styled(format!("#{} ", entry.id()), p.bold(p.text)),
        ];
        match entry {
            Entry::Fold(fold) => {
                head.extend([
                    Span::styled(
                        fold.agent.clone(),
                        p.bold(agent_tint(seen.board, &fold.agent, p)),
                    ),
                    Span::styled(
                        format!(
                            "  {} step{}",
                            fold.steps.len(),
                            if fold.steps.len() == 1 { "" } else { "s" }
                        ),
                        p.fg(p.faint),
                    ),
                ]);
                if !fold.closed {
                    head.push(Span::styled("  at work", p.fg(p.accent)));
                }
                lines.push(Line::from(head));
                for step in &fold.steps {
                    let (first, body) = step_line(step, p);
                    if whole {
                        let indent = [Span::raw("    ")];
                        for (at, line) in step.whole().lines().enumerate() {
                            let said = [Span::styled(short(line), p.fg(p.text))];
                            let head = if at == 0 { first.as_slice() } else { &indent };
                            lines.extend(wrap(&said, width, head, &indent));
                        }
                    } else {
                        lines.extend(wrap(&body, width, &first, &[Span::raw("    ")]));
                    }
                    if let Step::Tool { output, ok, .. } = step {
                        let shown = match (whole, ok) {
                            (true, _) => OUTPUT_LINES,
                            (false, false) => FAILED_LINES,
                            (false, true) => 0,
                        };
                        for line in output.lines().filter(|l| !l.trim().is_empty()).take(shown) {
                            lines.push(Line::from(Span::styled(
                                format!("    {}", cut(&short(line), width.saturating_sub(4))),
                                p.fg(if *ok { p.faint } else { p.dim }),
                            )));
                        }
                    }
                }
            }
            Entry::Said(said) => {
                head.extend([
                    Span::styled("✉ ", p.fg(p.info)),
                    Span::styled(
                        said.from.clone(),
                        p.fg(agent_tint(seen.board, &said.from, p)),
                    ),
                    Span::styled(" → ", p.fg(p.faint)),
                    Span::styled(said.to.clone(), p.fg(agent_tint(seen.board, &said.to, p))),
                ]);
                if said.task {
                    head.push(Span::styled("  its task", p.fg(p.faint)));
                }
                lines.push(Line::from(head));
                let indent = [Span::raw("    ")];
                let text = if whole {
                    said.text.clone()
                } else {
                    (seen.summed)(said.id).unwrap_or_else(|| {
                        said.text
                            .lines()
                            .filter(|l| !l.trim().is_empty())
                            .take(MESSAGE_LINES)
                            .map(|line| cut(line, width.saturating_sub(4)))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                };
                let tint = if whole { p.text } else { p.dim };
                for line in text.lines() {
                    if line.trim().is_empty() && !whole {
                        continue;
                    }
                    lines.extend(wrap(
                        &[Span::styled(line.to_owned(), p.fg(tint))],
                        width,
                        &indent,
                        &indent,
                    ));
                }
            }
            // A job folded: what it does, how it ended; unfolded, its
            // command whole and the end of its output.
            Entry::Ran(ran) => {
                let colour = if ran.said == "exit status 0" {
                    p.ok
                } else {
                    p.warn
                };
                head.extend([
                    Span::styled("⚙ ", p.fg(colour)),
                    Span::styled(
                        ran.agent.clone(),
                        p.fg(agent_tint(seen.board, &ran.agent, p)),
                    ),
                    Span::styled(format!("  job {} · {}", ran.job, ran.said), p.fg(p.faint)),
                ]);
                lines.push(Line::from(head));
                let indent = [Span::raw("    ")];
                if whole {
                    for line in ran.line.lines() {
                        lines.extend(wrap(
                            &[Span::styled(short(line), p.fg(p.text))],
                            width,
                            &indent,
                            &indent,
                        ));
                    }
                    if !ran.output.trim().is_empty() {
                        lines.push(Line::from(Span::styled(
                            "    the end of its output:",
                            p.fg(p.faint),
                        )));
                    }
                    for line in ran
                        .output
                        .lines()
                        .filter(|l| !l.trim().is_empty())
                        .take(OUTPUT_LINES)
                    {
                        lines.push(Line::from(Span::styled(
                            format!("    {}", cut(&short(line), width.saturating_sub(4))),
                            p.fg(p.dim),
                        )));
                    }
                } else {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "    {}",
                            iq_ui::shell::few_words(&ran.line, width.saturating_sub(4))
                        ),
                        p.fg(p.dim),
                    )));
                }
            }
        }
    }
    if entries.is_empty() {
        lines.push(Line::default());
        seen.said(
            &mut lines,
            "No agent took a step or wrote to another yet.",
            p.dim,
        );
    }
    (lines, focus)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Depth, Palette};
    use iq_agent::{Event, PERSON};
    use iq_ui::look::Theme;
    use iq_ui::{Board, Update};

    fn shown(board: &Board, summed: &dyn Fn(usize) -> Option<String>) -> (Vec<String>, usize) {
        let p = Palette::new(Theme::Ink, Depth::True);
        let seen = Seen {
            board,
            p: &p,
            now: 0,
            width: 80,
            summed,
            motion: &crate::motion::Motion::default(),
            labels: &|_| None,
            writing: &|_| None,
        };
        let (lines, focus) = lines(&seen);
        (
            lines.iter().map(ToString::to_string).collect(),
            focus.unwrap(),
        )
    }

    #[test]
    fn a_message_shows_summed_up_and_whole_once_unfolded_as_a_fold_its_outputs() {
        let mut board = Board::default();
        for event in [
            Event::Joined {
                agent: "fix".into(),
                model: "lab/m".into(),
                by: PERSON.into(),
            },
            Event::Joined {
                agent: "fix/tests".into(),
                model: "lab/m".into(),
                by: "fix".into(),
            },
            Event::Working {
                agent: "fix/tests".into(),
                prompt: "first\nsecond\nthird\nfourth".into(),
            },
            Event::Tool {
                agent: "fix/tests".into(),
                name: "run_command".into(),
                input: "make test".into(),
                output: "ok 1\nok 2".into(),
                ok: true,
            },
            Event::Posted {
                from: "fix/tests".into(),
                to: "fix".into(),
                text: "3 pass".into(),
            },
        ] {
            board.update(Update::Crew(event));
        }
        let summed = |id: usize| (id == 1).then(|| "fix asks for the tests".to_owned());
        let (lines, focus) = shown(&board, &summed);
        // Folded: a message summed up, a tool in a line; the latest chosen.
        assert!(
            lines.contains(&"    fix asks for the tests".to_owned()),
            "{lines:#?}"
        );
        assert!(
            !lines
                .iter()
                .any(|l| l.contains("fourth") || l.contains("ok 1"))
        );
        assert_eq!(lines[focus], "▸#3 ✉ fix/tests → fix");
        board.screen_mut().unfold(1);
        board.screen_mut().unfold(2);
        let (lines, _) = shown(&board, &summed);
        assert!(lines.contains(&"    fourth".to_owned()), "{lines:#?}");
        assert!(lines.contains(&"    ok 1".to_owned()), "{lines:#?}");
        assert!(!lines.iter().any(|l| l.contains("fix asks for the tests")));
    }
}
