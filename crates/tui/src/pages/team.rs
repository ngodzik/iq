//! The person's chats, to choose one from: each with its model, how it
//! stands, what it spent, what it was first asked and when it last worked,
//! the one used last first, the one chosen marked; and a chat's whole
//! thread, written as the conversation writes it. What the agents a chat
//! started do is read on the steps' page and the map.

use iq_agent::PERSON;
use iq_ui::crew::Entry;
use ratatui::text::{Line, Span};

use super::Seen;
use crate::history::{agent_tint, short};
use crate::markdown::{Markdown, Out, code};
use crate::wrap::{cut, wrap};

/// The chats, one a line, what tells them apart under each.
pub(super) fn lines(seen: &Seen<'_>) -> Vec<Line<'static>> {
    let p = seen.p;
    let board = seen.board;
    let mut lines = Vec::new();
    let chats = board.chats_listed();
    if chats.is_empty() {
        seen.said(
            &mut lines,
            "No chat yet: write a task, and the first one starts.",
            p.dim,
        );
        return lines;
    }
    lines.push(Line::default());
    let chosen = board.screen().chosen();
    let talking = board.screen().talking_to();
    let name_width = chats
        .iter()
        .map(|chat| chat.name.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let model_width = chats
        .iter()
        .map(|chat| chat.model.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let now = iq_ui::now();
    for (at, chat) in chats.iter().enumerate() {
        let tint = agent_tint(board, &chat.name, p);
        let on = at == chosen;
        let state = if chat.working {
            ("working".to_owned(), p.accent)
        } else {
            match chat.thread.last() {
                Some(Entry::Done(Err(_))) => ("stopped".to_owned(), p.bad),
                Some(Entry::Done(Ok(_))) => ("done".to_owned(), p.ok),
                _ => ("resting".to_owned(), p.faint),
            }
        };
        let spent = match chat.context().0 {
            0 => "·".to_owned(),
            _ => format!("{} · context {}", chat.billed, chat.context()),
        };
        let mut spans = vec![
            Span::styled(if on { " ▸ " } else { "   " }, p.fg(p.accent)),
            Span::styled("● ", p.fg(tint)),
            Span::styled(
                format!("{:<name_width$}", chat.name),
                if on { p.bold(tint) } else { p.fg(p.text) },
            ),
            Span::styled(format!("{:<model_width$}", chat.model), p.fg(p.dim)),
            Span::styled(format!("{:<10}", state.0), p.fg(state.1)),
            Span::styled(spent, p.fg(p.text)),
        ];
        let at_work = board
            .crew()
            .agents()
            .iter()
            .filter(|agent| {
                agent.working
                    && agent.by != PERSON
                    && board.crew().chat_of(&agent.name).as_deref() == Some(chat.name.as_str())
            })
            .count();
        if at_work > 0 {
            spans.push(Span::styled(
                format!("  {at_work} of its agents at work",),
                p.fg(p.accent),
            ));
        }
        if Some(chat.name.as_str()) == talking {
            spans.push(Span::styled("  you write to it", p.fg(p.accent)));
        }
        let waiting = board
            .questions()
            .iter()
            .filter(|q| board.crew().chat_of(&q.agent).as_deref() == Some(chat.name.as_str()))
            .count();
        if waiting > 0 {
            spans.push(Span::styled(
                format!("  {waiting} waiting for you"),
                p.fg(p.warn),
            ));
        }
        lines.push(Line::from(spans));
        let under = "       ";
        // What it was first asked, and when it last worked: what tells
        // one chat from another.
        if !chat.task.trim().is_empty() || chat.worked.is_some() {
            let worked = chat
                .worked
                .map(|at| format!("at work {} · ", iq_ui::ago(at, now)))
                .unwrap_or_default();
            let room = seen
                .width
                .saturating_sub(under.chars().count() + worked.chars().count() + 3);
            let asked = first_line(given(&chat.task));
            let mut said = vec![Span::raw(under), Span::styled(worked, p.fg(p.faint))];
            if !asked.is_empty() {
                said.push(Span::styled(
                    format!("“{}”", cut(&asked, room)),
                    p.fg(p.dim),
                ));
            }
            lines.push(Line::from(said));
        }
        if let Some(place) = &chat.place {
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
        for said in seen.board.plugins().lines(&chat.name) {
            lines.push(Line::from(Span::styled(
                format!(
                    "{under}{}",
                    cut(said, seen.width.saturating_sub(under.chars().count() + 2))
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
    let Some(view) = board
        .crew()
        .agent(agent)
        .or_else(|| board.crew().retired(agent))
    else {
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
                        format!("autopilot {word} {}: {why}", short(action)),
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
            Entry::Chose { question, answer } => {
                lines.extend(wrap(
                    &[
                        Span::styled(question.trim().to_owned(), p.fg(p.text)),
                        Span::styled(" → ", p.fg(p.faint)),
                        Span::styled(
                            answer.clone().unwrap_or_else(|| "no answer".to_owned()),
                            p.fg(p.accent),
                        ),
                    ],
                    width,
                    &[Span::raw("  "), Span::styled("? ", p.fg(p.warn))],
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Depth, Palette};
    use iq_agent::Event;
    use iq_ui::look::Theme;
    use iq_ui::{Board, Update};

    #[test]
    fn the_chats_are_listed_alone_with_how_many_of_their_agents_work() {
        let mut board = Board::default();
        for (agent, by) in [
            ("login", PERSON),
            ("docs", PERSON),
            ("login/review", "login"),
        ] {
            board.update(Update::Crew(Event::Joined {
                agent: agent.into(),
                model: "lab/m".into(),
                by: by.into(),
            }));
        }
        board.update(Update::Crew(Event::Working {
            agent: "login/review".into(),
            prompt: "review it".into(),
        }));
        let p = Palette::new(Theme::Ink, Depth::True);
        let seen = Seen {
            board: &board,
            p: &p,
            now: 0,
            width: 100,
            images: false,
            summed: &|_| None,
            motion: &crate::motion::Motion::default(),
            labels: &|_| None,
            writing: &|_| None,
        };
        let rows: Vec<String> = lines(&seen).iter().map(ToString::to_string).collect();
        let chats: Vec<&String> = rows.iter().filter(|row| row.contains("● ")).collect();
        assert_eq!(chats.len(), 2, "{rows:#?}");
        assert!(
            chats
                .iter()
                .any(|row| row.contains("login") && row.contains("1 of its agents at work"))
        );
        assert!(!rows.iter().any(|row| row.contains("login/review")));
    }
}
