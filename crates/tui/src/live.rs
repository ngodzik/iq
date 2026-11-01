//! The live part, below the conversation: the agents at work and what
//! they do, each with its latest steps since its last line of text, the
//! oldest question waiting for the person, a word to them, the switcher
//! of their chats while it is open, the commands that complete what they
//! write, where they write, the folder of its own the agent they write to
//! works in, and the status line, along which messages run between the
//! agents.

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

use crate::history::{History, agent_tint, chat_of, label, seconds, short, step_line};
use crate::motion::{Motion, YOU};
use crate::theme::Palette;
use crate::wrap::{cut, spans_width, width as text_width, wrap};

/// How much of a chat's first request the box where the person writes
/// shows, in characters.
const HINT_TASK_CHARS: usize = 48;

/// The most agents of the chat written to given a pane each, as they work
/// side by side.
const PANES: usize = 4;

/// The fewest lines within a pane: its latest steps, then what it writes.
const PANE_LINES: usize = 4;

/// The most lines within a pane.
const PANE_MOST: usize = 14;

/// How much of the screen's height the panes take, in hundredths.
const PANES_SHARE: usize = 45;

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

/// A pane: its title, its lines and its tint.
type Pane = (Line<'static>, Vec<Line<'static>>, crate::theme::Rgb);

/// How many panes go on a row.
fn pane_columns(panes: usize) -> usize {
    panes.clamp(1, 2)
}

