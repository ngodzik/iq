//! The tools a model of iq's works with: files and commands, each acting
//! through the gate, and the board it shares with the other agents. What
//! they take and how they answer, apart from the crew that runs them.

use iq_core::ToolSpec;
use serde_json::{Value, json};

/// The most lines `read_file` gives at once.
const LINES: usize = 2_000;

/// The longest line `read_file` gives whole.
const LINE_CHARS: usize = 2_000;

/// How long `wait_for_messages` waits by default, in seconds: short enough
/// for an agent's tool call to come back within its client's own limits.
pub(crate) const WAIT_SECONDS: u64 = 30;

/// The longest it waits.
pub(crate) const WAIT_MOST: u64 = 600;

/// The tools that act on the workspace.
pub(crate) fn work() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read_file".into(),
            description: "Reads a text file, its lines numbered. A file of secrets, such as \
                          `.env`, is refused."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The file, relative to the workspace or absolute."},
                    "offset": {"type": "integer", "description": "The first line to read, from 1."},
                    "limit": {"type": "integer", "description": "How many lines, 2000 at most."},
                },
                "required": ["path"],
            }),
        },
        ToolSpec {
            name: "write_file".into(),
            description: "Writes a whole file, made with its folders if it is not there.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"},
                },
                "required": ["path", "content"],
            }),
        },
        ToolSpec {
            name: "edit_file".into(),
            description: "Replaces text in a file: `old_text` must be found exactly once, \
                          unless `all` is true."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "old_text": {"type": "string"},
                    "new_text": {"type": "string"},
                    "all": {"type": "boolean"},
                },
                "required": ["path", "old_text", "new_text"],
            }),
        },
        ToolSpec {
            name: "run".into(),
            description: "Runs a shell command line in the workspace, in a sandbox: it writes \
                          only the workspace and reaches localhost. The internet is given to \
                          what fetches packages and sources (npm install, cargo fetch, pip \
                          install, git fetch); ask for it for another command with `network`, \
                          which waits for the person unless they opened it to every command. \
                          Its output comes back, errors included."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "network": {"type": "boolean"},
                },
                "required": ["command"],
            }),
        },
    ]
}

/// The tools of the board, offered to iq's models.
pub fn board() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "start_agent".into(),
            description: "Starts another agent on a task, beside you. `model` is a model of the \
                          provider, by its whole name; `effort`, how hard it thinks, lower for \
                          plain work, as it costs less. It returns at once; the agent's answer \
                          comes to you as a message."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "A short name for it, one word."},
                    "model": {"type": "string"},
                    "task": {"type": "string", "description": "All it needs to know: it sees nothing of your conversation."},
                    "role": {"type": "string", "description": "Its part in the crew, which it keeps to: tests, review, the front end."},
                    "effort": {"type": "string", "enum": ["low", "medium", "high", "xhigh", "max"], "description": "How hard it thinks; left out, as the crew's settings say."},
                },
                "required": ["name", "model", "task"],
            }),
        },
        ToolSpec {
            name: "start_team".into(),
            description: "Starts a team on a task: agents with the roles you give them, each on \
                          its model, who write to each other. The first member works on the task \
                          at once, and its answer comes to you as a message; the others wait until \
                          a teammate writes to them, and answer the first. Each sees the task, the \
                          team and its own part, nothing of your conversation."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "A short name for the team, one word: its members are named after it and their roles."},
                    "task": {"type": "string", "description": "The brief all members share: all they need to know of the work."},
                    "members": {
                        "type": "array",
                        "description": "The members, the one that works first first.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "role": {"type": "string", "description": "Its role, one word: writer, reviewer, tests."},
                                "model": {"type": "string", "description": "A model of the provider, by its whole name."},
                                "effort": {"type": "string", "enum": ["low", "medium", "high", "xhigh", "max"], "description": "How hard it thinks; left out, as the crew's settings say."},
                                "does": {"type": "string", "description": "What it does, and how it works with the others: when it writes to whom, how many rounds at most."},
                            },
                            "required": ["role", "model", "does"],
                        },
                    },
                },
                "required": ["name", "task", "members"],
            }),
        },
        ToolSpec {
            name: "send_message".into(),
            description: "Writes to another agent by its name, to `all`, or to `person`.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "to": {"type": "string"},
                    "text": {"type": "string"},
                },
                "required": ["to", "text"],
            }),
        },
        ToolSpec {
            name: "wait_for_messages".into(),
            description: "Gives the messages written to you since you last read them, waiting \
                          up to `seconds` (30 by default) for one when there is none."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "seconds": {"type": "integer"},
                },
            }),
        },
        ToolSpec {
            name: "list_agents".into(),
            description: "The agents of the crew: their names, models, and whether they work."
                .into(),
            parameters: json!({"type": "object", "properties": {}}),
        },
    ]
}

/// A call's arguments, read from the JSON text the model wrote.
#[derive(Debug)]
pub(crate) struct Args(Value);

