//! The conversation as the terminal keeps it: what the crew does, written
//! once into the terminal's own scrollback as it happens, where the person
//! scrolls, selects and copies it as any output. Each agent's text is
//! written a line at a time as it comes, under the agent's name when
//! another spoke last; its messages and the end of its turns each take a
//! line. What it does between two lines of text (its tools, what was
//! allowed or refused it) shows in the live part as it comes; once it goes
//! on, only their count is written, the steps unfolded on their page.
//!
//! What agents write to each other (the task one starts another on, a
//! message, an answer) is written as a few lines, numbered as their page
//! numbers it, where it is given whole: summed up by a cheap model when
//! the settings give one, else its first lines. An agent another started
//! writes in its pane while it works, not here, unless the person wrote to
//! it: what it answers comes as a message.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use iq_agent::{Event, PERSON};
use iq_core::{TokenCount, Usd};
use iq_ui::Board;
use iq_ui::keys::Action;
use iq_ui::steps::{Fold, Step};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::diagram::{Diagram, Draw, Image, Shown};
use crate::markdown::{Markdown, Out, Styled, code, inline};
use crate::summary::{Summarize, Summary};
use crate::table::Table;
use crate::theme::Palette;
use crate::wrap::{cut, fit, wrap};

/// How many lines of a message between agents are written, when no model
/// sums it up.
const MESSAGE_LINES: usize = 4;

/// The longest line of a message written, in characters: a paragraph
/// written as one line is cut there.
const LINE_CHARS: usize = 200;

/// How many of a chat's last exchanges of an earlier session are written
/// again as the person comes back to it: as far back as a scrollback is
/// worth filling.
const RECALLED: usize = 40;

/// How many lines of a failed tool's output are written.
const FAILED_LINES: usize = 3;

/// How many lines of a file's change are written; the conversation iq
/// draws unfolds the rest in place.
const CHANGE_LINES: usize = 12;

/// The longest tool input written, in characters.
pub(crate) const INPUT_CHARS: usize = 160;

/// The longest question the person answered written among the steps, in
/// characters: its answer comes after it.
const QUESTION_CHARS: usize = 100;

/// How many kinds of step a fold's count names.
const KINDS: usize = 4;

/// How many paragraphs of each chat's window are kept, to write it again
/// as the person comes back to it, or when the terminal is made narrower.
const KEPT: usize = 4000;

/// A paragraph waiting to be written: folded to the terminal's width only
/// then.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Para {
    first: Vec<Span<'static>>,
    rest: Vec<Span<'static>>,
    body: Vec<Span<'static>>,
}

impl Para {
    fn blank() -> Para {
        Para {
            first: Vec::new(),
            rest: Vec::new(),
            body: Vec::new(),
        }
    }

    /// `body` under `indent`, on every line.
    fn indented(indent: &str, body: Vec<Span<'static>>) -> Para {
        Para {
            first: vec![Span::raw(indent.to_owned())],
            rest: vec![Span::raw(indent.to_owned())],
            body,
        }
    }

    /// A styled line of an agent's text, under its block's indent.
    fn text(styled: Styled) -> Para {
        let indent = || Span::raw("  ");
        Para {
            first: std::iter::once(indent()).chain(styled.first).collect(),
            rest: std::iter::once(indent()).chain(styled.rest).collect(),
            body: styled.body,
        }
    }
}

/// What a window holds: a paragraph, folded at the width it is written at;
/// a line of code, written as it is; a table, drawn at it; a diagram, its
/// source written as code unless it is drawn, with the styles of what is
/// said of it.
#[derive(Debug, Clone)]
pub(crate) enum Block {
    Para(Para),
    /// The head of a turn: an agent's name, or what the person wrote. The
    /// conversation iq draws goes from one to the next.
    Head {
        para: Para,
        person: bool,
    },
    /// A line of an agent's text, folded as a paragraph is.
    Text(Para),
    Whole(Line<'static>),
    /// One row, cut to the width: a step said in a few words.
    Row(Vec<Span<'static>>),
    /// What stands for the steps' page's entry `id` (a fold of steps, a
    /// message between agents, a job that ended), which the conversation
    /// iq draws unfolds in place.
    Entry {
        id: usize,
        blocks: Vec<Block>,
    },
    Table(Table),
    Diagram {
        diagram: Arc<Diagram>,
        code: Vec<Block>,
        failed: Style,
    },
    /// A diagram shown folded, as the settings say: `line`, what it is,
    /// until the person unfolds it while it is the last thing written;
    /// once anything comes after it, that line for good.
    Folded {
        diagram: Arc<Diagram>,
        code: Vec<Block>,
        failed: Style,
        line: Vec<Span<'static>>,
    },
    /// A file an agent changed: `head`, its name and how many lines came
    /// and went, then its lines changed with a few around them, numbered,
    /// the first [`CHANGE_LINES`] until the person unfolds them, how many
    /// more said in `faint`; `diff`, what it is copied as.
    Change {
        head: Vec<Span<'static>>,
        rows: Vec<Vec<Span<'static>>>,
        faint: Style,
        diff: String,
    },
    /// A command run in the background, in one line never folded: `head`,
    /// then what it does in a few words (the summarizer's, else `instead`,
    /// read from the line), then `tail`.
    Job {
        head: Vec<Span<'static>>,
        label: Label,
        tail: Vec<Span<'static>>,
    },
    /// A message between agents summed up, under `indent`, in `style`; its
    /// first lines, `instead`, until it is, or when it cannot be.
    Summed {
        summary: Arc<Summary>,
        instead: Vec<Para>,
        indent: String,
        style: Style,
    },
}

/// Whose turn a head begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Speaker {
    Person,
    Agent,
}

impl Block {
    /// A styled line of an agent's text: code at the line's start, whole,
    /// the rest under its block's indent.
    fn text(styled: Styled) -> Block {
        if styled.whole {
            Block::Whole(Line::from(
                styled
                    .first
                    .into_iter()
                    .chain(styled.body)
                    .collect::<Vec<_>>(),
            ))
        } else {
            Block::Text(Para::text(styled))
        }
    }

    /// What it says, as it was written, for the person to copy: a line of
    /// text without its indent nor folds, a line of code whole, a table's
    /// rows, a diagram's source, a message's summary or its first lines.
    pub(crate) fn written(&self) -> String {
        let spans = |spans: &[Span<'static>]| -> String {
            spans.iter().map(|span| span.content.as_ref()).collect()
        };
        match self {
            Block::Para(para) | Block::Text(para) | Block::Head { para, .. } => spans(&para.body),
            Block::Whole(line) => line.to_string(),
            Block::Row(row) => spans(row).trim().to_owned(),
            Block::Table(table) => table
                .lines(usize::MAX / 4, &[])
                .iter()
                .map(|line| line.to_string().trim_end().to_owned())
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Diagram { code, .. } | Block::Folded { code, .. } => code
                .iter()
                .map(Block::written)
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Job { head, label, tail } => {
                format!("{}{}{}", spans(head), label.text(), spans(tail))
                    .trim()
                    .to_owned()
            }
            Block::Change { diff, .. } => diff.clone(),
            Block::Summed {
                summary, instead, ..
            } => summary.text().unwrap_or_else(|| {
                instead
                    .iter()
                    .map(|para| spans(&para.body))
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
            Block::Entry { blocks, .. } => blocks
                .iter()
                .map(Block::written)
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    /// Whether it is the head of an agent's turn, of the person's, or
    /// neither.
    pub(crate) fn head(&self) -> Option<Speaker> {
        match self {
            Block::Head { person: true, .. } => Some(Speaker::Person),
            Block::Head { person: false, .. } => Some(Speaker::Agent),
            _ => None,
        }
    }

    /// A file's change whole, every line of it, within `width` columns;
    /// none for anything else.
    pub(crate) fn change_whole(&self, width: usize) -> Option<Vec<Line<'static>>> {
        let Block::Change { head, rows, .. } = self else {
            return None;
        };
        Some(
            std::iter::once(head)
                .chain(rows)
                .map(|row| fit(row.clone(), width))
                .collect(),
        )
    }

    /// Whether it is a line left empty.
    pub(crate) fn blank(&self) -> bool {
        matches!(self, Block::Para(para) if para.first.iter().chain(&para.body).all(|span| span.content.trim().is_empty()))
    }

    /// Whether it is a diagram being drawn, which what comes after it waits
    /// for.
    fn drawing(&self) -> bool {
        match self {
            Block::Diagram { diagram, .. } => diagram.drawing(),
            Block::Entry { blocks, .. } => blocks.iter().any(Block::drawing),
            Block::Summed { summary, .. } => summary.making(),
            Block::Job { label, .. } => label.making(),
            Block::Para(_)
            | Block::Change { .. }
            | Block::Head { .. }
            | Block::Text(_)
            | Block::Whole(_)
            | Block::Row(_)
            | Block::Table(_)
            | Block::Folded { .. } => false,
        }
    }

    /// Its lines within `width` columns, on cells of `cell` pixels, the
    /// images they show put in `images`.
    pub(crate) fn lines(
        &self,
        width: usize,
        cell: (u16, u16),
        images: &mut Vec<Image>,
    ) -> Vec<Line<'static>> {
        let indent = [Span::raw("  ")];
        match self {
            Block::Para(para) | Block::Text(para) | Block::Head { para, .. } => {
                wrap(&para.body, width, &para.first, &para.rest)
            }
            Block::Whole(line) => vec![line.clone()],
            Block::Row(spans) | Block::Folded { line: spans, .. } => {
                vec![fit(spans.clone(), width)]
            }
            Block::Entry { blocks, .. } => blocks
                .iter()
                .flat_map(|block| block.lines(width, cell, images))
                .collect(),
            // Too long, the hint to unfold it goes first, then what it does
            // is cut: how it ended always shows.
            Block::Job { head, label, tail } => {
                let used = |spans: &[Span<'static>]| crate::wrap::spans_width(spans);
                let said = label.span();
                let mut tail = tail.clone();
                if tail.len() > 3
                    && width < used(head) + crate::wrap::width(&said.content) + used(&tail)
                {
                    tail.truncate(3);
                }
                let room = width
                    .saturating_sub(used(head) + used(&tail))
                    .max(LABEL_LEAST);
                let mut spans = head.clone();
                spans.push(Span::styled(cut(&said.content, room), said.style));
                spans.extend(tail);
                vec![fit(spans, width)]
            }
            Block::Table(table) => table.lines(width, &indent),
            Block::Change {
                head, rows, faint, ..
            } => {
                let mut lines = vec![fit(head.clone(), width)];
                lines.extend(
                    rows.iter()
                        .take(CHANGE_LINES)
                        .map(|row| fit(row.clone(), width)),
                );
                if rows.len() > CHANGE_LINES {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(
                            format!("… {} more lines", rows.len() - CHANGE_LINES),
                            *faint,
                        ),
                    ]));
                }
                lines
            }
            Block::Summed {
                summary,
                instead,
                indent,
                style,
            } => match summary.text() {
                Some(said) => said
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .flat_map(|line| {
                        let at = [Span::raw(indent.clone())];
                        wrap(&[Span::styled(line.to_owned(), *style)], width, &at, &at)
                    })
                    .collect(),
                None => instead
                    .iter()
                    .flat_map(|para| wrap(&para.body, width, &para.first, &para.rest))
                    .collect(),
            },
            Block::Diagram {
                diagram,
                code,
                failed,
            } => match diagram.shown(width, &indent, cell, images) {
                Shown::Image(lines) => lines,
                Shown::Source(why) => {
                    let mut lines: Vec<Line<'static>> = Vec::new();
                    for block in code {
                        lines.extend(block.lines(width, cell, images));
                    }
                    if let Some(why) = why {
                        lines.extend(wrap(
                            &[Span::styled(why, *failed)],
                            width,
                            &[Span::raw("  "), Span::styled("! ", *failed)],
                            &[Span::raw("    ")],
                        ));
                    }
                    lines
                }
            },
        }
    }
}

/// What draws diagrams, in a terminal that shows them: none elsewhere, or
/// as the settings say.
#[derive(Clone, Default)]
struct Drawer(Option<Arc<dyn Draw>>);

impl fmt::Debug for Drawer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_some() { "Drawer" } else { "None" })
    }
}

/// What sums up the messages between agents, when the settings give a
/// model for it.
#[derive(Clone, Default)]
struct Summarizer(Option<Arc<dyn Summarize>>);

impl fmt::Debug for Summarizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_some() {
            "Summarizer"
        } else {
            "None"
        })
    }
}

/// An agent's text being written.
#[derive(Debug, Default)]
struct Stream {
    /// What came after the last whole line.
    partial: String,
    markdown: Markdown,
}

/// An agent's turn: since when, and what it spent.
#[derive(Debug)]
struct Turn {
    since: Instant,
    billed: Usd,
    tokens: u64,
}

/// What a paragraph of a window is, for the lines between two kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blank,
    Text,
    Steps,
    Other,
}

