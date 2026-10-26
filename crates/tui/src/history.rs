//! The conversation as the terminal keeps it: what the crew does, written
//! once into the terminal's own scrollback as it happens, where the person
//! scrolls, selects and copies it as any output. Each agent's text is
//! written a line at a time as it comes, under the agent's name when
//! another spoke last; its tools, its questions answered, its messages and
//! the end of its turns each take a line.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use iq_agent::{Event, PERSON};
use iq_core::{TokenCount, Usd};
use iq_ui::Board;
use ratatui::text::{Line, Span};

use crate::markdown::{Markdown, Styled, inline};
use crate::theme::Palette;
use crate::wrap::{cut, wrap};

/// How many lines of a message between agents are written.
const MESSAGE_LINES: usize = 4;

/// How many lines of a failed tool's output are written.
const FAILED_LINES: usize = 3;

/// The longest tool input written, in characters.
const INPUT_CHARS: usize = 160;

/// How many paragraphs written are kept, to write the end of the
/// conversation again when the terminal is made narrower.
const KEPT: usize = 400;

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

/// The conversation, as written so far.
#[derive(Debug, Default)]
pub(crate) struct History {
    paras: Vec<Para>,
    /// The last paragraphs written.
    written: VecDeque<Para>,
    /// Whose lines were written last: the person, an agent, or nobody's
    /// since a line of the crew's own.
    speaker: Option<String>,
    streams: HashMap<String, Stream>,
    turns: HashMap<String, Turn>,
    /// The agents whose task was written.
    tasked: HashSet<String>,
    /// What each question waiting asks, to say it once answered.
    asked: HashMap<u64, (String, String)>,
    /// What to say after the person's next message to an agent: the models
    /// they named in it.
    after: HashMap<String, String>,
}