impl Args {
    /// Reads `text`, saying what is wrong with it for the model to see.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let text = if text.trim().is_empty() { "{}" } else { text };
        match serde_json::from_str::<Value>(text) {
            Ok(value) if value.is_object() => Ok(Self(value)),
            Ok(_) => Err("the arguments are not a JSON object".into()),
            Err(error) => Err(format!("the arguments are not valid JSON: {error}")),
        }
    }

    /// A text argument the call must have.
    pub(crate) fn text(&self, name: &str) -> Result<&str, String> {
        self.0[name]
            .as_str()
            .ok_or_else(|| format!("`{name}` is missing or not text"))
    }

    /// A whole number, when given.
    pub(crate) fn number(&self, name: &str) -> Option<u64> {
        self.0[name].as_u64()
    }

    /// A yes or no, no when not given.
    pub(crate) fn flag(&self, name: &str) -> bool {
        self.0[name].as_bool().unwrap_or(false)
    }

    /// The arguments whole, as an extension reads them.
    pub(crate) fn value(&self) -> &Value {
        &self.0
    }
}

/// `text`'s lines from `offset` (from 1), at most `limit`, numbered, as
/// `read_file` gives them.
pub(crate) fn numbered(text: &str, offset: Option<u64>, limit: Option<u64>) -> String {
    let from = offset.map_or(1, |o| o.max(1)) as usize;
    let most = limit.map_or(LINES, |l| (l as usize).clamp(1, LINES));
    let total = text.lines().count();
    let mut out = String::new();
    for (at, line) in text.lines().enumerate().skip(from - 1).take(most) {
        let line: String = if line.chars().count() > LINE_CHARS {
            let cut: String = line.chars().take(LINE_CHARS).collect();
            format!("{cut} [line cut]")
        } else {
            line.to_owned()
        };
        out.push_str(&format!("{:>6}\t{line}\n", at + 1));
    }
    let last = (from - 1 + most).min(total);
    if last < total {
        out.push_str(&format!(
            "[lines {from} to {last} of {total}: read on with offset {}]\n",
            last + 1
        ));
    }
    if out.is_empty() {
        out = if total == 0 {
            "[the file is empty]\n".into()
        } else {
            format!("[the file has {total} lines]\n")
        };
    }
    out
}

/// `text` with `old` replaced by `new`: once, where it is found exactly
/// once, or everywhere when `all`.
pub(crate) fn replaced(text: &str, old: &str, new: &str, all: bool) -> Result<String, String> {
    if old.is_empty() {
        return Err("`old_text` is empty".into());
    }
    match text.matches(old).count() {
        0 => Err("`old_text` is not in the file: read it again and copy it exactly".into()),
        1 => Ok(text.replacen(old, new, 1)),
        _ if all => Ok(text.replace(old, new)),
        n => Err(format!(
            "`old_text` is found {n} times: give more of the text around it, or set `all`"
        )),
    }
}

/// A call's input, in short, as the window shows it.
pub(crate) fn summary(args: &Value) -> String {
    let short = |text: &str| -> String {
        let line = text.lines().next().unwrap_or_default();
        if line.chars().count() > 120 || text.lines().nth(1).is_some() {
            let cut: String = line.chars().take(120).collect();
            format!("{cut}…")
        } else {
            line.to_owned()
        }
    };
    for key in [
        "command",
        "path",
        "to",
        "name",
        "file_path",
        "pattern",
        "url",
    ] {
        if let Some(text) = args[key].as_str() {
            return short(text);
        }
    }
    short(&args.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_numbered_and_the_rest_announced() {
        let text = "a\nb\nc\nd\n";
        assert_eq!(
            numbered(text, None, None),
            "     1\ta\n     2\tb\n     3\tc\n     4\td\n"
        );
        assert_eq!(
            numbered(text, Some(2), Some(2)),
            "     2\tb\n     3\tc\n[lines 2 to 3 of 4: read on with offset 4]\n"
        );
        assert_eq!(numbered("", None, None), "[the file is empty]\n");
        assert_eq!(numbered(text, Some(9), None), "[the file has 4 lines]\n");
    }

    #[test]
    fn an_edit_needs_its_text_once() {
        assert_eq!(replaced("a b a", "b", "c", false).unwrap(), "a c a");
        assert!(
            replaced("a b a", "a", "c", false)
                .unwrap_err()
                .contains("2 times")
        );
        assert_eq!(replaced("a b a", "a", "c", true).unwrap(), "c b c");
        assert!(replaced("a", "z", "c", false).is_err());
        assert!(replaced("a", "", "c", false).is_err());
    }

    #[test]
    fn arguments_say_what_is_wrong() {
        assert!(Args::parse("{").unwrap_err().contains("not valid JSON"));
        assert!(Args::parse("[]").is_err());
        let args = Args::parse("").unwrap();
        assert!(args.text("path").unwrap_err().contains("`path`"));
        let args = Args::parse(r#"{"path": "a", "limit": 3, "all": true}"#).unwrap();
        assert_eq!(args.text("path").unwrap(), "a");
        assert_eq!(args.number("limit"), Some(3));
        assert!(args.flag("all"));
        assert_eq!(
            summary(&json!({"command": "cargo test\nmore"})),
            "cargo test…"
        );
    }
}