/// The conversation, as written so far: a window of its own for each of
/// the person's chats, where what it and the agents it started do is
/// written, the window of the chat written to on the terminal.
#[derive(Debug, Default)]
pub(crate) struct History {
    /// The blocks waiting to be written, each with the chat whose window it
    /// goes in: none for the window on the terminal.
    paras: Vec<(Option<String>, Block)>,
    /// What each chat's window holds, the latest last, as many as
    /// [`KEPT`]: none is the window of a new chat.
    kept: HashMap<Option<String>, VecDeque<Block>>,
    /// How many of each window's oldest blocks were let go: the number of
    /// its first kept.
    dropped: HashMap<Option<String>, usize>,
    /// Whether iq draws the conversation itself, on the other screen.
    drawn: bool,
    /// The lines iq opens with, at the top of every window.
    opening: Vec<Para>,
    /// Whether the window on the terminal shows them.
    opened: bool,
    /// The chat whose window what is being written goes in.
    at: Option<String>,
    /// Whose lines each window wrote last: the person, an agent, or
    /// nobody's since a line of the crew's own.
    speakers: HashMap<Option<String>, String>,
    /// What each window wrote last.
    kinds: HashMap<Option<String>, Kind>,
    streams: HashMap<String, Stream>,
    turns: HashMap<String, Turn>,
    /// The agents whose task was written.
    tasked: HashSet<String>,
    /// What to say after the person's next message to an agent: the models
    /// they named in it.
    after: HashMap<String, String>,
    /// The files each agent changed since its steps were last written,
    /// written under them.
    changes: HashMap<String, Vec<Block>>,
    /// The number of each agent's last fold of steps written.
    told: HashMap<String, usize>,
    /// Where the line saying an agent joined waits to be written: one that
    /// comes back from an earlier session did not join now.
    joins: HashMap<String, usize>,
    /// Whether a link is written as its text alone, a hyperlink.
    links: bool,
    /// What draws diagrams.
    drawer: Drawer,
    /// The size of a cell in pixels, for the images.
    cell: (u16, u16),
    /// The images the lines written last show, for the terminal to be sent
    /// before them.
    images: Vec<Image>,
    /// Where the next image takes its id with the terminal.
    image_ids: Arc<AtomicU32>,
    /// Whether what follows a diagram being drawn waits for it: not as iq
    /// ends.
    hurried: bool,
    /// The commands left to run in the background, by number: what they
    /// do in a few words, and since when they run.
    jobs: HashMap<u64, (Label, Instant)>,
    /// What sums up the messages between agents.
    summarizer: Summarizer,
    /// The summaries of the messages between agents, by their numbers.
    summaries: HashMap<usize, Arc<Summary>>,
    /// The agents another started that the person wrote to: what they
    /// write until their turn ends is theirs to read.
    woken: HashSet<String>,
    /// The model the turns of a thread written again answered on, as it
    /// says: their heads name it rather than the chat's model now.
    recalled_on: Option<String>,
}

impl History {
    /// Writes a link as its text alone, a hyperlink, when `links`.
    pub(crate) fn set_links(&mut self, links: bool) {
        self.links = links;
    }

