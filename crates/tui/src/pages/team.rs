//! The crew's page: the person's chats, each with the agents it started
//! under it, who started each, what it does and what it spent, what it was
//! first asked and when it last worked, the one chosen marked; and an
//! agent's whole thread, written as the conversation writes it.

use iq_agent::PERSON;
use iq_ui::crew::Entry;
use ratatui::text::{Line, Span};

use super::Seen;
use crate::history::{agent_tint, short};
use crate::markdown::{Markdown, Out, code};
use crate::wrap::{cut, wrap};

/// The agents, one a line.
pub(super) fn lines(seen: &Seen<'_>) -> Vec<Line<'static>> {
    let p = seen.p;
    let board = seen.board;
    let mut lines = Vec::new();
    let agents = board.crew().agents();
    if agents.is_empty() {
        seen.said(
            &mut lines,
            "No agent has joined yet: write a task, and the first one starts.",
            p.dim,
        );
        return lines;
    }
    lines.push(Line::default());
    let chosen = board.screen().chosen();
    let talking = board.screen().talking_to();
    let listed = board.crew().listed();
    let name_width = listed
        .iter()
        .map(|(depth, a)| a.name.chars().count() + 2 * depth)
        .max()
        .unwrap_or(0)
        + 2;
    let model_width = agents
        .iter()
        .map(|a| a.model.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let now = iq_ui::now();
    for (at, (depth, agent)) in listed.iter().enumerate() {
        let tint = agent_tint(board, &agent.name, p);
        let on = at == chosen;
        let depth = *depth;
        let nested = "  ".repeat(depth);
        let state = if agent.working {
            ("working".to_owned(), p.accent)
        } else {
            match agent.thread.last() {
                Some(Entry::Done(Err(_))) => ("stopped".to_owned(), p.bad),
                Some(Entry::Done(Ok(_))) => ("done".to_owned(), p.ok),
                _ => ("resting".to_owned(), p.faint),
            }
        };
        let spent = match agent.context().0 {
            0 => "·".to_owned(),
            _ => format!("{} · context {}", agent.billed, agent.context()),
        };
        let by = if agent.by == PERSON {
            "by you".to_owned()
        } else {
            format!("by {}", agent.by)
        };
        let named = format!("{nested}{}", agent.name);
        let mut spans = vec![
            Span::styled(if on { " ▸ " } else { "   " }, p.fg(p.accent)),
            Span::styled(if depth > 0 { "◦ " } else { "● " }, p.fg(tint)),
            Span::styled(
                format!("{named:<name_width$}"),
                if on { p.bold(tint) } else { p.fg(p.text) },
            ),
            Span::styled(format!("{:<model_width$}", agent.model), p.fg(p.dim)),
            Span::styled(format!("{:<9}", state.0), p.fg(state.1)),
            Span::styled(format!("{spent:<22}"), p.fg(p.text)),
            Span::styled(by, p.fg(p.faint)),
        ];
        if Some(agent.name.as_str()) == talking {
            spans.push(Span::styled("  you write to it", p.fg(p.accent)));
        }
        let waiting = board
            .questions()
            .iter()
            .filter(|q| q.agent == agent.name)
            .count();
        if waiting > 0 {
            spans.push(Span::styled(
                format!("  {waiting} waiting for you"),
                p.fg(p.warn),
            ));
        }
        lines.push(Line::from(spans));
        let under = format!("       {nested}");
        // What it was first asked, and when it last worked: what tells
        // one chat from another.
        if !agent.task.trim().is_empty() || agent.worked.is_some() {
            let worked = agent
                .worked
                .map(|at| format!("at work {} · ", iq_ui::ago(at, now)))
                .unwrap_or_default();
            let room = seen
                .width
                .saturating_sub(under.chars().count() + worked.chars().count() + 3);
            let asked = first_line(given(&agent.task));
            let mut said = vec![
                Span::raw(under.clone()),
                Span::styled(worked, p.fg(p.faint)),
            ];
            if !asked.is_empty() {
                said.push(Span::styled(
                    format!("“{}”", cut(&asked, room)),
                    p.fg(p.dim),
                ));
            }
            lines.push(Line::from(said));
        }
        if let Some(place) = &agent.place
            && depth == 0
        {
            lines.push(Line::from(Span::styled(
                format!(
                    "{under}in {}",
                    cut(
                        &place.display().to_string(),
                        seen.width.saturating_sub(under.chars().count() + 4)
                    )
                ),
                p.fg(p.faint),
            )));
        }
        // What the person's plugins say of it: its containers, say.
        for said in seen.board.plugins().lines(&agent.name) {
            lines.push(Line::from(Span::styled(
                format!(
                    "{under}{}",
                    cut(said, seen.width.saturating_sub(under.chars().count() + 2))
                ),
                p.fg(p.dim),
            )));
        }
    }
    if let Some((_, agent)) = listed.get(chosen) {
        seen.heading(
            &mut lines,
            "Lately",
            &format!("{}'s last steps", agent.name),
        );
        let steps: Vec<&Entry> = agent
            .thread
            .iter()
            .filter(|e| !matches!(e, Entry::Text(t) if t.trim().is_empty()))
            .filter(|e| !matches!(e, Entry::Message { from, .. } if from == PERSON))
            .collect();
        for entry in steps.iter().rev().take(8).rev() {
            let said = match entry {
                Entry::Prompt(text) => format!("› {}", first_line(given(text))),
                Entry::Text(text) => first_line(text),
                Entry::Tool {
                    name, input, ok, ..
                } => {
                    format!("{} {name} {}", if *ok { "▸" } else { "✗" }, short(input))
                }
                Entry::Reviewed {
                    action, approved, ..
                } => {
                    format!(
                        "⏵⏵ {} {}",
                        if *approved { "allowed" } else { "refused" },
                        short(action)
                    )
                }
                Entry::Asked { action, answer, .. } => match answer {
                    Some(true) => format!("✓ you allowed {}", short(action)),
                    Some(false) => format!("✗ refused {}", short(action)),
                    None => format!("? asks to {}", short(action)),
                },
                Entry::Message { from, to, text } => {
                    format!("✉ {from} → {to}: {}", first_line(text))
                }
                Entry::Notice(text) => format!("! {text}"),
                Entry::Done(Ok(_)) => "✔ done".to_owned(),
                Entry::Done(Err(error)) => format!("✘ stopped: {error}"),
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "   {}",
                    cut(&said.replace('\n', " "), seen.width.saturating_sub(4))
                ),
                p.fg(p.dim),
            )));
        }
    }
    lines
}

