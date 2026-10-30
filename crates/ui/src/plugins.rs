//! What the person's plugins show: the commands each takes, written after
//! a `/` as iq's own are, what it says on the status line, its lines under
//! a chat on the Team page, and the line that sets a place's variables in
//! another terminal. Each says it in place of what it said before; a
//! plugin that stopped takes all of it with it.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::command;

/// A command a plugin takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCommand {
    /// Its name, after the `/`.
    pub name: String,
    /// What it takes, in words; empty for nothing.
    pub args: String,
    /// What it does.
    pub what: String,
}

/// What a plugin told iq to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Told {
    /// The commands it takes, as it started.
    Commands(Vec<PluginCommand>),
    /// What it shows on the status line; empty for nothing.
    Status(String),
    /// Its lines under a chat on the Team page.
    Lines {
        /// The chat.
        chat: String,
        /// The lines; none for nothing.
        lines: Vec<String>,
    },
    /// The shell line that sets a place's variables in a terminal there,
    /// after `cd` to it; none for nothing.
    Enter {
        /// The place.
        folder: PathBuf,
        /// The line.
        line: Option<String>,
    },
    /// It stopped.
    Gone,
}

/// One plugin as the board shows it.
#[derive(Debug, Clone, Default)]
struct Shown {
    commands: Vec<PluginCommand>,
    status: String,
    lines: HashMap<String, Vec<String>>,
    enters: HashMap<PathBuf, String>,
}

/// The person's plugins, by name.
#[derive(Debug, Clone, Default)]
pub struct Plugins {
    shown: BTreeMap<String, Shown>,
}

impl Plugins {
    /// Takes what `plugin` told. A command named as one of iq's is left
    /// out: iq's own come first.
    pub fn told(&mut self, plugin: &str, told: Told) {
        if told == Told::Gone {
            self.shown.remove(plugin);
            return;
        }
        let shown = self.shown.entry(plugin.to_owned()).or_default();
        match told {
            Told::Commands(commands) => {
                shown.commands = commands
                    .into_iter()
                    .filter(|c| !c.name.is_empty() && !command::known(&c.name))
                    .collect();
            }
            Told::Status(text) => shown.status = text,
            Told::Lines { chat, lines } => {
                if lines.is_empty() {
                    shown.lines.remove(&chat);
                } else {
                    shown.lines.insert(chat, lines);
                }
            }
            Told::Enter { folder, line } => match line {
                Some(line) => {
                    shown.enters.insert(folder, line);
                }
                None => {
                    shown.enters.remove(&folder);
                }
            },
            Told::Gone => {}
        }
    }

    /// Every plugin command, each with its plugin, in the order of the
    /// plugins' names.
    pub fn commands(&self) -> impl Iterator<Item = (&str, &PluginCommand)> {
        self.shown.iter().flat_map(|(plugin, shown)| {
            shown
                .commands
                .iter()
                .map(move |command| (plugin.as_str(), command))
        })
    }

    /// The plugin that takes the command `name`, and the command.
    #[must_use]
    pub fn command(&self, name: &str) -> Option<(&str, &PluginCommand)> {
        self.commands().find(|(_, command)| command.name == name)
    }

    /// What the plugins show on the status line, each that shows
    /// something.
    pub fn statuses(&self) -> impl Iterator<Item = &str> {
        self.shown
            .values()
            .map(|shown| shown.status.as_str())
            .filter(|status| !status.is_empty())
    }

    /// Their lines under `chat` on the Team page.
    #[must_use]
    pub fn lines(&self, chat: &str) -> Vec<&str> {
        self.shown
            .values()
            .filter_map(|shown| shown.lines.get(chat))
            .flatten()
            .map(String::as_str)
            .collect()
    }

    /// The line that sets the variables of `folder` in a terminal there,
    /// when a plugin gave it some.
    #[must_use]
    pub fn enter(&self, folder: &Path) -> Option<&str> {
        self.shown
            .values()
            .find_map(|shown| shown.enters.get(folder))
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serve() -> PluginCommand {
        PluginCommand {
            name: "serve".into(),
            args: "[chat]".into(),
            what: "localhost to this chat".into(),
        }
    }

    #[test]
    fn a_plugin_shows_what_it_said_last_and_takes_it_all_when_it_stops() {
        let mut plugins = Plugins::default();
        let costs = PluginCommand {
            name: "costs".into(),
            ..serve()
        };
        plugins.told("ports", Told::Commands(vec![serve(), costs]));
        // iq's own commands come first.
        assert_eq!(plugins.commands().count(), 1);
        assert_eq!(plugins.command("serve").unwrap().0, "ports");
        assert!(plugins.command("costs").is_none());
        plugins.told("ports", Told::Status("localhost → lead".into()));
        plugins.told("ports", Told::Status("localhost → fix".into()));
        assert_eq!(plugins.statuses().collect::<Vec<_>>(), ["localhost → fix"]);
        plugins.told(
            "ports",
            Told::Lines {
                chat: "lead".into(),
                lines: vec!["stack running".into()],
            },
        );
        assert_eq!(plugins.lines("lead"), ["stack running"]);
        assert!(plugins.lines("fix").is_empty());
        let folder = PathBuf::from("/w/lead");
        plugins.told(
            "ports",
            Told::Enter {
                folder: folder.clone(),
                line: Some("eval \"$(iq-ports env)\"".into()),
            },
        );
        assert_eq!(plugins.enter(&folder), Some("eval \"$(iq-ports env)\""));
        plugins.told("ports", Told::Gone);
        assert!(plugins.command("serve").is_none());
        assert_eq!(plugins.statuses().count(), 0);
        assert!(plugins.lines("lead").is_empty());
        assert!(plugins.enter(&folder).is_none());
    }
}