impl History {
    /// The lines iq opens with: who leads, where, and how to begin.
    pub(crate) fn opening(&mut self, lead: &str, reviewer: &str, p: &Palette) {
        let folder = home_short(
            &std::env::current_dir()
                .map(|f| f.display().to_string())
                .unwrap_or_default(),
        );
        self.paras.push(Para::indented(
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
        self.paras.push(Para::indented(
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
        match event {
            Event::Joined { agent, model, by } if by != PERSON => {
                self.speaker = None;
                let tint = agent_tint(board, agent, p);
                self.paras.push(Para::indented(
                    "",
                    vec![
                        Span::styled("◦ ", p.fg(tint)),
                        Span::styled(agent.clone(), p.fg(tint)),
                        Span::styled(format!(" joins, on {model}, started by {by}"), p.fg(p.dim)),
                    ],
                ));
            }
            // The box where the person writes says how hard its agent thinks.
            Event::Joined { .. } | Event::Effort { .. } => {}
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
                let by = board.crew().agent(agent).map(|a| a.by.as_str());
                if by == Some(PERSON) || by.is_none() {
                    self.person(prompt, agent, board, p);
                } else {
                    self.header(agent, board, p);
                    self.lines_of(prompt, "  ↳ its task: ", p, MESSAGE_LINES);
                }
            }
            Event::TextStart { agent } => self.finish(agent, board, p),
            Event::Text { agent, text } => {
                let stream = self.streams.entry(agent.clone()).or_default();
                stream.partial.push_str(text);
                while let Some(stream) = self.streams.get_mut(agent)
                    && let Some(end) = stream.partial.find('\n')
                {
                    let line: String = stream.partial.drain(..=end).collect();
                    let styled = stream.markdown.line(line.trim_end_matches(['\n', '\r']), p);
                    if let Some(styled) = styled {
                        self.header(agent, board, p);
                        self.paras.push(Para::text(styled));
                    }
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
            Event::Tool {
                agent,
                name,
                input,
                output,
                ok,
            } => {
                self.finish(agent, board, p);
                self.header(agent, board, p);
                let (mark, tint) = if *ok {
                    ("▸ ", p.faint)
                } else {
                    ("✗ ", p.bad)
                };
                self.paras.push(Para {
                    first: vec![Span::raw("  "), Span::styled(mark, p.fg(tint))],
                    rest: vec![Span::raw("    ")],
                    body: vec![
                        Span::styled(name.clone(), p.fg(p.text)),
                        Span::raw(" "),
                        Span::styled(
                            cut(&short(input).replace('\n', " "), INPUT_CHARS),
                            p.fg(p.dim),
                        ),
                    ],
                });
                if !ok {
                    for line in output
                        .lines()
                        .filter(|l| !l.trim().is_empty())
                        .take(FAILED_LINES)
                    {
                        self.paras.push(Para::indented(
                            "    ",
                            vec![Span::styled(short(line), p.fg(p.faint))],
                        ));
                    }
                }
            }
            Event::Asked {
                id, agent, action, ..
            } => {
                self.asked.insert(*id, (agent.clone(), action.clone()));
            }
            Event::Answered { id, approved } => {
                if let Some((agent, action)) = self.asked.remove(id) {
                    self.finish(&agent, board, p);
                    self.header(&agent, board, p);
                    let (said, tint) = if *approved {
                        ("✓ you allowed ", p.ok)
                    } else {
                        ("✗ refused ", p.bad)
                    };
                    self.paras.push(Para::indented(
                        "  ",
                        vec![
                            Span::styled(said, p.fg(tint)),
                            Span::styled(short(&action), p.fg(p.dim)),
                        ],
                    ));
                }
            }
            Event::Reviewed {
                agent,
                action,
                approved,
                why,
                ..
            } => {
                self.finish(agent, board, p);
                self.header(agent, board, p);
                let (said, tint) = if *approved {
                    ("⏵⏵ the reviewer allowed ", p.ok)
                } else {
                    ("⏵⏵ the reviewer refused ", p.bad)
                };
                self.paras.push(Para::indented(
                    "  ",
                    vec![
                        Span::styled(said, p.fg(tint)),
                        Span::styled(format!("{}: {why}", short(action)), p.fg(p.dim)),
                    ],
                ));
            }
            Event::Posted { from, to, text } => {
                if from == PERSON {
                    self.person(text, to, board, p);
                } else if to == PERSON {
                    self.finish(from, board, p);
                    self.header(from, board, p);
                    self.paras.push(Para::indented(
                        "  ",
                        vec![Span::styled("✉ to you", p.fg(p.info))],
                    ));
                    let mut markdown = Markdown::default();
                    for line in text.lines() {
                        if let Some(styled) = markdown.line(line, p) {
                            self.paras.push(Para::text(styled));
                        }
                    }
                } else {
                    self.finish(from, board, p);
                    self.speaker = None;
                    self.paras.push(Para::indented(
                        "  ",
                        vec![
                            Span::styled("✉ ", p.fg(p.info)),
                            Span::styled(from.clone(), p.fg(agent_tint(board, from, p))),
                            Span::styled(" → ", p.fg(p.faint)),
                            Span::styled(to.clone(), p.fg(agent_tint(board, to, p))),
                        ],
                    ));
                    self.lines_of(text, "    ", p, MESSAGE_LINES);
                }
            }
            Event::Done { agent, outcome } => {
                self.finish(agent, board, p);
                self.header(agent, board, p);
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
                        self.paras.push(Para::indented(
                            "  ",
                            vec![
                                Span::styled("✔ ", p.fg(p.ok)),
                                Span::styled(said, p.fg(p.faint)),
                            ],
                        ));
                    }
                    Err(error) => self.paras.push(Para {
                        first: vec![Span::raw("  "), Span::styled("✘ stopped: ", p.fg(p.bad))],
                        rest: vec![Span::raw("    ")],
                        body: vec![Span::styled(short(error), p.fg(p.dim))],
                    }),
                }
                self.speaker = None;
            }
            Event::Notice { agent, text } => {
                self.finish(agent, board, p);
                self.header(agent, board, p);
                self.paras.push(Para {
                    first: vec![Span::raw("  "), Span::styled("! ", p.fg(p.warn))],
                    rest: vec![Span::raw("    ")],
                    body: vec![Span::styled(short(text), p.fg(p.dim))],
                });
            }
        }
    }

    /// Something said to the person by iq itself, in passing.
    pub(crate) fn say(&mut self, text: &str, p: &Palette) {
        self.speaker = None;
        self.paras.push(Para::indented(
            "  ",
            vec![Span::styled(text.to_owned(), p.fg(p.dim))],
        ));
    }

    /// Says `text` right after the person's next message to `agent`.
    pub(crate) fn after_message(&mut self, agent: &str, text: String) {
        self.after.insert(agent.to_owned(), text);
    }

    /// What the person wrote, to `agent`.
    fn person(&mut self, text: &str, agent: &str, board: &Board, p: &Palette) {
        self.paras.push(Para::blank());
        let mut body = Vec::new();
        if board.crew().agents().len() > 1 {
            body.push(Span::styled(
                format!("@{agent} "),
                p.fg(agent_tint(board, agent, p)),
            ));
        }
        body.extend(inline(text.trim(), p.fg(p.text), p));
        self.paras.push(Para {
            first: vec![Span::styled("› ", p.bold(p.accent))],
            rest: vec![Span::raw("  ")],
            body,
        });
        if let Some(said) = self.after.remove(agent) {
            self.paras
                .push(Para::indented("  ", vec![Span::styled(said, p.fg(p.info))]));
        }
        self.speaker = Some(PERSON.to_owned());
    }

    /// The first lines of `text`, dimmed, after `first`.
    fn lines_of(&mut self, text: &str, first: &str, p: &Palette, most: usize) {
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        for (at, line) in lines.iter().take(most).enumerate() {
            let lead = if at == 0 {
                Span::styled(first.to_owned(), p.fg(p.info))
            } else {
                Span::raw(" ".repeat(crate::wrap::width(first)))
            };
            let more = if at + 1 == most && lines.len() > most {
                format!("{line} …")
            } else {
                (*line).to_owned()
            };
            self.paras.push(Para {
                first: vec![lead],
                rest: vec![Span::raw(" ".repeat(crate::wrap::width(first)))],
                body: vec![Span::styled(more, p.fg(p.dim))],
            });
        }
    }

    /// `agent`'s name and model, when another spoke last.
    fn header(&mut self, agent: &str, board: &Board, p: &Palette) {
        if self.speaker.as_deref() == Some(agent) {
            return;
        }
        let tint = agent_tint(board, agent, p);
        let model = board
            .crew()
            .agent(agent)
            .map(|a| a.serving().to_owned())
            .unwrap_or_default();
        self.paras.push(Para::blank());
        self.paras.push(Para::indented(
            "",
            vec![
                Span::styled("● ", p.fg(tint)),
                Span::styled(agent.to_owned(), p.bold(tint)),
                Span::styled(format!("  {model}"), p.fg(p.faint)),
            ],
        ));
        self.speaker = Some(agent.to_owned());
    }

    /// Writes what is left of `agent`'s text after its last whole line.
    fn finish(&mut self, agent: &str, board: &Board, p: &Palette) {
        let Some(stream) = self.streams.get_mut(agent) else {
            return;
        };
        let rest = std::mem::take(&mut stream.partial);
        if rest.trim().is_empty() {
            return;
        }
        if let Some(styled) = stream.markdown.line(&rest, p) {
            self.header(agent, board, p);
            self.paras.push(Para::text(styled));
        }
    }

    /// Writes what every agent left unfinished, as iq ends.
    pub(crate) fn finish_all(&mut self, board: &Board, p: &Palette) {
        let agents: Vec<String> = self.streams.keys().cloned().collect();
        for agent in agents {
            self.finish(&agent, board, p);
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
        !self.paras.is_empty()
    }

    /// The lines waiting, folded within `width` columns.
    pub(crate) fn take(&mut self, width: usize) -> Vec<Line<'static>> {
        let paras: Vec<Para> = std::mem::take(&mut self.paras);
        let lines = paras
            .iter()
            .flat_map(|para| wrap(&para.body, width, &para.first, &para.rest))
            .collect();
        self.written.extend(paras);
        while self.written.len() > KEPT {
            self.written.pop_front();
        }
        lines
    }

    /// The last of the lines written, as many as `rows`, folded within
    /// `width` columns.
    pub(crate) fn recent(&self, width: usize, rows: usize) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        for para in self.written.iter().rev() {
            let mut folded = wrap(&para.body, width, &para.first, &para.rest);
            folded.append(&mut lines);
            lines = folded;
            if lines.len() >= rows {
                break;
            }
        }
        let extra = lines.len().saturating_sub(rows);
        lines.split_off(extra)
    }
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
        let lines: Vec<String> = history.take(80).iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "",
                "› fix the build",
                "",
                "● lead  anthropic/claude-opus-5-5",
                "  I will look",
                "  at the tests",
                "  ✗ Bash cargo test",
                "    error: one",
                "    error: two",
                "  ✔ 0s",
            ]
        );
        assert!(!history.waiting());
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
        let lines: Vec<String> = history.take(80).iter().map(ToString::to_string).collect();
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
        let lines: Vec<String> = history.take(80).iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "◦ helper joins, on m, started by lead",
                "  ✉ lead → helper",
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
}
