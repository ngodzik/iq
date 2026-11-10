//! The map of a chat's turn, drawn: a line for the turn (how long it runs,
//! what it spent against what it may, by model), then a row for each agent
//! under the one that started it and for each job under its agent, then a
//! foot for what no longer matters. Each row says its state with a glyph
//! that holds without colour, what it spent in the turn and in how many
//! calls, how full its conversation is, and what it does now in a few
//! words. A message between agents runs along the left as a spark, from
//! its writer's row to its reader's, which then glows; one between two
//! agents neither of which started the other draws an arc a moment, with
//! its first words. Nothing moves while nothing happens.
//!
//! The page shows it whole; the strip, below the conversation, its head
//! and the agents at work, the rest folded.

use iq_ui::Board;
use iq_ui::crew::{AgentView, Entry, JobView, TurnSpending, TurnView};
use iq_ui::map::{Kind, Row};
use ratatui::text::{Line, Span};

use crate::history::{agent_tint, seconds, short};
use crate::motion::Motion;
use crate::theme::Palette;
use crate::wrap::{cut, fit, width as text_width};

/// The cells of a gauge.
const GAUGE: usize = 9;

/// The cells the split of a turn's cost by model takes.
const SPLIT: usize = 16;

/// Where a conversation's gauge is full when its model says nothing:
/// a long conversation, for most models.
const FULL: u64 = 200_000;

/// Past this share of what a turn may spend, its gauge warns.
const NEAR: f64 = 0.8;

/// What the map is drawn from.
pub(crate) struct Map<'a> {
    pub(crate) board: &'a Board,
    pub(crate) p: &'a Palette,
    pub(crate) motion: &'a Motion,
    /// Seconds since 1970.
    pub(crate) now: u64,
    /// What a job does in a few words, by its number, as the conversation
    /// says it.
    pub(crate) labels: &'a dyn Fn(u64) -> Option<String>,
    /// What an agent writes now, while it writes.
    pub(crate) writing: &'a dyn Fn(&str) -> Option<String>,
}

