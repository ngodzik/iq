//! What iq and its plugins say to each other.
//!
//! A plugin is a program of its own, apart from iq's binary: the plugin
//! named `ports` is the program `iq-ports`, found on the `PATH`, which iq
//! starts with [`FLAG`] once the person's settings list it, and never
//! otherwise. Each says what it has to say as one line of JSON on its
//! standard output, and reads iq's on its standard input; what it writes
//! on its standard error goes to iq's log.
//!
//! iq speaks first, with [`ToPlugin::Hello`], and the plugin answers with
//! [`FromPlugin::Hello`]: the protocol it speaks, which iq checks, and the
//! commands it takes. Then iq tells it where the chats work and what the
//! person asks of it, and the plugin tells iq what to show and what the
//! commands run in a chat's place get. A plugin is the person's program:
//! it runs on the machine with their rights, as they would run it, and
//! answers to them alone. No agent can start one, nor speak to it.
//!
//! The crate holds the messages and how they are written, and nothing
//! else: no I/O, so that iq and every plugin read the same words.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The argument iq starts a plugin with, for it to speak the protocol on
/// its standard input and output rather than do what it does alone.
pub const FLAG: &str = "--iq-plugin";

/// The version of the protocol: a plugin that speaks another one is not
/// spoken to, and the person is told to install both from the same
/// commit.
pub const PROTOCOL: u32 = 1;

/// What a plugin's program is called before its name.
pub const PREFIX: &str = "iq-";

/// The program of the plugin `name`: `iq-ports` for `ports`.
///
/// # Examples
///
/// ```
/// assert_eq!(iq_plugin::program("ports"), "iq-ports");
/// ```
#[must_use]
pub fn program(name: &str) -> String {
    format!("{PREFIX}{name}")
}

/// Whether `name` may name a plugin: lowercase letters, digits and
/// hyphens, a letter first, so that its program is found on the `PATH`
/// and nowhere else.
///
/// # Examples
///
/// ```
/// assert!(iq_plugin::valid_name("ports"));
/// assert!(!iq_plugin::valid_name("../ports"));
/// assert!(!iq_plugin::valid_name(""));
/// ```
#[must_use]
pub fn valid_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// What iq says to a plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToPlugin {
    /// The first thing iq says: the protocol it speaks, and the folder it
    /// works in, the person's checkout.
    Hello {
        /// iq's [`PROTOCOL`].
        protocol: u32,
        /// The folder iq works in.
        workspace: PathBuf,
    },
    /// A chat works in a place of its own, a worktree: as it starts, or as
    /// iq starts again with it.
    Placed {
        /// The chat, as the board names it.
        chat: String,
        /// Its folder.
        folder: PathBuf,
    },
    /// The person ran one of the plugin's commands.
    Command {
        /// The command, as the plugin named it.
        name: String,
        /// What the person wrote after it, trimmed.
        args: String,
        /// The chat the person writes to, when they write to one.
        chat: Option<String>,
        /// Its place of its own, when it has one.
        folder: Option<PathBuf>,
    },
    /// iq ends: the plugin ends too, once it let go of what it holds.
    Bye,
}

/// What a plugin says to iq.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FromPlugin {
    /// Its answer to iq's hello: the protocol it speaks, and the commands
    /// it takes.
    Hello {
        /// The plugin's [`PROTOCOL`].
        protocol: u32,
        /// Its commands, written after a `/` as iq's own are.
        #[serde(default)]
        commands: Vec<CommandSpec>,
    },
    /// What it shows on the status line, in place of what it showed
    /// before; empty for nothing.
    Status {
        /// The text.
        text: String,
    },
    /// The variables every command run in `folder` gets from now on, in
    /// place of those given before, the agents' as the person's; and the
    /// line that sets them in a terminal there, shown for the person to
    /// paste.
    Variables {
        /// The place, as [`ToPlugin::Placed`] named it.
        folder: PathBuf,
        /// The variables, by name.
        vars: BTreeMap<String, String>,
        /// A shell line setting them, run after `cd` to the folder.
        enter: Option<String>,
    },
    /// What the Team page shows under a chat, in place of what it showed
    /// before; none for nothing.
    Lines {
        /// The chat.
        chat: String,
        /// The lines.
        lines: Vec<String>,
    },
    /// Something said to the person in the conversation.
    Say {
        /// The text.
        text: String,
    },
    /// A command the plugin is about to run on the machine, for the audit
    /// log to show it beside the gate's.
    Running {
        /// The command line.
        line: String,
        /// Where it runs.
        folder: PathBuf,
    },
}

