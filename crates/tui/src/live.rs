//! The live part, below the conversation: the agents at work and what
//! they do, each with its latest steps since its last line of text, the
//! oldest question waiting for the person, a word to them,
//! the commands that complete what they write, where they write, the
//! folder of its own the agent they write to works in, and the status
//! line, along which messages run between the agents.

use std::collections::HashMap;

use iq_ui::Board;
use iq_ui::crew::Entry;
use iq_ui::keys::Action;
use iq_ui::screen::Talk;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use unicode_width::UnicodeWidthChar;

use crate::history::{History, agent_tint, seconds, short, step_line};
use crate::motion::{Motion, YOU};
use crate::theme::Palette;
use crate::wrap::{cut, spans_width, width as text_width, wrap};

/// The most agents at work given a line each.
const WORKING: usize = 3;

/// The most completions listed at once.
const LISTED: usize = 6;

/// The most lines of what the person writes shown at once.
const WRITTEN: usize = 8;

/// What the live part draws from.
pub(crate) struct Live<'a> {
    pub(crate) board: &'a Board,
    pub(crate) history: &'a History,
    pub(crate) p: &'a Palette,
    /// Seconds since 1970.
    pub(crate) now: u64,
}

/// One part of the live part, with the rows it takes.
enum Part {
    Lines(Vec<Line<'static>>),
    Boxed {
        title: Line<'static>,
        lines: Vec<Line<'static>>,
        tint: crate::theme::Rgb,
    },
    Input {
        title: Line<'static>,
        spent: Line<'static>,
        lines: Vec<Line<'static>>,
        cursor: Option<(u16, u16)>,
    },
    Status,
}

impl Part {
    fn rows(&self) -> u16 {
        match self {
            Part::Lines(lines) => lines.len() as u16,
            Part::Boxed { lines, .. } | Part::Input { lines, .. } => lines.len() as u16 + 2,
            Part::Status => 1,
        }
    }
}

impl Live<'_> {
    /// How many rows the live part takes at `width`.
    pub(crate) fn height(&self, motion: &Motion, width: u16) -> u16 {
        self.parts(motion, width).iter().map(Part::rows).sum()
    }

    /// Draws the live part in `frame`, whose area is its place. Returns
    /// where the cursor stands, in what the person writes.
    pub(crate) fn draw(&self, frame: &mut Frame<'_>, motion: &mut Motion) -> Option<(u16, u16)> {
        let area = frame.area();
        let mut y = area.top();
        let mut cursor = None;
        let mut status_row = None;
        for part in self.parts(motion, area.width) {
            let rows = part.rows().min(area.bottom().saturating_sub(y));
            let place = Rect::new(area.left(), y, area.width, rows);
            match part {
                Part::Lines(lines) => frame.render_widget(Paragraph::new(lines), place),
                Part::Boxed { title, lines, tint } => {
                    let block = Block::bordered()
                        .border_type(BorderType::Rounded)
                        .border_style(self.p.fg(tint))
                        .title(title);
                    frame.render_widget(Paragraph::new(lines).block(block), place);
                }
                Part::Input {
                    title,
                    spent,
                    lines,
                    cursor: at,
                } => {
                    let block = Block::bordered()
                        .border_type(BorderType::Rounded)
                        .border_style(self.p.fg(self.p.faint))
                        .title(title)
                        .title_top(spent.right_aligned());
                    frame.render_widget(Paragraph::new(lines).block(block), place);
                    cursor = at.map(|(x, line)| (place.left() + 1 + x, place.top() + 1 + line));
                }
                Part::Status => status_row = Some(place),
            }
            y += rows;
        }
        if let Some(row) = status_row {
            let marks = self.status(frame, row, motion);
            motion.fly(frame.buffer_mut(), row, &marks, self.p);
        }
        if let Some(at) = cursor {
            frame.set_cursor_position(at);
        }
        cursor
    }

