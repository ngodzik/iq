//! The pages over the conversation, each on the terminal's other screen,
//! which leaves the conversation as it was when it closes: a title, a
//! question when one waits, the page's lines scrolled as the person reads
//! them, and the keys it takes at its foot.

mod assay;
mod costs;
mod keys;
mod observations;
mod team;

use iq_ui::Board;
use iq_ui::keys::Action;
use iq_ui::screen::Page;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::history::short;
use crate::theme::Palette;
use crate::wrap::{cut, spans_width};

/// What a page draws from.
pub(crate) struct Seen<'a> {
    pub(crate) board: &'a Board,
    pub(crate) p: &'a Palette,
    /// Seconds since 1970.
    pub(crate) now: u64,
    /// The columns it has.
    pub(crate) width: usize,
}

impl Seen<'_> {
    /// The key that does `action`, as a hint names it.
    pub(crate) fn key(&self, action: Action) -> String {
        self.board
            .keys()
            .chord(action)
            .map(ToString::to_string)
            .unwrap_or_default()
    }

    /// A section's heading, after a blank line.
    pub(crate) fn heading(&self, lines: &mut Vec<Line<'static>>, title: &str, what: &str) {
        let p = self.p;
        lines.push(Line::default());
        let mut spans = vec![Span::styled(format!(" {title}"), p.bold(p.text))];
        if !what.is_empty() {
            spans.push(Span::styled(format!("  {what}"), p.fg(p.faint)));
        }
        lines.push(Line::from(spans));
    }

    /// A line of text in passing, folded to the page.
    pub(crate) fn said(&self, lines: &mut Vec<Line<'static>>, text: &str, tint: crate::theme::Rgb) {
        let p = self.p;
        lines.extend(crate::wrap::wrap(
            &[Span::styled(text.to_owned(), p.fg(tint))],
            self.width.saturating_sub(2),
            &[Span::raw(" ")],
            &[Span::raw(" ")],
        ));
    }
}

/// The page's lines, and the keys it takes.
fn content(page: &Page, seen: &Seen<'_>) -> (Vec<Line<'static>>, &'static str) {
    match page {
        Page::Team => (
            team::lines(seen),
            "↑↓ choose · Enter write to it · v its thread · n a new chat",
        ),
        Page::Thread(agent) => (team::thread(seen, agent), "↑↓ scroll · ← the team"),
        Page::Costs => (
            costs::lines(seen),
            "← → the period · e your own sessions too · ↑↓ scroll",
        ),
        Page::Assay => (
            assay::lines(seen),
            "t the assay's chat · s stop the runs · ↑↓ scroll",
        ),
        Page::Observations => (
            observations::lines(seen),
            "← → earlier, later · s the span · b by · n now · ↑↓ scroll",
        ),
        Page::Keys => (keys::lines(seen), "↑↓ scroll"),
    }
}

/// Draws the page open over the whole screen, and keeps its scroll
/// within its lines.
pub(crate) fn draw(frame: &mut Frame<'_>, board: &mut Board, p: &Palette, now: u64) {
    let Some(page) = board.screen().page().cloned() else {
        return;
    };
    let area = frame.area();
    let seen = Seen {
        board,
        p,
        now,
        width: usize::from(area.width),
    };
    let (lines, foot) = content(&page, &seen);
    let mut top: Vec<Line<'static>> = vec![title(&page, &seen)];
    if let Some(question) = board.questions().first() {
        top.push(Line::from(vec![
            Span::styled(format!(" {} asks to ", question.agent), p.fg(p.warn)),
            Span::styled(
                cut(&short(&question.action), usize::from(area.width) / 2),
                p.fg(p.text),
            ),
            Span::styled(
                format!(
                    "  {} allow · {} refuse",
                    seen.key(Action::Allow),
                    seen.key(Action::Refuse)
                ),
                p.fg(p.dim),
            ),
        ]));
    }
    top.push(rule(area.width, p));
    let body_rows = area.height.saturating_sub(top.len() as u16 + 2);
    let most = lines.len().saturating_sub(usize::from(body_rows));
    let scroll = board.screen().scroll().min(most);
    let shown: Vec<Line<'static>> = lines
        .iter()
        .skip(scroll)
        .take(usize::from(body_rows))
        .cloned()
        .collect();
    let mut y = area.top();
    let head = top.len() as u16;
    frame.render_widget(
        Paragraph::new(top),
        Rect::new(area.left(), y, area.width, head),
    );
    y += head;
    let body = Rect::new(area.left(), y, area.width, body_rows);
    frame.render_widget(Paragraph::new(shown), body);
    let place = if lines.len() > usize::from(body_rows) {
        format!(
            "{}–{} of {} ",
            scroll + 1,
            (scroll + usize::from(body_rows)).min(lines.len()),
            lines.len()
        )
    } else {
        String::new()
    };
    let foot_line = Line::from(vec![Span::styled(
        format!(" {foot} · Esc closes"),
        p.fg(p.faint),
    )]);
    let foot_width = spans_width(&foot_line.spans) as u16;
    let bottom = area.bottom().saturating_sub(2);
    frame.render_widget(
        Paragraph::new(rule(area.width, p)),
        Rect::new(area.left(), bottom, area.width, 1),
    );
    frame.render_widget(
        Paragraph::new(foot_line),
        Rect::new(area.left(), bottom + 1, area.width, 1),
    );
    if !place.is_empty() && foot_width + (place.len() as u16) < area.width {
        let x = area.right().saturating_sub(place.chars().count() as u16);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(place.clone(), p.fg(p.faint)))),
            Rect::new(x, bottom + 1, place.chars().count() as u16, 1),
        );
    }
    board.screen_mut().drawn(most, usize::from(body_rows));
}

/// The page's title: iq's name, the page's, and on the right the page's
/// key that closes it too.
fn title(page: &Page, seen: &Seen<'_>) -> Line<'static> {
    let p = seen.p;
    Line::from(vec![
        Span::styled(" i", p.bold(p.accent)),
        Span::styled("q", p.bold(p.accent.mix(p.info, 0.6))),
        Span::styled("  ·  ", p.fg(p.faint)),
        Span::styled(page.title(), p.bold(p.text)),
    ])
}

/// A rule across the page.
fn rule(width: u16, p: &Palette) -> Line<'static> {
    Line::from(Span::styled("─".repeat(usize::from(width)), p.fg(p.faint)))
}