    /// Draws diagrams with `draw`, when given, as images on cells of `cell`
    /// pixels. Their ids with the terminal start anywhere, so that they
    /// take none of an earlier session's, whose images the scrollback
    /// still shows.
    pub(crate) fn pictures(&mut self, draw: Option<Arc<dyn Draw>>, cell: (u16, u16)) {
        self.drawer = Drawer(draw);
        self.set_cell(cell);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| {
                d.subsec_nanos() ^ u32::try_from(d.as_secs() & 0xff_ffff).unwrap_or(1)
            });
        self.image_ids.store(now & 0x00ff_ffff, Ordering::Relaxed);
    }

    /// Sums up the messages between agents with `summarize`, when given;
    /// else their first lines are written.
    pub(crate) fn summaries(&mut self, summarize: Option<Arc<dyn Summarize>>) {
        self.summarizer = Summarizer(summarize);
    }

    /// The summary of the message numbered `id`, once made.
    pub(crate) fn summed(&self, id: usize) -> Option<String> {
        self.summaries.get(&id).and_then(|summary| summary.text())
    }

    /// The size of a cell is now `cell` pixels, as the terminal says; one
    /// it does not say leaves it as it was.
    pub(crate) fn set_cell(&mut self, cell: (u16, u16)) {
        if cell.0 > 0 && cell.1 > 0 {
            self.cell = cell;
        } else if self.cell == (0, 0) {
            self.cell = (8, 16);
        }
    }

    /// The images the lines written last show, to be sent first.
    pub(crate) fn take_images(&mut self) -> Vec<Image> {
        std::mem::take(&mut self.images)
    }

    /// The lines iq opens with: who leads, where, and how to begin.
    pub(crate) fn opening(&mut self, lead: &str, reviewer: &str, p: &Palette) {
        let folder = home_short(
            &std::env::current_dir()
                .map(|f| f.display().to_string())
                .unwrap_or_default(),
        );
        self.opening.push(Para::indented(
            " ",
            vec![
                Span::styled("i", p.bold(p.accent)),
                Span::styled("q", p.bold(p.accent.mix(p.info, 0.6))),
                Span::styled("  experimental", p.fg(p.dim)),
                Span::styled(
                    format!(" · lead {lead} · autopilot {reviewer}"),
                    p.fg(p.faint),
                ),
            ],
        ));
        self.opening.push(Para::indented(
            "    ",
            vec![Span::styled(
                format!(
                    "{folder} · Enter sends · / for commands · @ for an agent · Ctrl-S lists the keys"
                ),
                p.fg(p.faint),
            )],
        ));
    }

    /// What the crew just did, as `board` holds it now that it took it.
    pub(crate) fn event(&mut self, event: &Event, board: &Board, p: &Palette) {
        // Written in the window of the chat it is part of.
        self.at = match event {
            Event::Posted { from, to, .. } if from == PERSON => chat_of(board, to),
            Event::Posted { from, .. } => chat_of(board, from),
            Event::Renamed { to, .. } => chat_of(board, to),
            other => agent_of(other).and_then(|agent| chat_of(board, agent)),
        };
        match event {
            Event::Joined { agent, model, by } if by != PERSON => {
                self.set_speaker(None);
                self.joins.insert(agent.clone(), self.paras.len());
                let tint = agent_tint(board, agent, p);
                let starter = self.shown_name(board, by);
                self.push(Para::indented(
                    "",
                    vec![
                        Span::styled("◦ ", p.fg(tint)),
                        Span::styled(self.shown_name(board, agent), p.fg(tint)),
                        Span::styled(
                            format!(" joins, on {model}, started by {starter}"),
                            p.fg(p.dim),
                        ),
                    ],
                ));
            }
            // The box where the person writes says how hard its agent
            // thinks, and on which model; a model changed is said where it
            // happens too, as a choice the conversation keeps.
            Event::Joined { .. } | Event::Effort { .. } => {}
            Event::Model { agent, model } => {
                self.set_speaker(None);
                let tint = agent_tint(board, agent, p);
                self.push(Para::indented(
                    "",
                    vec![
                        Span::styled("◦ ", p.fg(tint)),
                        Span::styled(self.shown_name(board, agent), p.fg(tint)),
                        Span::styled(
                            format!(
                                " answers on {model} from its next turn, as you chose with /model"
                            ),
                            p.fg(p.dim),
                        ),
                    ],
                ));
            }
            Event::Working { agent, prompt } => {
                self.turns.insert(
                    agent.clone(),
                    Turn {
                        since: Instant::now(),
                        billed: Usd(0.0),
                        tokens: 0,
                    },
                );
                // Later turns are woken by messages, already written.
                if !self.tasked.insert(agent.clone()) {
                    return;
                }
                let by = board.crew().agent(agent).map(|a| a.by.clone());
                match by {
                    Some(by) if by != PERSON => {
                        self.header(agent, board, p);
                        let id = board
                            .steps()
                            .latest_said()
                            .filter(|said| said.task && said.to == *agent)
                            .map(|said| said.id);
                        let mut head = vec![Span::styled("↳ its task", p.fg(p.info))];
                        head.extend(numbered(id, p));
                        self.push(Para::indented("  ", head));
                        self.message(&by, agent, prompt, id, p);
                    }
                    _ => self.person(prompt, agent, board, p),
                }
            }
            Event::TextStart { agent } => self.finish(agent, board, p),
            // An agent another started writes in its pane, unless the
            // person wrote to it: what it answers comes as a message.
            Event::Text { agent, .. } if self.in_pane(agent, board) => {}
            Event::Text { agent, text } => {
                let links = self.links;
                let stream = self.streams.entry(agent.clone()).or_insert_with(|| Stream {
                    partial: String::new(),
                    markdown: Markdown::linking(links),
                });
                stream.partial.push_str(text);
                while let Some(stream) = self.streams.get_mut(agent)
                    && let Some(end) = stream.partial.find('\n')
                {
                    let line: String = stream.partial.drain(..=end).collect();
                    let out = stream.markdown.line(line.trim_end_matches(['\n', '\r']), p);
                    self.write(agent, out, board, p);
                }
            }
            Event::Spent {
                agent, usage, cost, ..
            } => {
                if let Some(turn) = self.turns.get_mut(agent) {
                    if let Some(cost) = cost {
                        turn.billed += *cost;
                    }
                    turn.tokens += usage.input.0 + usage.output.0;
                }
            }
            // A step: the board keeps it in the agent's fold, which the live
            // part shows as it grows. What the agent wrote so far comes
            // before it. A question waits in its box with its choices, and
            // becomes a step once answered; iq's notice is a step too.
            Event::Tool { agent, .. } => {
                self.finish(agent, board, p);
                // A file changed: the fold it closed is written now, with
                // the change.
                if self
                    .changes
                    .get(agent)
                    .is_some_and(|changes| !changes.is_empty())
                {
                    self.folds(agent, board, p);
                }
            }
            Event::Changed { agent, path, diff } => {
                if !self.in_pane(agent, board) {
                    let change = change(path, diff, p);
                    self.changes.entry(agent.clone()).or_default().push(change);
                }
            }
            Event::Reviewed { agent, .. }
            | Event::Choose { agent, .. }
            | Event::Notice { agent, .. } => {
                self.finish(agent, board, p);
            }
            Event::Chosen { .. } => {}
            Event::Asked { .. } | Event::Answered { .. } => {}
            // A job's end, told its agent by iq: the job's own line says it,
            // never its whole command.
            Event::Posted { from, text, .. } if from == IQ && text.starts_with("Job ") => {}
            Event::Posted { from, to, text } => {
                if from == PERSON {
                    if board.crew().agent(to).is_some_and(|view| view.by != PERSON) {
                        self.woken.insert(to.clone());
                    }
                    self.person(text, to, board, p);
                } else if to == PERSON {
                    self.finish(from, board, p);
                    self.header(from, board, p);
                    self.push(Para::indented(
                        "  ",
                        vec![Span::styled("✉ to you", p.fg(p.info))],
                    ));
                    let mut markdown = Markdown::linking(self.links);
                    for line in text.lines() {
                        let out = markdown.line(line, p);
                        self.write(from, out, board, p);
                    }
                    let out = markdown.flush(p);
                    self.write(from, out, board, p);
                } else {
                    self.finish(from, board, p);
                    self.folds(from, board, p);
                    self.set_speaker(None);
                    let (writer, reader) =
                        (self.shown_name(board, from), self.shown_name(board, to));
                    let id = board
                        .steps()
                        .latest_said()
                        .filter(|said| !said.task && said.from == *from && said.to == *to)
                        .map(|said| said.id);
                    let mut head = vec![
                        Span::styled("✉ ", p.fg(p.info)),
                        Span::styled(writer, p.fg(agent_tint(board, from, p))),
                        Span::styled(" → ", p.fg(p.faint)),
                        Span::styled(reader, p.fg(agent_tint(board, to, p))),
                    ];
                    head.extend(numbered(id, p));
                    self.push(Para::indented("  ", head));
                    self.message(from, to, text, id, p);
                }
            }
            Event::Done { agent, outcome } => {
                self.woken.remove(agent);
                self.finish(agent, board, p);
                self.header(agent, board, p);
                self.blank();
                let turn = self.turns.remove(agent);
                match outcome {
                    Ok(_) => {
                        let mut said = String::new();
                        if let Some(turn) = turn {
                            said.push_str(&seconds(turn.since.elapsed().as_secs()));
                            if turn.billed.0 > 0.0 {
                                said.push_str(&format!(" · {}", turn.billed));
                            }
                            if turn.tokens > 0 {
                                said.push_str(&format!(" · {} tokens", TokenCount(turn.tokens)));
                            }
                        }
                        self.push(Para::indented(
                            "  ",
                            vec![
                                Span::styled("✔ ", p.fg(p.ok)),
                                Span::styled(said, p.fg(p.faint)),
                            ],
                        ));
                    }
                    Err(error) => self.push(Para {
                        first: vec![Span::raw("  "), Span::styled("✘ stopped: ", p.fg(p.bad))],
                        rest: vec![Span::raw("    ")],
                        body: vec![Span::styled(short(error), p.fg(p.dim))],
                    }),
                }
                self.set_speaker(None);
            }
            // A command left to run, and how it ended: the live part says
            // meanwhile for how long it has run.
            // A job in a line as it starts and as it ends, never its whole
            // command: the steps' page gives it, with its output's end.
            Event::JobStarted { agent, id, line } => {
                self.finish(agent, board, p);
                self.header(agent, board, p);
                let label = Label::new(agent, line, self.summarizer.0.as_ref(), p);
                self.jobs.insert(*id, (label.clone(), Instant::now()));
                self.push_job(Block::Job {
                    head: vec![
                        Span::raw("  "),
                        Span::styled("⚙ ", p.fg(p.info)),
                        Span::styled(format!("job {id} · "), p.fg(p.dim)),
                    ],
                    label,
                    tail: Vec::new(),
                });
            }
            Event::JobOutput { .. } => {}
            Event::JobEnded {
                agent, id, said, ..
            } => {
                self.header(agent, board, p);
                let (label, since) = self
                    .jobs
                    .remove(id)
                    .unwrap_or_else(|| (Label::new(agent, "", None, p), Instant::now()));
                let (ended, colour) = match said.strip_prefix("exit status ") {
                    Some("0") => ("ended 0".to_owned(), p.ok),
                    Some(code) => (format!("ended {code}"), p.warn),
                    None => (said.clone(), p.warn),
                };
                let mut tail = vec![
                    Span::styled(" · ", p.fg(p.faint)),
                    Span::styled(ended, p.fg(colour)),
                    Span::styled(
                        format!(" · {}", seconds(since.elapsed().as_secs())),
                        p.fg(p.faint),
                    ),
                ];
                let unfold = board.keys().chord(Action::Steps);
                let ran = board.steps().latest_ran().map(|ran| ran.id);
                if let (Some(ran), Some(unfold)) = (ran, unfold) {
                    tail.push(Span::styled(
                        format!("  {unfold} unfolds #{ran}"),
                        p.fg(p.faint),
                    ));
                }
                let job = Block::Job {
                    head: vec![
                        Span::raw("  "),
                        Span::styled("⚙ ", p.fg(colour)),
                        Span::styled(format!("job {id} · "), p.fg(p.dim)),
                    ],
                    label,
                    tail,
                };
                self.push_job(match ran {
                    Some(id) => Block::Entry {
                        id,
                        blocks: vec![job],
                    },
                    None => job,
                });
            }
            // A line once it waits for those it started: how long, the live
            // part says.
            Event::Waiting { agent, on } => {
                self.finish(agent, board, p);
                self.header(agent, board, p);
                let names: Vec<String> =
                    on.iter().map(|name| self.shown_name(board, name)).collect();
                self.push_step(Para::indented(
                    "  ",
                    vec![
                        Span::styled("⧗ ", p.fg(p.info)),
                        Span::styled(format!("waiting for {}", names.join(", ")), p.fg(p.dim)),
                    ],
                ));
            }
            // Its turn's end says it, with the facts; the agents line
            // counts those retired.
            Event::Failed { .. }
            | Event::Retired { .. }
            | Event::Turn { .. }
            | Event::Window { .. } => {}
            Event::Placed {
                agent,
                said: Some(said),
                ..
            } => {
                self.finish(agent, board, p);
                self.header(agent, board, p);
                self.push(Para {
                    first: vec![Span::raw("  "), Span::styled("! ", p.fg(p.warn))],
                    rest: vec![Span::raw("    ")],
                    body: vec![Span::styled(short(&format!("{agent} {said}")), p.fg(p.dim))],
                });
            }
            Event::Placed { said: None, .. } => {}
            Event::Renamed { from, to } => {
                let renamed = |name: &mut String| {
                    if name == from {
                        name.clone_from(to);
                    }
                };
                for speaker in self.speakers.values_mut() {
                    renamed(speaker);
                }
                // A chat's window follows it.
                let (old, new) = (Some(from.clone()), Some(to.clone()));
                if let Some(speaker) = self.speakers.remove(&old) {
                    self.speakers.insert(new.clone(), speaker);
                }
                if let Some(kept) = self.kept.remove(&old) {
                    self.kept.insert(new.clone(), kept);
                }
                if let Some(dropped) = self.dropped.remove(&old) {
                    self.dropped.insert(new.clone(), dropped);
                }
                if let Some(kind) = self.kinds.remove(&old) {
                    self.kinds.insert(new.clone(), kind);
                }
                for (chat, _) in &mut self.paras {
                    if *chat == old {
                        chat.clone_from(&new);
                    }
                }
                if self.at == old {
                    self.at.clone_from(&new);
                }
                if let Some(stream) = self.streams.remove(from) {
                    self.streams.insert(to.clone(), stream);
                }
                if let Some(turn) = self.turns.remove(from) {
                    self.turns.insert(to.clone(), turn);
                }
                if self.tasked.remove(from) {
                    self.tasked.insert(to.clone());
                }
                if let Some(after) = self.after.remove(from) {
                    self.after.insert(to.clone(), after);
                }
                if let Some(told) = self.told.remove(from) {
                    self.told.insert(to.clone(), told);
                }
                self.set_speaker(None);
                self.push(Para::indented(
                    "  ",
                    vec![
                        Span::styled(from.clone(), p.fg(p.dim)),
                        Span::styled(" is called ", p.fg(p.faint)),
                        Span::styled(to.clone(), p.fg(agent_tint(board, to, p))),
                        Span::styled(" now", p.fg(p.faint)),
                    ],
                ));
            }
        }
    }

    /// A chat of an earlier session came back: what it was given then was
    /// written then, and its next turn is woken by what the person writes.
    pub(crate) fn back(&mut self, agent: &str) {
        self.tasked.insert(agent.to_owned());
        if let Some(at) = self.joins.remove(agent)
            && at < self.paras.len()
        {
            self.paras.remove(at);
            for later in self.joins.values_mut() {
                if *later > at {
                    *later -= 1;
                }
            }
        }
    }

    /// Writes again a chat of an earlier session the person comes back to,
    /// as it was shown: what was asked of it, its text as the Markdown it
    /// is, its steps folded, what was asked of the person and answered,
    /// the messages, the ends of its turns; from its last [`RECALLED`]
    /// exchanges, the earlier counted, and where its whole thread is. As
    /// iq opens on it, says that it is the chat written to last.
    pub(crate) fn recall(&mut self, recall: &iq_ui::Recall, board: &Board, p: &Palette) {
        let agent = recall.agent.as_str();
        let Some(view) = board.crew().agent(agent) else {
            return;
        };
        self.at = chat_of(board, agent);
        self.set_speaker(None);
        self.named(agent, board, p);
        let mut said = String::from("↺ back");
        if let Some(worked) = view.worked {
            said.push_str(&format!(", at work {}", iq_ui::ago(worked, iq_ui::now())));
        }
        let team = board
            .keys()
            .chord(Action::Team)
            .map_or("/chats".to_owned(), |chord| chord.to_string());
        said.push_str(&format!(" · its whole thread: {team}, then →"));
        self.push(Para::indented("  ", vec![Span::styled(said, p.fg(p.info))]));
        let asked: Vec<usize> = view
            .thread
            .iter()
            .enumerate()
            .filter(|(_, entry)| matches!(entry, iq_ui::crew::Entry::Prompt(_)))
            .map(|(at, _)| at)
            .collect();
        let skipped = asked.len().saturating_sub(RECALLED);
        let from = if skipped > 0 { asked[skipped] } else { 0 };
        if skipped > 0 {
            self.push(Para::indented(
                "  ",
                vec![Span::styled(
                    format!(
                        "… {skipped} exchange{} before",
                        if skipped == 1 { "" } else { "s" }
                    ),
                    p.fg(p.faint),
                )],
            ));
        }
        // The board made the thread's steps into folds as it came back,
        // told at once, so that none is said again as a new one: each is
        // written where it ends, as live.
        let recalled = board
            .steps()
            .folds()
            .filter(|fold| fold.agent == agent && fold.begins.is_some())
            .map(|fold| fold.id)
            .max();
        if let Some(last) = recalled {
            let told = self.told.entry(agent.to_owned()).or_default();
            *told = (*told).max(last);
        }
        let mut fold: Option<usize> = None;
        // The model of the first turn written, as the thread said before.
        self.recalled_on = view.thread[..from]
            .iter()
            .rev()
            .find_map(|entry| match entry {
                iq_ui::crew::Entry::Model(model) => Some(model.clone()),
                _ => None,
            });
        for (at, entry) in view.thread.iter().enumerate().skip(from) {
            use iq_ui::crew::Entry;
            if let Some(begun) = board.steps().begun(agent, at) {
                fold = Some(begun.id);
            }
            if matches!(
                entry,
                Entry::Tool { .. }
                    | Entry::Reviewed { .. }
                    | Entry::Asked { .. }
                    | Entry::Chose { .. }
                    | Entry::Notice(_)
            ) {
                continue;
            }
            self.recalled_fold(agent, fold.take(), board, p);
            match entry {
                Entry::Prompt(text) => {
                    let text = text.trim();
                    match text
                        .strip_prefix("Message from ")
                        .and_then(|rest| rest.split_once(": "))
                    {
                        Some((PERSON, said)) => self.person(said, agent, board, p),
                        // A job's end: its own line said it.
                        Some((IQ, said)) if said.starts_with("Job ") => {}
                        Some((from, said)) => {
                            self.set_speaker(None);
                            self.push(Para::indented(
                                "  ",
                                vec![
                                    Span::styled("✉ ", p.fg(p.info)),
                                    Span::styled(
                                        self.shown_name(board, from),
                                        p.fg(agent_tint(board, from, p)),
                                    ),
                                    Span::styled(" → ", p.fg(p.faint)),
                                    Span::styled(
                                        self.shown_name(board, agent),
                                        p.fg(agent_tint(board, agent, p)),
                                    ),
                                ],
                            ));
                            self.lines_of(said, "    ", p, MESSAGE_LINES);
                        }
                        None => self.person(text, agent, board, p),
                    }
                }
                Entry::Text(text) => {
                    let mut markdown = Markdown::linking(self.links);
                    for line in text.lines() {
                        let out = markdown.line(line, p);
                        self.write(agent, out, board, p);
                    }
                    let out = markdown.flush(p);
                    self.write(agent, out, board, p);
                }
                Entry::Message { from, to, text } => {
                    if to == PERSON {
                        self.header(from, board, p);
                        self.push(Para::indented(
                            "  ",
                            vec![Span::styled("✉ to you", p.fg(p.info))],
                        ));
                        let mut markdown = Markdown::linking(self.links);
                        for line in text.lines() {
                            let out = markdown.line(line, p);
                            self.write(from, out, board, p);
                        }
                        let out = markdown.flush(p);
                        self.write(from, out, board, p);
                    } else if from != agent || to != agent {
                        self.set_speaker(None);
                        self.push(Para::indented(
                            "  ",
                            vec![
                                Span::styled("✉ ", p.fg(p.info)),
                                Span::styled(
                                    self.shown_name(board, from),
                                    p.fg(agent_tint(board, from, p)),
                                ),
                                Span::styled(" → ", p.fg(p.faint)),
                                Span::styled(
                                    self.shown_name(board, to),
                                    p.fg(agent_tint(board, to, p)),
                                ),
                            ],
                        ));
                        self.lines_of(text, "    ", p, MESSAGE_LINES);
                    }
                }
                Entry::Done(outcome) => {
                    self.header(agent, board, p);
                    self.blank();
                    match outcome {
                        Ok(_) => {
                            self.push(Para::indented("  ", vec![Span::styled("✔", p.fg(p.ok))]))
                        }
                        Err(error) => self.push(Para {
                            first: vec![Span::raw("  "), Span::styled("✘ stopped: ", p.fg(p.bad))],
                            rest: vec![Span::raw("    ")],
                            body: vec![Span::styled(short(error), p.fg(p.dim))],
                        }),
                    }
                    self.set_speaker(None);
                }
                // The turns from here answered on another model: the next
                // line of the agent's says so under a head of its own.
                Entry::Model(model) => {
                    if self.recalled_on.as_ref() != Some(model) {
                        self.recalled_on = Some(model.clone());
                        self.set_speaker(None);
                    }
                }
                Entry::Tool { .. }
                | Entry::Reviewed { .. }
                | Entry::Asked { .. }
                | Entry::Chose { .. }
                | Entry::Notice(_) => {}
            }
        }
        self.recalled_fold(agent, fold, board, p);
        self.recalled_on = None;
        if recall.opening {
            let others = board.crew().chats().count().saturating_sub(1);
            let mut said = format!("you write to {agent}, the chat you wrote to last");
            if others > 0 {
                said.push_str(&format!(
                    "; {team} or /chats lists the {others} other{}",
                    if others == 1 { "" } else { "s" }
                ));
            }
            self.push(Para::indented(
                "  ",
                vec![Span::styled(said, p.fg(p.accent))],
            ));
        }
        self.set_speaker(None);
    }

    /// Writes the fold `id` of a chat coming back, under its agent's name,
    /// as a fold is written once its agent goes on.
    fn recalled_fold(&mut self, agent: &str, id: Option<usize>, board: &Board, p: &Palette) {
        if let Some(fold) = id.and_then(|id| board.steps().fold(id)) {
            self.header(agent, board, p);
            self.fold(fold, board, p);
        }
    }

    /// Something said to the person by iq itself, in passing.
    pub(crate) fn say(&mut self, text: &str, p: &Palette) {
        self.at = None;
        self.set_speaker(None);
        self.push(Para::indented(
            "  ",
            vec![Span::styled(text.to_owned(), p.fg(p.dim))],
        ));
    }

    /// Says `text` right after the person's next message to `agent`.
    pub(crate) fn after_message(&mut self, agent: &str, text: String) {
        self.after.insert(agent.to_owned(), text);
    }

    /// What the person wrote, to `agent`, after what it did before.
    fn person(&mut self, text: &str, agent: &str, board: &Board, p: &Palette) {
        self.folds(agent, board, p);
        self.blank();
        let mut body = Vec::new();
        // In the chat's window, written to its own agent but for one it
        // started.
        if board
            .crew()
            .agent(agent)
            .is_some_and(|view| view.by != PERSON)
        {
            body.push(Span::styled(
                format!("@{} ", label(board, agent).0),
                p.fg(agent_tint(board, agent, p)),
            ));
        }
        body.extend(inline(text.trim(), p.fg(p.text), p, self.links));
        self.push_block(Block::Head {
            para: Para {
                first: vec![Span::styled("› ", p.bold(p.accent))],
                rest: vec![Span::raw("  ")],
                body,
            },
            person: true,
        });
        if let Some(said) = self.after.remove(agent) {
            self.push(Para::indented("  ", vec![Span::styled(said, p.fg(p.info))]));
        }
        self.set_speaker(Some(PERSON.to_owned()));
    }

    /// The first lines of `text`, dimmed, after `first`.
    fn lines_of(&mut self, text: &str, first: &str, p: &Palette, most: usize) {
        for para in first_lines(text, first, p, most) {
            self.push(para);
        }
    }

    /// What `from` wrote to `to`, numbered `id` on the steps' page, under
    /// its head: whole when it is short; else summed up, when a model sums
    /// messages up, what comes after it waiting a little for it; else its
    /// first lines.
    fn message(&mut self, from: &str, to: &str, text: &str, id: Option<usize>, p: &Palette) {
        const INDENT: &str = "    ";
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let short = lines.len() <= MESSAGE_LINES
            && lines.iter().all(|line| line.chars().count() <= LINE_CHARS);
        let said = match (&self.summarizer.0, id) {
            (Some(summarize), Some(id)) if !short => {
                let summary = Summary::new(from, to, text, Arc::clone(summarize));
                self.summaries.insert(id, Arc::clone(&summary));
                vec![Block::Summed {
                    summary,
                    instead: first_lines(text, INDENT, p, MESSAGE_LINES),
                    indent: INDENT.to_owned(),
                    style: p.fg(p.dim),
                }]
            }
            _ => first_lines(text, INDENT, p, MESSAGE_LINES)
                .into_iter()
                .map(Block::Para)
                .collect(),
        };
        match id {
            Some(id) => self.push_block(Block::Entry { id, blocks: said }),
            None => said.into_iter().for_each(|block| self.push_block(block)),
        }
    }

    /// Whether `agent`'s text goes in its pane rather than its chat's
    /// window: another agent started it, and the person did not write to
    /// it.
    fn in_pane(&self, agent: &str, board: &Board) -> bool {
        board
            .crew()
            .agent(agent)
            .is_some_and(|view| view.by != PERSON)
            && !self.woken.contains(agent)
    }

    /// `agent`'s name and model, when another spoke last; then what it
    /// did since its last line.
    fn header(&mut self, agent: &str, board: &Board, p: &Palette) {
        self.named(agent, board, p);
        self.folds(agent, board, p);
    }

    /// `agent`'s folds of steps closed since the last written, under its
    /// name: a fold of one step as that step, a larger one as its count.
    fn folds(&mut self, agent: &str, board: &Board, p: &Palette) {
        self.folds_of(agent, board, p, false);
    }

    /// The same, the fold being made with them when `all`, as iq ends.
    fn folds_of(&mut self, agent: &str, board: &Board, p: &Palette, all: bool) {
        let told = self.told.get(agent).copied().unwrap_or(0);
        let mut new: Vec<&Fold> = board
            .steps()
            .folds()
            .rev()
            .take_while(|fold| fold.id > told)
            .filter(|fold| fold.agent == agent && (fold.closed || all))
            .collect();
        if new.is_empty() {
            return;
        }
        new.reverse();
        self.told
            .insert(agent.to_owned(), new.last().map_or(told, |fold| fold.id));
        self.named(agent, board, p);
        let changes = self.changes.remove(agent).unwrap_or_default();
        let last = new.len() - 1;
        for (at, fold) in new.into_iter().enumerate() {
            // The one step that changed a file: the change stands for it.
            let alone = at == last
                && !changes.is_empty()
                && matches!(fold.steps.as_slice(), [Step::Tool { ok: true, .. }]);
            if !alone {
                self.fold(fold, board, p);
            }
        }
        for change in changes {
            self.push_step_block(change);
        }
    }

    /// Writes `fold`: of one step, that step; of more, their count and the
    /// key that unfolds them.
    fn fold(&mut self, fold: &Fold, board: &Board, p: &Palette) {
        let blocks = match fold.steps.as_slice() {
            [] => return,
            [step @ Step::Tool { output, ok, .. }] => {
                let (first, body) = step_line(step, p);
                let mut blocks = vec![Block::Para(Para {
                    first,
                    rest: vec![Span::raw("    ")],
                    body,
                })];
                if !ok {
                    blocks.extend(
                        output
                            .lines()
                            .filter(|l| !l.trim().is_empty())
                            .take(FAILED_LINES)
                            .map(|line| {
                                Block::Para(Para::indented(
                                    "    ",
                                    vec![Span::styled(
                                        cut(&short(line), LINE_CHARS),
                                        p.fg(p.faint),
                                    )],
                                ))
                            }),
                    );
                }
                blocks
            }
            // A verdict, an answer, a question, a notice: one row, whole
            // on the steps' page.
            [step] => {
                let (mut first, body) = step_line(step, p);
                first.extend(body);
                vec![Block::Row(first)]
            }
            _ => {
                let unfold = board
                    .keys()
                    .chord(Action::Steps)
                    .map(|chord| chord.to_string());
                vec![Block::Para(Para {
                    first: vec![Span::raw("  "), Span::styled("▸ ", p.fg(p.faint))],
                    rest: vec![Span::raw("    ")],
                    body: counted(fold, unfold.as_deref(), p),
                })]
            }
        };
        self.push_step_block(Block::Entry {
            id: fold.id,
            blocks,
        });
    }

    /// `agent`'s name and model, when another spoke last.
    fn named(&mut self, agent: &str, board: &Board, p: &Palette) {
        if self.speaker() == Some(agent) {
            return;
        }
        let tint = agent_tint(board, agent, p);
        // In its chat's window: by its model, or its part and its model.
        let (name, beside) = label_on(board, agent, self.recalled_on.as_deref());
        let beside = if beside.is_empty() {
            String::new()
        } else {
            format!("  {beside}")
        };
        self.blank();
        self.push_block(Block::Head {
            para: Para::indented(
                "",
                vec![
                    Span::styled("● ", p.fg(tint)),
                    Span::styled(name, p.bold(tint)),
                    Span::styled(beside, p.fg(p.faint)),
                ],
            ),
            person: false,
        });
        self.set_speaker(Some(agent.to_owned()));
    }

    /// Writes what is left of `agent`'s text after its last whole line,
    /// and what its styling held: a table that ended it.
    fn finish(&mut self, agent: &str, board: &Board, p: &Palette) {
        let Some(stream) = self.streams.get_mut(agent) else {
            return;
        };
        let rest = std::mem::take(&mut stream.partial);
        let mut out = if rest.trim().is_empty() {
            Vec::new()
        } else {
            stream.markdown.line(&rest, p)
        };
        out.extend(stream.markdown.flush(p));
        self.write(agent, out, board, p);
    }

    /// Writes what `agent`'s text gave, under its name: lines, a table, a
    /// diagram, which is drawn if anything draws diagrams.
    fn write(&mut self, agent: &str, out: Vec<Out>, board: &Board, p: &Palette) {
        for out in out {
            // An empty line of text leaves one between two of its
            // paragraphs: none after the agent's name or its steps, as a
            // model's text that is only new lines would, and never two.
            if let Out::Line(styled) = &out
                && styled.is_empty()
            {
                if self.speaker() == Some(agent) && self.kind() == Some(Kind::Text) {
                    self.blank();
                }
                continue;
            }
            self.header(agent, board, p);
            match out {
                Out::Line(styled) => self.push_text_block(Block::text(styled)),
                Out::Table(table) => self.push_text_block(Block::Table(table)),
                Out::Diagram(source) => {
                    let code = code("mermaid", &source, p)
                        .into_iter()
                        .map(Block::text)
                        .collect();
                    let said = diagram_said(&source);
                    let diagram =
                        Diagram::new(source, Arc::clone(&self.image_ids), self.drawer.0.clone());
                    // Folded, when the settings say and it can be drawn.
                    let block = if board.look().diagrams_folded() && self.drawer.0.is_some() {
                        Block::Folded {
                            diagram,
                            code,
                            failed: p.fg(p.warn),
                            line: vec![
                                Span::raw("  "),
                                Span::styled("▸ ", p.fg(p.faint)),
                                Span::styled(format!("diagram · {said}"), p.fg(p.dim)),
                            ],
                        }
                    } else {
                        Block::Diagram {
                            diagram,
                            code,
                            failed: p.fg(p.warn),
                        }
                    };
                    self.push_text_block(block);
                }
            }
        }
    }

    /// Writes what every agent left unfinished, as iq ends: a diagram still
    /// being drawn is no longer waited for.
    pub(crate) fn finish_all(&mut self, board: &Board, p: &Palette) {
        self.hurried = true;
        let agents: Vec<String> = self.streams.keys().cloned().collect();
        for agent in agents {
            self.finish(&agent, board, p);
        }
        let agents: Vec<String> = board
            .crew()
            .agents()
            .iter()
            .map(|a| a.name.clone())
            .collect();
        for agent in agents {
            self.folds_of(&agent, board, p, true);
        }
    }

    /// How long `agent`'s turn has lasted, while it works.
    pub(crate) fn working_for(&self, agent: &str) -> Option<std::time::Duration> {
        self.turns.get(agent).map(|turn| turn.since.elapsed())
    }

    /// What `agent`'s turn spent so far, as a status says it: dollars
    /// billed, else the tokens of calls whose price is unknown.
    pub(crate) fn turn_spent(&self, agent: &str) -> Option<String> {
        let turn = self.turns.get(agent)?;
        if turn.billed.0 > 0.0 {
            Some(turn.billed.to_string())
        } else if turn.tokens > 0 {
            Some(format!("{} tokens", TokenCount(turn.tokens)))
        } else {
            None
        }
    }

    /// The line `agent` is writing, not whole yet.
    pub(crate) fn partial(&self, agent: &str) -> Option<&str> {
        self.streams
            .get(agent)
            .map(|s| s.partial.trim())
            .filter(|s| !s.is_empty())
    }

    /// Whether lines wait to be written.
    pub(crate) fn waiting(&self) -> bool {
        !self.paras.is_empty() || !self.opened && !self.opening.is_empty()
    }

    /// The lines waiting for `window`'s chat, or for a new chat's when
    /// none, folded within `width` columns; those of other chats are kept
    /// in their windows, written as the person comes back to them.
    pub(crate) fn take(&mut self, width: usize, window: Option<&str>) -> Vec<Line<'static>> {
        self.joins.clear();
        let window = window.map(str::to_owned);
        let mut lines = Vec::new();
        if !self.opened {
            self.opened = true;
            lines.extend(
                self.opening
                    .iter()
                    .flat_map(|para| wrap(&para.body, width, &para.first, &para.rest)),
            );
        }
        let mut queued = std::mem::take(&mut self.paras).into_iter();
        while let Some((given, block)) = queued.next() {
            let chat = given.clone().or_else(|| window.clone());
            // A diagram being drawn holds back what its window writes after
            // it, all of it, so that the conversation keeps its order.
            if chat == window && !self.hurried && block.drawing() {
                self.paras.push((given, block));
                self.paras.extend(queued);
                break;
            }
            // A diagram folded waits as the last thing its window writes,
            // for the person to unfold it.
            let last = !queued
                .as_slice()
                .iter()
                .any(|(other, _)| other.clone().or_else(|| window.clone()) == chat);
            if chat == window
                && !self.hurried
                && !self.drawn
                && last
                && matches!(block, Block::Folded { .. })
            {
                self.paras.push((given, block));
                continue;
            }
            if chat == window {
                lines.extend(block.lines(width, self.cell, &mut self.images));
            }
            let kept = self.kept.entry(chat.clone()).or_default();
            kept.push_back(block);
            while kept.len() > KEPT {
                kept.pop_front();
                *self.dropped.entry(chat.clone()).or_default() += 1;
            }
        }
        lines
    }

    /// The line of the diagram folded as the last thing `window` writes,
    /// while it waits for the person to unfold it.
    pub(crate) fn folded(&self, window: Option<&str>) -> Option<Vec<Span<'static>>> {
        let window = window.map(str::to_owned);
        match self
            .paras
            .iter()
            .rev()
            .find(|(chat, _)| chat.clone().or_else(|| window.clone()) == window)
        {
            Some((_, Block::Folded { line, .. })) => Some(line.clone()),
            _ => None,
        }
    }

    /// Unfolds the diagram folded as the last thing `window` writes: its
    /// image is written next, once drawn, for good.
    pub(crate) fn unfold(&mut self, window: Option<&str>) {
        let window = window.map(str::to_owned);
        let last = self
            .paras
            .iter_mut()
            .rev()
            .find(|(chat, _)| chat.clone().or_else(|| window.clone()) == window);
        if let Some((_, block)) = last
            && let Block::Folded {
                diagram,
                code,
                failed,
                ..
            } = block
        {
            *block = Block::Diagram {
                diagram: Arc::clone(diagram),
                code: std::mem::take(code),
                failed: *failed,
            };
        }
    }

    /// Makes what the new chat's window holds `chat`'s, as the chat starts
    /// there, when it has no window of its own yet. Whether it did.
    pub(crate) fn adopt(&mut self, chat: Option<&str>) -> bool {
        let Some(chat) = chat else {
            return false;
        };
        let key = Some(chat.to_owned());
        if self.kept.get(&key).is_some_and(|kept| !kept.is_empty()) {
            return false;
        }
        let mut kept = self.kept.remove(&None).unwrap_or_default();
        kept.extend(self.kept.remove(&key).unwrap_or_default());
        self.kept.insert(key.clone(), kept);
        let dropped = self.dropped.remove(&None).unwrap_or_default();
        self.dropped.insert(key.clone(), dropped);
        if let Some(kind) = self.kinds.remove(&None) {
            self.kinds.insert(key.clone(), kind);
        }
        if let Some(speaker) = self.speakers.remove(&None) {
            self.speakers.insert(key, speaker);
        }
        true
    }

    /// Whether iq draws the conversation itself, on the other screen: a
    /// diagram folded then waits for nothing, unfolded with a click.
    pub(crate) fn set_drawn(&mut self, drawn: bool) {
        self.drawn = drawn;
    }

    /// The blocks `window` keeps, the oldest first, and the number of the
    /// first: a block's number stays its own as the oldest are let go.
    pub(crate) fn blocks(&self, window: Option<&str>) -> (usize, Option<&VecDeque<Block>>) {
        let key = window.map(str::to_owned);
        (
            self.dropped.get(&key).copied().unwrap_or_default(),
            self.kept.get(&key),
        )
    }

    /// What iq opens with, folded within `width` columns.
    pub(crate) fn opening_lines(&self, width: usize) -> Vec<Line<'static>> {
        self.opening
            .iter()
            .flat_map(|para| wrap(&para.body, width, &para.first, &para.rest))
            .collect()
    }

    /// The size of a cell in pixels, as the terminal said it.
    pub(crate) fn cell(&self) -> (u16, u16) {
        self.cell
    }

    /// `window`'s chat whole, as the person comes to it, folded within
    /// `width` columns: what iq opens with, then all its window kept.
    pub(crate) fn window(&mut self, width: usize, window: Option<&str>) -> Vec<Line<'static>> {
        // What waits for it first, kept with the rest.
        let _ = self.take(width, window);
        self.opened = true;
        let key = window.map(str::to_owned);
        let mut lines: Vec<Line<'static>> = self
            .opening
            .iter()
            .flat_map(|para| wrap(&para.body, width, &para.first, &para.rest))
            .collect();
        let blocks: Vec<Block> = self.kept.get(&key).into_iter().flatten().cloned().collect();
        for block in blocks {
            lines.extend(block.lines(width, self.cell, &mut self.images));
        }
        lines
    }

    /// The last of the lines written in `window`, as many as `rows`,
    /// folded within `width` columns.
    pub(crate) fn recent(
        &mut self,
        width: usize,
        rows: usize,
        window: Option<&str>,
    ) -> Vec<Line<'static>> {
        let key = window.map(str::to_owned);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for block in self.kept.get(&key).into_iter().flatten().rev() {
            let mut folded = block.lines(width, self.cell, &mut self.images);
            folded.append(&mut lines);
            lines = folded;
            if lines.len() >= rows {
                break;
            }
        }
        let extra = lines.len().saturating_sub(rows);
        lines.split_off(extra)
    }

    /// How `agent` is named in the window being written: as [`label`]
    /// names it when it is of that window's chat, else by its whole name.
    fn shown_name(&self, board: &Board, agent: &str) -> String {
        if self.at.is_some() && chat_of(board, agent) == self.at {
            label(board, agent).0
        } else {
            agent.to_owned()
        }
    }

    /// Puts `para` in the window being written.
    fn push(&mut self, para: Para) {
        self.push_block(Block::Para(para));
    }

    /// Puts `block` in the window being written.
    fn push_block(&mut self, block: Block) {
        self.kinds.insert(self.at.clone(), Kind::Other);
        self.paras.push((self.at.clone(), block));
    }

    /// Puts what an agent wrote in the window, a line left between it and
    /// the steps before.
    fn push_text_block(&mut self, block: Block) {
        if self.kind() == Some(Kind::Steps) {
            self.blank();
        }
        self.push_block(block);
        self.kinds.insert(self.at.clone(), Kind::Text);
    }

    /// Puts a job's line in the window, as a step.
    fn push_job(&mut self, block: Block) {
        if self.kind() == Some(Kind::Text) {
            self.blank();
        }
        self.push_block(block);
        self.kinds.insert(self.at.clone(), Kind::Steps);
    }

    /// What the job `id` does in a few words, while it runs.
    pub(crate) fn job_label(&self, id: u64) -> Option<String> {
        self.jobs.get(&id).map(|(label, _)| label.text())
    }

    /// Puts a line of an agent's steps in the window, a line left between
    /// it and the text before.
    fn push_step(&mut self, para: Para) {
        self.push_step_block(Block::Para(para));
    }

    /// The same, of any block.
    fn push_step_block(&mut self, block: Block) {
        if self.kind() == Some(Kind::Text) {
            self.blank();
        }
        self.push_block(block);
        self.kinds.insert(self.at.clone(), Kind::Steps);
    }

    /// Leaves a line empty in the window, one at most between two others.
    fn blank(&mut self) {
        if self.kind() == Some(Kind::Blank) {
            return;
        }
        self.push(Para::blank());
        self.kinds.insert(self.at.clone(), Kind::Blank);
    }

    /// What the window being written wrote last; none yet when empty.
    fn kind(&self) -> Option<Kind> {
        self.kinds.get(&self.at).copied()
    }

    /// Whose lines the window being written wrote last.
    fn speaker(&self) -> Option<&str> {
        self.speakers.get(&self.at).map(String::as_str)
    }

    fn set_speaker(&mut self, speaker: Option<String>) {
        match speaker {
            Some(speaker) => self.speakers.insert(self.at.clone(), speaker),
            None => self.speakers.remove(&self.at),
        };
    }
}

