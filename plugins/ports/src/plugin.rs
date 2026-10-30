//! The plugin as iq speaks with it: told where each chat works, it keeps
//! that worktree's stack apart and gives its commands the variables that
//! make their compose its own; asked to serve a chat, it starts that
//! stack when it does not run and leads the usual ports of localhost to
//! it; and it shows on iq's status line where they lead.
//!
//! As it starts, the stacks of worktrees that are gone are stopped, their
//! data kept and said, and the usual ports lead again where they led.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use iq_plugin::{CommandSpec, FromPlugin, PROTOCOL, ToPlugin};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::relay::Relay;
use crate::shim::Shim;
use crate::stack::Stack;
use crate::store::{Kept, Store};
use crate::{PortsError, docker};

/// What the person pastes in a terminal in a worktree to have its
/// compose there.
pub const ENTER: &str = "eval \"$(iq-ports env)\"";

/// The commands the plugin takes.
fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            name: "serve".into(),
            args: "[chat]".into(),
            what: "localhost's usual ports to this chat's stack, started if it is not".into(),
        },
        CommandSpec {
            name: "unserve".into(),
            args: String::new(),
            what: "localhost's usual ports to no chat".into(),
        },
    ]
}

/// What work done apart tells the plugin.
enum Done {
    /// A chat's stack runs, or could not start.
    Started {
        chat: String,
        folder: PathBuf,
        stack: Result<Stack, String>,
    },
}

/// The plugin's state while iq speaks with it.
struct Plugin {
    store: Store,
    config: Config,
    /// The person's `COMPOSE_FILE`, when they set one of their own.
    listed: Option<String>,
    /// The compose that takes docker's in a worktree.
    shim: Option<Shim>,
    /// Each chat's place, as iq names it.
    chats: HashMap<String, PathBuf>,
    /// Each place's stack, by the folder iq names.
    stacks: HashMap<PathBuf, Stack>,
    relay: Relay,
    /// The chat the usual ports lead to, and its place.
    served: Option<(String, PathBuf)>,
    /// The places whose stack is starting.
    starting: HashSet<PathBuf>,
    /// The places whose notes were said.
    noted: HashSet<PathBuf>,
    out: mpsc::UnboundedSender<FromPlugin>,
    done: mpsc::UnboundedSender<Done>,
}

/// Speaks with iq over `input` and `output` until it says goodbye, or
/// its side closes: the plugin's files in `store`, its settings `config`,
/// the person's own `COMPOSE_FILE` in `listed`, the compose that takes
/// docker's in a worktree `shim`. What it leads stops being led as it
/// ends; the stacks go on running.
///
/// # Errors
///
/// What iq says cannot be read.
pub async fn speak(
    input: impl AsyncBufRead + Unpin,
    output: impl AsyncWrite + Unpin + Send + 'static,
    store: Store,
    config: Config,
    listed: Option<String>,
    shim: Option<Shim>,
) -> Result<(), PortsError> {
    let (out, mut said) = mpsc::unbounded_channel::<FromPlugin>();
    let writer = tokio::spawn(async move {
        let mut output = output;
        while let Some(message) = said.recv().await {
            let Ok(line) = iq_plugin::encode(&message) else {
                continue;
            };
            if output.write_all(line.as_bytes()).await.is_err() || output.flush().await.is_err() {
                return;
            }
        }
    });
    let (done, mut finished) = mpsc::unbounded_channel();
    // A COMPOSE_FILE the plugin's own variables set is not the person's.
    let ours = store.folder().display().to_string();
    let listed = listed.filter(|files| !files.contains(&ours));
    let mut plugin = Plugin {
        store,
        config,
        listed,
        shim,
        chats: HashMap::new(),
        stacks: HashMap::new(),
        relay: Relay::default(),
        served: None,
        starting: HashSet::new(),
        noted: HashSet::new(),
        out,
        done,
    };
    let mut lines = input.lines();
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                let message: ToPlugin = match iq_plugin::decode(&line) {
                    Ok(message) => message,
                    Err(error) => {
                        eprintln!("{error}");
                        continue;
                    }
                };
                if !plugin.told(message).await {
                    break;
                }
            }
            Some(done) = finished.recv() => plugin.finished(done).await,
        }
    }
    drop(plugin);
    // What was said is written, unless a stack still starting holds it.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), writer).await;
    Ok(())
}

impl Plugin {
    fn say(&self, text: impl Into<String>) {
        let _ = self.out.send(FromPlugin::Say { text: text.into() });
    }

