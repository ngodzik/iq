//! Every step the agents took, unfolded: fold by fold, the latest last,
//! each under its number and its agent, as the conversation's counts name
//! them; a tool's output in its first lines, more of it when it failed.

use iq_ui::steps::Step;
use ratatui::text::{Line, Span};

use super::Seen;
use crate::history::{agent_tint, short, step_line};
use crate::wrap::{cut, wrap};

/// How many lines of a tool's output show when it succeeded.
const OUTPUT_LINES: usize = 3;

/// The same when it failed.
const FAILED_LINES: usize = 12;

/// The page's lines.
pub(super) fn lines(seen: &Seen<'_>) -> Vec<Line<'static>> {
    let p = seen.p;
    let mut lines = Vec::new();
    let width = seen.width.saturating_sub(1);
    let mut any = false;
    for fold in seen.board.steps().folds() {
        any = true;
        let tint = agent_tint(seen.board, &fold.agent, p);
        let mut head = vec![
            Span::styled(format!(" #{} ", fold.id), p.bold(p.text)),
            Span::styled(fold.agent.clone(), p.bold(tint)),
            Span::styled(
                format!(
                    "  {} step{}",
                    fold.steps.len(),
                    if fold.steps.len() == 1 { "" } else { "s" }
                ),
                p.fg(p.faint),
            ),
        ];
        if !fold.closed {
            head.push(Span::styled("  at work", p.fg(p.accent)));
        }
        lines.push(Line::default());
        lines.push(Line::from(head));
        for step in &fold.steps {
            let (first, body) = step_line(step, p);
            lines.extend(wrap(&body, width, &first, &[Span::raw("    ")]));
            if let Step::Tool { output, ok, .. } = step {
                for line in output
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .take(if *ok { OUTPUT_LINES } else { FAILED_LINES })
                {
                    lines.push(Line::from(Span::styled(
                        format!("    {}", cut(&short(line), width.saturating_sub(4))),
                        p.fg(if *ok { p.faint } else { p.dim }),
                    )));
                }
            }
        }
    }
    if !any {
        lines.push(Line::default());
        seen.said(&mut lines, "No agent took a step yet.", p.dim);
    }
    lines
}