/// What a Mermaid diagram is, in a few words: its kind, and how many nodes
/// a flowchart has, how many take part in a sequence, else how many lines
/// it is.
fn diagram_said(source: &str) -> String {
    let lines: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("%%"))
        .collect();
    let Some(head) = lines.first() else {
        return "empty".to_owned();
    };
    let kind = head.split_whitespace().next().unwrap_or_default();
    let body = &lines[1..];
    let ident = |part: &str| -> Option<String> {
        let id: String = part
            .trim()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        (!id.is_empty()).then_some(id)
    };
    let mut seen: Vec<String> = Vec::new();
    let mut count = |id: Option<String>| {
        if let Some(id) = id
            && !seen.contains(&id)
        {
            seen.push(id);
        }
    };
    match kind {
        "graph" | "flowchart" => {
            const NOT_NODES: [&str; 7] = [
                "subgraph",
                "end",
                "style",
                "classDef",
                "class",
                "click",
                "linkStyle",
            ];
            for line in body {
                if NOT_NODES.contains(&line.split_whitespace().next().unwrap_or_default()) {
                    continue;
                }
                // Without the labels of its links, parted at its arrows.
                let mut plain = String::new();
                let mut label = false;
                for c in line.chars() {
                    match c {
                        '|' => label = !label,
                        _ if label => {}
                        '-' | '=' | '.' | '<' | '>' => plain.push('\n'),
                        _ => plain.push(c),
                    }
                }
                for part in plain.split('\n').flat_map(|part| part.split('&')) {
                    count(ident(part));
                }
            }
            format!(
                "flowchart, {} node{}",
                seen.len(),
                if seen.len() == 1 { "" } else { "s" }
            )
        }
        "sequenceDiagram" => {
            for line in body {
                let mut words = line.split_whitespace();
                match words.next() {
                    Some("participant" | "actor") => count(words.next().map(str::to_owned)),
                    _ if line.contains("->") || line.contains("--") => {
                        let said = line.split(':').next().unwrap_or_default();
                        for part in said.split(['-', '>', 'x', ')', '+']) {
                            count(ident(part));
                        }
                    }
                    _ => {}
                }
            }
            format!("sequence, {} taking part", seen.len())
        }
        _ => format!(
            "{kind}, {} line{}",
            body.len(),
            if body.len() == 1 { "" } else { "s" }
        ),
    }
}