impl Map<'_> {
    /// The turn's line: its number, how long it runs, what it spent, with
    /// a gauge against what it may when the settings say, and its cost
    /// split by model.
    pub(crate) fn head(&self, chat: &str, width: usize) -> Line<'static> {
        let p = self.p;
        let Some(turn) = self.board.crew().turn(chat) else {
            return Line::from(Span::styled(format!(" {chat}: no turn yet"), p.fg(p.faint)));
        };
        let mut spans = vec![
            Span::styled(format!(" turn {}", turn.number), p.bold(p.text)),
            Span::styled(
                format!(" · {}", seconds(self.now.saturating_sub(turn.since))),
                p.fg(p.dim),
            ),
            Span::styled(format!(" · {}", turn.total), p.fg(p.text)),
        ];
        if let Some(at) = turn.ask_at.filter(|at| at.0 > 0.0) {
            let share = turn.total.0 / at.0;
            let tint = if share >= NEAR { p.warn } else { p.accent };
            spans.push(Span::styled(format!(" of {at} "), p.fg(p.dim)));
            spans.extend(gauge(share, tint, p));
        }
        let split = split(turn);
        if !split.is_empty() {
            spans.push(Span::raw("   "));
            for (model, cells) in split {
                spans.push(Span::styled(format!("{model} "), p.fg(p.dim)));
                spans.push(Span::styled("█".repeat(cells), p.fg(p.accent)));
                spans.push(Span::raw(" "));
            }
        }
        fit(spans, width)
    }

    /// The rows of `chat`'s turn within `width`, the row `chosen` marked:
    /// all of them, or those at work alone, the strip's.
    pub(crate) fn rows(
        &self,
        chat: &str,
        width: usize,
        chosen: Option<usize>,
        finished: bool,
        at_work: bool,
    ) -> Vec<Line<'static>> {
        let crew = self.board.crew();
        let rows = iq_ui::map::rows(crew, chat, finished, self.now);
        let turn = crew.turn(chat);
        let columns = Columns::of(&rows, width);
        let job_lines = self.board.look().job_lines();
        // Where each agent's row stands, for the sparks and the arcs.
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut at_row: Vec<(String, usize)> = Vec::new();
        for (at, row) in rows.iter().enumerate() {
            let shown = match row.kind {
                Kind::Agent(agent) => !at_work || agent.working || row.depth() == 0,
                Kind::Job(_) => true,
            };
            if !shown {
                continue;
            }
            let on = chosen == Some(at);
            match row.kind {
                Kind::Agent(agent) => {
                    at_row.push((agent.name.clone(), lines.len()));
                    lines.push(self.agent_row(row, agent, turn, &columns, on, width));
                    // Between two agents neither of which started the other:
                    // a spark runs along the tree between the others.
                    let peers = |to: &str| {
                        agent.by != to
                            && crew.agent(to).is_none_or(|reader| reader.by != agent.name)
                    };
                    for arc in self
                        .motion
                        .arcs()
                        .iter()
                        .filter(|arc| arc.from == agent.name && peers(&arc.to))
                    {
                        lines.push(self.arc_row(row, &arc.to, &arc.words, width));
                    }
                }
                Kind::Job(job) => {
                    lines.push(self.job_row(row, job, on, width));
                    let from = job.tail.len().saturating_sub(job_lines);
                    for said in &job.tail[from..] {
                        let mut spans = vec![Span::raw(" ".to_owned())];
                        spans.extend(tree(&row.lasts, true, self.p));
                        spans.push(Span::styled("    │ ", self.p.fg(self.p.faint)));
                        spans.push(Span::styled(short(said), self.p.fg(self.p.dim)));
                        lines.push(fit(spans, width));
                    }
                }
            }
        }
        self.sparks(&mut lines, &at_row);
        lines
    }

    /// The foot: how many finished, folded, and what the turn spent beside
    /// its agents (the autopilot's model, the summaries).
    pub(crate) fn foot(&self, chat: &str, width: usize, finished: bool) -> Option<Line<'static>> {
        let p = self.p;
        let crew = self.board.crew();
        let mut said: Vec<Span<'static>> = Vec::new();
        let count = iq_ui::map::finished(crew, chat);
        if count > 0 {
            let how = if finished { "shown" } else { "folded" };
            said.push(Span::styled(
                format!("{count} finished, {how}"),
                p.fg(p.dim),
            ));
        }
        if let Some(turn) = crew.turn(chat) {
            for spent in turn.by.iter().filter(|spent| beside(spent)) {
                if !said.is_empty() {
                    said.push(Span::styled(" · ", p.fg(p.faint)));
                }
                let who = if spent.who == "reviewer" {
                    "autopilot"
                } else {
                    spent.who.as_str()
                };
                said.push(Span::styled(format!("{who} {}", spent.spent), p.fg(p.dim)));
            }
        }
        if said.is_empty() {
            return None;
        }
        let mut spans = vec![Span::raw("   ")];
        spans.extend(said);
        Some(fit(spans, width))
    }

    /// An agent's row: its place in the tree, its state, its part, model,
    /// what it spent in the turn and in how many calls, how full its
    /// conversation is, what it does.
    fn agent_row(
        &self,
        row: &Row<'_>,
        agent: &AgentView,
        turn: Option<&TurnView>,
        columns: &Columns,
        on: bool,
        width: usize,
    ) -> Line<'static> {
        let p = self.p;
        let tint = agent_tint(self.board, &agent.name, p);
        let (glyph, state) = self.state(agent);
        let glow = self.motion.glow(&agent.name);
        let glyph_tint = if glow > 0.0 {
            state.mix(p.accent, glow)
        } else {
            state
        };
        let mut spans = vec![Span::styled(if on { "▸" } else { " " }, p.bold(p.accent))];
        spans.extend(tree(&row.lasts, false, p));
        spans.push(Span::styled(format!("{glyph} "), p.fg(glyph_tint)));
        let named = part(&agent.name);
        let pad = columns
            .name
            .saturating_sub(tree_width(&row.lasts) + 2 + text_width(&named));
        spans.push(Span::styled(
            named,
            if on { p.bold(tint) } else { p.fg(tint) },
        ));
        spans.push(Span::raw(" ".repeat(pad + 1)));
        let model = short_model(agent.serving());
        spans.push(Span::styled(
            format!("{model:<width$} ", width = columns.model),
            p.fg(p.dim),
        ));
        // One retired before the turn began, of an earlier session, spent
        // nothing in it: another may have its name now.
        let spent = turn
            .filter(|turn| !agent.retired || agent.worked.is_some_and(|at| at >= turn.since))
            .and_then(|turn| turn.by.iter().find(|spent| spent.who == agent.name));
        // A price unknown is said by nothing, not by a cost of nothing.
        let (cost, calls) = spent.map_or((String::new(), 0), |spent| {
            let cost = if spent.spent.0 > 0.0 {
                spent.spent.to_string()
            } else {
                String::new()
            };
            (cost, spent.calls)
        });
        spans.push(Span::styled(format!("{cost:>7} "), p.fg(p.text)));
        if columns.calls {
            let calls = match calls {
                0 => String::new(),
                1 => "1 call".to_owned(),
                n => format!("{n} calls"),
            };
            spans.push(Span::styled(format!("{calls:<9} "), p.fg(p.faint)));
        }
        if columns.context {
            let read = agent.context().0;
            if read > 0 {
                let full = agent.rises.or(agent.most).unwrap_or(FULL);
                let share = read as f64 / full as f64;
                let tint = if share >= 1.0 { p.warn } else { p.dim };
                spans.push(Span::styled("ctx ", p.fg(p.faint)));
                spans.extend(gauge(share, tint, p));
                spans.push(Span::styled(
                    format!(" {:<5}", iq_core::TokenCount(read).to_string()),
                    p.fg(p.faint),
                ));
            } else {
                spans.push(Span::raw(" ".repeat(GAUGE + 12)));
            }
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            self.doing(agent),
            p.fg(if agent.working { p.text } else { p.dim }),
        ));
        fit(spans, width)
    }

    /// A job's row: its number, what it does, how long it has run.
    fn job_row(&self, row: &Row<'_>, job: &JobView, on: bool, width: usize) -> Line<'static> {
        let p = self.p;
        let label =
            (self.labels)(job.id).unwrap_or_else(|| iq_ui::shell::few_words(&job.line, width));
        let mut spans = vec![Span::styled(if on { "▸" } else { " " }, p.bold(p.accent))];
        spans.extend(tree(&row.lasts, false, p));
        spans.extend([
            Span::styled(format!("{} ", self.motion.spinner()), p.fg(p.info)),
            Span::styled(format!("job {} · ", job.id), p.fg(p.dim)),
            Span::styled(label, p.fg(p.text)),
            Span::styled(
                format!(" · {}", seconds(self.now.saturating_sub(job.since))),
                p.fg(p.faint),
            ),
        ]);
        fit(spans, width)
    }

    /// A message between two agents neither of which started the other,
    /// under its writer's row a moment: to whom, and its first words.
    fn arc_row(&self, row: &Row<'_>, to: &str, words: &str, width: usize) -> Line<'static> {
        let p = self.p;
        let mut spans = vec![Span::raw(" ")];
        spans.extend(tree(&row.lasts, true, p));
        spans.extend([
            Span::styled("  ╰─ ✦ → ", p.fg(p.accent)),
            Span::styled(part(to), p.fg(agent_tint(self.board, to, p))),
            Span::styled(format!("  \"{words}\""), p.fg(p.dim)),
        ]);
        fit(spans, width)
    }

    /// The sparks on their way, each in the left margin between its
    /// writer's row and its reader's, as far as it ran.
    fn sparks(&self, lines: &mut [Line<'static>], at_row: &[(String, usize)]) {
        let row_of = |name: &str| {
            at_row
                .iter()
                .find(|(agent, _)| agent == name)
                .map(|(_, at)| *at)
        };
        for (from, to, t) in self.motion.sparks() {
            let (Some(from), Some(to)) = (row_of(from), row_of(to)) else {
                continue;
            };
            let at = (from as f32 + (to as f32 - from as f32) * t).round() as usize;
            if let Some(line) = lines.get_mut(at)
                && line.spans.first().is_some_and(|first| first.content == " ")
            {
                line.spans[0] = Span::styled("✦", self.p.fg(self.p.accent));
            }
        }
    }

    /// An agent's state, as a glyph that says it without colour, and the
    /// colour that says it too.
    fn state(&self, agent: &AgentView) -> (&'static str, crate::theme::Rgb) {
        let p = self.p;
        let crew = self.board.crew();
        let asks = crew.questions().iter().any(|q| q.agent == agent.name)
            || crew.choices().iter().any(|c| c.agent == agent.name);
        if asks {
            return ("?", p.warn);
        }
        if agent.working {
            return if agent.waiting.is_some() {
                ("◌", p.info)
            } else {
                (self.motion.spinner(), p.accent)
            };
        }
        match agent.thread.last() {
            Some(Entry::Done(Err(why))) if why.contains("stopped") => ("⏸", p.warn),
            Some(Entry::Done(Err(_))) => ("✗", p.bad),
            Some(Entry::Done(Ok(_))) => ("✓", p.ok),
            _ if agent.retired => ("✓", p.ok),
            _ => ("◌", p.faint),
        }
    }

    /// What an agent does now, in a few words: whom it waits for, what it
    /// writes, the tool it calls and on what; once done, how it ended.
    fn doing(&self, agent: &AgentView) -> String {
        if let Some(waiting) = &agent.waiting {
            let names: Vec<String> = waiting.on.iter().map(|name| part(name)).collect();
            return format!("waiting for {}", names.join(", "));
        }
        if agent.working {
            if let Some(writing) = (self.writing)(&agent.name).filter(|w| !w.trim().is_empty()) {
                return writing.replace('\n', " ");
            }
            return match agent.thread.iter().rev().find_map(|entry| match entry {
                Entry::Tool { name, input, .. } => Some((name, input)),
                _ => None,
            }) {
                Some((name, input)) => tool_said(name, input),
                None => "working".to_owned(),
            };
        }
        match agent.thread.last() {
            Some(Entry::Done(Ok(answer))) => format!("answered · {}", first_words(answer)),
            Some(Entry::Done(Err(why))) => format!("stopped · {}", first_words(why)),
            _ if agent.retired => "finished".to_owned(),
            _ => "resting".to_owned(),
        }
    }
}

/// The widths of the map's columns, those that fit: on a narrow screen,
/// the context's gauge goes first, then the calls.
struct Columns {
    name: usize,
    model: usize,
    calls: bool,
    context: bool,
}

impl Columns {
    fn of(rows: &[Row<'_>], width: usize) -> Columns {
        let name = rows
            .iter()
            .filter_map(|row| match row.kind {
                Kind::Agent(agent) => {
                    Some(tree_width(&row.lasts) + 2 + text_width(&part(&agent.name)))
                }
                Kind::Job(_) => None,
            })
            .max()
            .unwrap_or(0)
            .min(28);
        let model = rows
            .iter()
            .filter_map(|row| match row.kind {
                Kind::Agent(agent) => Some(text_width(&short_model(agent.serving()))),
                Kind::Job(_) => None,
            })
            .max()
            .unwrap_or(0);
        Columns {
            name,
            model,
            calls: width >= 90,
            context: width >= 110,
        }
    }
}

/// The tree's lines before a row whose place is `lasts`: for each
/// ancestor, a line down unless it was the last; then its own branch, or,
/// for what goes `under` the row, a line down unless it is the last.
fn tree(lasts: &[bool], under: bool, p: &Palette) -> Vec<Span<'static>> {
    let mut drawn = String::new();
    if let Some((own, above)) = lasts.split_last() {
        for last in above {
            drawn.push_str(if *last { "   " } else { "│  " });
        }
        drawn.push_str(match (under, *own) {
            (true, true) => "   ",
            (true, false) => "│  ",
            (false, true) => "└─ ",
            (false, false) => "├─ ",
        });
    }
    vec![Span::styled(drawn, p.fg(p.faint))]
}

/// How wide the tree's lines are before a row whose place is `lasts`.
fn tree_width(lasts: &[bool]) -> usize {
    lasts.len() * 3
}

/// A gauge of `share`, from empty to full, in `tint`.
fn gauge(share: f64, tint: crate::theme::Rgb, p: &Palette) -> Vec<Span<'static>> {
    let full = ((share.clamp(0.0, 1.0) * GAUGE as f64).round() as usize).min(GAUGE);
    vec![
        Span::styled("▕", p.fg(p.faint)),
        Span::styled("█".repeat(full), p.fg(tint)),
        Span::styled("░".repeat(GAUGE - full), p.fg(p.faint)),
        Span::styled("▏", p.fg(p.faint)),
    ]
}

/// The turn's cost split by model, each with as many of [`SPLIT`] cells
/// as its share, one at least: the dearest first.
fn split(turn: &TurnView) -> Vec<(String, usize)> {
    if turn.total.0 <= 0.0 {
        return Vec::new();
    }
    let mut by: Vec<(String, f64)> = Vec::new();
    for spent in &turn.by {
        let model = short_model(&spent.model);
        match by.iter_mut().find(|(seen, _)| *seen == model) {
            Some((_, cost)) => *cost += spent.spent.0,
            None => by.push((model, spent.spent.0)),
        }
    }
    by.retain(|(_, cost)| *cost > 0.0);
    by.sort_by(|a, b| b.1.total_cmp(&a.1));
    by.into_iter()
        .map(|(model, cost)| {
            let cells = ((cost / turn.total.0) * SPLIT as f64).round() as usize;
            (model, cells.max(1))
        })
        .collect()
}

/// What spent in a turn beside its agents: the autopilot's model, the
/// summaries.
fn beside(spent: &TurnSpending) -> bool {
    spent.who == "reviewer" || spent.who == "summaries"
}

/// A model as the map names it: the last part of its name, short.
pub(crate) fn short_model(model: &str) -> String {
    cut(model.rsplit('/').next().unwrap_or(model), 14)
}

/// An agent as its chat's map names it: its part, its chat's own agent by
/// its name.
fn part(name: &str) -> String {
    name.split_once('/')
        .map_or(name, |(_, part)| part)
        .to_owned()
}

/// The first words of `text`, its first line cut, its Markdown's marks
/// left out.
fn first_words(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let plain = line.replace("**", "").replace("__", "").replace('`', "");
    cut(plain.trim().trim_start_matches(['#', '-', '*', ' ']), 60)
}

/// What a tool call does, said with its target: `reading src/hook.rs`,
/// `running cargo test`.
fn tool_said(name: &str, input: &str) -> String {
    let target = short(input).replace('\n', " ");
    match name {
        "read_file" => format!("reading {target}"),
        "write_file" => format!("writing {target}"),
        "edit_file" => format!("editing {target}"),
        "run" => format!("running {}", iq_ui::shell::few_words(input, 60)),
        "start_agent" | "start_team" => "starting agents".to_owned(),
        "send_message" => format!("writing to {}", part(&target)),
        "wait_for_messages" => "waiting for a message".to_owned(),
        "ask_person" => "asking you".to_owned(),
        "web_search" => format!("searching the web for {target}"),
        "fetch" | "web_fetch" => format!("reading {target}"),
        "job" => "looking at a job".to_owned(),
        other => format!("{other} {target}"),
    }
}

/// How wide `line` is, for the tests.
#[cfg(test)]
fn width_of(line: &Line<'_>) -> usize {
    crate::wrap::spans_width(&line.spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Depth;
    use iq_agent::{Event, PERSON};
    use iq_ui::Update;
    use iq_ui::look::Theme;

    fn text(line: &Line<'_>) -> String {
        line.to_string().trim_end().to_owned()
    }

    fn board() -> Board {
        let mut board = Board::default();
        board.update(Update::Started {
            lead: "lab/sol".into(),
            reviewer: "r".into(),
            autopilot: false,
            internet: false,
            allowed: Vec::new(),
        });
        let spent = |agent: &str, model: &str, cost: f64| Event::Spent {
            agent: agent.into(),
            model: model.into(),
            usage: iq_core::Usage {
                input: iq_core::TokenCount(40_000),
                output: iq_core::TokenCount(100),
            },
            cost: Some(iq_core::Usd(cost)),
            cache: None,
            chat: Some("fix".into()),
        };
        for event in [
            Event::Joined {
                agent: "fix".into(),
                model: "lab/sol".into(),
                by: PERSON.into(),
            },
            Event::Turn {
                chat: "fix".into(),
                number: 7,
                ask_at: Some(iq_core::Usd(1.0)),
            },
            Event::Working {
                agent: "fix".into(),
                prompt: "review it".into(),
            },
            Event::Joined {
                agent: "fix/review-core".into(),
                model: "lab/glm".into(),
                by: "fix".into(),
            },
            Event::Working {
                agent: "fix/review-core".into(),
                prompt: "core".into(),
            },
            Event::Tool {
                agent: "fix/review-core".into(),
                name: "read_file".into(),
                input: "src/core.rs".into(),
                output: "…".into(),
                ok: true,
            },
            Event::Joined {
                agent: "fix/review-api".into(),
                model: "lab/glm".into(),
                by: "fix".into(),
            },
            Event::Working {
                agent: "fix/review-api".into(),
                prompt: "api".into(),
            },
            Event::Done {
                agent: "fix/review-api".into(),
                outcome: Ok("1 medium, 4 minor".into()),
            },
            Event::JobStarted {
                agent: "fix".into(),
                id: 3,
                line: "cargo test -q".into(),
            },
            spent("fix", "lab/sol", 0.71),
            spent("fix/review-core", "lab/glm", 0.03),
            spent("fix/review-api", "lab/glm", 0.04),
            spent("reviewer", "lab/quick", 0.03),
        ] {
            board.update(Update::Crew(event));
        }
        board
    }

    #[test]
    fn the_map_draws_the_turn_its_agents_as_a_tree_and_its_jobs_as_leaves() {
        let board = board();
        let p = Palette::new(Theme::Graphite, Depth::True);
        let motion = Motion::default();
        let map = Map {
            board: &board,
            p: &p,
            motion: &motion,
            now: iq_ui::now(),
            labels: &|_| None,
            writing: &|_| None,
        };
        let head = text(&map.head("fix", 120));
        assert!(
            head.starts_with(" turn 7 · 0s · $0.81 of $1.00 ▕███████░░▏"),
            "{head}"
        );
        assert!(head.contains("sol ██████████████ glm █ quick █"), "{head}");
        let rows: Vec<String> = map
            .rows("fix", 120, Some(1), false, false)
            .iter()
            .map(text)
            .collect();
        assert_eq!(rows.len(), 4, "{rows:#?}");
        assert!(rows[0].starts_with(" ⠋ fix"), "{rows:#?}");
        assert!(rows[0].contains("$0.71 1 call"), "{}", rows[0]);
        assert!(rows[1].starts_with("▸├─ ⠋ review-core"), "{}", rows[1]);
        assert!(rows[1].ends_with("reading src/core.rs"), "{}", rows[1]);
        assert!(rows[1].contains("ctx ▕██░░░░░░░▏ 40.0k"), "{}", rows[1]);
        assert!(rows[2].starts_with(" ├─ ✓ review-api"), "{}", rows[2]);
        assert!(
            rows[2].ends_with("answered · 1 medium, 4 minor"),
            "{}",
            rows[2]
        );
        assert_eq!(
            first_words("**3** lines in `notes.txt`\nmore"),
            "3 lines in notes.txt"
        );
        assert!(
            rows[3].starts_with(" └─ ⠋ job 3 · cargo test · 0s"),
            "{}",
            rows[3]
        );
        assert_eq!(
            map.foot("fix", 120, false)
                .map(|line| text(&line))
                .as_deref(),
            Some("   autopilot $0.03")
        );
        // At 80 columns, what fits: never wider, never folded.
        for line in map.rows("fix", 80, None, false, false) {
            assert!(width_of(&line) <= 80, "{line:?}");
        }
        // The strip: those at work alone.
        let strip: Vec<String> = map
            .rows("fix", 120, None, false, true)
            .iter()
            .map(text)
            .collect();
        assert!(
            !strip.iter().any(|row| row.contains("review-api")),
            "{strip:#?}"
        );
        assert!(strip.iter().any(|row| row.contains("review-core")));
    }
}