/// The rows the panes of `count` agents take at the top of a screen
/// `rows` tall, a share of it, the same whatever they hold, so that the
/// screen is laid out again only as agents come and go or it is resized.
fn panes_height(count: usize, rows: u16) -> u16 {
    if count < 2 {
        return 0;
    }
    let count = count.min(PANES);
    let rows_of_panes = count.div_ceil(pane_columns(count));
    let share = usize::from(rows) * PANES_SHARE / 100 / rows_of_panes;
    let inner = share.saturating_sub(2).clamp(PANE_LINES, PANE_MOST);
    (rows_of_panes * (inner + 2)) as u16
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
    /// The agents of the chat written to at work, its own first.
    fn here(&self) -> Vec<&iq_ui::crew::AgentView> {
        let window = self
            .board
            .screen()
            .talking_to()
            .and_then(|agent| chat_of(self.board, agent));
        let mut here: Vec<_> = self
            .board
            .crew()
            .agents()
            .iter()
            .filter(|a| a.working)
            .filter(|a| window.is_some() && chat_of(self.board, &a.name) == window)
            .collect();
        here.sort_by_key(|a| (Some(&a.name) != window.as_ref(), a.name.clone()));
        here
    }

    /// How many rows at the top of a screen `rows` tall the panes take:
    /// none unless several agents of the chat written to work side by side.
    pub(crate) fn head_height(&self, rows: u16) -> u16 {
        panes_height(self.here().len(), rows)
    }

    /// Draws the panes of the agents of the chat written to at work side
    /// by side, two to a row, as many as [`PANES`], in `frame`, whose area
    /// is the top of the screen.
    pub(crate) fn draw_head(&self, frame: &mut Frame<'_>, motion: &Motion) {
        let here = self.here();
        let count = here.len().min(PANES);
        if count < 2 {
            return;
        }
        let area = frame.area();
        let columns = pane_columns(count);
        let wide = area.width / columns as u16;
        // As tall as the area the screen gave them allows.
        let tall = (area.height / count.div_ceil(columns) as u16).max(3);
        let inner = usize::from(tall - 2);
        let panes: Vec<Pane> = here
            .iter()
            .take(PANES)
            .map(|agent| self.pane(agent, motion, usize::from(wide), inner))
            .collect();
        for (at, (title, lines, tint)) in panes.into_iter().enumerate() {
            let (row, column) = (at / columns, at % columns);
            let x = area.left() + column as u16 * wide;
            // The last of a row takes what the division left.
            let w = if column + 1 == columns {
                area.right().saturating_sub(x)
            } else {
                wide
            };
            let y = area.top() + row as u16 * tall;
            let pane = Rect::new(x, y, w, tall).intersection(area);
            let block = Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(self.p.fg(tint))
                .title(title);
            frame.render_widget(Paragraph::new(lines).block(block), pane);
        }
    }

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
        parts.extend(self.working(motion, width));
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
        if let Some(switcher) = self.switcher(motion, width) {
            parts.push(switcher);
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
    /// of it does not fit. When a plugin gave the place variables, the
    /// line is the command that goes there and sets them, as its commands
    /// get them.
    fn place(&self, width: usize) -> Option<Line<'static>> {
        let p = self.p;
        let agent = self.board.screen().talking_to()?;
        let place = self.board.crew().agent(agent)?.place.as_ref()?;
        let enter = self.board.plugins().enter(place);
        let (lead, after) = match enter {
            Some(line) => ("  cd ", format!(" && {line}")),
            None => ("  in ", String::new()),
        };
        let full = place.display().to_string();
        let mut path = if enter.is_some() {
            shell_word(&full)
        } else {
            full.clone()
        };
        if text_width(lead) + text_width(&path) + text_width(&after) >= width
            && shell_word(&full) == full
        {
            path = short(&full);
        }
        Some(Line::from(vec![
            Span::styled(lead, p.fg(p.faint)),
            Span::styled(path, p.fg(p.dim)),
            Span::styled(after, p.fg(p.dim)),
        ]))
    }

    /// The agents at work. Those of the chat written to: alone, a line
    /// with its latest steps under it; side by side, a pane each, as many
    /// as [`PANES`]. Those of other chats: a line saying which work, their
    /// questions coming to the person all the same.
    fn working(&self, motion: &Motion, width: usize) -> Vec<Part> {
        let p = self.p;
        let here = self.here();
        let elsewhere: Vec<_> = self
            .board
            .crew()
            .agents()
            .iter()
            .filter(|a| a.working && !here.iter().any(|h| h.name == a.name))
            .collect();
        let mut parts = Vec::new();
        match here.len() {
            0 => {}
            1 => parts.push(Part::Lines(self.alone(here[0], motion, width))),
            // Their panes are at the top of the screen.
            more if more > PANES => parts.push(Part::Lines(vec![Line::from(Span::styled(
                format!("  and {} more of this chat at work", more - PANES),
                p.fg(p.faint),
            ))])),
            _ => {}
        }
        if !elsewhere.is_empty() {
            let mut chats: Vec<(String, usize)> = Vec::new();
            for agent in &elsewhere {
                let chat = chat_of(self.board, &agent.name).unwrap_or_else(|| agent.name.clone());
                match chats.iter_mut().find(|(name, _)| *name == chat) {
                    Some((_, count)) => *count += 1,
                    None => chats.push((chat, 1)),
                }
            }
            let mut spans = vec![
                Span::styled(format!("  {} ", motion.spinner()), p.fg(p.faint)),
                Span::styled("also at work: ", p.fg(p.faint)),
            ];
            for (at, (chat, count)) in chats.iter().enumerate() {
                if at > 0 {
                    spans.push(Span::styled(" · ", p.fg(p.faint)));
                }
                spans.push(Span::styled(
                    chat.clone(),
                    p.fg(agent_tint(self.board, chat, p)),
                ));
                if *count > 1 {
                    spans.push(Span::styled(format!(" ({count} agents)"), p.fg(p.faint)));
                }
            }
            let switch = self
                .board
                .keys()
                .chord(Action::Switch)
                .map(|chord| format!("  {} goes there", chord))
                .unwrap_or_default();
            spans.push(Span::styled(switch, p.fg(p.faint)));
            parts.push(Part::Lines(vec![fit(spans, width)]));
        }
        parts
    }

    /// How `agent` is said at work: its spinner, its name in its chat's
    /// window and beside it its model and effort.
    fn titled(&self, agent: &iq_ui::crew::AgentView, motion: &Motion) -> Vec<Span<'static>> {
        let p = self.p;
        let tint = agent_tint(self.board, &agent.name, p);
        let (name, beside) = label(self.board, &agent.name);
        let mut spans = vec![
            Span::styled(format!("{} ", motion.spinner()), p.fg(p.accent)),
            Span::styled(name, p.bold(tint)),
        ];
        if !beside.is_empty() {
            spans.push(Span::styled(format!(" · {beside}"), p.fg(p.dim)));
        }
        spans
    }

    /// For how long `agent` works, and what its turn spent.
    fn lasted(&self, agent: &iq_ui::crew::AgentView) -> Span<'static> {
        let p = self.p;
        let mut after = self
            .history
            .working_for(&agent.name)
            .map(|d| format!(" · {}", seconds(d.as_secs())))
            .unwrap_or_default();
        if let Some(spent) = self.history.turn_spent(&agent.name) {
            after.push_str(&format!(" · {spent}"));
        }
        Span::styled(after, p.fg(p.faint))
    }

    /// What `agent` does now: the line it writes, else its last tool.
    fn doing(&self, agent: &iq_ui::crew::AgentView, steps_shown: bool) -> Option<String> {
        self.history
            .partial(&agent.name)
            .map(|text| text.replace('\n', " "))
            .or_else(|| match agent.thread.last() {
                Some(Entry::Tool { name, input, .. }) if !steps_shown => {
                    Some(format!("▸ {name} {}", short(input).replace('\n', " ")))
                }
                _ => None,
            })
    }

    /// The one agent of the chat written to at work: a line, its latest
    /// steps under it, as many as the settings say, the earlier counted.
    fn alone(
        &self,
        agent: &iq_ui::crew::AgentView,
        motion: &Motion,
        width: usize,
    ) -> Vec<Line<'static>> {
        let p = self.p;
        let mut spans = self.titled(agent, motion);
        spans.push(Span::raw(" "));
        spans.extend(motion.shimmer("working…", p.dim, p.text, p));
        spans.push(self.lasted(agent));
        if Some(agent.name.as_str()) == self.board.screen().talking_to()
            && let Some(stop) = self.board.keys().chord(Action::Stop)
        {
            spans.push(Span::styled(
                format!(" · {} to stop", hint(&stop.to_string())),
                p.fg(p.faint),
            ));
        }
        let shown = self.board.look().steps();
        let fold = self.board.steps().open(&agent.name).filter(|_| shown > 0);
        if let Some(doing) = self.doing(agent, fold.is_some()) {
            let room = width.saturating_sub(spans_width(&spans) + 3);
            if room > 8 {
                spans.push(Span::styled(
                    format!("  {}", cut(&doing, room)),
                    p.fg(p.dim),
                ));
            }
        }
        let mut lines = vec![Line::from(spans)];
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
        lines
    }

    /// A pane of an agent of the chat written to, as several work side by
    /// side: its title, then its latest steps and what it writes, within
    /// `width` columns and `lines` lines.
    fn pane(
        &self,
        agent: &iq_ui::crew::AgentView,
        motion: &Motion,
        width: usize,
        lines_in: usize,
    ) -> Pane {
        let p = self.p;
        let inner = width.saturating_sub(4);
        let mut title = vec![Span::raw(" ")];
        title.extend(self.titled(agent, motion));
        title.push(self.lasted(agent));
        title.push(Span::raw(" "));
        let fold = self.board.steps().open(&agent.name);
        let doing = self.doing(agent, fold.is_some());
        let room = lines_in.max(1) - usize::from(doing.is_some());
        let mut lines: Vec<Line<'static>> = Vec::new();
        if let Some(fold) = fold {
            let earlier = fold.steps.len().saturating_sub(room);
            if earlier > 0 {
                lines.push(Line::from(Span::styled(
                    format!(" ⋯ {earlier} before"),
                    p.fg(p.faint),
                )));
            }
            let left = room - lines.len();
            for step in fold
                .steps
                .iter()
                .skip(fold.steps.len().saturating_sub(left))
            {
                let (mut first, body) = step_line(step, p);
                // Within the pane, the step's own indent is the border's.
                first.remove(0);
                first.insert(0, Span::raw(" "));
                first.extend(body);
                lines.push(fit(first, inner));
            }
        }
        match doing {
            Some(doing) => lines.push(Line::from(Span::styled(
                format!(" {}", cut(&doing, inner.saturating_sub(1))),
                p.fg(p.text),
            ))),
            None => {
                let mut said = vec![Span::raw(" ")];
                said.extend(motion.shimmer("working…", p.dim, p.text, p));
                lines.push(Line::from(said));
            }
        }
        (
            Line::from(title),
            lines,
            agent_tint(self.board, &agent.name, p),
        )
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
        // Named as in its chat's window when it is this one's; else by its
        // whole name, its chat's first.
        let window = self
            .board
            .screen()
            .talking_to()
            .and_then(|agent| chat_of(self.board, agent));
        let theirs = chat_of(self.board, &question.agent);
        let (who, from) = if theirs.is_some() && theirs == window {
            (label(self.board, &question.agent).0, String::new())
        } else {
            (question.agent.clone(), " · from another chat".to_owned())
        };
        Some(Part::Boxed {
            title: Line::from(vec![
                Span::raw(" "),
                Span::styled(who, p.bold(tint)),
                Span::styled(" asks", p.fg(p.warn)),
                Span::styled(format!("{from} "), p.fg(p.faint)),
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

    /// The person's chats while the switcher is open, the one used last
    /// first, as many as it shows at once: each with whether it works,
    /// what it was first asked and when the person last used it; under
    /// them, the keys, and how many more there are above and below.
    fn switcher(&self, motion: &Motion, width: usize) -> Option<Part> {
        let p = self.p;
        let switcher = self.board.switcher()?;
        let shown = switcher
            .chats
            .iter()
            .enumerate()
            .skip(switcher.first)
            .take(iq_ui::SWITCH_ROWS);
        let name_width = switcher
            .chats
            .iter()
            .map(|(view, _)| text_width(&view.name))
            .max()
            .unwrap_or(0)
            + 2;
        let here = self.board.screen().talking_to();
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (at, (view, used)) in shown {
            let on = at == switcher.chosen;
            let tint = agent_tint(self.board, &view.name, p);
            let state = if view.working {
                Span::styled(format!("{} ", motion.spinner()), p.fg(p.accent))
            } else {
                Span::styled("· ", p.fg(p.faint))
            };
            let when = match used {
                Some(at) => iq_ui::ago(*at, self.now),
                None => String::new(),
            };
            let when = if Some(view.name.as_str()) == here {
                "here".to_owned()
            } else {
                when
            };
            let task = view
                .task
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            // Within the box's borders: its mark and state before the
            // name, a space at least before when, and one after.
            let room = width.saturating_sub(2 + 5 + name_width + text_width(&when) + 2);
            let gap = room.saturating_sub(text_width(&cut(&task, room)));
            lines.push(Line::from(vec![
                Span::styled(if on { " ▸ " } else { "   " }, p.fg(p.accent)),
                state,
                Span::styled(
                    format!("{:<name_width$}", view.name),
                    if on { p.bold(tint) } else { p.fg(tint) },
                ),
                Span::styled(cut(&task, room), p.fg(if on { p.text } else { p.dim })),
                Span::raw(" ".repeat(gap + 1)),
                Span::styled(when, p.fg(p.faint)),
            ]));
        }
        let key = self
            .board
            .keys()
            .chord(Action::Switch)
            .map(ToString::to_string)
            .unwrap_or_default();
        let mut foot = vec![
            Span::raw("   "),
            Span::styled(key, p.bold(p.accent)),
            Span::styled(" or ↑ ↓ to choose · ", p.fg(p.dim)),
            Span::styled("Enter", p.bold(p.accent)),
            Span::styled(" to go there · ", p.fg(p.dim)),
            Span::styled("Esc", p.bold(p.accent)),
        ];
        let above = switcher.first;
        let below = switcher
            .chats
            .len()
            .saturating_sub(switcher.first + iq_ui::SWITCH_ROWS);
        let more: Vec<String> = [(above, "↑"), (below, "↓")]
            .iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, arrow)| format!("{count} more {arrow}"))
            .collect();
        if !more.is_empty() {
            foot.push(Span::styled(
                format!(" · {}", more.join(", ")),
                p.fg(p.faint),
            ));
        }
        lines.push(Line::from(foot));
        Some(Part::Boxed {
            title: Line::from(vec![
                Span::raw(" "),
                Span::styled("Chats", p.bold(p.accent)),
                Span::styled(
                    format!(" · {} · the one used last first ", switcher.chats.len()),
                    p.fg(p.faint),
                ),
            ]),
            lines,
            tint: p.accent,
        })
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
                    // What it was first asked tells the chat from another.
                    match view.map(|v| v.task.trim()).filter(|task| !task.is_empty()) {
                        Some(task) => {
                            let asked: String = task
                                .lines()
                                .next()
                                .unwrap_or_default()
                                .chars()
                                .take(HINT_TASK_CHARS)
                                .collect();
                            format!("Write to {agent}, first asked “{asked}”…")
                        }
                        None => format!("Write to {agent}…"),
                    },
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
                // The name the person gave it, before it starts.
                let named = chat.name.trim();
                let (title, hint) = if named.is_empty() {
                    (
                        vec![Span::styled(" new chat", p.bold(p.accent))],
                        format!("The task for {short_model}…"),
                    )
                } else {
                    (
                        vec![
                            Span::styled(format!(" {named}"), p.bold(p.accent)),
                            Span::styled(" · new chat", p.fg(p.faint)),
                        ],
                        format!("The first task of {named}, for {short_model}…"),
                    )
                };
                let mut title = title;
                title.push(Span::styled(said, p.fg(p.faint)));
                (Line::from(title), Line::default(), hint)
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
        // What the person's plugins show: where localhost leads, say.
        for status in self.board.plugins().statuses() {
            left.push(sep());
            left.push(Span::styled(status.to_owned(), p.fg(p.info)));
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

/// `text` as one word of a shell's line: as it is when nothing in it
/// means more to the shell, else between single quotes.
fn shell_word(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+=:@%".contains(c));
    if plain {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
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

    /// The rows the panes draw at the top of the screen, as text.
    fn headed(board: &Board, history: &History, width: u16) -> Vec<String> {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let motion = Motion::default();
        let live = Live {
            board,
            history,
            p: &p,
            now: iq_ui::now(),
        };
        let height = live.head_height(30);
        if height == 0 {
            return Vec::new();
        }
        let area = Rect::new(0, 0, width, height);
        let mut terminal = Terminal::with_options(
            TestBackend::new(width, height),
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )
        .unwrap();
        terminal
            .draw(|frame| live.draw_head(frame, &motion))
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
        // Named before it starts, the new chat shows it at once.
        board.run(
            iq_ui::command::Command::Name("billing".into()),
            iq_ui::now(),
        );
        let named = drawn(&board, &history, 60);
        assert!(
            named
                .iter()
                .any(|r| r.contains("The new chat is named billing")),
            "{named:#?}"
        );
        assert!(
            named.iter().any(|r| r.contains("billing · new chat")),
            "{named:#?}"
        );
        assert!(
            named
                .iter()
                .any(|r| r.contains("The first task of billing")),
            "{named:#?}"
        );
        board.run(iq_ui::command::Command::New(None), iq_ui::now());
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
        // In its chat's window, the agent is named by its model.
        assert!(rows[0].contains("claude-opus-5-5 working…"), "{}", rows[0]);
        assert!(rows[0].contains("Esc to stop"));
        assert!(rows[0].ends_with("Reading the tes"));
        // Its chat's own agent, by its model, in its window.
        assert!(rows[1].contains("claude-opus-5-5 asks"), "{}", rows[1]);
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
        // A plugin that gave the place variables makes it the line that
        // goes there and sets them; what it shows is on the status line.
        for told in [
            iq_ui::plugins::Told::Enter {
                folder: "/data/iq/worktrees/lead".into(),
                line: Some("eval \"$(iq-ports env)\"".into()),
            },
            iq_ui::plugins::Told::Status("localhost → lead".into()),
        ] {
            board.update(Update::Plugin {
                plugin: "ports".into(),
                told,
            });
        }
        board.screen_mut().talk_to("lead");
        let rows = drawn(&board, &history, 80);
        assert_eq!(
            rows[rows.len() - 2],
            "  cd /data/iq/worktrees/lead && eval \"$(iq-ports env)\""
        );
        assert!(rows[rows.len() - 1].contains("· localhost → lead"));
    }

    #[test]
    fn the_switcher_shows_the_chats_used_last_in_a_box_of_its_own() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        let chats = [
            ("login", "fix the login redirect"),
            ("refunds", "code the refunds of the payment module"),
            ("docs", "translate the docs"),
            ("ci", "make the CI faster"),
            ("api", "version the API"),
            ("tests", "flaky tests"),
            ("theme", "a dark theme"),
        ];
        for (agent, task) in chats {
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
            crew(
                &mut board,
                &mut history,
                Event::Working {
                    agent: agent.into(),
                    prompt: task.into(),
                },
                &p,
            );
        }
        for agent in ["docs", "refunds", "login"] {
            board.run(iq_ui::command::Command::To(agent.into()), iq_ui::now());
        }
        board.press(
            &iq_ui::keys::Chord::parse("ctrl+space").unwrap(),
            None,
            iq_ui::now(),
        );
        let rows = drawn(&board, &history, 72);
        let at = rows.iter().position(|r| r.contains("Chats · 7")).unwrap();
        assert!(
            rows[at].starts_with("╭ Chats · 7 · the one used last first"),
            "{rows:#?}"
        );
        assert!(
            rows[at + 1].contains("login") && rows[at + 1].ends_with("here │"),
            "{rows:#?}"
        );
        assert!(
            rows[at + 2].contains("▸") && rows[at + 2].contains("refunds"),
            "{rows:#?}"
        );
        assert!(
            rows[at + 2].contains("code the refunds of the payment module"),
            "{rows:#?}"
        );
        assert!(rows[at + 2].contains("just now"), "{rows:#?}");
        assert!(
            rows[at + 6].contains("Ctrl-Space or ↑ ↓ to choose"),
            "{rows:#?}"
        );
        assert!(rows[at + 6].contains("2 more ↓"), "{rows:#?}");
        assert!(rows[at + 7].starts_with("╰"), "{rows:#?}");
    }

    #[test]
    fn agents_of_the_chat_at_work_side_by_side_have_a_pane_each() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        for (agent, model, by) in [
            ("api", "lab/sage", PERSON),
            ("api/tests", "lab/cheap", "api"),
            ("api/review", "lab/sage", "api"),
            ("docs", "lab/sage", PERSON),
        ] {
            crew(
                &mut board,
                &mut history,
                Event::Joined {
                    agent: agent.into(),
                    model: model.into(),
                    by: by.into(),
                },
                &p,
            );
            crew(
                &mut board,
                &mut history,
                Event::Working {
                    agent: agent.into(),
                    prompt: "go".into(),
                },
                &p,
            );
        }
        crew(
            &mut board,
            &mut history,
            Event::Effort {
                agent: "api/tests".into(),
                effort: Some(iq_core::Effort::Low),
            },
            &p,
        );
        crew(
            &mut board,
            &mut history,
            Event::Tool {
                agent: "api/tests".into(),
                name: "run".into(),
                input: "pytest -q".into(),
                output: String::new(),
                ok: true,
            },
            &p,
        );
        board.screen_mut().talk_to("api");
        // A share of the screen: on a taller one, taller panes, as far as
        // a most.
        assert_eq!(panes_height(4, 30), 12);
        assert_eq!(panes_height(4, 50), 22);
        assert_eq!(panes_height(2, 50), 16);
        assert_eq!(panes_height(1, 50), 0);
        // At the top of the screen, two to a row, each six rows tall.
        let head = headed(&board, &history, 90);
        let joined = head.join("\n");
        assert_eq!(head.len(), 12, "{joined}");
        // The chat's own agent first, by its model; those it started by
        // their part, each with its model and effort.
        assert!(head[0].starts_with("╭ ⠋ sage"), "{joined}");
        assert!(head[0].contains("╭ ⠋ review · sage"), "{joined}");
        assert!(head[6].starts_with("╭ ⠋ tests · cheap · low"), "{joined}");
        assert!(head[7].contains("▸ run pytest -q"), "{joined}");
        // Not in the live part, where another chat at work is said in a
        // line, to go to.
        let rows = drawn(&board, &history, 90).join("\n");
        assert!(!rows.contains("╭ ⠋"), "{rows}");
        assert!(
            rows.contains("also at work: docs  Ctrl-Space goes there"),
            "{rows}"
        );
    }

    #[test]
    fn a_path_is_one_word_of_the_shell_quoted_only_when_it_must_be() {
        assert_eq!(
            shell_word("/data/iq/worktrees/lead-2"),
            "/data/iq/worktrees/lead-2"
        );
        assert_eq!(shell_word("/Users/Jo Doe/w"), "'/Users/Jo Doe/w'");
        assert_eq!(shell_word("/w/it's"), "'/w/it'\\''s'");
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
        // Written to, its chat's window is on the terminal.
        board.screen_mut().talk_to("lead");
        let rows = drawn(&board, &history, 80);
        let at = rows.iter().position(|r| r.contains("m working…")).unwrap();
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
        quiet.screen_mut().talk_to("lead");
        let rows = drawn(&quiet, &history, 120);
        let line = rows.iter().find(|r| r.contains("m working…")).unwrap();
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