/// The first `most` lines of `text` that say something, dimmed, after
/// `first`, each cut within [`LINE_CHARS`]: a paragraph written as one line
/// would fill the screen.
fn first_lines(text: &str, first: &str, p: &Palette, most: usize) -> Vec<Para> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let under = || Span::raw(" ".repeat(crate::wrap::width(first)));
    lines
        .iter()
        .take(most)
        .enumerate()
        .map(|(at, line)| {
            let lead = if at == 0 {
                Span::styled(first.to_owned(), p.fg(p.info))
            } else {
                under()
            };
            let mut said = cut(line, LINE_CHARS);
            if at + 1 == most && lines.len() > most && !said.ends_with('…') {
                said.push_str(" …");
            }
            Para {
                first: vec![lead],
                rest: vec![under()],
                body: vec![Span::styled(said, p.fg(p.dim))],
            }
        })
        .collect()
}

/// How the file at `path` changed, as `diff`, a unified diff, says it:
/// its name and how many lines came and went, then each line numbered as
/// it stands in the file, those added green, those removed red.
fn change(path: &str, diff: &str, p: &Palette) -> Block {
    enum Kind {
        Kept,
        Added,
        Removed,
        Apart,
        Said,
    }
    // Where a part of it begins, in the old file and the new.
    let begins = |line: &str| -> Option<(usize, usize)> {
        let rest = line.strip_prefix("@@ -")?;
        let (old, rest) = rest.split_once(" +")?;
        let new = rest.split_once(" @@")?.0;
        let start = |part: &str| part.split(',').next()?.parse::<usize>().ok();
        Some((start(old)?, start(new)?))
    };
    let mut lines: Vec<(Kind, usize, String)> = Vec::new();
    let (mut old, mut new) = (0, 0);
    for line in diff.lines() {
        if let Some((from_old, from_new)) = begins(line) {
            if !lines.is_empty() {
                lines.push((Kind::Apart, 0, String::new()));
            }
            (old, new) = (from_old, from_new);
            continue;
        }
        // Written as iq writes code: tabs as spaces, no control character.
        let text = |rest: &str| -> String {
            rest.replace('\t', "    ")
                .chars()
                .filter(|c| !c.is_control())
                .collect()
        };
        match line.split_at_checked(1) {
            Some(("+", rest)) => {
                lines.push((Kind::Added, new, text(rest)));
                new += 1;
            }
            Some(("-", rest)) => {
                lines.push((Kind::Removed, old, text(rest)));
                old += 1;
            }
            Some((" ", rest)) => {
                lines.push((Kind::Kept, new, text(rest)));
                (old, new) = (old + 1, new + 1);
            }
            Some(("\\", rest)) => lines.push((Kind::Said, 0, text(rest.trim()))),
            _ => {}
        }
    }
    let count = |kind: fn(&Kind) -> bool| lines.iter().filter(|(k, ..)| kind(k)).count();
    let added = count(|kind| matches!(kind, Kind::Added));
    let removed = count(|kind| matches!(kind, Kind::Removed));
    let mut head = vec![
        Span::styled("  ✎ ", p.fg(p.info)),
        Span::styled(short(path), p.fg(p.text)),
    ];
    if added > 0 {
        head.push(Span::styled(format!("  +{added}"), p.fg(p.ok)));
    }
    if removed > 0 {
        head.push(Span::styled(format!("  -{removed}"), p.fg(p.bad)));
    }
    let width = lines
        .iter()
        .map(|(_, number, _)| number.to_string().len())
        .max()
        .unwrap_or(1);
    let rows = lines
        .into_iter()
        .map(|(kind, number, text)| {
            let (sign, style) = match kind {
                Kind::Added => ("+", p.fg(p.ok)),
                Kind::Removed => ("-", p.fg(p.bad)),
                Kind::Kept => (" ", p.fg(p.dim)),
                Kind::Apart => {
                    return vec![
                        Span::raw("    "),
                        Span::styled(format!("{:>width$}", "⋮"), p.fg(p.faint)),
                    ];
                }
                Kind::Said => {
                    return vec![
                        Span::raw("    "),
                        Span::styled(format!("… {text}"), p.fg(p.faint)),
                    ];
                }
            };
            vec![
                Span::raw("    "),
                Span::styled(format!("{number:>width$} "), p.fg(p.faint)),
                Span::styled(format!("{sign} "), style),
                Span::styled(text, style),
            ]
        })
        .collect();
    Block::Change {
        head,
        rows,
        faint: p.fg(p.faint),
        diff: diff.to_owned(),
    }
}

/// The number a message has on the steps' page, as its head ends: the
/// way to it whole.
fn numbered(id: Option<usize>, p: &Palette) -> Vec<Span<'static>> {
    id.map(|id| vec![Span::styled(format!("  #{id}"), p.fg(p.faint))])
        .unwrap_or_default()
}

/// The agent `event` is of, when it is of one.
fn agent_of(event: &Event) -> Option<&str> {
    match event {
        Event::Joined { agent, .. }
        | Event::Placed { agent, .. }
        | Event::Model { agent, .. }
        | Event::Effort { agent, .. }
        | Event::Working { agent, .. }
        | Event::TextStart { agent }
        | Event::Text { agent, .. }
        | Event::Spent { agent, .. }
        | Event::Tool { agent, .. }
        | Event::Changed { agent, .. }
        | Event::Asked { agent, .. }
        | Event::Reviewed { agent, .. }
        | Event::Done { agent, .. }
        | Event::Waiting { agent, .. }
        | Event::Choose { agent, .. }
        | Event::JobStarted { agent, .. }
        | Event::JobEnded { agent, .. }
        | Event::JobOutput { agent, .. }
        | Event::Failed { agent, .. }
        | Event::Window { agent, .. }
        | Event::Retired { agent }
        | Event::Notice { agent, .. } => Some(agent),
        Event::Posted { from, .. } => Some(from),
        Event::Renamed { to, .. } => Some(to),
        Event::Answered { .. } | Event::Chosen { .. } => None,
        Event::Turn { chat, .. } => Some(chat),
    }
}

/// The person's chat `agent` is part of: itself when the person started
/// it, else that of the agent that started it. None for one unknown.
pub(crate) fn chat_of(board: &Board, agent: &str) -> Option<String> {
    let mut name = agent;
    // As many steps as there are agents, so that a loop ends.
    for _ in 0..=board.crew().agents().len() {
        let view = board.crew().agent(name)?;
        if view.by == PERSON {
            return Some(view.name.clone());
        }
        name = &view.by;
    }
    None
}

/// How `agent` is named in its chat's window, where the chat's name says
/// nothing more: the chat's own agent by its model, one it started by its
/// part; then what is said beside, its model and how hard it thinks.
pub(crate) fn label(board: &Board, agent: &str) -> (String, String) {
    label_on(board, agent, None)
}

/// The same, `agent` answering on `model` when given rather than on its
/// own now, as one of its past turns did.
fn label_on(board: &Board, agent: &str, model: Option<&str>) -> (String, String) {
    let Some(view) = board.crew().agent(agent) else {
        return (agent.to_owned(), String::new());
    };
    let model = model.unwrap_or_else(|| view.serving());
    let short = model.rsplit('/').next().unwrap_or(model).to_owned();
    let effort = view.effort.map(|e| e.as_str().to_owned());
    if view.by == PERSON {
        return (short, effort.unwrap_or_default());
    }
    let part = agent
        .split_once('/')
        .map_or(agent, |(_, part)| part)
        .to_owned();
    let beside = match effort {
        Some(effort) => format!("{short} · {effort}"),
        None => short,
    };
    (part, beside)
}

/// A step in a line: its mark, then what it did.
pub(crate) fn step_line(step: &Step, p: &Palette) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let one = |text: &str| cut(&short(text).replace('\n', " "), INPUT_CHARS);
    match step {
        Step::Tool {
            name, input, ok, ..
        } => {
            let (mark, tint) = if *ok {
                ("▸ ", p.faint)
            } else {
                ("✗ ", p.bad)
            };
            // A long command in a few words: its page gives it whole.
            let said =
                if name == "run" && (input.contains('\n') || input.chars().count() > INPUT_CHARS) {
                    iq_ui::shell::few_words(input, INPUT_CHARS)
                } else {
                    one(input)
                };
            (
                vec![Span::raw("  "), Span::styled(mark, p.fg(tint))],
                vec![
                    Span::styled(name.clone(), p.fg(p.text)),
                    Span::raw(" "),
                    Span::styled(said, p.fg(p.dim)),
                ],
            )
        }
        // An action in a few words, never a command's body: the steps' page
        // gives it whole.
        Step::Answered { action, approved }
        | Step::Reviewed {
            action, approved, ..
        } => {
            let (mark, tint) = if *approved {
                ("✓ ", p.ok)
            } else {
                ("✗ ", p.bad)
            };
            let mut body = vec![
                Span::styled(format!("{} ", step.kind()), p.fg(tint)),
                Span::styled(
                    iq_ui::shell::action_words(&short(action), INPUT_CHARS),
                    p.fg(p.text),
                ),
            ];
            if let Step::Reviewed { why, .. } = step {
                body.push(Span::styled(format!(": {}", one(why)), p.fg(p.dim)));
            }
            (vec![Span::raw("  "), Span::styled(mark, p.fg(tint))], body)
        }
        Step::Question { question, answer } => {
            let answer = match answer {
                Some(answer) => Span::styled(one(answer), p.fg(p.accent)),
                None => Span::styled("no answer", p.fg(p.faint)),
            };
            (
                vec![Span::raw("  "), Span::styled("? ", p.fg(p.warn))],
                vec![
                    Span::styled(cut(&one(question), QUESTION_CHARS), p.fg(p.dim)),
                    Span::styled(" → ", p.fg(p.faint)),
                    answer,
                ],
            )
        }
        Step::Notice { text } => (
            vec![Span::raw("  "), Span::styled("! ", p.fg(p.warn))],
            vec![Span::styled(one(text), p.fg(p.dim))],
        ),
    }
}

