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

/// The longest tool input written, in characters.
pub(crate) const INPUT_CHARS: usize = 160;

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
enum Block {
    Para(Para),
    Whole(Line<'static>),
    Table(Table),
    Diagram {
        diagram: Arc<Diagram>,
        code: Vec<Block>,
        said: Style,
        failed: Style,
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
            Block::Para(Para::text(styled))
        }
    }

    /// Whether it is a diagram being drawn, which what comes after it waits
    /// for.
    fn drawing(&self) -> bool {
        match self {
            Block::Diagram { diagram, .. } => diagram.drawing(),
            Block::Summed { summary, .. } => summary.making(),
            Block::Job { label, .. } => label.making(),
            Block::Para(_) | Block::Whole(_) | Block::Table(_) => false,
        }
    }

    /// Its lines within `width` columns, on cells of `cell` pixels, the
    /// images they show put in `images`.
    fn lines(&self, width: usize, cell: (u16, u16), images: &mut Vec<Image>) -> Vec<Line<'static>> {
        let indent = [Span::raw("  ")];
        match self {
            Block::Para(para) => wrap(&para.body, width, &para.first, &para.rest),
            Block::Whole(line) => vec![line.clone()],
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
                said,
                failed,
            } => match diagram.shown(width, &indent, cell, *said, images) {
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
    /// The id the next image takes with the terminal.
    next_image: u32,
    /// Whether what follows a diagram being drawn waits for it: not as iq
    /// ends.
    hurried: bool,
    /// Whether diagrams are drawn as images, in Kitty.
    drawn_images: bool,
    /// The questions with choices asked, by number: the agent that asked,
    /// and the window the answer goes in.
    chosen: HashMap<u64, (String, Option<String>)>,
    /// The commands left to run in the background, by number: what they
    /// do in a few words, and since when they run.
    jobs: HashMap<u64, (Label, Instant)>,
    /// The diagrams shown as images since last asked, each with the chat
    /// whose window shows it and the file its image is kept in.
    drawn: Vec<(Option<String>, std::path::PathBuf)>,
    /// The images of the diagrams shown, by their files: their ids with
    /// the terminal, their PNGs and their sizes in pixels.
    pictures: HashMap<std::path::PathBuf, crate::diagram::Shot>,
    /// What sums up the messages between agents.
    summarizer: Summarizer,
    /// The summaries of the messages between agents, by their numbers.
    summaries: HashMap<usize, Arc<Summary>>,
    /// The agents another started that the person wrote to: what they
    /// write until their turn ends is theirs to read.
    woken: HashSet<String>,
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
        self.drawn_images = draw.is_some();
        self.drawer = Drawer(draw);
        self.set_cell(cell);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| {
                d.subsec_nanos() ^ u32::try_from(d.as_secs() & 0xff_ffff).unwrap_or(1)
            });
        self.next_image = now & 0x00ff_ffff;
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