    fn parts(&self, motion: &Motion, width: u16) -> Vec<Part> {
        let width = usize::from(width);
        let mut parts = Vec::new();
        let working = self.working(motion, width);
        if !working.is_empty() {
            parts.push(Part::Lines(working));
        }
        if let Some(question) = self.question(width) {
            parts.push(question);
        }
        if let Some(said) = self.board.screen().noticed(self.now) {
            let p = self.p;
            parts.push(Part::Lines(wrap(
                &[Span::styled(said.to_owned(), p.fg(p.warn))],
                width.saturating_sub(1),
                &[Span::raw("  ")],
                &[Span::raw("  ")],
            )));
        }
        let listed = self.completions(width);
        if !listed.is_empty() {
            parts.push(Part::Lines(listed));
        }
        parts.push(self.input(width));
        if let Some(place) = self.place(width) {
            parts.push(Part::Lines(vec![place]));
        }
        parts.push(Part::Status);
        parts
    }

    /// Where the agent the person writes to works, when it is a place of
    /// its own: the whole path, alone on its line for the person to select
    /// and paste in another terminal; written from `~` only when the whole
    /// of it does not fit.
    fn place(&self, width: usize) -> Option<Line<'static>> {
        let p = self.p;
        let agent = self.board.screen().talking_to()?;
        let place = self.board.crew().agent(agent)?.place.as_ref()?;
        let lead = "  in ";
        let mut path = place.display().to_string();
        if text_width(lead) + text_width(&path) >= width {
            path = short(&path);
        }
        Some(Line::from(vec![
            Span::styled(lead, p.fg(p.faint)),
            Span::styled(path, p.fg(p.dim)),
        ]))
    }

    /// A line for each agent at work: what it does, for how long, and
    /// what it is writing; under it, its latest steps since its last line
    /// of text, as many as the settings say, the earlier counted.
    fn working(&self, motion: &Motion, width: usize) -> Vec<Line<'static>> {
        let p = self.p;
        let talking = self.board.screen().talking_to();
        let mut agents: Vec<_> = self
            .board
            .crew()
            .agents()
            .iter()
            .filter(|a| a.working)
            .collect();
        agents.sort_by_key(|a| Some(a.name.as_str()) != talking);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for agent in agents.iter().take(WORKING) {
            let tint = agent_tint(self.board, &agent.name, p);
            let mut spans = vec![
                Span::styled(format!("{} ", motion.spinner()), p.fg(p.accent)),
                Span::styled(agent.name.clone(), p.bold(tint)),
                Span::styled(format!(" · {} ", agent.serving()), p.fg(p.dim)),
            ];
            spans.extend(motion.shimmer("working…", p.dim, p.text, p));
            let lasted = self
                .history
                .working_for(&agent.name)
                .map(|d| seconds(d.as_secs()))
                .unwrap_or_default();
            let mut after = format!(" {lasted}");
            if let Some(spent) = self.history.turn_spent(&agent.name) {
                after.push_str(&format!(" · {spent}"));
            }
            if Some(agent.name.as_str()) == talking
                && let Some(stop) = self.board.keys().chord(Action::Stop)
            {
                after.push_str(&format!(" · {} to stop", hint(&stop.to_string())));
            }
            spans.push(Span::styled(after, p.fg(p.faint)));
            let shown = self.board.look().steps();
            let fold = self.board.steps().open(&agent.name).filter(|_| shown > 0);
            let doing = self
                .history
                .partial(&agent.name)
                .map(|text| format!("  {}", text.replace('\n', " ")))
                .or_else(|| match agent.thread.last() {
                    // Its steps show under it, unless the settings show none.
                    Some(Entry::Tool { name, input, .. }) if fold.is_none() => {
                        Some(format!("  ▸ {name} {}", short(input).replace('\n', " ")))
                    }
                    _ => None,
                });
            if let Some(doing) = doing {
                let room = width.saturating_sub(spans_width(&spans) + 1);
                if room > 8 {
                    spans.push(Span::styled(cut(&doing, room), p.fg(p.dim)));
                }
            }
            lines.push(Line::from(spans));
            if let Some(fold) = fold {
                let earlier = fold.steps.len().saturating_sub(shown);
                if earlier > 0 {
                    let mut said = format!(
                        "    ⋯ {earlier} step{} before",
                        if earlier == 1 { "" } else { "s" }
                    );
                    if let Some(unfold) = self.board.keys().chord(Action::Steps) {
                        said.push_str(&format!(" · {} unfolds", hint(&unfold.to_string())));
                    }
                    lines.push(Line::from(Span::styled(said, p.fg(p.faint))));
                }
                for step in &fold.steps[earlier..] {
                    let (mut first, body) = step_line(step, p);
                    first.insert(0, Span::raw("  "));
                    first.extend(body);
                    lines.push(fit(first, width));
                }
            }
        }
        if agents.len() > WORKING {
            lines.push(Line::from(Span::styled(
                format!("  and {} more at work", agents.len() - WORKING),
                p.fg(p.faint),
            )));
        }
        lines
    }

    /// The oldest question waiting, in a box, with the keys that answer it.
    fn question(&self, width: usize) -> Option<Part> {
        let p = self.p;
        let questions = self.board.questions();
        let Some(question) = questions.first() else {
            return self.sync_question(width);
        };
        let inner = width.saturating_sub(4);
        let pad = [Span::raw(" ")];
        let mut lines = wrap(
            &[Span::styled(short(&question.action), p.fg(p.text))],
            inner,
            &pad,
            &pad,
        );
        let mut why = format!("held because {}", short(&question.reason));
        if question.widens {
            why.push_str(": it reaches beyond the sandbox, only you may allow it");
        }
        lines.extend(wrap(&[Span::styled(why, p.fg(p.dim))], inner, &pad, &pad));
        let key = |action| {
            self.board
                .keys()
                .chord(action)
                .map(ToString::to_string)
                .unwrap_or_default()
        };
        let mut keys = vec![
            Span::raw(" "),
            Span::styled(key(Action::Allow), p.bold(p.ok)),
            Span::styled(" allow  ", p.fg(p.dim)),
            Span::styled(key(Action::Refuse), p.bold(p.bad)),
            Span::styled(" refuse", p.fg(p.dim)),
        ];
        if questions.len() > 1 {
            keys.push(Span::styled(
                format!("  · {} more waiting", questions.len() - 1),
                p.fg(p.faint),
            ));
        }
        lines.push(Line::from(keys));
        let tint = agent_tint(self.board, &question.agent, p);
        Some(Part::Boxed {
            title: Line::from(vec![
                Span::raw(" "),
                Span::styled(question.agent.clone(), p.bold(tint)),
                Span::styled(" asks ", p.fg(p.warn)),
            ]),
            lines,
            tint: p.warn,
        })
    }

    /// What a sync would send, asked in a box, when no agent asks
    /// anything: the files, and the keys that answer.
    fn sync_question(&self, width: usize) -> Option<Part> {
        let p = self.p;
        let ask = self.board.sync_ask()?;
        let inner = width.saturating_sub(4);
        let pad = [Span::raw(" ")];
        let files = match ask.files.len() {
            1 => ask.said.clone(),
            n => format!("{n} files: {}", ask.said),
        };
        let mut lines = wrap(
            &[Span::styled(
                format!("send what changed to {}?", ask.remote),
                p.fg(p.text),
            )],
            inner,
            &pad,
            &pad,
        );
        lines.extend(wrap(
            &[Span::styled(format!("changed: {files}"), p.fg(p.dim))],
            inner,
            &pad,
            &pad,
        ));
        let key = |action| {
            self.board
                .keys()
                .chord(action)
                .map(ToString::to_string)
                .unwrap_or_default()
        };
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(key(Action::Allow), p.bold(p.ok)),
            Span::styled(" send  ", p.fg(p.dim)),
            Span::styled(key(Action::Refuse), p.bold(p.bad)),
            Span::styled(" not now  ", p.fg(p.dim)),
            Span::styled(key(Action::Changes), p.bold(p.info)),
            Span::styled(" the diff", p.fg(p.dim)),
            Span::styled("  · /sync auto never asks", p.fg(p.faint)),
        ]));
        Some(Part::Boxed {
            title: Line::from(vec![
                Span::raw(" "),
                Span::styled("sync", p.bold(p.info)),
                Span::styled(" asks ", p.fg(p.warn)),
            ]),
            lines,
            tint: p.warn,
        })
    }

    /// The commands, or agents, that complete what the person writes.
    fn completions(&self, width: usize) -> Vec<Line<'static>> {
        let p = self.p;
        let all = self.board.completions();
        if all.is_empty() {
            return Vec::new();
        }
        let picked = self.board.screen().picked().min(all.len() - 1);
        let first = picked.saturating_sub(LISTED - 1);
        let label_width = all.iter().map(|c| text_width(&c.label)).max().unwrap_or(0) + 2;
        all.iter()
            .enumerate()
            .skip(first)
            .take(LISTED)
            .map(|(at, completion)| {
                let on = at == picked;
                let mark = if on { "▸ " } else { "  " };
                let label = format!("{:<label_width$}", completion.label);
                let room = width.saturating_sub(label_width + 4);
                Line::from(vec![
                    Span::styled(format!(" {mark}"), p.fg(p.accent)),
                    Span::styled(label, if on { p.bold(p.accent) } else { p.fg(p.text) }),
                    Span::styled(cut(&completion.what, room), p.fg(p.dim)),
                ])
            })
            .collect()
    }

    /// Where the person writes: to whom in the box's title, what the agent
    /// spent on its right, and the cursor where it stands.
    fn input(&self, width: usize) -> Part {
        let p = self.p;
        let screen = self.board.screen();
        let (title, spent, hint) = match screen.talk() {
            Talk::To(agent) => {
                let view = self.board.crew().agent(agent);
                let tint = agent_tint(self.board, agent, p);
                let mut model = view.map(|v| v.model.clone()).unwrap_or_default();
                if let Some(effort) = view.and_then(|v| v.effort) {
                    model.push_str(&format!(" · {}", effort.as_str()));
                }
                let spent = view.map_or(String::new(), |v| match v.context().0 {
                    0 => String::new(),
                    _ => format!(" {} · context {} ", v.billed, v.context()),
                });
                (
                    Line::from(vec![
                        Span::raw(" "),
                        Span::styled(agent.clone(), p.bold(tint)),
                        Span::styled(format!(" {model} "), p.fg(p.faint)),
                    ]),
                    Line::from(Span::styled(spent, p.fg(p.faint))),
                    format!("Write to {agent}…"),
                )
            }
            Talk::New(chat) => {
                let mut said = format!(" · {} ", chat.model);
                if let Some(effort) = chat.effort {
                    said.push_str(&format!("· {} ", effort.as_str()));
                }
                if chat.pattern != iq_agent::Pattern::Solo {
                    said.push_str(&format!("· {} ", chat.pattern.name()));
                    if !chat.partner.trim().is_empty() {
                        said.push_str(&format!("with {} ", chat.partner.trim()));
                    }
                }
                if !chat.role.trim().is_empty() {
                    said.push_str(&format!("· {} ", chat.role.trim()));
                }
                let short_model = chat
                    .model
                    .rsplit_once('/')
                    .map_or(chat.model.as_str(), |(_, name)| name)
                    .to_owned();
                (
                    Line::from(vec![
                        Span::styled(" new chat", p.bold(p.accent)),
                        Span::styled(said, p.fg(p.faint)),
                    ]),
                    Line::default(),
                    format!("The task for {short_model}…"),
                )
            }
        };
        let inner = width.saturating_sub(4).max(1);
        let editor = screen.editor();
        if editor.is_empty() {
            let line = Line::from(vec![
                Span::styled("› ", p.bold(p.accent)),
                Span::styled(hint, p.fg(p.faint)),
                Span::styled(
                    "   / commands · @ agents",
                    p.fg(p.faint).add_modifier(Modifier::DIM),
                ),
            ]);
            return Part::Input {
                title,
                spent,
                lines: vec![line],
                cursor: Some((2, 0)),
            };
        }
        let (rows, cursor) = fold(editor.text(), editor.cursor(), inner);
        let shown = rows.len().min(WRITTEN);
        let first = cursor.1.saturating_sub(shown - 1).min(rows.len() - shown);
        let lines = rows
            .iter()
            .enumerate()
            .skip(first)
            .take(shown)
            .map(|(at, row)| {
                let lead = if at == 0 {
                    Span::styled("› ", p.bold(p.accent))
                } else {
                    Span::raw("  ")
                };
                Line::from(vec![lead, Span::styled(row.clone(), p.fg(p.text))])
            })
            .collect();
        Part::Input {
            title,
            spent,
            lines,
            cursor: Some((cursor.0 as u16 + 2, (cursor.1 - first) as u16)),
        }
    }

    /// The status line: the autopilot, the internet when open, what the
    /// session spent and what was spent since midnight, the sync, the questions waiting; on its right the person
    /// and the agents. Returns where each mark is drawn, for the sparks.
    fn status(&self, frame: &mut Frame<'_>, row: Rect, motion: &Motion) -> HashMap<String, u16> {
        let p = self.p;
        let crew = self.board.crew();
        let mut left: Vec<Span<'static>> = vec![Span::raw(" ")];
        let sep = || Span::styled(" · ", p.fg(p.faint));
        if crew.autopilot() {
            left.push(Span::styled("⏵⏵ autopilot", p.fg(p.accent)));
        } else {
            left.push(Span::styled("autopilot off", p.fg(p.faint)));
        }
        if crew.internet() {
            left.push(sep());
            left.push(Span::styled("internet open", p.fg(p.warn)));
        }
        let mut spent = iq_core::Usd(0.0);
        for agent in crew.agents() {
            spent += agent.billed;
        }
        if spent.0 > 0.0 {
            left.push(sep());
            left.push(Span::styled(format!("{spent} spent"), p.fg(p.dim)));
        }
        // Since midnight, earlier sessions with this one: said when it is
        // more than this one.
        let today = self.board.billed_today(self.now);
        if today.0 > 0.0 && today.to_string() != spent.to_string() {
            left.push(sep());
            left.push(Span::styled(format!("{today} today"), p.fg(p.faint)));
        }
        if let Some(synced) = self.board.synced() {
            left.push(sep());
            let ago = self.now.saturating_sub(synced.at) / 60;
            if synced.ok {
                let when = match ago {
                    0 => "synced just now".to_owned(),
                    m if m < 60 => format!("synced {m} min ago"),
                    m => format!("synced {}h ago", m / 60),
                };
                left.push(Span::styled(when, p.fg(p.faint)));
            } else {
                left.push(Span::styled("sync failed", p.fg(p.warn)));
            }
        }
        let waiting = self.board.questions().len();
        if waiting > 0 {
            left.push(sep());
            left.push(Span::styled(
                format!("{waiting} waiting for you"),
                p.fg(p.warn),
            ));
        }
        // On the right, the person and the agents, by name while they fit.
        let agents = crew.agents();
        let talking = self.board.screen().talking_to();
        let right = |named: bool| -> (Vec<Span<'static>>, Vec<(String, u16)>) {
            let mut spans = Vec::new();
            let mut marks = Vec::new();
            let mut at = 0u16;
            let put = |spans: &mut Vec<Span<'static>>, span: Span<'static>, at: &mut u16| {
                *at += text_width(&span.content) as u16;
                spans.push(span);
            };
            marks.push((YOU.to_owned(), at));
            let you = p.dim.mix(p.text, motion.glow(YOU));
            put(&mut spans, Span::styled(YOU, p.fg(you)), &mut at);
            for (index, agent) in agents.iter().enumerate() {
                put(&mut spans, Span::raw(" "), &mut at);
                let tint = p.agent(index);
                let dot = if agent.working {
                    p.faint.mix(tint, 0.35 + 0.65 * motion.pulse(5.0))
                } else {
                    tint.mix(p.faint, 0.45)
                };
                let dot = dot.mix(p.text, motion.glow(&agent.name));
                marks.push((agent.name.clone(), at));
                put(&mut spans, Span::styled("●", p.fg(dot)), &mut at);
                if named {
                    let style = if Some(agent.name.as_str()) == talking {
                        p.bold(tint).add_modifier(Modifier::UNDERLINED)
                    } else {
                        p.fg(p.dim)
                    };
                    put(&mut spans, Span::raw(" "), &mut at);
                    put(&mut spans, Span::styled(agent.name.clone(), style), &mut at);
                }
            }
            put(&mut spans, Span::raw(" "), &mut at);
            (spans, marks)
        };
        let room = usize::from(row.width);
        let (mut right_spans, mut right_marks) = right(true);
        if spans_width(&left) + spans_width(&right_spans) + 2 > room {
            (right_spans, right_marks) = right(false);
        }
        let right_width = spans_width(&right_spans);
        let left_room = room.saturating_sub(right_width + 1);
        let left_line = fit(left, left_room);
        let start = row.left() + row.width.saturating_sub(right_width as u16);
        frame.render_widget(Paragraph::new(left_line), row);
        if right_width < room {
            frame.render_widget(
                Paragraph::new(Line::from(right_spans)),
                Rect::new(start, row.top(), right_width as u16, 1),
            );
        }
        right_marks
            .into_iter()
            .map(|(name, at)| (name, start + at))
            .collect()
    }
}