/// A fold's count: how many steps, of which kinds, how many went wrong,
/// and the key that unfolds them.
fn counted(fold: &Fold, unfold: Option<&str>, p: &Palette) -> Vec<Span<'static>> {
    let kinds = fold.kinds();
    let mut named: Vec<String> = kinds
        .iter()
        .take(KINDS)
        .map(|(kind, count)| {
            if *count > 1 {
                format!("{kind} ×{count}")
            } else {
                kind.clone()
            }
        })
        .collect();
    if kinds.len() > KINDS {
        named.push("…".to_owned());
    }
    let mut spans = vec![
        Span::styled(format!("{} steps", fold.steps.len()), p.fg(p.text)),
        Span::styled(format!(": {}", named.join(", ")), p.fg(p.dim)),
    ];
    let failed = fold.failed();
    if failed > 0 {
        spans.push(Span::styled(format!(" · {failed} went wrong"), p.fg(p.bad)));
    }
    if let Some(key) = unfold {
        spans.push(Span::styled(
            format!("  {key} unfolds #{}", fold.id),
            p.fg(p.faint),
        ));
    }
    spans
}

/// The colour of `agent`, by the order it joined; the person's is the
/// text's.
pub(crate) fn agent_tint(board: &Board, agent: &str, p: &Palette) -> crate::theme::Rgb {
    board
        .crew()
        .agents()
        .iter()
        .position(|a| a.name == agent)
        .map_or(p.text, |at| p.agent(at))
}

/// A length of time, as `40s`, `3m 05s`.
pub(crate) fn seconds(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

/// `path` with the person's home written `~`.
fn home_short(path: &str) -> String {
    match std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        Some(home) => {
            let home = std::path::Path::new(&home).display().to_string();
            match path.strip_prefix(&home) {
                Some(rest) => format!("~{rest}"),
                None => path.to_owned(),
            }
        }
        None => path.to_owned(),
    }
}

/// `text` with the folder iq works in written `.`, and the person's home
/// `~`: paths read shorter, and the rest of them is what tells them apart.
pub(crate) fn short(text: &str) -> String {
    static PLACES: std::sync::OnceLock<Vec<(String, &'static str)>> = std::sync::OnceLock::new();
    let places = PLACES.get_or_init(|| {
        let mut places = Vec::new();
        if let Ok(folder) = std::env::current_dir() {
            places.push((format!("{}/", folder.display()), "./"));
            places.push((folder.display().to_string(), "."));
        }
        if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
            places.push((std::path::Path::new(&home).display().to_string(), "~"));
        }
        places
    });
    places.iter().fold(text.to_owned(), |text, (long, short)| {
        text.replace(long, short)
    })
}

/// Who writes iq's own messages to an agent: a job that ended, a wake.
const IQ: &str = "iq";

/// The longest of the few words a job is said in.
const LABEL_CHARS: usize = 72;

/// The fewest columns left to what a job does, on a narrow screen.
const LABEL_LEAST: usize = 12;

/// What a command does in a few words: the summarizer's once made, read
/// from the line itself until then, or without one.
#[derive(Debug, Clone)]
pub(crate) struct Label {
    summary: Option<Arc<Summary>>,
    instead: String,
    style: Style,
}

impl Label {
    /// What `agent`'s `line` does, asked of `summarize` when there is one
    /// and the line is long enough to need it.
    fn new(agent: &str, line: &str, summarize: Option<&Arc<dyn Summarize>>, p: &Palette) -> Label {
        let instead = iq_ui::shell::few_words(line, LABEL_CHARS);
        let long = line.contains('\n') || line.chars().count() > LABEL_CHARS;
        Label {
            summary: summarize
                .filter(|_| long)
                .map(|summarize| Summary::labelled(agent, line, Arc::clone(summarize))),
            instead: if instead.is_empty() {
                cut(&short(line).replace('\n', " "), LABEL_CHARS)
            } else {
                instead
            },
            style: p.fg(p.text),
        }
    }

    /// Whether the summarizer's words are still waited for.
    fn making(&self) -> bool {
        self.summary
            .as_ref()
            .is_some_and(|summary| summary.making())
    }

    /// Its words.
    fn text(&self) -> String {
        self.summary
            .as_ref()
            .and_then(|summary| summary.text())
            .and_then(|said| {
                said.lines()
                    .find(|l| !l.trim().is_empty())
                    .map(str::to_owned)
            })
            .map_or_else(
                || self.instead.clone(),
                |said| cut(said.trim(), LABEL_CHARS),
            )
    }

    fn span(&self) -> Span<'static> {
        Span::styled(self.text(), self.style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Depth;
    use iq_ui::Update;
    use iq_ui::look::Theme;

    fn crew(board: &mut Board, history: &mut History, event: Event, p: &Palette) {
        board.update(Update::Crew(event.clone()));
        history.event(&event, board, p);
    }

    /// Draws a diagram as a PNG of 200 by 100 pixels once let go.
    struct Held(std::sync::Mutex<std::sync::mpsc::Receiver<()>>);

    impl Draw for Held {
        fn draw(&self, _source: &str) -> Result<crate::diagram::Drawn, String> {
            let _ = self.0.lock().unwrap().recv();
            let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
            png.extend(200u32.to_be_bytes());
            png.extend(100u32.to_be_bytes());
            Ok(crate::diagram::Drawn { png })
        }
    }

    #[test]
    fn a_table_is_drawn_whole_and_a_diagram_holds_back_what_follows_until_drawn() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        let (go, wait) = std::sync::mpsc::channel();
        history.pictures(Some(Arc::new(Held(std::sync::Mutex::new(wait)))), (10, 20));
        let run = |board: &mut Board, history: &mut History, event| crew(board, history, event, &p);
        run(
            &mut board,
            &mut history,
            Event::Joined {
                agent: "lead".into(),
                model: "lab/lead".into(),
                by: PERSON.into(),
            },
        );
        run(
            &mut board,
            &mut history,
            Event::Text {
                agent: "lead".into(),
                text: "| a | b |\n|---|---|\n| one | two |\n```mermaid\ngraph TD\n A --> B\n```\nafter it\n"
                    .into(),
            },
        );
        let text = |lines: Vec<Line<'static>>| -> Vec<String> {
            lines
                .iter()
                .map(|l| l.to_string().trim_end().to_owned())
                .collect()
        };
        let before = text(history.take(60, Some("lead")));
        assert!(before.contains(&"  a   │ b".to_owned()), "{before:#?}");
        assert!(before.contains(&"  one │ two".to_owned()), "{before:#?}");
        // Held while it is drawn, and what follows with it.
        assert!(
            !before
                .iter()
                .any(|l| l.contains("after it") || l.contains("graph TD"))
        );
        assert!(history.waiting());
        assert!(text(history.take(60, Some("lead"))).is_empty());
        go.send(()).unwrap();
        let mut after = Vec::new();
        for _ in 0..500 {
            after = text(history.take(60, Some("lead")));
            if !after.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // 200 by 100 pixels shown half as large again on cells of 10 by
        // 20: 8 rows of 30 cells.
        assert_eq!(after.iter().filter(|l| l.contains('\u{10EEEE}')).count(), 8);
        assert_eq!(after.last().map(String::as_str), Some("  after it"));
        let images = history.take_images();
        assert_eq!((images.len(), images[0].cols, images[0].rows), (1, 30, 8));
    }

    /// Draws every diagram at once as a PNG of `.0` by `.1` pixels.
    struct Sized(u32, u32);

    impl Draw for Sized {
        fn draw(&self, _source: &str) -> Result<crate::diagram::Drawn, String> {
            let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
            png.extend(self.0.to_be_bytes());
            png.extend(self.1.to_be_bytes());
            Ok(crate::diagram::Drawn { png })
        }
    }

