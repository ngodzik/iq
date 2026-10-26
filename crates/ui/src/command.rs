//! The commands the person writes after a `/`, as every terminal agent
//! takes them: what each is called and does, how its line is read, and
//! what completes it as it is written.

use iq_agent::Pattern;

use crate::look::Theme;

/// A command, as the list of commands gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spec {
    /// Its name, after the `/`.
    pub name: &'static str,
    /// What it takes, in words; empty for nothing.
    pub args: &'static str,
    /// What it does.
    pub what: &'static str,
}

/// Every command, in the order the list gives them.
pub const COMMANDS: [Spec; 16] = [
    Spec {
        name: "new",
        args: "[model]",
        what: "start a new chat, on the lead's model or this one",
    },
    Spec {
        name: "pattern",
        args: "solo|lead|pair|debate [partner]",
        what: "how the new chat's team works",
    },
    Spec {
        name: "role",
        args: "[role]",
        what: "the new chat's part in the crew: a word names it",
    },
    Spec {
        name: "to",
        args: "agent",
        what: "write to this agent; @agent before a message writes it there once",
    },
    Spec {
        name: "team",
        args: "",
        what: "the crew: each agent, what it does and spends",
    },
    Spec {
        name: "costs",
        args: "",
        what: "the costs, the limits and the subscriptions",
    },
    Spec {
        name: "assay",
        args: "",
        what: "the assay: agents tried on the same exercises",
    },
    Spec {
        name: "judge",
        args: "",
        what: "talk with the assay's judge",
    },
    Spec {
        name: "observations",
        args: "",
        what: "what the work went to, over time",
    },
    Spec {
        name: "keys",
        args: "",
        what: "the shortcuts and the commands",
    },
    Spec {
        name: "autopilot",
        args: "",
        what: "the reviewer answering for you, or not",
    },
    Spec {
        name: "internet",
        args: "",
        what: "the internet open to every command, or only to what fetches packages",
    },
    Spec {
        name: "steward",
        args: "",
        what: "the steward writing to who spends too fast, or only watching",
    },
    Spec {
        name: "theme",
        args: "[graphite|ink|porcelain]",
        what: "the next theme, or this one",
    },
    Spec {
        name: "stop",
        args: "[agent|all]",
        what: "stop the agent you write to, this one, or all",
    },
    Spec {
        name: "quit",
        args: "",
        what: "quit",
    },
];

/// A command as read from its line.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// A new chat, on this model when one is named.
    New(Option<String>),
    /// How the new chat's team works, and its partner's model.
    Pattern(Pattern, Option<String>),
    /// The new chat's role; none when empty.
    Role(String),
    /// Write to this agent.
    To(String),
    /// The crew's page.
    Team,
    /// The costs' page.
    Costs,
    /// The assay's page.
    Assay,
    /// Talk with the assay's judge.
    Judge,
    /// The observations' page.
    Observations,
    /// The shortcuts and the commands.
    Keys,
    /// The autopilot on or off.
    Autopilot,
    /// The internet open to every command, or not.
    Internet,
    /// The steward writing, or only watching.
    Steward,
    /// The next theme, or this one.
    Theme(Option<Theme>),
    /// Stop the agent written to, this one, or `all`.
    Stop(Option<String>),
    /// Quit.
    Quit,
}

/// Reads a command's line, `/` first.
///
/// # Errors
///
/// What is wrong with it, in words for the person: a command iq does not
/// have, or what it takes missing or wrong.
///
/// # Examples
///
/// ```
/// use iq_ui::command::{Command, parse};
///
/// assert_eq!(parse("/to  helper").unwrap(), Command::To("helper".into()));
/// assert!(parse("/fly").is_err());
/// ```
pub fn parse(line: &str) -> Result<Command, String> {
    let line = line.trim();
    let line = line.strip_prefix('/').unwrap_or(line);
    let (name, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    let words: Vec<&str> = rest.split_whitespace().collect();
    let one = |what: &str| -> Result<Option<String>, String> {
        match words.as_slice() {
            [] => Ok(None),
            [word] => Ok(Some((*word).to_owned())),
            _ => Err(format!("/{name} takes one {what}")),
        }
    };
    let bare = |command: Command| {
        if words.is_empty() {
            Ok(command)
        } else {
            Err(format!("/{name} takes nothing after it"))
        }
    };
    match name {
        "new" => Ok(Command::New(one("model")?)),
        "pattern" => {
            let Some(first) = words.first() else {
                return Err("/pattern takes solo, lead, pair or debate".to_owned());
            };
            let pattern: Pattern = first.parse()?;
            match &words[1..] {
                [] => Ok(Command::Pattern(pattern, None)),
                [partner] => Ok(Command::Pattern(pattern, Some((*partner).to_owned()))),
                _ => Err("/pattern takes a pattern and a partner's model".to_owned()),
            }
        }
        "role" => Ok(Command::Role(rest.to_owned())),
        "to" => one("agent")?
            .map(|agent| Command::To(agent.trim_start_matches('@').to_owned()))
            .ok_or_else(|| "/to takes the agent to write to".to_owned()),
        "team" | "agents" => bare(Command::Team),
        "costs" | "cost" => bare(Command::Costs),
        "assay" => bare(Command::Assay),
        "judge" => bare(Command::Judge),
        "observations" | "work" => bare(Command::Observations),
        "keys" | "help" => bare(Command::Keys),
        "autopilot" => bare(Command::Autopilot),
        "internet" => bare(Command::Internet),
        "steward" => bare(Command::Steward),
        "theme" => match one("theme")? {
            None => Ok(Command::Theme(None)),
            Some(name) => name
                .parse::<Theme>()
                .map(|theme| Command::Theme(Some(theme)))
                .map_err(|error| error.to_string()),
        },
        "stop" => Ok(Command::Stop(one("agent")?)),
        "quit" | "exit" => bare(Command::Quit),
        "" => Err("a command's name follows the /".to_owned()),
        other => {
            let names: Vec<String> = COMMANDS.iter().map(|c| format!("/{}", c.name)).collect();
            Err(format!("iq has no /{other}: it has {}", names.join(", ")))
        }
    }
}

/// One way to complete what is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The line once completed.
    pub line: String,
    /// What the list shows of it.
    pub label: String,
    /// What it does, or is.
    pub what: String,
}