/// A key as a hint writes it, short: `Esc` for Escape.
pub(crate) fn hint(key: &str) -> String {
    key.replace("Escape", "Esc")
}

/// `spans` cut to `room` columns.
fn fit(spans: Vec<Span<'static>>, room: usize) -> Line<'static> {
    let mut used = 0;
    let mut kept = Vec::new();
    for span in spans {
        let w = text_width(&span.content);
        if used + w <= room {
            used += w;
            kept.push(span);
        } else {
            let left = room.saturating_sub(used);
            if left > 1 {
                kept.push(Span::styled(cut(&span.content, left), span.style));
            }
            break;
        }
    }
    Line::from(kept)
}

/// `text` folded into rows of `width` columns at its new lines and where
/// it is full, and where `cursor`, a byte in it, falls: its column and row.
pub(crate) fn fold(text: &str, cursor: usize, width: usize) -> (Vec<String>, (usize, usize)) {
    let width = width.max(2) - 1;
    let mut rows = vec![String::new()];
    let mut used = 0;
    let mut at = (0, 0);
    for (byte, c) in text.char_indices() {
        if byte == cursor {
            at = (used, rows.len() - 1);
        }
        if c == '\n' {
            rows.push(String::new());
            used = 0;
            continue;
        }
        let w = c.width().unwrap_or(0);
        if used + w > width {
            rows.push(String::new());
            used = 0;
            if byte == cursor {
                at = (0, rows.len() - 1);
            }
        }
        if let Some(row) = rows.last_mut() {
            row.push(c);
        }
        used += w;
    }
    if cursor >= text.len() {
        at = (used, rows.len() - 1);
    }
    (rows, at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Depth;
    use iq_agent::{Event, PERSON};
    use iq_ui::Update;
    use iq_ui::look::Theme;
    use ratatui::backend::TestBackend;
    use ratatui::{Terminal, TerminalOptions, Viewport};

    fn crew(board: &mut Board, history: &mut History, event: Event, p: &Palette) {
        board.update(Update::Crew(event.clone()));
        history.event(&event, board, p);
    }

    /// The rows the live part draws, as text.
    fn drawn(board: &Board, history: &History, width: u16) -> Vec<String> {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut motion = Motion::default();
        let live = Live {
            board,
            history,
            p: &p,
            now: iq_ui::now(),
        };
        let height = live.height(&motion, width);
        let area = Rect::new(0, 0, width, height);
        let mut terminal = Terminal::with_options(
            TestBackend::new(width, height),
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )
        .unwrap();
        terminal
            .draw(|frame| {
                live.draw(frame, &mut motion);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn the_live_part_shows_who_works_what_waits_and_where_to_write() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        board.update(Update::Started {
            lead: "anthropic/claude-opus-5-5".into(),
            reviewer: "r".into(),
            autopilot: true,
            internet: false,
        });
        let rows = drawn(&board, &history, 60);
        assert_eq!(rows.len(), 4);
        assert!(rows[0].contains("new chat · anthropic/claude-opus-5-5"));
        assert!(rows[1].contains("The task for claude-opus-5-5…"));
        assert!(rows[3].starts_with(" ⏵⏵ autopilot"));
        assert!(rows[3].ends_with("you"));
        crew(
            &mut board,
            &mut history,
            Event::Joined {
                agent: "lead".into(),
                model: "anthropic/claude-opus-5-5".into(),
                by: PERSON.into(),
            },
            &p,
        );
        board.screen_mut().talk_to("lead");
        crew(
            &mut board,
            &mut history,
            Event::Working {
                agent: "lead".into(),
                prompt: "go".into(),
            },
            &p,
        );
        crew(
            &mut board,
            &mut history,
            Event::Text {
                agent: "lead".into(),
                text: "Reading the tes".into(),
            },
            &p,
        );
        crew(
            &mut board,
            &mut history,
            Event::Asked {
                id: 1,
                agent: "lead".into(),
                action: "run `git push`".into(),
                reason: "it publishes commits".into(),
                widens: true,
            },
            &p,
        );
        let rows = drawn(&board, &history, 80);
        assert!(
            rows[0].contains("lead · anthropic/claude-opus-5-5 working…"),
            "{}",
            rows[0]
        );
        assert!(rows[0].contains("Esc to stop"));
        assert!(rows[0].ends_with("Reading the tes"));
        assert!(rows[1].contains("lead asks"));
        assert!(rows[2].contains("run `git push`"));
        assert!(
            rows.iter()
                .any(|r| r.contains("Ctrl-Y allow  Ctrl-R refuse"))
        );
        let status = rows.last().unwrap();
        assert!(status.contains("1 waiting for you"));
        assert!(status.ends_with("you ● lead"));
    }

    #[test]
    fn the_folder_of_its_own_the_agent_written_to_works_in_stays_in_sight() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        for agent in ["lead", "other"] {
            crew(
                &mut board,
                &mut history,
                Event::Joined {
                    agent: agent.into(),
                    model: "m".into(),
                    by: PERSON.into(),
                },
                &p,
            );
        }
        crew(
            &mut board,
            &mut history,
            Event::Placed {
                agent: "lead".into(),
                folder: "/data/iq/worktrees/lead".into(),
                said: Some("works in a worktree of its own".into()),
            },
            &p,
        );
        // Written to, its folder shows whole, just over the status line.
        board.screen_mut().talk_to("lead");
        let rows = drawn(&board, &history, 60);
        assert_eq!(rows[rows.len() - 2], "  in /data/iq/worktrees/lead");
        // An agent in the crew's workspace, or a new chat, has no such line.
        board.screen_mut().talk_to("other");
        let others = drawn(&board, &history, 60);
        assert_eq!(others.len(), rows.len() - 1);
        assert!(others.iter().all(|row| !row.contains("worktrees")));
    }

    #[test]
    fn an_agent_at_work_shows_its_latest_steps_and_counts_the_earlier() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::new(
            iq_ui::keys::Keymap::default(),
            iq_ui::look::Look::new(Theme::Graphite).showing(2),
        );
        let mut history = History::default();
        crew(
            &mut board,
            &mut history,
            Event::Joined {
                agent: "lead".into(),
                model: "m".into(),
                by: PERSON.into(),
            },
            &p,
        );
        crew(
            &mut board,
            &mut history,
            Event::Working {
                agent: "lead".into(),
                prompt: "look".into(),
            },
            &p,
        );
        for file in ["a.rs", "b.rs", "c.rs", "d.rs"] {
            crew(
                &mut board,
                &mut history,
                Event::Tool {
                    agent: "lead".into(),
                    name: "read_file".into(),
                    input: file.into(),
                    output: String::new(),
                    ok: true,
                },
                &p,
            );
        }
        let rows = drawn(&board, &history, 80);
        let at = rows.iter().position(|r| r.contains("lead · m")).unwrap();
        assert_eq!(
            rows[at + 1..at + 4],
            [
                "    ⋯ 2 steps before · Ctrl-X unfolds",
                "    ▸ read_file c.rs",
                "    ▸ read_file d.rs",
            ]
        );
        // Its text closes them: they go, counted in the conversation.
        crew(
            &mut board,
            &mut history,
            Event::TextStart {
                agent: "lead".into(),
            },
            &p,
        );
        let rows = drawn(&board, &history, 80);
        assert!(!rows.iter().any(|r| r.contains("read_file")), "{rows:#?}");
        // None shown, as the settings may say: the latest in its line.
        let mut quiet = Board::new(
            iq_ui::keys::Keymap::default(),
            iq_ui::look::Look::new(Theme::Graphite).showing(0),
        );
        for event in [
            Event::Joined {
                agent: "lead".into(),
                model: "m".into(),
                by: PERSON.into(),
            },
            Event::Working {
                agent: "lead".into(),
                prompt: "look".into(),
            },
            Event::Tool {
                agent: "lead".into(),
                name: "read_file".into(),
                input: "a.rs".into(),
                output: String::new(),
                ok: true,
            },
        ] {
            crew(&mut quiet, &mut history, event, &p);
        }
        let rows = drawn(&quiet, &history, 120);
        let line = rows.iter().find(|r| r.contains("lead · m")).unwrap();
        assert!(line.ends_with("▸ read_file a.rs"), "{line}");
        assert!(!rows.iter().any(|r| r.starts_with("    ▸")));
    }

    #[test]
    fn what_is_written_folds_and_the_cursor_follows() {
        assert_eq!(fold("", 0, 10), (vec![String::new()], (0, 0)));
        let (rows, at) = fold("abcdefghij\nxy", 13, 6);
        assert_eq!(rows, ["abcde", "fghij", "xy"]);
        assert_eq!(at, (2, 2));
        let (_, at) = fold("abcdefghij", 5, 6);
        assert_eq!(at, (0, 1));
    }
}