    /// Takes what iq said; false once it said goodbye.
    async fn told(&mut self, message: ToPlugin) -> bool {
        match message {
            ToPlugin::Hello { .. } => {
                let _ = self.out.send(FromPlugin::Hello {
                    protocol: PROTOCOL,
                    commands: commands(),
                });
                self.opened().await;
            }
            ToPlugin::Placed { chat, folder } => self.placed(chat, folder).await,
            ToPlugin::Command {
                name,
                args,
                chat,
                folder,
            } => match name.as_str() {
                "serve" => self.serve(args.trim(), chat, folder),
                "unserve" => self.unserve().await,
                other => self.say(format!("iq-ports has no /{other}")),
            },
            ToPlugin::Bye => return false,
        }
        true
    }

    /// As iq opens: the stacks of worktrees that are gone stopped, and the
    /// usual ports led again where they led, until the chat comes back.
    async fn opened(&mut self) {
        let mut kept = self.store.load();
        let gone: Vec<PathBuf> = kept
            .places
            .keys()
            .filter(|folder| !folder.is_dir())
            .cloned()
            .collect();
        for folder in gone {
            let Some(place) = kept.places.remove(&folder) else {
                continue;
            };
            let at = self.store.folder();
            let _ = std::fs::create_dir_all(at);
            let down = docker::run(
                "docker",
                &["compose", "-p", &place.project, "down"],
                at,
                &Default::default(),
                docker::READING,
            )
            .await;
            self.store.forget(&place.project);
            match down {
                Ok(down) if down.ok => self.say(format!(
                    "{} is gone: its stack {} is stopped. Its data is kept: `docker compose -p \
                     {} down --volumes` removes it.",
                    folder.display(),
                    place.project,
                    place.project
                )),
                _ => self.say(format!(
                    "{} is gone, and its stack {} could not be stopped: `docker compose -p {} \
                     down` stops it.",
                    folder.display(),
                    place.project,
                    place.project
                )),
            }
        }
        if kept.served.as_ref().is_some_and(|served| !served.is_dir()) {
            kept.served = None;
        }
        let served = kept.served.clone();
        if let Err(error) = self.store.save(&kept) {
            self.say(error.to_string());
        }
        if let Some(served) = served {
            let listed = self.listed.as_deref();
            match Stack::prepare(&served, listed, &self.store, &kept, self.shim.as_ref()).await {
                Ok(stack) => {
                    let name = leaf(&stack.folder);
                    self.lead(&name, served, stack).await;
                }
                Err(_) => self.save(|kept| kept.served = None),
            }
        }
    }

    /// Changes what is kept, as `change` does.
    fn save(&self, change: impl FnOnce(&mut Kept)) {
        let mut kept = self.store.load();
        change(&mut kept);
        if let Err(error) = self.store.save(&kept) {
            self.say(error.to_string());
        }
    }

    /// A chat works in `folder`: its stack kept apart, its commands given
    /// what makes their compose its own.
    async fn placed(&mut self, chat: String, folder: PathBuf) {
        self.chats.insert(chat.clone(), folder.clone());
        let kept = self.store.load();
        let listed = self.listed.as_deref();
        let shim = self.shim.as_ref();
        let stack = match Stack::prepare(&folder, listed, &self.store, &kept, shim).await {
            Ok(stack) => stack,
            // Not a project of containers: nothing to keep apart.
            Err(PortsError::NoCompose(_)) => return,
            Err(error) => {
                if self.noted.insert(folder.clone()) {
                    self.say(format!("{chat}'s stack is not kept apart: {error}"));
                }
                return;
            }
        };
        self.save(|kept| kept.keep(stack.folder.clone(), stack.place()));
        let _ = self.out.send(FromPlugin::Variables {
            folder: folder.clone(),
            vars: stack.vars(),
            enter: Some(ENTER.to_owned()),
        });
        if self.noted.insert(folder.clone()) {
            for note in &stack.notes {
                self.say(format!("{chat}'s stack: {note}"));
            }
        }
        // Led there before iq opened, it is now this chat's.
        if let Some((served, at)) = &mut self.served
            && self.stacks.get(at.as_path()).map(|s| &s.folder) == Some(&stack.folder)
        {
            *served = chat.clone();
            *at = folder.clone();
            let _ = self.out.send(FromPlugin::Status {
                text: format!("localhost → {chat}"),
            });
        }
        let served = self.served.as_ref().is_some_and(|(_, at)| *at == folder);
        let lines = stack.said(stack.running().await, served);
        self.stacks.insert(folder, stack);
        let _ = self.out.send(FromPlugin::Lines { chat, lines });
    }

