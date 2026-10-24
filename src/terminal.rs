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
    /// Calls under a subscription, and their tokens: paid for by the
    /// month, they are counted, not priced.
    subscribed: usize,
    tokens: u64,
}

impl Tally {
    /// What the session cost, as its last line says it.
    pub(crate) fn summary(&self) -> String {
        let mut text = format!("Billed to the provider: {}", self.billed);
        if self.subscribed > 0 {
            let calls = if self.subscribed == 1 {
                "call"
            } else {
                "calls"
            };
            text.push_str(&format!(
                ", and {} {calls} under subscriptions, {} tokens",
                self.subscribed,
                iq_core::TokenCount(self.tokens)
            ));
        }
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
        Event::Spent {
            cost,
            billed,
            usage,
            ..
        } => {
            match (cost, billed) {
                (Some(cost), true) => tally.billed += *cost,
                (None, true) => tally.unpriced += 1,
                (_, false) => {
                    tally.subscribed += 1;
                    tally.tokens += usage.input.0 + usage.output.0;
                }
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
        Event::Quota(_) => Ok(()),
    };
    let _ = out.flush();
    None
}

/// Asks the person on the terminal; no answer refuses.
pub(crate) fn ask(question: &str) -> bool {
    print!("{question} Allow it? [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim(), "y" | "Y" | "yes")
}