/// What completes `line` while it is a command being written: the
/// commands its start names, then what the command takes, from `agents`
/// and `models` for those that take one.
#[must_use]
pub fn complete(line: &str, agents: &[&str], models: &[&str]) -> Vec<Completion> {
    let Some(written) = line.strip_prefix('/') else {
        return Vec::new();
    };
    if written.contains('\n') {
        return Vec::new();
    }
    let Some((name, arg)) = written.split_once(' ') else {
        return COMMANDS
            .iter()
            .filter(|spec| spec.name.starts_with(written))
            .map(|spec| Completion {
                line: if spec.args.is_empty() {
                    format!("/{}", spec.name)
                } else {
                    format!("/{} ", spec.name)
                },
                label: if spec.args.is_empty() {
                    format!("/{}", spec.name)
                } else {
                    format!("/{} {}", spec.name, spec.args)
                },
                what: spec.what.to_owned(),
            })
            .collect();
    };
    if arg.contains(' ') {
        return Vec::new();
    }
    let values: Vec<(String, String)> = match name {
        "to" => agents
            .iter()
            .map(|a| ((*a).to_owned(), "an agent".to_owned()))
            .collect(),
        "stop" => agents
            .iter()
            .map(|a| ((*a).to_owned(), "an agent".to_owned()))
            .chain(std::iter::once((
                "all".to_owned(),
                "every agent".to_owned(),
            )))
            .collect(),
        "new" => models
            .iter()
            .map(|m| ((*m).to_owned(), "a model".to_owned()))
            .collect(),
        "theme" => Theme::ALL
            .iter()
            .map(|t| (t.name().to_owned(), "a theme".to_owned()))
            .collect(),
        "pattern" => Pattern::ALL
            .iter()
            .map(|p| (p.name().to_owned(), p.what().to_owned()))
            .collect(),
        _ => Vec::new(),
    };
    values
        .into_iter()
        .filter(|(value, _)| value.starts_with(arg) && value != arg)
        .map(|(value, what)| Completion {
            line: format!("/{name} {value}"),
            label: value,
            what,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_read_or_refused_with_a_reason() {
        assert_eq!(parse("/new").unwrap(), Command::New(None));
        assert_eq!(
            parse("/new codex").unwrap(),
            Command::New(Some("codex".into()))
        );
        assert_eq!(
            parse("/pattern pair claude-code/haiku").unwrap(),
            Command::Pattern(Pattern::Pair, Some("claude-code/haiku".into()))
        );
        assert_eq!(parse("/to @helper").unwrap(), Command::To("helper".into()));
        assert_eq!(
            parse("/theme ink").unwrap(),
            Command::Theme(Some(Theme::Ink))
        );
        assert_eq!(parse("/help").unwrap(), Command::Keys);
        assert_eq!(parse("/role ").unwrap(), Command::Role(String::new()));
        assert!(parse("/to").is_err());
        assert!(parse("/costs now").is_err());
        assert!(parse("/pattern solo a b").is_err());
        assert!(parse("/fly").unwrap_err().contains("/new"));
    }

    #[test]
    fn a_command_is_completed_then_what_it_takes() {
        let names =
            |done: Vec<Completion>| -> Vec<String> { done.into_iter().map(|c| c.line).collect() };
        assert_eq!(complete("/", &[], &[]).len(), COMMANDS.len());
        assert_eq!(names(complete("/co", &[], &[])), ["/costs"]);
        assert_eq!(
            names(complete("/t", &[], &[])),
            ["/to ", "/team", "/theme "]
        );
        assert_eq!(
            names(complete("/stop h", &["helper", "lead"], &[])),
            ["/stop helper"]
        );
        assert_eq!(names(complete("/new ", &[], &["codex"])), ["/new codex"]);
        // Once complete, or when a message is written, nothing is offered.
        assert!(complete("/theme ink", &[], &[]).is_empty());
        assert!(complete("fix it", &[], &[]).is_empty());
    }
}