    /// The size of a cell in pixels, as the terminal said it.
    pub(crate) fn cell(&self) -> (u16, u16) {
        self.cell
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
                    format!(" · lead {lead} · reviewer {reviewer}"),
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
            // before it.
            Event::Tool { agent, .. } | Event::Reviewed { agent, .. } => {
                self.finish(agent, board, p);
            }
            Event::Asked { .. } | Event::Answered { .. } => {}
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
            // The question as it is asked, its answer below it once given:
            // the live part shows its choices meanwhile.
            Event::Choose {
                id,
                agent,
                question,
                ..
            } => {
                self.finish(agent, board, p);
                self.header(agent, board, p);
                self.chosen.insert(*id, (agent.clone(), self.at.clone()));
                self.push_step(Para {
                    first: vec![Span::raw("  "), Span::styled("? ", p.fg(p.warn))],
                    rest: vec![Span::raw("    ")],
                    body: inline(question.trim(), p.fg(p.text), p, self.links),
                });
            }
            Event::Chosen { id, answer } => {
                if let Some((agent, at)) = self.chosen.remove(id) {
                    self.at = at;
                    self.header(&agent, board, p);
                    let (said, style) = match answer {
                        Some(answer) => (answer.clone(), p.fg(p.accent)),
                        None => ("no answer".to_owned(), p.fg(p.faint)),
                    };
                    self.push_step(Para {
                        first: vec![Span::raw("    "), Span::styled("→ ", p.fg(p.faint))],
                        rest: vec![Span::raw("      ")],
                        body: vec![Span::styled(said, style)],
                    });
                }
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
                if let (Some(ran), Some(unfold)) = (board.steps().latest_ran(), unfold) {
                    tail.push(Span::styled(
                        format!("  {unfold} unfolds #{}", ran.id),
                        p.fg(p.faint),
                    ));
                }
                self.push_job(Block::Job {
                    head: vec![
                        Span::raw("  "),
                        Span::styled("⚙ ", p.fg(colour)),
                        Span::styled(format!("job {id} · "), p.fg(p.dim)),
                    ],
                    label,
                    tail,
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
            Event::Notice { agent, text } => {
                self.finish(agent, board, p);
                self.header(agent, board, p);
                self.push(Para {
                    first: vec![Span::raw("  "), Span::styled("! ", p.fg(p.warn))],
                    rest: vec![Span::raw("    ")],
                    body: vec![Span::styled(short(text), p.fg(p.dim))],
                });
            }
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
        let mut steps: Vec<(String, String, bool)> = Vec::new();
        for entry in &view.thread[from..] {
            use iq_ui::crew::Entry;
            if !matches!(entry, Entry::Tool { .. }) {
                self.recalled_steps(agent, &mut steps, board, p);
            }
            match entry {
                Entry::Prompt(text) => {
                    let text = text.trim();
                    match text
                        .strip_prefix("Message from ")
                        .and_then(|rest| rest.split_once(": "))
                    {
                        Some((PERSON, said)) => self.person(said, agent, board, p),
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
                Entry::Tool {
                    name, input, ok, ..
                } => steps.push((name.clone(), input.clone(), *ok)),
                Entry::Reviewed {
                    action,
                    approved,
                    why,
                } => {
                    self.header(agent, board, p);
                    let (mark, colour, did) = if *approved {
                        ("✓ ", p.ok, "allowed")
                    } else {
                        ("✗ ", p.bad, "refused")
                    };
                    self.push_step(Para {
                        first: vec![Span::raw("  "), Span::styled(mark, p.fg(colour))],
                        rest: vec![Span::raw("    ")],
                        body: vec![Span::styled(
                            format!("the reviewer {did} {}: {}", short(action), short(why)),
                            p.fg(p.dim),
                        )],
                    });
                }
                Entry::Asked { action, answer, .. } => {
                    self.header(agent, board, p);
                    let said = match answer {
                        Some(true) => "you allowed",
                        Some(false) => "you refused",
                        None => "asked you, unanswered:",
                    };
                    self.push_step(Para {
                        first: vec![Span::raw("  "), Span::styled("? ", p.fg(p.warn))],
                        rest: vec![Span::raw("    ")],
                        body: vec![Span::styled(
                            format!("{said} {}", short(action)),
                            p.fg(p.dim),
                        )],
                    });
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
                Entry::Notice(text) => {
                    self.header(agent, board, p);
                    self.push(Para {
                        first: vec![Span::raw("  "), Span::styled("! ", p.fg(p.warn))],
                        rest: vec![Span::raw("    ")],
                        body: vec![Span::styled(short(text), p.fg(p.dim))],
                    });
                }
                Entry::Chose { question, answer } => {
                    self.header(agent, board, p);
                    self.push_step(Para {
                        first: vec![Span::raw("  "), Span::styled("? ", p.fg(p.warn))],
                        rest: vec![Span::raw("    ")],
                        body: vec![
                            Span::styled(question.trim().to_owned(), p.fg(p.text)),
                            Span::styled(" → ", p.fg(p.faint)),
                            Span::styled(
                                answer.clone().unwrap_or_else(|| "no answer".to_owned()),
                                p.fg(p.accent),
                            ),
                        ],
                    });
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
            }
        }
        self.recalled_steps(agent, &mut steps, board, p);
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

    /// Writes the steps of a chat coming back since its last line, as a
    /// fold is written once its agent goes on: one step as itself, more as
    /// their count, by tool.
    fn recalled_steps(
        &mut self,
        agent: &str,
        steps: &mut Vec<(String, String, bool)>,
        board: &Board,
        p: &Palette,
    ) {
        if steps.is_empty() {
            return;
        }
        let taken = std::mem::take(steps);
        self.header(agent, board, p);
        let body = match taken.as_slice() {
            [(name, input, ok)] => {
                let colour = if *ok { p.dim } else { p.bad };
                vec![
                    Span::styled(name.clone(), p.fg(p.text)),
                    Span::styled(format!(" {}", short(input)), p.fg(colour)),
                ]
            }
            all => {
                let mut tools: Vec<(String, usize)> = Vec::new();
                for (name, _, _) in all {
                    match tools.iter_mut().find(|(seen, _)| seen == name) {
                        Some((_, count)) => *count += 1,
                        None => tools.push((name.clone(), 1)),
                    }
                }
                let named: Vec<String> = tools
                    .iter()
                    .map(|(name, count)| {
                        if *count > 1 {
                            format!("{name} ×{count}")
                        } else {
                            name.clone()
                        }
                    })
                    .collect();
                let failed = all.iter().filter(|(_, _, ok)| !ok).count();
                let mut said = format!("{} steps: {}", all.len(), named.join(", "));
                if failed > 0 {
                    said.push_str(&format!(", {failed} failed"));
                }
                vec![Span::styled(said, p.fg(p.dim))]
            }
        };
        self.push_step(Para {
            first: vec![Span::raw("  "), Span::styled("▸ ", p.fg(p.faint))],
            rest: vec![Span::raw("    ")],
            body,
        });
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
        self.push(Para {
            first: vec![Span::styled("› ", p.bold(p.accent))],
            rest: vec![Span::raw("  ")],
            body,
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
        match (&self.summarizer.0, id) {
            (Some(summarize), Some(id)) if !short => {
                let summary = Summary::new(from, to, text, Arc::clone(summarize));
                self.summaries.insert(id, Arc::clone(&summary));
                self.push_block(Block::Summed {
                    summary,
                    instead: first_lines(text, INDENT, p, MESSAGE_LINES),
                    indent: INDENT.to_owned(),
                    style: p.fg(p.dim),
                });
            }
            _ => self.lines_of(text, INDENT, p, MESSAGE_LINES),
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
        let unfold = board
            .keys()
            .chord(Action::Steps)
            .map(|chord| chord.to_string());
        for fold in new {
            match fold.steps.as_slice() {
                [] => {}
                [step] => {
                    let (first, body) = step_line(step, p);
                    self.push_step(Para {
                        first,
                        rest: vec![Span::raw("    ")],
                        body,
                    });
                    if let Step::Tool {
                        output, ok: false, ..
                    } = step
                    {
                        for line in output
                            .lines()
                            .filter(|l| !l.trim().is_empty())
                            .take(FAILED_LINES)
                        {
                            self.push_step(Para::indented(
                                "    ",
                                vec![Span::styled(cut(&short(line), LINE_CHARS), p.fg(p.faint))],
                            ));
                        }
                    }
                }
                _ => self.push_step(Para {
                    first: vec![Span::raw("  "), Span::styled("▸ ", p.fg(p.faint))],
                    rest: vec![Span::raw("    ")],
                    body: counted(fold, unfold.as_deref(), p),
                }),
            }
        }
    }

    /// `agent`'s name and model, when another spoke last.
    fn named(&mut self, agent: &str, board: &Board, p: &Palette) {
        if self.speaker() == Some(agent) {
            return;
        }
        let tint = agent_tint(board, agent, p);
        // In its chat's window: by its model, or its part and its model.
        let (name, beside) = label(board, agent);
        let beside = if beside.is_empty() {
            String::new()
        } else {
            format!("  {beside}")
        };
        self.blank();
        self.push(Para::indented(
            "",
            vec![
                Span::styled("● ", p.fg(tint)),
                Span::styled(name, p.bold(tint)),
                Span::styled(beside, p.fg(p.faint)),
            ],
        ));
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
                    let id = self.next_image.max(1);
                    self.next_image = (id + 1) & 0x00ff_ffff;
                    let diagram = Diagram::new(source, id, self.drawer.0.clone());
                    self.push_text_block(Block::Diagram {
                        diagram,
                        code,
                        said: p.fg(p.faint),
                        failed: p.fg(p.warn),
                    });
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
            if chat == window {
                lines.extend(block.lines(width, self.cell, &mut self.images));
                self.shown(&chat, &block);
            }
            let kept = self.kept.entry(chat).or_default();
            kept.push_back(block);
            while kept.len() > KEPT {
                kept.pop_front();
            }
        }
        lines
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
        if let Some(kind) = self.kinds.remove(&None) {
            self.kinds.insert(key.clone(), kind);
        }
        if let Some(speaker) = self.speakers.remove(&None) {
            self.speakers.insert(key, speaker);
        }
        true
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
            self.shown(&key, &block);
        }
        lines
    }

    /// Notes the image of `block`, when it is a diagram shown as one, as
    /// shown in `chat`'s window.
    fn shown(&mut self, chat: &Option<String>, block: &Block) {
        let Block::Diagram { diagram, .. } = block else {
            return;
        };
        if let Some((shot, path)) = diagram.image()
            && self.drawn_images
        {
            self.pictures.insert(path.clone(), shot);
            self.drawn.push((chat.clone(), path));
        }
    }

    /// The diagrams shown as images since last asked, with their chats.
    pub(crate) fn take_drawn(&mut self) -> Vec<(Option<String>, std::path::PathBuf)> {
        std::mem::take(&mut self.drawn)
    }

    /// The image of the diagram kept in `path`: its id, its PNG and its
    /// size in pixels.
    pub(crate) fn picture(&self, path: &std::path::Path) -> Option<crate::diagram::Shot> {
        self.pictures.get(path).cloned()
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
        if self.kind() == Some(Kind::Text) {
            self.blank();
        }
        self.push(para);
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
    let Some(view) = board.crew().agent(agent) else {
        return (agent.to_owned(), String::new());
    };
    let model = view.serving();
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
        Step::Answered { action, approved } => {
            let (said, tint) = if *approved {
                ("✓ you allowed ", p.ok)
            } else {
                ("✗ refused ", p.bad)
            };
            (
                vec![Span::raw("  "), Span::styled(said, p.fg(tint))],
                vec![Span::styled(one(action), p.fg(p.dim))],
            )
        }
        Step::Reviewed {
            action,
            approved,
            why,
        } => {
            let (said, tint) = if *approved {
                ("⏵⏵ the reviewer allowed ", p.ok)
            } else {
                ("⏵⏵ the reviewer refused ", p.bad)
            };
            (
                vec![Span::raw("  "), Span::styled(said, p.fg(tint))],
                vec![Span::styled(
                    format!("{}: {}", one(action), short(why)),
                    p.fg(p.dim),
                )],
            )
        }
    }
}

/// A fold's count: how many steps, of which kinds, how many went wrong,
/// and the key that unfolds them.
fn counted(fold: &Fold, unfold: Option<&str>, p: &Palette) -> Vec<Span<'static>> {
    let kinds = fold.kinds();
    let mut named: Vec<String> = kinds
        .iter()
        .take(KINDS)
        .map(|(kind, count)| format!("{kind} {count}"))
        .collect();
    if kinds.len() > KINDS {
        named.push("…".to_owned());
    }
    let mut spans = vec![
        Span::styled(format!("{} steps", fold.steps.len()), p.fg(p.text)),
        Span::styled(format!(" · {}", named.join(", ")), p.fg(p.dim)),
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
            Ok(crate::diagram::Drawn {
                png,
                source: None,
                image: None,
            })
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
        // 200 by 100 pixels on cells of 10 by 20: 5 rows of 20 cells.
        assert_eq!(after.iter().filter(|l| l.contains('\u{10EEEE}')).count(), 5);
        assert_eq!(after.last().map(String::as_str), Some("  after it"));
        let images = history.take_images();
        assert_eq!((images.len(), images[0].cols, images[0].rows), (1, 20, 5));
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
        let mut thread = Vec::new();
        for n in 1..=4 {
            thread.push(Entry::Prompt(format!("Message from person: ask {n}")));
            thread.push(Entry::Tool {
                name: "run".into(),
                input: "make test".into(),
                output: "ok".into(),
                ok: true,
            });
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
        // step folded, its answer.
        for n in 1..=4 {
            shown.extend([
                String::new(),
                format!("› ask {n}"),
                String::new(),
                "● model".to_owned(),
                "  ▸ run make test".to_owned(),
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
        // Gone to again, its window is the same conversation.
        let again: Vec<String> = history
            .window(100, Some("login"))
            .iter()
            .map(|l| l.to_string().trim_end().to_owned())
            .collect();
        assert!(again.ends_with(&trimmed), "{again:#?}");
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
                "  ▸ 4 steps · read_file 2, run 1, asked you 1 · 1 went wrong  Ctrl-X unfolds #1",
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
