//! Claude Code's tool calls judged by iq's gate, through the hook it runs
//! before each one: its files where the gate judges them, its commands in
//! iq's sandbox, the network asked of the person.
//!
//! A command the gate allows is given back to Claude Code changed: it runs
//! iq's program, which has the running iq run the command it holds the
//! permit for, in its sandbox. Claude Code's own checks are then spared,
//! as iq's gate judged it and its sandbox holds it.

use std::path::{Path, PathBuf};

use iq_gate::Action;
use serde_json::{Value, json};

/// What starts a line of Claude Code's that asks for the internet, which
/// its commands are without: the gate then asks the person, unless a rule
/// gives it or the person opened it to every command.
pub(crate) const ASK_NETWORK: &str = "IQ_NETWORK=1";

/// What a call is, as the gate sees it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Call {
    /// The gate has nothing to say of it: Claude Code's own mode decides,
    /// as for its plans and its lists of tasks.
    Defer,
    /// The gate judges this action; `command` is the call's input when it
    /// is a command, to give back changed.
    Judge {
        action: Action,
        command: Option<Value>,
    },
    /// Refused as it is.
    Refuse(String),
}

/// What Claude Code asks, from the hook's input: its tool, its arguments,
/// the folder it works in.
pub(crate) fn read(input: &Value, workspace: &Path) -> Call {
    let tool = input["tool_name"].as_str().unwrap_or_default();
    let args = &input["tool_input"];
    let folder = input["cwd"]
        .as_str()
        .map_or_else(|| workspace.to_path_buf(), PathBuf::from);
    let path = |key: &str| args[key].as_str().map(|path| folder.join(path));
    let file = |key: &str, write: bool| match path(key) {
        Some(path) if write => Call::Judge {
            action: Action::Write { path },
            command: None,
        },
        Some(path) => Call::Judge {
            action: Action::Read { path },
            command: None,
        },
        None => Call::Refuse(format!("{tool} was called without `{key}`")),
    };
    match tool {
        "Bash" => match args["command"].as_str() {
            Some(line) => Call::Judge {
                action: Action::Run {
                    line: line.to_owned(),
                    folder,
                    network: line
                        .trim_start()
                        .strip_prefix(ASK_NETWORK)
                        .is_some_and(|rest| rest.starts_with(char::is_whitespace)),
                },
                command: Some(args.clone()),
            },
            None => Call::Refuse("Bash was called without a command".into()),
        },
        "Read" => file("file_path", false),
        "NotebookRead" => file("notebook_path", false),
        "Write" | "Edit" | "MultiEdit" => file("file_path", true),
        "NotebookEdit" => file("notebook_path", true),
        "Glob" | "Grep" | "LS" => Call::Judge {
            action: Action::Read {
                path: path("path").unwrap_or(folder),
            },
            command: None,
        },
        "WebFetch" | "WebSearch" => {
            let target = args["url"]
                .as_str()
                .or_else(|| args["query"].as_str())
                .unwrap_or_default();
            Call::Judge {
                action: Action::Fetch {
                    target: target.to_owned(),
                },
                command: None,
            }
        }
        _ => Call::Defer,
    }
}

/// The most of the messages a hook's answer carries: Claude Code keeps
/// 10,000 characters of it, and puts the rest in a file it does not read.
const CONTEXT_CHARS: usize = 8_000;

/// The messages for the agent, as context beside the call's result: how
/// they reach Claude Code while it works. Cut to what it keeps.
fn with_context(mut output: Value, news: Option<&str>) -> String {
    if let Some(news) = news {
        let mut text: String = news.chars().take(CONTEXT_CHARS).collect();
        if news.chars().count() > CONTEXT_CHARS {
            text.push_str("\n[cut: call wait_for_messages for the rest]");
        }
        output["additionalContext"] = Value::from(format!("On iq's board, for you:\n\n{text}"));
    }
    json!({ "hookSpecificOutput": output }).to_string()
}

/// The hook's answer letting a call through, changed to `input` when
/// given, with the messages for the agent.
pub(crate) fn allow(input: Option<Value>, news: Option<&str>) -> String {
    let mut output = json!({
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "permissionDecisionReason": "iq's gate allowed it",
    });
    if let Some(input) = input {
        output["updatedInput"] = input;
    }
    with_context(output, news)
}

/// The hook's answer refusing a call, the reason for the agent to read,
/// with the messages for it.
pub(crate) fn deny(reason: &str, news: Option<&str>) -> String {
    with_context(
        json!({
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": format!("iq refused it: {reason}"),
        }),
        news,
    )
}