    /// Serves `named`, or the chat written to: its stack started apart
    /// when it does not run, then the usual ports led to it.
    fn serve(&mut self, named: &str, chat: Option<String>, folder: Option<PathBuf>) {
        let (chat, folder) = if named.is_empty() {
            (chat, folder)
        } else {
            (Some(named.to_owned()), None)
        };
        let Some(chat) = chat else {
            self.say("/serve serves a chat: write to one first, or name it, /serve <chat>");
            return;
        };
        let Some(folder) = folder.or_else(|| self.chats.get(&chat).cloned()) else {
            self.say(format!(
                "{chat} works in your checkout, not in a worktree of its own: with `worktree = \
                 true` in iq's settings, each chat you start has one"
            ));
            return;
        };
        if !self.starting.insert(folder.clone()) {
            self.say(format!("{chat}'s stack is starting already"));
            return;
        }
        let (store, listed, shim, out, done) = (
            self.store.clone(),
            self.listed.clone(),
            self.shim.clone(),
            self.out.clone(),
            self.done.clone(),
        );
        let config = self.config.clone();
        tokio::spawn(async move {
            let reading = Reading {
                store: &store,
                listed: listed.as_deref(),
                shim: shim.as_ref(),
            };
            let stack = start(&chat, &folder, &reading, &config, &out).await;
            let _ = done.send(Done::Started {
                chat,
                folder,
                stack,
            });
        });
    }

    /// What work done apart says.
    async fn finished(&mut self, done: Done) {
        let Done::Started {
            chat,
            folder,
            stack,
        } = done;
        self.starting.remove(&folder);
        match stack {
            Ok(stack) => self.lead(&chat, folder, stack).await,
            Err(why) => self.say(format!("{chat}'s stack did not start: {why}")),
        }
    }

    /// Leads the usual ports to `chat`'s stack.
    async fn lead(&mut self, chat: &str, folder: PathBuf, stack: Stack) {
        let refused = self.relay.lead(stack.relayed()).await;
        for (port, error) in refused {
            self.say(format!(
                "localhost:{port} does not lead to {chat}: another program holds it ({error}), \
                 a stack started without iq-ports, say"
            ));
        }
        let before = self.served.replace((chat.to_owned(), folder.clone()));
        self.save(|kept| {
            kept.keep(stack.folder.clone(), stack.place());
            kept.served = Some(stack.folder.clone());
        });
        let _ = self.out.send(FromPlugin::Status {
            text: format!("localhost → {chat}"),
        });
        let _ = self.out.send(FromPlugin::Lines {
            chat: chat.to_owned(),
            lines: stack.said(true, true),
        });
        self.stacks.insert(folder.clone(), stack);
        if let Some((was, at)) = before
            && at != folder
        {
            self.tell_lines(&was, &at).await;
        }
    }

    /// Leads the usual ports nowhere.
    async fn unserve(&mut self) {
        self.relay.lead(HashMap::new()).await;
        let before = self.served.take();
        self.save(|kept| kept.served = None);
        let _ = self.out.send(FromPlugin::Status {
            text: String::new(),
        });
        if let Some((was, at)) = before {
            self.tell_lines(&was, &at).await;
        }
    }

    /// Tells the Team page what `chat`'s stack is now.
    async fn tell_lines(&self, chat: &str, folder: &Path) {
        if let Some(stack) = self.stacks.get(folder) {
            let served = self.served.as_ref().is_some_and(|(_, at)| at == folder);
            let _ = self.out.send(FromPlugin::Lines {
                chat: chat.to_owned(),
                lines: stack.said(stack.running().await, served),
            });
        }
    }
}

/// What a stack is read with.
struct Reading<'a> {
    store: &'a Store,
    listed: Option<&'a str>,
    shim: Option<&'a Shim>,
}

impl Reading<'_> {
    /// The stack of the worktree `folder` is in, as it now is.
    async fn stack(&self, folder: &Path) -> Result<Stack, String> {
        let kept = self.store.load();
        Stack::prepare(folder, self.listed, self.store, &kept, self.shim)
            .await
            .map_err(|error| error.to_string())
    }
}