/// `agent`'s whole thread.
pub(super) fn thread(seen: &Seen<'_>, agent: &str) -> Vec<Line<'static>> {
    let p = seen.p;
    let board = seen.board;
    let mut lines = Vec::new();
    let Some(view) = board.crew().agent(agent) else {
        seen.said(&mut lines, "This agent is gone.", p.dim);
        return lines;
    };
    let tint = agent_tint(board, agent, p);
    let width = seen.width.saturating_sub(1);
    let indent = [Span::raw("  ")];
    lines.push(Line::from(vec![
        Span::styled(" ● ", p.fg(tint)),
        Span::styled(agent.to_owned(), p.bold(tint)),
        Span::styled(format!("  {}", view.model), p.fg(p.faint)),
    ]));
    for entry in &view.thread {
        match entry {
            Entry::Prompt(text) => {
                lines.push(Line::default());
                lines.extend(wrap(
                    &[Span::styled(given(text).to_owned(), p.fg(p.text))],
                    width,
                    &[Span::styled(" › ", p.bold(p.accent))],
                    &[Span::raw("   ")],
                ));
            }
            Entry::Text(text) => {
                let mut markdown = Markdown::default();
                let mut out = Vec::new();
                for line in text.lines() {
                    out.extend(markdown.line(line, p));
                }
                out.extend(markdown.flush(p));
                // A page shows a diagram as its source: images are the
                // conversation's.
                let styled = out.into_iter().flat_map(|out| match out {
                    Out::Line(styled) => vec![Ok(styled)],
                    Out::Table(table) => vec![Err(table)],
                    Out::Diagram(source) => {
                        code("mermaid", &source, p).into_iter().map(Ok).collect()
                    }
                });
                for styled in styled {
                    match styled {
                        Ok(styled) => {
                            let first: Vec<Span<'static>> =
                                indent.iter().cloned().chain(styled.first).collect();
                            let rest: Vec<Span<'static>> =
                                indent.iter().cloned().chain(styled.rest).collect();
                            lines.extend(wrap(&styled.body, width, &first, &rest));
                        }
                        Err(table) => lines.extend(table.lines(width, &indent)),
                    }
                }
            }
            Entry::Tool {
                name,
                input,
                output,
                ok,
            } => {
                let (mark, colour) = if *ok {
                    ("▸ ", p.faint)
                } else {
                    ("✗ ", p.bad)
                };
                lines.extend(wrap(
                    &[
                        Span::styled(name.clone(), p.fg(p.text)),
                        Span::styled(format!(" {}", short(input)), p.fg(p.dim)),
                    ],
                    width,
                    &[Span::raw("  "), Span::styled(mark, p.fg(colour))],
                    &[Span::raw("    ")],
                ));
                for line in output
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .take(if *ok { 4 } else { 12 })
                {
                    lines.push(Line::from(Span::styled(
                        format!("    {}", cut(&short(line), width.saturating_sub(4))),
                        p.fg(p.faint),
                    )));
                }
            }
            Entry::Reviewed {
                action,
                approved,
                why,
            } => {
                let (word, colour) = if *approved {
                    ("allowed", p.ok)
                } else {
                    ("refused", p.bad)
                };
                lines.extend(wrap(
                    &[Span::styled(
                        format!("the reviewer {word} {}: {why}", short(action)),
                        p.fg(colour),
                    )],
                    width,
                    &[Span::raw("  ⏵⏵ ")],
                    &[Span::raw("     ")],
                ));
            }
            Entry::Asked {
                action,
                answer,
                reason,
                ..
            } => {
                let (said, colour) = match answer {
                    Some(true) => (format!("you allowed {}", short(action)), p.ok),
                    Some(false) => (format!("refused {}", short(action)), p.bad),
                    None => (
                        format!("asks to {}: {}", short(action), short(reason)),
                        p.warn,
                    ),
                };
                lines.extend(wrap(
                    &[Span::styled(said, p.fg(colour))],
                    width,
                    &[Span::raw("  ")],
                    &[Span::raw("  ")],
                ));
            }
            // The person's message shows as the turn it wakes.
            Entry::Message { from, .. } if from == PERSON => {}
            Entry::Message { from, to, text } => {
                lines.push(Line::from(Span::styled(
                    format!("  ✉ {from} → {to}"),
                    p.fg(p.info),
                )));
                lines.extend(wrap(
                    &[Span::styled(text.trim().to_owned(), p.fg(p.dim))],
                    width,
                    &[Span::raw("    ")],
                    &[Span::raw("    ")],
                ));
            }
            Entry::Notice(text) => {
                lines.extend(wrap(
                    &[Span::styled(text.clone(), p.fg(p.warn))],
                    width,
                    &[Span::raw("  ! ")],
                    &[Span::raw("    ")],
                ));
            }
            Entry::Done(Ok(_)) => lines.push(Line::from(Span::styled("  ✔ done", p.fg(p.ok)))),
            Entry::Done(Err(error)) => {
                lines.extend(wrap(
                    &[Span::styled(short(error), p.fg(p.bad))],
                    width,
                    &[Span::raw("  ✘ stopped: ")],
                    &[Span::raw("    ")],
                ));
            }
        }
    }
    lines
}

/// What a turn was given, without what says the person wrote it: their
/// messages wake the agent as `Message from person: …`.
fn given(text: &str) -> &str {
    text.trim()
        .strip_prefix("Message from person: ")
        .unwrap_or(text.trim())
}

/// The first line of `text`, trimmed.
fn first_line(text: &str) -> String {
    text.trim().lines().next().unwrap_or_default().to_owned()
}
