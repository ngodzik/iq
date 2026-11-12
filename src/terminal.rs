//! The crew in the terminal: what it does printed as it happens, what the
//! gate asks put to the person on the terminal.

use std::io::{BufRead, Write};

use iq_agent::{Event, PERSON};
use iq_core::Usd;

/// What the crew spent, for the summary.
#[derive(Debug, Default)]
pub(crate) struct Tally {
    billed: Usd,
    unpriced: usize,
}

impl Tally {
    /// What the session cost, as its last line says it.
    pub(crate) fn summary(&self) -> String {
        let mut text = format!("Billed to the provider: {}", self.billed);
        if self.unpriced > 0 {
            text.push_str(&format!(", {} calls of unknown price", self.unpriced));
        }
        text
    }
}

/// Prints `event`, and gives the question to put to the person, if it is
/// one.
pub(crate) fn show(event: &Event, tally: &mut Tally) -> Option<(u64, String)> {
    let mut out = std::io::stdout().lock();
    let first = |text: &str, most: usize| -> String {
        let line = text.lines().next().unwrap_or_default();
        if line.chars().count() > most || text.lines().nth(1).is_some() {
            format!("{}…", line.chars().take(most).collect::<String>())
        } else {
            line.to_owned()
        }
    };
    let _ = match event {
        Event::Joined { agent, model, by } => {
            let by = if by == PERSON {
                String::new()
            } else {
                format!(", started by {by}")
            };
            writeln!(out, "● {agent} joined, on {model}{by}")
        }
        Event::Working { agent, prompt } => writeln!(out, "→ {agent}: {}", first(prompt, 100)),
        Event::TextStart { agent } => write!(out, "\n{agent}: "),
        Event::Text { text, .. } => write!(out, "{text}"),
        Event::Spent { cost, .. } => {
            match cost {
                Some(cost) => tally.billed += *cost,
                None => tally.unpriced += 1,
            }
            Ok(())
        }
        Event::Tool {
            agent,
            name,
            input,
            output,
            ok,
        } => {
            if *ok {
                writeln!(out, "\n  · {agent} {name} {input}")
            } else {
                writeln!(
                    out,
                    "\n  · {agent} {name} {input}: failed, {}",
                    first(output, 160)
                )
            }
        }
        Event::Changed { agent, path, diff } => {
            let lines: Vec<String> = diff.lines().map(|line| format!("    {line}")).collect();
            writeln!(out, "  ✎ {agent} changed {path}\n{}", lines.join("\n"))
        }
        Event::Asked {
            id,
            agent,
            action,
            reason,
            widens,
        } => {
            let beyond = if *widens {
                " It reaches beyond the sandbox."
            } else {
                ""
            };
            return Some((
                *id,
                format!("\n? {agent} asks to {action}: held because {reason}.{beyond}"),
            ));
        }
        Event::Answered { approved, .. } => {
            writeln!(out, "  {}", if *approved { "allowed" } else { "refused" })
        }
        Event::Reviewed {
            agent,
            action,
            approved,
            why,
            ..
        } => {
            let word = if *approved { "allowed" } else { "refused" };
            writeln!(out, "\n  the reviewer {word} {agent}'s {action}: {why}")
        }
        Event::Posted { from, to, text } => {
            writeln!(out, "\n✉ {from} → {to}: {}", first(text, 300))
        }
        Event::Done { agent, outcome } => match outcome {
            Ok(_) => writeln!(out, "\n✔ {agent} done"),
            Err(error) => writeln!(out, "\n✘ {agent} stopped: {error}"),
        },
        Event::Notice { agent, text } => writeln!(out, "\n! {agent}: {text}"),
        // Its turn's end says it, with the facts.
        Event::Failed { .. } => Ok(()),
        Event::Retired { agent } => writeln!(out, "  ◦ {agent} is done, and retired"),
        Event::Waiting { agent, on } => writeln!(out, "  ⧗ {agent} waits for {}", on.join(", ")),
        Event::JobStarted { agent, id, line } => writeln!(
            out,
            "  ⚙ {agent} runs job {id} in the background: {}",
            iq_ui::shell::few_words(line, 72)
        ),
        Event::JobOutput { .. } | Event::Turn { .. } | Event::Window { .. } => Ok(()),
        Event::JobEnded {
            agent, id, said, ..
        } => {
            writeln!(out, "  ⚙ job {id} of {agent} ended: {said}")
        }
        // Asked where `iq do` reads its answers: see [`choose`].
        Event::Choose { .. } => Ok(()),
        Event::Chosen { answer, .. } => {
            writeln!(out, "  → {}", answer.as_deref().unwrap_or("no answer"))
        }
        Event::Renamed { from, to } => writeln!(out, "\n! {from} is called {to} now"),
        Event::Model { agent, model } => writeln!(out, "\n! {agent} answers on {model} now"),
        Event::Placed {
            agent,
            said: Some(said),
            ..
        } => writeln!(out, "\n! {agent} {said}"),
        Event::Effort { .. } | Event::Placed { said: None, .. } => Ok(()),
    };
    let _ = out.flush();
    None
}

/// Prints what a plugin said that the person reads: its words, and the line
/// that sets a place's variables in a terminal there. What it shows on the
/// status line and the chats' page is for iq's own terminal.
pub(crate) fn heard(update: &iq_ui::Update) {
    use iq_ui::plugins::Told;
    let mut out = std::io::stdout().lock();
    let _ = match update {
        iq_ui::Update::Said(text) => writeln!(out, "\n! {text}"),
        iq_ui::Update::Plugin {
            told:
                Told::Enter {
                    folder,
                    line: Some(line),
                },
            ..
        } => writeln!(
            out,
            "\n! in a terminal there: cd {} && {line}",
            folder.display()
        ),
        _ => Ok(()),
    };
    let _ = out.flush();
}

/// Asks the person on the terminal; no answer refuses.
/// Asks the person `question` on the terminal, `agent`'s, offering
/// `choices` by number; their own words when they write something else,
/// none when they write nothing.
pub(crate) fn choose(
    agent: &str,
    question: &str,
    choices: &[String],
    raised: Option<&str>,
) -> Option<String> {
    println!("\n? {agent} asks: {question}");
    if let Some(raised) = raised {
        println!("  ({raised})");
    }
    for (at, choice) in choices.iter().enumerate() {
        println!("  {}. {choice}", at + 1);
    }
    print!("A number, or your own answer: ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return None;
    }
    let answer = answer.trim();
    match answer.parse::<usize>() {
        Ok(at) if (1..=choices.len()).contains(&at) => Some(choices[at - 1].clone()),
        _ => Some(answer.to_owned()).filter(|a| !a.is_empty()),
    }
}

pub(crate) fn ask(question: &str) -> bool {
    print!("{question} Allow it? [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim(), "y" | "Y" | "yes")
}