/// The hook's answer leaving the call to Claude Code's own rules, with the
/// messages for the agent, if any: nothing at all else.
pub(crate) fn defer(news: Option<&str>) -> String {
    match news {
        Some(_) => with_context(json!({ "hookEventName": "PreToolUse" }), news),
        None => String::new(),
    }
}

/// A command's input changed to run `program` with `args` instead: iq's,
/// which runs the command judged in iq's sandbox.
pub(crate) fn through(mut input: Value, program: &Path, args: &[&str]) -> Value {
    let line = std::iter::once(program.display().to_string())
        .chain(args.iter().map(|a| (*a).to_owned()))
        .map(|word| format!("'{}'", word.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ");
    input["command"] = Value::from(line);
    input
}

#[cfg(test)]
mod tests {
    #[test]
    fn messages_ride_on_the_hooks_answers_and_are_cut_to_what_is_kept() {
        let allowed: Value =
            serde_json::from_str(&allow(None, Some("Message from tests: done"))).unwrap();
        let context = allowed["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(context.ends_with("Message from tests: done"));
        assert_eq!(defer(None), "");
        let long = "x".repeat(20_000);
        let deferred: Value = serde_json::from_str(&defer(Some(&long))).unwrap();
        let context = deferred["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(context.chars().count() < 10_000);
        assert!(
            deferred["hookSpecificOutput"]
                .get("permissionDecision")
                .is_none()
        );
        let denied: Value = serde_json::from_str(&deny("no", None)).unwrap();
        assert!(
            denied["hookSpecificOutput"]
                .get("additionalContext")
                .is_none()
        );
    }

    use super::*;

    #[test]
    fn calls_become_actions() {
        let workspace = Path::new("/w");
        let call = |tool: &str, args: Value| {
            read(
                &json!({"tool_name": tool, "tool_input": args, "cwd": "/w/sub"}),
                workspace,
            )
        };
        assert_eq!(
            call("Bash", json!({"command": "ls", "timeout": 5})),
            Call::Judge {
                action: Action::Run {
                    line: "ls".into(),
                    folder: "/w/sub".into(),
                    network: false
                },
                command: Some(json!({"command": "ls", "timeout": 5})),
            }
        );
        assert!(matches!(
            call(
                "Bash",
                json!({"command": "IQ_NETWORK=1 curl -s api.example"})
            ),
            Call::Judge {
                action: Action::Run { network: true, .. },
                ..
            }
        ));
        assert_eq!(
            call("Edit", json!({"file_path": "src/a.rs"})),
            Call::Judge {
                action: Action::Write {
                    path: "/w/sub/src/a.rs".into()
                },
                command: None
            }
        );
        assert_eq!(
            call("Read", json!({"file_path": "/etc/hosts"})),
            Call::Judge {
                action: Action::Read {
                    path: "/etc/hosts".into()
                },
                command: None
            }
        );
        assert_eq!(
            call("Grep", json!({"pattern": "x"})),
            Call::Judge {
                action: Action::Read {
                    path: "/w/sub".into()
                },
                command: None
            }
        );
        assert!(matches!(
            call("WebFetch", json!({"url": "https://a.b"})),
            Call::Judge {
                action: Action::Fetch { .. },
                ..
            }
        ));
        assert_eq!(call("TodoWrite", json!({})), Call::Defer);
        assert!(matches!(call("Write", json!({})), Call::Refuse(_)));
    }

    #[test]
    fn a_command_is_given_back_through_iq() {
        let input = through(
            json!({"command": "rm -rf x", "description": "clean"}),
            Path::new("/opt/it's/iq"),
            &["--iq-exec", "ab12"],
        );
        assert_eq!(
            input,
            json!({"command": "'/opt/it'\\''s/iq' '--iq-exec' 'ab12'", "description": "clean"})
        );
        let answer: Value = serde_json::from_str(&allow(Some(input), None)).unwrap();
        assert_eq!(answer["hookSpecificOutput"]["permissionDecision"], "allow");
        assert_eq!(
            answer["hookSpecificOutput"]["updatedInput"]["description"],
            "clean"
        );
        let answer: Value = serde_json::from_str(&deny("it publishes commits", None)).unwrap();
        assert_eq!(
            answer["hookSpecificOutput"]["permissionDecisionReason"],
            "iq refused it: it publishes commits"
        );
    }
}
