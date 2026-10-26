//! The shortcuts as the settings bind them, the commands, and the keys
//! that edit what the person writes.

use iq_ui::command::COMMANDS;
use ratatui::text::{Line, Span};

use super::Seen;

/// The keys that edit, as every terminal's line editors take them.
const EDITING: [(&str, &str); 9] = [
    ("Enter", "send; a line ending in \\ goes on"),
    ("Shift-Enter, Alt-Enter", "a new line"),
    ("↑ ↓", "the line above, below, or the messages sent before"),
    ("Ctrl-A, Ctrl-E", "the start, the end of the line"),
    ("Alt-B, Alt-F", "a word back, forward"),
    ("Ctrl-W, Alt-Backspace", "delete the word before"),
    (
        "Ctrl-K, Ctrl-U",
        "delete to the end, to the start of the line",
    ),
    ("Tab", "complete a command, an agent after @"),
    (
        "Ctrl-C",
        "clear what is written, stop the agent, twice to quit",
    ),
];

/// The page's lines.
pub(super) fn lines(seen: &Seen<'_>) -> Vec<Line<'static>> {
    let p = seen.p;
    let mut lines = Vec::new();
    seen.heading(
        &mut lines,
        "Shortcuts",
        "change them in the [keys] of ~/.iq/settings.toml",
    );
    let list = seen.board.keys().list();
    let key_width = list
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0)
        + 3;
    for (key, action) in list {
        lines.push(Line::from(vec![
            Span::styled(format!("  {key:<key_width$}"), p.bold(p.accent)),
            Span::styled(action.what(), p.fg(p.text)),
            Span::styled(format!("   {}", action.name()), p.fg(p.faint)),
        ]));
    }
    seen.heading(
        &mut lines,
        "Commands",
        "written after a /, completed with Tab",
    );
    let name_width = COMMANDS
        .iter()
        .map(|c| c.name.len() + c.args.len() + 2)
        .max()
        .unwrap_or(0)
        + 2;
    for command in COMMANDS {
        let named = if command.args.is_empty() {
            format!("/{}", command.name)
        } else {
            format!("/{} {}", command.name, command.args)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {named:<name_width$}"), p.fg(p.accent)),
            Span::styled(command.what, p.fg(p.text)),
        ]));
    }
    seen.heading(&mut lines, "Writing", "");
    for (key, what) in EDITING {
        lines.push(Line::from(vec![
            Span::styled(format!("  {key:<26}"), p.fg(p.accent)),
            Span::styled(what, p.fg(p.text)),
        ]));
    }
    seen.heading(&mut lines, "Naming a model", "");
    seen.said(
        &mut lines,
        " Write \"with opus\", \"avec haiku\" or \"sur <a model of the provider>\" in a message: a new chat starts on the first model named, and an agent may start others on those named, which it is told. Only after such a word does a name count.",
        p.dim,
    );
    seen.heading(&mut lines, "On a page", "");
    seen.said(
        &mut lines,
        " ↑ ↓, Page Up, Page Down, Home and End scroll it, or the wheel; its own keys are at its foot; Escape closes it, and its shortcut again.",
        p.dim,
    );
    lines
}
