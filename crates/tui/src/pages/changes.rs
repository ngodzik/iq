//! What a sync would send, line by line: the diff git makes of it, each
//! file named as the repository names it, lines added and removed in their
//! colours, for the person to read before they say whether it goes.

use iq_ui::keys::Action;
use ratatui::text::{Line, Span};

use super::Seen;
use crate::wrap::cut;

/// The page's lines.
pub(super) fn lines(seen: &Seen<'_>) -> Vec<Line<'static>> {
    let p = seen.p;
    let mut lines = Vec::new();
    if let Some(ask) = seen.board.sync_ask() {
        seen.heading(&mut lines, &format!("To {}", ask.remote), "");
        let files = match ask.files.len() {
            1 => ask.said.clone(),
            n => format!("{n} files: {}", ask.said),
        };
        seen.said(&mut lines, &files, p.dim);
    }
    lines.push(Line::default());
    match seen.board.changes() {
        None => seen.said(&mut lines, "reading what changed…", p.dim),
        Some("") => seen.said(
            &mut lines,
            "nothing changed: a sync would send nothing",
            p.dim,
        ),
        Some(diff) => {
            let width = seen.width.saturating_sub(1);
            for line in diff.lines() {
                let text = cut(&format!(" {}", line.replace('\t', "    ")), width);
                let style = if line.starts_with("diff --git ") {
                    p.bold(p.text)
                } else if line.starts_with("+++ ") || line.starts_with("--- ") {
                    p.bold(p.dim)
                } else if line.starts_with("@@") {
                    p.fg(p.info)
                } else if line.starts_with('+') {
                    p.fg(p.ok)
                } else if line.starts_with('-') {
                    p.fg(p.bad)
                } else if line.starts_with(' ') {
                    p.fg(p.text)
                } else {
                    // `index`, a mode, `Binary files`, the lines left out.
                    p.fg(p.faint)
                };
                lines.push(Line::from(Span::styled(text, style)));
            }
        }
    }
    lines
}

/// The keys the page takes: those that answer, while a sync asks.
pub(super) fn foot(seen: &Seen<'_>) -> String {
    if seen.board.sync_ask().is_some() {
        format!(
            "{} send · {} not now · ↑↓ scroll",
            seen.key(Action::Allow),
            seen.key(Action::Refuse)
        )
    } else {
        "↑↓ scroll · /sync sends it".to_owned()
    }
}
