//! The pages over the conversation, each on the terminal's other screen,
//! which leaves the conversation as it was when it closes: a title, a
//! question when one waits, the page's lines scrolled as the person reads
//! them, and the keys it takes at its foot.

mod changes;
mod costs;
mod keys;
mod observations;
mod steps;
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

/// What the pages read beside the board: what the conversation made of
/// it, and what moves.
pub(crate) struct Beside<'a> {
    /// The summary of the message between agents of a number, once made.
    pub(crate) summed: &'a dyn Fn(usize) -> Option<String>,
    /// What moves: the spinners, the messages' sparks.
    pub(crate) motion: &'a crate::motion::Motion,
    /// What a job does in a few words, by its number.
    pub(crate) labels: &'a dyn Fn(u64) -> Option<String>,
    /// What an agent writes now, while it writes.
    pub(crate) writing: &'a dyn Fn(&str) -> Option<String>,
}

/// What a page draws from.
pub(crate) struct Seen<'a> {
    pub(crate) board: &'a Board,
    pub(crate) p: &'a Palette,
    /// Seconds since 1970.
    pub(crate) now: u64,
    /// The columns it has.
    pub(crate) width: usize,
    /// Whether the terminal shows images, Kitty.
    pub(crate) images: bool,
    /// The summary of the message between agents of a number, once made.
    pub(crate) summed: &'a dyn Fn(usize) -> Option<String>,
    /// What moves: the spinners, the messages' sparks.
    pub(crate) motion: &'a crate::motion::Motion,
    /// What a job does in a few words, by its number.
    pub(crate) labels: &'a dyn Fn(u64) -> Option<String>,
    /// What an agent writes now, while it writes.
    pub(crate) writing: &'a dyn Fn(&str) -> Option<String>,
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

/// The page's lines, the keys it takes, and the line it is to show when
/// it shows a chosen entry.
fn content(page: &Page, seen: &Seen<'_>) -> (Vec<Line<'static>>, String, Option<usize>) {
    if *page == Page::Steps {
        let (lines, focus) = steps::lines(seen);
        let foot = "↑↓ choose · Enter whole, or folded again · the latest last";
        return (lines, foot.to_owned(), focus);
    }
    let (lines, foot) = match page {
        Page::Team => (
            team::lines(seen),
            "↑↓ choose · Enter write to it · → its whole thread · n a new chat",
        ),
        Page::Thread(agent) => (team::thread(seen, agent), "↑↓ scroll · ← the chats"),
        Page::Costs => (costs::lines(seen), "← → the period · ↑↓ scroll"),
        Page::Observations => (
            observations::lines(seen),
            "← → earlier, later · s the span · b by · n now · ↑↓ scroll",
        ),
        Page::Keys => (keys::lines(seen), "↑↓ scroll"),
        Page::Map => (
            map(seen),
            "↑↓ choose · Enter its thread, or the job whole · w write to it · s stop it · f those finished",
        ),
        Page::Changes => return (changes::lines(seen), changes::foot(seen), None),
        Page::Steps => (Vec::new(), ""),
        Page::Diagram => (
            diagram(seen),
            "+ − enlarge · ← → ↑ ↓ move · 0 whole · [ ] an earlier, a later · o your viewer",
        ),
    };
    (lines, foot.to_owned(), None)
}

/// The map of the turn of the chat written to, whole.
fn map(seen: &Seen<'_>) -> Vec<Line<'static>> {
    let p = seen.p;
    let Some(chat) = seen.board.chat_shown() else {
        let mut lines = Vec::new();
        seen.said(&mut lines, "Write to a chat: its turn shows here.", p.dim);
        return lines;
    };
    let map = crate::map::Map {
        board: seen.board,
        p,
        motion: seen.motion,
        now: seen.now,
        labels: seen.labels,
        writing: seen.writing,
    };
    let finished = seen.board.screen().finished_shown();
    let mut lines = vec![
        Line::default(),
        map.head(&chat, seen.width),
        Line::default(),
    ];
    lines.extend(map.rows(
        &chat,
        seen.width,
        Some(seen.board.screen().chosen()),
        finished,
        false,
    ));
    if let Some(foot) = map.foot(&chat, seen.width, finished) {
        lines.push(Line::default());
        lines.push(foot);
    }
    lines
}

/// What the diagram page says beside the image: what it is, and, where
/// the terminal shows no image, how to see it.
fn diagram(seen: &Seen<'_>) -> Vec<Line<'static>> {
    let p = seen.p;
    let mut lines = Vec::new();
    match seen.board.picture() {
        None => seen.said(&mut lines, "no diagram is drawn in this chat", p.dim),
        Some(_) if !seen.images => seen.said(
            &mut lines,
            "the diagram shows here in Kitty alone: o opens it in your viewer",
            p.dim,
        ),
        Some((path, picture)) => seen.said(
            &mut lines,
            &format!("{} · ×{}", path.display(), picture.zoom.max(1)),
            p.faint,
        ),
    }
    lines
}

/// Draws the page open over the whole screen, and keeps its scroll
/// within its lines. Returns where its body is, under its heading and
/// above its foot.
pub(crate) fn draw(
    frame: &mut Frame<'_>,
    board: &mut Board,
    p: &Palette,
    now: u64,
    images: bool,
    beside: &Beside<'_>,
) -> Rect {
    let Some(page) = board.screen().page().cloned() else {
        return Rect::default();
    };
    let area = frame.area();
    let seen = Seen {
        board,
        p,
        now,
        width: usize::from(area.width),
        images,
        summed: beside.summed,
        motion: beside.motion,
        labels: beside.labels,
        writing: beside.writing,
    };
    let (lines, foot, focus) = content(&page, &seen);
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
    // The entry just chosen, or shown whole, comes into sight: its head
    // at the top when it was out of it, or when it grew past the bottom.
    if let Some(focus) = focus
        && board.screen_mut().take_follow()
    {
        let scroll = board.screen().scroll().min(most);
        let rows = usize::from(body_rows);
        let next = lines[focus..]
            .iter()
            .skip(1)
            .position(|line| line.spans.is_empty())
            .map_or(lines.len(), |end| focus + 1 + end);
        if focus < scroll || next > scroll + rows {
            board.screen_mut().scroll_to(focus.saturating_sub(1));
        }
    }
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
    body
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