    #[test]
    fn two_diagrams_of_an_answer_take_their_own_rows_and_what_follows_comes_below() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        // A tall one, held to the most rows.
        history.pictures(Some(Arc::new(Sized(200, 2000))), (10, 20));
        crew(
            &mut board,
            &mut history,
            Event::Joined {
                agent: "lead".into(),
                model: "lab/lead".into(),
                by: PERSON.into(),
            },
            &p,
        );
        let diagram = |n: u32| format!("```mermaid\ngraph TD\n A{n} --> B{n}\n```\n");
        crew(
            &mut board,
            &mut history,
            Event::Text {
                agent: "lead".into(),
                text: format!("First:\n{}Between.\n{}After.\n", diagram(1), diagram(2)),
            },
            &p,
        );
        let mut lines = Vec::new();
        for _ in 0..500 {
            lines.extend(history.take(80, Some("lead")));
            if lines.iter().any(|l| l.to_string().contains("After.")) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let images = history.take_images();
        assert_eq!(images.len(), 2);
        assert_ne!(images[0].id, images[1].id);
        // 200 by 2000 pixels, half as large again, on cells of 10 by 20:
        // held to 30 rows, 6 columns.
        assert!(images.iter().all(|i| (i.cols, i.rows) == (6, 30)));
        let rows: Vec<String> = lines.iter().map(ToString::to_string).collect();
        let at = |text: &str| rows.iter().position(|r| r.contains(text)).unwrap();
        // Each image's rows exactly, one after the other, in its own
        // colour, what follows each below it.
        for (image, (from, to)) in images.iter().zip([
            (at("First:"), at("Between.")),
            (at("Between."), at("After.")),
        ]) {
            let colour = ratatui::style::Color::Rgb(
                ((image.id >> 16) & 0xff) as u8,
                ((image.id >> 8) & 0xff) as u8,
                (image.id & 0xff) as u8,
            );
            let theirs: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.spans.iter().any(|s| s.style.fg == Some(colour)))
                .map(|(at, _)| at)
                .collect();
            assert_eq!(theirs.len(), usize::from(image.rows), "{rows:#?}");
            assert!(theirs.windows(2).all(|pair| pair[1] == pair[0] + 1));
            assert!(
                from < theirs[0] && *theirs.last().unwrap() < to,
                "{rows:#?}"
            );
        }
    }

    #[test]
    fn a_diagram_folded_waits_last_for_the_down_arrow_and_stays_folded_once_passed() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let look = iq_ui::look::Look::default().with_diagrams_folded(true);
        let mut board = Board::new(iq_ui::keys::Keymap::default(), look);
        let mut history = History::default();
        let (go, wait) = std::sync::mpsc::channel();
        history.pictures(Some(Arc::new(Held(std::sync::Mutex::new(wait)))), (10, 20));
        let run = |board: &mut Board, history: &mut History, event| crew(board, history, event, &p);
        run(
            &mut board,
            &mut history,
            Event::Joined {
                agent: "lead".into(),
                model: "lab/lead".into(),
                by: PERSON.into(),
            },
        );
        let diagram = "```mermaid\nflowchart LR\n A[Parse] -->|ok| B & C\n B --> D\n```\n";
        let said = |board: &mut Board, history: &mut History, text: &str| {
            run(
                board,
                history,
                Event::Text {
                    agent: "lead".into(),
                    text: text.into(),
                },
            );
        };
        let text = |lines: Vec<Line<'static>>| -> Vec<String> {
            lines
                .iter()
                .map(|l| l.to_string().trim_end().to_owned())
                .collect()
        };
        said(&mut board, &mut history, &format!("Here:\n{diagram}"));
        // The last thing written, it waits folded, said where the live part
        // begins; nothing waits for its drawing.
        let written = text(history.take(60, Some("lead")));
        assert!(written.contains(&"  Here:".to_owned()), "{written:#?}");
        assert!(
            !written.iter().any(|l| l.contains("diagram")),
            "{written:#?}"
        );
        let folded = Line::from(history.folded(Some("lead")).unwrap()).to_string();
        assert_eq!(folded, "  ▸ diagram · flowchart, 4 nodes");
        // Unfolded, its image is written once drawn.
        history.unfold(Some("lead"));
        assert!(history.folded(Some("lead")).is_none());
        go.send(()).unwrap();
        let mut image = Vec::new();
        for _ in 0..500 {
            image = text(history.take(60, Some("lead")));
            if !image.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(image.iter().all(|l| l.contains('\u{10EEEE}')), "{image:#?}");
        // Another, then text after it: folded for good.
        said(&mut board, &mut history, &format!("{diagram}And on.\n"));
        let written = text(history.take(60, Some("lead")));
        assert!(
            written.contains(&"  ▸ diagram · flowchart, 4 nodes".to_owned()),
            "{written:#?}"
        );
        assert_eq!(written.last().map(String::as_str), Some("  And on."));
        assert!(history.folded(Some("lead")).is_none());
    }

    #[test]
    fn an_answer_streamed_in_any_chunks_has_one_empty_row_between_blocks() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let answer = "Done:\n- the parser\n- its tests\n\n### Next\nThe docs.\n";
        let expected = [
            "● lead",
            "  Done:",
            "",
            "  • the parser",
            "  • its tests",
            "",
            "  Next",
            "",
            "  The docs.",
        ];
        let rows = |chunks: &[&str]| -> Vec<String> {
            let mut board = Board::default();
            let mut history = History::default();
            crew(
                &mut board,
                &mut history,
                Event::Joined {
                    agent: "lead".into(),
                    model: "lab/lead".into(),
                    by: PERSON.into(),
                },
                &p,
            );
            for chunk in chunks {
                crew(
                    &mut board,
                    &mut history,
                    Event::Text {
                        agent: "lead".into(),
                        text: (*chunk).into(),
                    },
                    &p,
                );
            }
            crew(
                &mut board,
                &mut history,
                Event::Done {
                    agent: "lead".into(),
                    outcome: Ok(answer.into()),
                },
                &p,
            );
            history
                .take(60, Some("lead"))
                .iter()
                .map(|l| l.to_string().trim_end().to_owned())
                .skip_while(String::is_empty)
                .collect()
        };
        // Cut at every place, once and twice.
        let at: Vec<usize> = answer.char_indices().map(|(i, _)| i).skip(1).collect();
        for &one in &at {
            let shown = rows(&[&answer[..one], &answer[one..]]);
            assert_eq!(
                shown[..expected.len()],
                expected,
                "cut at {one}: {shown:#?}"
            );
            for &two in at.iter().filter(|&&two| two > one).step_by(5) {
                let shown = rows(&[&answer[..one], &answer[one..two], &answer[two..]]);
                assert_eq!(shown[..expected.len()], expected, "cut at {one}, {two}");
            }
        }
        // An answer whose model left empty lines everywhere, and new lines
        // as Windows writes them.
        let loose = "Done:\r\n\r\n\r\n- the parser\r\n\r\n- its tests\r\n\r\n\r\n\r\n### Next\r\n\r\nThe docs.\r\n";
        let shown = rows(&[loose]);
        assert_eq!(shown[..expected.len()], expected, "{shown:#?}");
    }

    #[test]
    fn a_chat_come_back_to_names_the_model_each_of_its_turns_answered_on() {
        use iq_ui::crew::{Back, Entry};
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        crew(
            &mut board,
            &mut history,
            Event::Joined {
                agent: "fix".into(),
                model: "lab/haiku".into(),
                by: PERSON.into(),
            },
            &p,
        );
        history.back("fix");
        let mut thread = Vec::new();
        for (n, model) in [(1, "lab/sol"), (2, "lab/sol"), (3, "lab/haiku")] {
            thread.push(Entry::Model(model.into()));
            thread.push(Entry::Prompt(format!("Message from person: ask {n}")));
            thread.push(Entry::Text(format!("answer {n}")));
        }
        board.update(Update::Back(Back {
            agent: "fix".into(),
            task: "ask 1".into(),
            worked: iq_ui::now(),
            written: Some(iq_ui::now()),
            thread,
        }));
        let recall = board.take_recall().unwrap();
        history.recall(&recall, &board, &p);
        let lines: Vec<String> = history
            .take(100, Some("fix"))
            .iter()
            .map(|line| line.to_string().trim_end().to_owned())
            .filter(|line| line.starts_with('●') || line.starts_with("  answer"))
            .collect();
        // The chat as it is now, then each answer under the model that gave
        // it.
        assert_eq!(
            lines,
            [
                "● haiku",
                "● sol",
                "  answer 1",
                "● sol",
                "  answer 2",
                "● haiku",
                "  answer 3"
            ]
        );
        // Its turns from now are its model's.
        let next = label(&board, "fix");
        assert_eq!(next.0, "haiku");
    }

    #[test]
    fn a_chat_come_back_to_is_written_as_it_was_shown() {
        use iq_ui::crew::{Back, Entry};
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        for agent in ["login", "build"] {
            crew(
                &mut board,
                &mut history,
                Event::Joined {
                    agent: agent.into(),
                    model: "lab/model".into(),
                    by: PERSON.into(),
                },
                &p,
            );
        }
        let run = || Entry::Tool {
            name: "run".into(),
            input: "make test".into(),
            output: "ok".into(),
            ok: true,
        };
        let verdict = || Entry::Reviewed {
            action: "run `cat >> tests/test_x.py <<'EOF'\nassert x\nEOF\nuv run pytest` in /w, \
                     on the machine itself, outside the sandbox"
                .into(),
            approved: true,
            why: "These focused tests serve the requested fixes.".into(),
        };
        let mut thread = Vec::new();
        for n in 1..=4 {
            thread.push(Entry::Prompt(format!("Message from person: ask {n}")));
            match n {
                // A verdict alone, in a few words.
                3 => thread.push(verdict()),
                // A verdict, a notice and a question answered fold with the
                // tools around them.
                4 => thread.extend([
                    run(),
                    verdict(),
                    Entry::Notice("set apart 36 of its old tool results".into()),
                    Entry::Chose {
                        question: "Which environment?".into(),
                        answer: Some("staging".into()),
                    },
                    run(),
                ]),
                _ => thread.push(run()),
            }
            thread.push(Entry::Text(format!("answer {n}")));
        }
        thread.push(Entry::Text("| a | b |\n|---|---|\n| one | two |".into()));
        // An agent a chat started, back with it, did not join now.
        crew(
            &mut board,
            &mut history,
            Event::Joined {
                agent: "review".into(),
                model: "lab/model".into(),
                by: "login".into(),
            },
            &p,
        );
        history.back("review");
        history.back("login");
        board.update(Update::Back(Back {
            agent: "login".into(),
            task: "ask 1".into(),
            worked: iq_ui::now(),
            written: Some(iq_ui::now()),
            thread,
        }));
        let recall = board.take_recall().unwrap();
        history.recall(&recall, &board, &p);
        let lines: Vec<String> = history
            .take(100, Some("login"))
            .iter()
            .map(ToString::to_string)
            .collect();
        let mut shown: Vec<String> = [
            "",
            // In its own window, the chat's agent is named by its model.
            "● model",
            "  ↺ back, at work just now · its whole thread: Ctrl-T, then →",
        ]
        .map(str::to_owned)
        .to_vec();
        // Each exchange as it was shown: what was asked, the agent, its
        // steps folded under the number their page gives them, its answer.
        for n in 1..=4 {
            let steps = match n {
                3 => {
                    "  ✓ autopilot allowed run cat, uv run · tests/test_x.py · on the machine: These focused tests serve…"
                }
                4 => "  ▸ 5 steps: run ×2, autopilot allowed, notice, question  Ctrl-X unfolds #4",
                _ => "  ▸ run make test",
            };
            shown.extend([
                String::new(),
                format!("› ask {n}"),
                String::new(),
                "● model".to_owned(),
                steps.to_owned(),
                String::new(),
                format!("  answer {n}"),
            ]);
        }
        shown.extend(
            [
                "  a   │ b",
                "  ────┼────",
                "  one │ two",
                "  you write to login, the chat you wrote to last; Ctrl-T or /chats lists the 1 other",
            ]
            .map(str::to_owned),
        );
        let trimmed: Vec<String> = lines.iter().map(|l| l.trim_end().to_owned()).collect();
        assert_eq!(trimmed, shown);
        // Their page gives each step whole.
        let fold = board.steps().fold(4).unwrap();
        assert_eq!(fold.begins, Some(10));
        assert!(fold.steps[1].whole().contains("assert x"));
        assert!(fold.steps[3].whole().ends_with("→ staging"));
        // Gone to again, its window is the same conversation.
        let again: Vec<String> = history
            .window(100, Some("login"))
            .iter()
            .map(|l| l.to_string().trim_end().to_owned())
            .collect();
        assert!(again.ends_with(&trimmed), "{again:#?}");
        // Its folds were told: the next step opens one of its own.
        crew(
            &mut board,
            &mut history,
            Event::Tool {
                agent: "login".into(),
                name: "run".into(),
                input: "ls".into(),
                output: String::new(),
                ok: true,
            },
            &p,
        );
        assert_eq!(board.steps().open("login").unwrap().id, 5);
        // Its next turn, woken by the person's message, is not written as
        // a task of its own.
        crew(
            &mut board,
            &mut history,
            Event::Working {
                agent: "login".into(),
                prompt: "Message from person: go on".into(),
            },
            &p,
        );
        assert!(history.take(100, Some("login")).is_empty());
    }

    #[test]
    fn a_turn_is_written_line_by_line_under_its_agent() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        let run = |board: &mut Board, history: &mut History, event| crew(board, history, event, &p);
        run(
            &mut board,
            &mut history,
            Event::Joined {
                agent: "lead".into(),
                model: "anthropic/claude-opus-5-5".into(),
                by: PERSON.into(),
            },
        );
        run(
            &mut board,
            &mut history,
            Event::Working {
                agent: "lead".into(),
                prompt: "fix the build".into(),
            },
        );
        run(
            &mut board,
            &mut history,
            Event::TextStart {
                agent: "lead".into(),
            },
        );
        run(
            &mut board,
            &mut history,
            Event::Text {
                agent: "lead".into(),
                text: "I will **look**\nat the te".into(),
            },
        );
        assert_eq!(history.partial("lead"), Some("at the te"));
        run(
            &mut board,
            &mut history,
            Event::Text {
                agent: "lead".into(),
                text: "sts".into(),
            },
        );
        run(
            &mut board,
            &mut history,
            Event::Tool {
                agent: "lead".into(),
                name: "Bash".into(),
                input: "cargo test".into(),
                output: "error: one\nerror: two".into(),
                ok: false,
            },
        );
        run(
            &mut board,
            &mut history,
            Event::Done {
                agent: "lead".into(),
                outcome: Ok("done".into()),
            },
        );
        let lines: Vec<String> = history
            .take(80, Some("lead"))
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            lines,
            [
                "",
                "› fix the build",
                "",
                "● claude-opus-5-5",
                "  I will look",
                "  at the tests",
                // A line between its text and its steps, and before the end
                // of its turn.
                "",
                "  ✗ Bash cargo test",
                "    error: one",
                "    error: two",
                "",
                "  ✔ 0s",
            ]
        );
        assert!(!history.waiting());
    }

    #[test]
    fn empty_lines_leave_one_between_paragraphs_and_code_is_written_whole() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        let run = |board: &mut Board, history: &mut History, event| crew(board, history, event, &p);
        run(
            &mut board,
            &mut history,
            Event::Joined {
                agent: "lead".into(),
                model: "lab/m".into(),
                by: PERSON.into(),
            },
        );
        let long = format!("run --flag {}", "x".repeat(60));
        for text in [
            // Pieces of an answer that are only new lines, as some models
            // send them before a tool.
            "\n".to_owned(),
            "\n\n\n".to_owned(),
            "Run this:\n\n\n\n".to_owned(),
            format!("```sh\n{long}\n\n  echo done\n```\n\n"),
        ] {
            run(
                &mut board,
                &mut history,
                Event::Text {
                    agent: "lead".into(),
                    text,
                },
            );
        }
        run(
            &mut board,
            &mut history,
            Event::Tool {
                agent: "lead".into(),
                name: "run".into(),
                input: "ls".into(),
                output: String::new(),
                ok: true,
            },
        );
        run(
            &mut board,
            &mut history,
            Event::TextStart {
                agent: "lead".into(),
            },
        );
        run(
            &mut board,
            &mut history,
            Event::Text {
                agent: "lead".into(),
                text: "\n\nDone.\n".into(),
            },
        );
        run(
            &mut board,
            &mut history,
            Event::Done {
                agent: "lead".into(),
                outcome: Ok("done".into()),
            },
        );
        let lines: Vec<String> = history
            .take(40, Some("lead"))
            .iter()
            .map(|l| l.to_string().trim_end().to_owned())
            .collect();
        assert_eq!(
            lines,
            [
                "",
                "● m",
                "  Run this:",
                "",
                // Code at the line's start, its marks above and below it,
                // a line longer than the screen not folded by iq.
                "╭ sh",
                long.as_str(),
                "",
                "  echo done",
                "╰",
                "",
                "  ▸ run ls",
                "",
                "  Done.",
                "",
                "  ✔",
            ]
        );
    }

    #[test]
    fn the_steps_between_two_lines_are_written_as_their_count_once_the_agent_goes_on() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        let run = |board: &mut Board, history: &mut History, event| crew(board, history, event, &p);
        run(
            &mut board,
            &mut history,
            Event::Joined {
                agent: "lead".into(),
                model: "m".into(),
                by: PERSON.into(),
            },
        );
        run(
            &mut board,
            &mut history,
            Event::Working {
                agent: "lead".into(),
                prompt: "look".into(),
            },
        );
        let read = |path: &str| Event::Tool {
            agent: "lead".into(),
            name: "read_file".into(),
            input: path.into(),
            output: "1\tfn main() {}".into(),
            ok: true,
        };
        run(&mut board, &mut history, read("src/a.rs"));
        run(&mut board, &mut history, read("src/b.rs"));
        run(
            &mut board,
            &mut history,
            Event::Tool {
                agent: "lead".into(),
                name: "run".into(),
                input: "cargo test".into(),
                output: "ok".into(),
                ok: true,
            },
        );
        run(
            &mut board,
            &mut history,
            Event::Asked {
                id: 1,
                agent: "lead".into(),
                action: "git push".into(),
                reason: "it leaves the machine".into(),
                widens: true,
            },
        );
        run(
            &mut board,
            &mut history,
            Event::Answered {
                id: 1,
                approved: false,
            },
        );
        // While it works, its steps are the live part's, not the
        // conversation's.
        let lines: Vec<String> = history
            .take(80, Some("lead"))
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(lines, ["", "› look"]);
        assert_eq!(board.steps().open("lead").unwrap().steps.len(), 4);
        run(
            &mut board,
            &mut history,
            Event::TextStart {
                agent: "lead".into(),
            },
        );
        run(
            &mut board,
            &mut history,
            Event::Text {
                agent: "lead".into(),
                text: "I add the test.\n".into(),
            },
        );
        run(&mut board, &mut history, read("src/c.rs"));
        run(
            &mut board,
            &mut history,
            Event::Done {
                agent: "lead".into(),
                outcome: Ok("done".into()),
            },
        );
        let lines: Vec<String> = history
            .take(120, Some("lead"))
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            lines,
            [
                "",
                "● m",
                "  ▸ 4 steps: read_file ×2, run, you refused · 1 went wrong  Ctrl-X unfolds #1",
                "",
                "  I add the test.",
                "",
                // One step is written as it is: nothing to unfold.
                "  ▸ read_file src/c.rs",
                "",
                "  ✔ 0s",
            ]
        );
        assert!(board.steps().folds().all(|fold| fold.closed));
    }

    #[test]
    fn a_verdict_a_notice_and_an_answered_question_are_steps_of_a_fold_that_stays_open() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        let run = |board: &mut Board, history: &mut History, event| crew(board, history, event, &p);
        let lead = || "lead".to_owned();
        run(
            &mut board,
            &mut history,
            Event::Joined {
                agent: lead(),
                model: "m".into(),
                by: PERSON.into(),
            },
        );
        run(
            &mut board,
            &mut history,
            Event::Working {
                agent: lead(),
                prompt: "fix it".into(),
            },
        );
        let verdict = || Event::Reviewed {
            agent: lead(),
            action: "run `cat >> tests/test_x.py <<'EOF'\nassert x\nEOF\nuv run pytest` in /w, \
                     on the machine itself, outside the sandbox"
                .into(),
            reason: "it runs on the machine".into(),
            approved: true,
            why: "These focused tests serve the requested fixes.".into(),
        };
        for event in [
            Event::Tool {
                agent: lead(),
                name: "read_file".into(),
                input: "src/a.rs".into(),
                output: String::new(),
                ok: true,
            },
            verdict(),
            Event::Notice {
                agent: lead(),
                text: "set apart 36 of its old tool results".into(),
            },
            Event::Choose {
                id: 1,
                agent: lead(),
                question: "Which environment?".into(),
                choices: vec!["staging".into(), "production".into()],
                raised: None,
            },
            Event::Chosen {
                id: 1,
                answer: Some("staging".into()),
            },
        ] {
            run(&mut board, &mut history, event);
        }
        // Neither the notice nor the question closed the fold, nor wrote
        // anything of their own.
        let fold = board.steps().open("lead").unwrap();
        assert_eq!((fold.id, fold.steps.len()), (1, 4));
        assert_eq!(history.take(120, Some("lead")).len(), 2);
        for event in [
            Event::TextStart { agent: lead() },
            Event::Text {
                agent: lead(),
                text: "Tested.\n".into(),
            },
            verdict(),
            Event::Done {
                agent: lead(),
                outcome: Ok("done".into()),
            },
        ] {
            run(&mut board, &mut history, event);
        }
        let lines: Vec<String> = history
            .take(100, Some("lead"))
            .iter()
            .map(|l| l.to_string().trim_end().to_owned())
            .collect();
        assert_eq!(
            lines,
            [
                "",
                "● m",
                "  ▸ 4 steps: read_file, autopilot allowed, notice, question  Ctrl-X unfolds #1",
                "",
                "  Tested.",
                "",
                // Alone, a verdict in a few words, in one row: never the
                // script's body.
                "  ✓ autopilot allowed run cat, uv run · tests/test_x.py · on the machine: These focused tests serve…",
                "",
                "  ✔ 0s",
            ]
        );
    }

    #[test]
    fn the_models_named_are_said_under_the_message_that_named_them() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        history.after_message(
            "haiku",
            "→ on anthropic/claude-haiku-5-5, as you named it".into(),
        );
        crew(
            &mut board,
            &mut history,
            Event::Joined {
                agent: "haiku".into(),
                model: "anthropic/claude-haiku-5-5".into(),
                by: PERSON.into(),
            },
            &p,
        );
        crew(
            &mut board,
            &mut history,
            Event::Working {
                agent: "haiku".into(),
                prompt: "avec haiku, corrige le test".into(),
            },
            &p,
        );
        let lines: Vec<String> = history
            .take(80, Some("haiku"))
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            lines,
            [
                "",
                "› avec haiku, corrige le test",
                "  → on anthropic/claude-haiku-5-5, as you named it",
            ]
        );
    }

    #[test]
    fn messages_between_agents_are_written_short_and_to_the_person_whole() {
        let p = Palette::new(Theme::Ink, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        for (agent, by) in [("lead", PERSON), ("helper", "lead")] {
            crew(
                &mut board,
                &mut history,
                Event::Joined {
                    agent: agent.into(),
                    model: "m".into(),
                    by: by.into(),
                },
                &p,
            );
        }
        crew(
            &mut board,
            &mut history,
            Event::Posted {
                from: "lead".into(),
                to: "helper".into(),
                text: "1\n2\n3\n4\n5\n6".into(),
            },
            &p,
        );
        crew(
            &mut board,
            &mut history,
            Event::Posted {
                from: "helper".into(),
                to: PERSON.into(),
                text: "- done".into(),
            },
            &p,
        );
        crew(
            &mut board,
            &mut history,
            Event::Posted {
                from: PERSON.into(),
                to: "helper".into(),
                text: "thanks".into(),
            },
            &p,
        );
        let lines: Vec<String> = history
            .take(80, Some("lead"))
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            lines,
            [
                // In the chat's window, its own agent goes by its model;
                // a message by its number on the steps' page too.
                "◦ helper joins, on m, started by m",
                "  ✉ m → helper  #1",
                "    1",
                "    2",
                "    3",
                "    4 …",
                "",
                "● helper  m",
                "  ✉ to you",
                "  • done",
                "",
                "› @helper thanks",
            ]
        );
    }

    /// Sums a message up as who wrote to whom, in a line.
    struct Sums;

    impl Summarize for Sums {
        fn summarize(&self, from: &str, to: &str, text: &str) -> Option<String> {
            Some(format!(
                "{from} asks {to} for {} lines",
                text.lines().count()
            ))
        }

        fn label(&self, agent: &str, line: &str) -> Option<String> {
            assert_eq!(agent, "lead");
            Some(format!(
                "patches {} lines, then checks",
                line.lines().count()
            ))
        }
    }

    #[test]
    fn a_job_is_one_short_line_as_it_starts_and_as_it_ends_never_its_script() {
        let p = Palette::new(Theme::Ink, Depth::True);
        let script = "cat > src/hook.py <<'PY'\nimport secret_body\nPY\ncargo fmt && cargo test -q";
        for summed in [false, true] {
            let mut board = Board::default();
            let mut history = History::default();
            if summed {
                history.summaries(Some(Arc::new(Sums)));
            }
            for event in [
                Event::Joined {
                    agent: "lead".into(),
                    model: "m".into(),
                    by: PERSON.into(),
                },
                Event::JobStarted {
                    agent: "lead".into(),
                    id: 12,
                    line: script.into(),
                },
                Event::JobOutput {
                    agent: "lead".into(),
                    id: 12,
                    tail: vec!["test a ... ok".into()],
                },
                Event::JobEnded {
                    agent: "lead".into(),
                    id: 12,
                    said: "exit status 0".into(),
                    output: "test result: ok".into(),
                },
                // What iq tells the agent of it, its command whole.
                Event::Posted {
                    from: "iq".into(),
                    to: "lead".into(),
                    text: format!("Job 12, `{script}`, ended: exit status 0."),
                },
            ] {
                crew(&mut board, &mut history, event, &p);
            }
            let written = settled(&mut history, "lead");
            let jobs: Vec<&String> = written.iter().filter(|l| l.contains("⚙")).collect();
            let label = if summed {
                "patches 4 lines, then checks"
            } else {
                "cat, cargo fmt, cargo test · src/hook.py"
            };
            assert_eq!(jobs.len(), 2, "{written:#?}");
            assert_eq!(jobs[0].trim_end(), format!("  ⚙ job 12 · {label}"));
            assert!(
                jobs[1].starts_with(&format!("  ⚙ job 12 · {label} · ended 0 · 0s")),
                "{}",
                jobs[1]
            );
            // Wide enough, it says where it unfolds.
            let wide = written_at(&mut board, &mut history, 120);
            let ended = wide.iter().rev().find(|l| l.contains("⚙")).unwrap();
            assert!(
                ended.ends_with(&format!("{label} · ended 0 · 0s  Ctrl-X unfolds #1")),
                "{ended}"
            );
            // Narrow, what it does is cut first, how it ended kept.
            let narrow = written_at(&mut board, &mut history, 50);
            let ended = narrow.iter().rev().find(|l| l.contains("⚙")).unwrap();
            assert!(ended.trim_end().ends_with("· ended 0 · 0s"), "{ended}");
            assert!(!written.iter().any(|l| l.contains("secret_body")));
            // Its page gives it whole.
            let ran = board.steps().latest_ran().unwrap();
            assert_eq!(
                (ran.line.as_str(), ran.output.as_str()),
                (script, "test result: ok")
            );
        }
    }

    /// The lead's window written again `width` wide.
    fn written_at(_board: &mut Board, history: &mut History, width: usize) -> Vec<String> {
        history
            .window(width, Some("lead"))
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// What `history` writes in `window` once nothing waits any more.
    fn settled(history: &mut History, window: &str) -> Vec<String> {
        let mut lines = Vec::new();
        for _ in 0..400 {
            lines.extend(
                history
                    .take(80, Some(window))
                    .iter()
                    .map(ToString::to_string),
            );
            if !history.waiting() {
                return lines;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("still waiting: {lines:?}");
    }

    #[test]
    fn a_file_changed_is_written_as_it_is_with_its_lines_added_and_removed() {
        let p = Palette::new(Theme::Ink, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        let tool = |name: &str, input: &str| Event::Tool {
            agent: "lead".into(),
            name: name.into(),
            input: input.into(),
            output: "Edited.".into(),
            ok: true,
        };
        let changed = |path: &str, diff: &str| Event::Changed {
            agent: "lead".into(),
            path: path.into(),
            diff: diff.into(),
        };
        let new: String = (1..=20).map(|n| format!("\n+line {n}")).collect();
        for event in [
            Event::Joined {
                agent: "lead".into(),
                model: "m".into(),
                by: PERSON.into(),
            },
            Event::Working {
                agent: "lead".into(),
                prompt: "fix it".into(),
            },
            tool("read_file", "src/a.rs"),
            changed(
                "src/a.rs",
                "@@ -9,3 +9,3 @@\n fn a() {\n-    old();\n+    new();\n }",
            ),
            tool("edit_file", "src/a.rs"),
        ] {
            crew(&mut board, &mut history, event, &p);
        }
        // Written as it is made, under the steps it closes.
        let lines = settled(&mut history, "lead");
        let at = lines
            .iter()
            .position(|line| line.contains("2 steps"))
            .expect("the steps before it");
        assert_eq!(
            lines[at + 1..at + 6],
            [
                "  ✎ src/a.rs  +1  -1",
                "     9   fn a() {",
                "    10 -     old();",
                "    10 +     new();",
                "    11   }",
            ]
        );
        // A file written alone: the change stands for its step, its first
        // lines written.
        for event in [
            changed("src/new.rs", &format!("@@ -0,0 +1,20 @@{new}")),
            tool("write_file", "src/new.rs"),
        ] {
            crew(&mut board, &mut history, event, &p);
        }
        let lines = settled(&mut history, "lead");
        assert!(
            !lines.iter().any(|line| line.contains("write_file")),
            "{lines:#?}"
        );
        let head = lines
            .iter()
            .position(|line| line == "  ✎ src/new.rs  +20")
            .unwrap();
        assert_eq!(lines[head + 1], "     1 + line 1");
        assert_eq!(lines[head + 12], "    12 + line 12");
        assert_eq!(lines[head + 13], "    … 8 more lines");
        // Green added, red removed.
        let p2 = Palette::new(Theme::Ink, Depth::True);
        let Block::Change { rows, .. } = change("x", "@@ -1,1 +1,1 @@\n-a\n+b", &p2) else {
            panic!("a change");
        };
        assert_eq!(rows[0][3].style, p2.fg(p2.bad));
        assert_eq!(rows[1][3].style, p2.fg(p2.ok));
    }

    #[test]
    fn long_messages_between_agents_are_summed_up_and_a_started_agent_writes_in_its_pane() {
        let p = Palette::new(Theme::Ink, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        history.summaries(Some(Arc::new(Sums)));
        let joined = |agent: &str, by: &str| Event::Joined {
            agent: agent.into(),
            model: "m".into(),
            by: by.into(),
        };
        let text = |agent: &str, text: &str| Event::Text {
            agent: agent.into(),
            text: text.into(),
        };
        for event in [
            joined("lead", PERSON),
            joined("lead/tests", "lead"),
            Event::Working {
                agent: "lead/tests".into(),
                prompt: "1\n2\n3\n4\n5\n6".into(),
            },
            // What it writes while it works is in its pane.
            text("lead/tests", "Reading the module.\n"),
            Event::Posted {
                from: "lead/tests".into(),
                to: "lead".into(),
                text: "word ".repeat(80),
            },
            // Written to by the person, it answers them here.
            Event::Posted {
                from: PERSON.into(),
                to: "lead/tests".into(),
                text: "and the docs?".into(),
            },
            text("lead/tests", "Docs too.\n"),
            Event::Done {
                agent: "lead/tests".into(),
                outcome: Ok("Docs too.".into()),
            },
            text("lead/tests", "Not this.\n"),
        ] {
            crew(&mut board, &mut history, event, &p);
        }
        let lines = settled(&mut history, "lead");
        let at = |line: &str| lines.iter().position(|l| l == line);
        assert!(at("  ↳ its task  #1").is_some(), "{lines:#?}");
        assert!(
            at("    lead asks lead/tests for 6 lines").is_some(),
            "{lines:#?}"
        );
        // Each by its name in the chat's window: its part, and the lead its
        // model.
        assert!(at("  ✉ tests → m  #2").is_some(), "{lines:#?}");
        assert!(
            at("    lead/tests asks lead for 1 lines").is_some(),
            "{lines:#?}"
        );
        assert!(at("  Docs too.").is_some(), "{lines:#?}");
        assert!(!lines.iter().any(|l| l.contains("Reading the module")));
        assert!(!lines.iter().any(|l| l.contains("Not this")));
        // Its summary is the page's too.
        assert_eq!(
            history.summed(2).as_deref(),
            Some("lead/tests asks lead for 1 lines")
        );
    }

    #[test]
    fn without_a_model_to_sum_up_a_message_its_first_lines_are_cut() {
        let p = Palette::new(Theme::Ink, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        for (agent, by) in [("lead", PERSON), ("lead/tests", "lead")] {
            crew(
                &mut board,
                &mut history,
                Event::Joined {
                    agent: agent.into(),
                    model: "m".into(),
                    by: by.into(),
                },
                &p,
            );
        }
        crew(
            &mut board,
            &mut history,
            Event::Posted {
                from: "lead".into(),
                to: "lead/tests".into(),
                text: format!("{}\nthe rest", "x".repeat(500)),
            },
            &p,
        );
        let lines = settled(&mut history, "lead");
        let kept: usize = lines.iter().map(|l| l.matches('x').count()).sum();
        assert!(kept < LINE_CHARS, "{lines:#?}");
        assert!(lines.iter().any(|l| l.ends_with('…')), "{lines:#?}");
        assert!(lines.contains(&"    the rest".to_owned()), "{lines:#?}");
    }
}