/// Starts `chat`'s stack in `folder` when it does not run, saying so.
async fn start(
    chat: &str,
    folder: &Path,
    reading: &Reading<'_>,
    config: &Config,
    out: &mpsc::UnboundedSender<FromPlugin>,
) -> Result<Stack, String> {
    let say = |text: String| {
        let _ = out.send(FromPlugin::Say { text });
    };
    let stack = reading.stack(folder).await?;
    if stack.running().await {
        return Ok(stack);
    }
    let up = config.up(&stack.repository).to_owned();
    if let Some(note) = stack.unmoved_by(&up).await {
        say(format!("{chat}'s stack: {note}"));
    }
    say(format!("{chat}'s stack starts: `{up}`…"));
    let _ = out.send(FromPlugin::Running {
        line: up.clone(),
        folder: stack.folder.clone(),
    });
    stack.start(&up).await.map_err(|error| error.to_string())?;
    // Read again: files of its own the start handed compose moved ports
    // of their own.
    reading.stack(folder).await
}

/// The last part of `folder`'s path: an iq worktree's is its chat's name.
fn leaf(folder: &Path) -> String {
    folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, BufReader};

    /// Speaks with the plugin: what iq says, then what the plugin said
    /// once iq's side closed.
    async fn spoken(said: &[ToPlugin], store: Store) -> Vec<FromPlugin> {
        let (mut iq, plugin) = tokio::io::duplex(1 << 16);
        let (from_plugin, to_iq) = tokio::io::duplex(1 << 16);
        let input: String = said
            .iter()
            .map(|message| iq_plugin::encode(message).unwrap())
            .collect();
        iq.write_all(input.as_bytes()).await.unwrap();
        drop(iq);
        speak(
            BufReader::new(plugin),
            from_plugin,
            store,
            Config::default(),
            None,
            None,
        )
        .await
        .unwrap();
        let mut text = String::new();
        let mut to_iq = to_iq;
        to_iq.read_to_string(&mut text).await.unwrap();
        text.lines()
            .map(|line| iq_plugin::decode(line).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn the_plugin_answers_with_its_commands_and_says_what_it_cannot_serve() {
        let state = tempfile::tempdir().unwrap();
        let plain = tempfile::tempdir().unwrap();
        let said = spoken(
            &[
                ToPlugin::Hello {
                    protocol: PROTOCOL,
                    workspace: "/w".into(),
                },
                // A place with no compose file is no stack: nothing said.
                ToPlugin::Placed {
                    chat: "notes".into(),
                    folder: plain.path().to_path_buf(),
                },
                ToPlugin::Command {
                    name: "serve".into(),
                    args: String::new(),
                    chat: None,
                    folder: None,
                },
                ToPlugin::Command {
                    name: "serve".into(),
                    args: "lead".into(),
                    chat: None,
                    folder: None,
                },
                ToPlugin::Command {
                    name: "fly".into(),
                    args: String::new(),
                    chat: None,
                    folder: None,
                },
                ToPlugin::Bye,
            ],
            Store::at(state.path()),
        )
        .await;
        let FromPlugin::Hello { protocol, commands } = &said[0] else {
            panic!("{said:?}");
        };
        assert_eq!(*protocol, PROTOCOL);
        let names: Vec<&str> = commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["serve", "unserve"]);
        let texts: Vec<&str> = said[1..]
            .iter()
            .map(|message| match message {
                FromPlugin::Say { text } => text.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(texts.len(), 3, "{texts:?}");
        assert!(texts[0].starts_with("/serve serves a chat"));
        assert!(texts[1].starts_with("lead works in your checkout"));
        assert_eq!(texts[2], "iq-ports has no /fly");
    }

    #[tokio::test]
    async fn as_it_opens_what_led_to_a_worktree_that_is_gone_is_forgotten() {
        let state = tempfile::tempdir().unwrap();
        let store = Store::at(state.path());
        let gone = PathBuf::from("/nowhere/shop/old");
        store
            .save(&Kept {
                places: std::collections::BTreeMap::from([(
                    gone.clone(),
                    crate::store::Place {
                        project: "shop-old".into(),
                        moved: Vec::new(),
                    },
                )]),
                served: Some(gone.clone()),
            })
            .unwrap();
        store
            .write_ports_file("shop-old", "services: {}\n")
            .unwrap();
        let said = spoken(
            &[ToPlugin::Hello {
                protocol: PROTOCOL,
                workspace: "/w".into(),
            }],
            store.clone(),
        )
        .await;
        assert!(
            said.iter().any(|m| matches!(m, FromPlugin::Say { text } if text.starts_with("/nowhere/shop/old is gone"))),
            "{said:?}"
        );
        assert_eq!(store.load(), Kept::default());
        assert!(!store.ports_file("shop-old").exists());
    }
}