/// A command a plugin takes, as the list of commands shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    /// Its name, after the `/`.
    pub name: String,
    /// What it takes, in words; empty for nothing.
    #[serde(default)]
    pub args: String,
    /// What it does.
    pub what: String,
}

/// What went wrong writing or reading a message.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// A message could not be written as JSON.
    #[error("the message could not be written")]
    Encode(#[source] serde_json::Error),
    /// A line is not a message this protocol has.
    #[error("{line:?} is not a message of the protocol")]
    Decode {
        /// The line, cut when it is long.
        line: String,
        /// Why.
        #[source]
        source: serde_json::Error,
    },
}

/// The most of a line a [`PluginError::Decode`] keeps.
const SHOWN: usize = 200;

/// `message` as the line that carries it, its newline last.
///
/// # Errors
///
/// It cannot be written as JSON.
///
/// # Examples
///
/// ```
/// use iq_plugin::{ToPlugin, encode};
///
/// assert_eq!(encode(&ToPlugin::Bye)?, "{\"kind\":\"bye\"}\n");
/// # Ok::<(), iq_plugin::PluginError>(())
/// ```
pub fn encode(message: &impl Serialize) -> Result<String, PluginError> {
    let mut line = serde_json::to_string(message).map_err(PluginError::Encode)?;
    line.push('\n');
    Ok(line)
}

/// The message `line` carries, its newline or none.
///
/// # Errors
///
/// It is not a message of the protocol: a plugin of another version, or
/// a line printed by mistake.
///
/// # Examples
///
/// ```
/// use iq_plugin::{FromPlugin, decode};
///
/// let said: FromPlugin = decode("{\"kind\":\"say\",\"text\":\"up\"}\n")?;
/// assert_eq!(said, FromPlugin::Say { text: "up".into() });
/// assert!(decode::<FromPlugin>("starting…").is_err());
/// # Ok::<(), iq_plugin::PluginError>(())
/// ```
pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, PluginError> {
    serde_json::from_str(line.trim_end_matches(['\n', '\r'])).map_err(|source| {
        PluginError::Decode {
            line: line.chars().take(SHOWN).collect(),
            source,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_iq_says_reads_as_fixed_lines() {
        let said = [
            (
                ToPlugin::Hello {
                    protocol: 1,
                    workspace: "/work/shop".into(),
                },
                r#"{"kind":"hello","protocol":1,"workspace":"/work/shop"}"#,
            ),
            (
                ToPlugin::Placed {
                    chat: "lead".into(),
                    folder: "/data/worktrees/shop/lead".into(),
                },
                r#"{"kind":"placed","chat":"lead","folder":"/data/worktrees/shop/lead"}"#,
            ),
            (
                ToPlugin::Command {
                    name: "serve".into(),
                    args: String::new(),
                    chat: Some("lead".into()),
                    folder: None,
                },
                r#"{"kind":"command","name":"serve","args":"","chat":"lead","folder":null}"#,
            ),
            (ToPlugin::Bye, r#"{"kind":"bye"}"#),
        ];
        for (message, line) in said {
            assert_eq!(encode(&message).unwrap(), format!("{line}\n"));
            assert_eq!(decode::<ToPlugin>(line).unwrap(), message);
        }
    }

    #[test]
    fn what_a_plugin_says_reads_from_fixed_lines_what_it_leaves_out_defaulted() {
        let hello: FromPlugin = decode(
            r#"{"kind":"hello","protocol":1,"commands":[{"name":"serve","what":"localhost to this chat"}]}"#,
        )
        .unwrap();
        assert_eq!(
            hello,
            FromPlugin::Hello {
                protocol: 1,
                commands: vec![CommandSpec {
                    name: "serve".into(),
                    args: String::new(),
                    what: "localhost to this chat".into(),
                }],
            }
        );
        let variables: FromPlugin = decode(
            r#"{"kind":"variables","folder":"/w/lead","vars":{"COMPOSE_PROJECT_NAME":"shop-lead"},"enter":"eval \"$(iq-ports env)\""}"#,
        )
        .unwrap();
        let FromPlugin::Variables { vars, enter, .. } = variables else {
            panic!("{variables:?}");
        };
        assert_eq!(vars["COMPOSE_PROJECT_NAME"], "shop-lead");
        assert_eq!(enter.as_deref(), Some("eval \"$(iq-ports env)\""));
        // Another version's message, or a line printed by mistake, is
        // told apart, cut when long.
        let long = "x".repeat(1000);
        let Err(PluginError::Decode { line, .. }) = decode::<FromPlugin>(&long) else {
            panic!("decoded");
        };
        assert_eq!(line.len(), SHOWN);
        assert!(decode::<FromPlugin>(r#"{"kind":"fly"}"#).is_err());
    }
}
