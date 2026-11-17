//! An audit of what the agents did since the last: what iq summed up, and
//! what having the audit's model read it should cost, for the person to
//! say whether it does; then what the model said, kept in `~/.iq/audits`.

use iq_ui::Audit;
use iq_ui::keys::Action;
use ratatui::text::{Line, Span};

use super::Seen;

/// The page's lines.
pub(super) fn lines(seen: &Seen<'_>) -> Vec<Line<'static>> {
    let p = seen.p;
    let mut lines = Vec::new();
    match seen.board.audit() {
        None => seen.said(&mut lines, "Summing up what the agents did…", p.dim),
        Some(Audit::Nothing(why)) => seen.said(&mut lines, why, p.dim),
        Some(Audit::Ready {
            model,
            skill,
            cost,
            calls,
            account,
        }) => {
            let calls = match calls {
                1 => "in one call".to_owned(),
                n => format!("in {n} calls, the account being read in parts"),
            };
            let cost = match cost.as_str() {
                "" => "its provider says no price".to_owned(),
                cost => format!("about {cost}"),
            };
            let asks = match skill {
                Some(skill) => format!(", looking for what your skill {skill} asks"),
                None => String::new(),
            };
            seen.said(
                &mut lines,
                &format!("Read by {model}{asks}, {calls}: {cost}. Nothing is sent before you say."),
                p.text,
            );
            seen.heading(&mut lines, "What iq summed up", "what the model would read");
            for line in account.lines() {
                seen.said(&mut lines, line, p.dim);
            }
        }
        Some(Audit::Reading { model }) => seen.said(
            &mut lines,
            &format!("{model} reads what the agents did…"),
            p.dim,
        ),
        Some(Audit::Read { report, file }) => {
            seen.said(&mut lines, &format!("Kept in {file}"), p.faint);
            lines.push(Line::default());
            seen.markdown(
                &mut lines,
                report,
                seen.width.saturating_sub(1),
                &[Span::raw(" ")],
            );
        }
    }
    lines
}

/// The keys the page takes.
pub(super) fn foot(seen: &Seen<'_>) -> String {
    match seen.board.audit() {
        Some(Audit::Ready { .. }) => format!(
            "{} have it read · {} not now · ↑↓ scroll",
            seen.key(Action::Allow),
            seen.key(Action::Refuse)
        ),
        _ => "↑↓ scroll".to_owned(),
    }
}
