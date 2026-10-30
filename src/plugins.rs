//! The person's plugins: the programs their settings list, `iq-<name>` on
//! the `PATH`, started as iq opens and spoken to over their standard input
//! and output, as `iq-plugin` writes it. Each is told where the chats work
//! and what the person asks of it; what it says goes to the board, and the
//! variables it gives a place to the gate, which gives them to that
//! place's commands. A plugin that stops takes what it showed and gave
//! with it, and the person is told why; iq goes on without it.
//!
//! A plugin is the person's program, run on the machine as they would run
//! it, with their environment but iq's own variables: only their settings
//! start one, and only they speak to it. Its start, and what it says it
//! runs, are written to the audit log beside the gate's actions.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iq_gate::Gate;
use iq_plugin::{FLAG, FromPlugin, PROTOCOL, ToPlugin};
use iq_ui::Update;
use iq_ui::plugins::{PluginCommand, Told};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

/// How long a plugin has to answer iq's hello.
const GREETING: Duration = Duration::from_secs(10);

/// How long a plugin told to end has to end before it is stopped.
const LEAVING: Duration = Duration::from_secs(3);

/// The last lines of its errors kept, to say why a plugin stopped.
const TOLD_LINES: usize = 5;

/// One plugin started.
struct Started {
    name: String,
    send: mpsc::UnboundedSender<ToPlugin>,
    /// Told once it ended, taken by the first to wait for it.
    ended: Mutex<Option<oneshot::Receiver<()>>>,
}

/// The plugins started.
pub(crate) struct Plugins {
    started: Vec<Started>,
}

impl Plugins {
    /// Starts the plugins `names` for the person's checkout `workspace`,
    /// each found on the `PATH` as `search` lists it, what they say told to
    /// the board through `tell` and the variables they give to `gate`. A
    /// plugin that cannot start is said and left. Must run within the
    /// runtime.
    pub(crate) fn start(
        names: &[String],
        workspace: &Path,
        search: Option<&std::ffi::OsStr>,
        gate: &Gate,
        tell: &std::sync::mpsc::Sender<Update>,
    ) -> Self {
        let mut started = Vec::new();
        for name in names {
            match start_one(name, workspace, search, gate, tell) {
                Ok(one) => started.push(one),
                Err(why) => {
                    let _ = tell.send(Update::Said(format!("the plugin {name} {why}")));
                }
            }
        }
        Self { started }
    }

    /// Tells every plugin that `chat` works in `folder`, a place of its
    /// own.
    pub(crate) fn placed(&self, chat: &str, folder: &Path) {
        for one in &self.started {
            let _ = one.send.send(ToPlugin::Placed {
                chat: chat.to_owned(),
                folder: folder.to_path_buf(),
            });
        }
    }

    /// Hands the person's command to the plugin `plugin`.
    pub(crate) fn command(
        &self,
        plugin: &str,
        name: String,
        args: String,
        chat: Option<String>,
        folder: Option<PathBuf>,
    ) {
        if let Some(one) = self.started.iter().find(|one| one.name == plugin) {
            let _ = one.send.send(ToPlugin::Command {
                name,
                args,
                chat,
                folder,
            });
        }
    }

    /// Tells every plugin iq ends, and waits for each a moment: one that
    /// does not end is stopped.
    pub(crate) async fn end(&self) {
        for one in &self.started {
            let _ = one.send.send(ToPlugin::Bye);
        }
        for one in &self.started {
            let ended = one.ended.lock().ok().and_then(|mut ended| ended.take());
            if let Some(ended) = ended {
                let _ = tokio::time::timeout(LEAVING + Duration::from_secs(1), ended).await;
            }
        }
    }
}

/// The program of the plugin `name` on the `PATH` as `search` lists it.
fn found(name: &str, search: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let program = iq_plugin::program(name);
    std::env::split_paths(search?)
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join(&program))
        .find(|path| {
            std::fs::metadata(path)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

/// Starts the plugin `name`, or says why it is not.
fn start_one(
    name: &str,
    workspace: &Path,
    search: Option<&std::ffi::OsStr>,
    gate: &Gate,
    tell: &std::sync::mpsc::Sender<Update>,
) -> Result<Started, String> {
    if !iq_plugin::valid_name(name) {
        return Err(
            "is not started: a plugin's name is lowercase letters, digits and hyphens".to_owned(),
        );
    }
    let program = found(name, search).ok_or_else(|| {
        format!(
            "is not started: {} is not on the PATH. In iq's repository, `cargo install --path \
             plugins/{name}` installs it.",
            iq_plugin::program(name)
        )
    })?;
    let by = format!("plugin {name}");
    gate.outside(&by, &format!("{} {FLAG}", program.display()), workspace)
        .map_err(|error| format!("is not started: {error}"))?;
    let mut child = tokio::process::Command::new(&program)
        .arg(FLAG)
        .current_dir(workspace)
        .env_clear()
        .envs(crate::persons_env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("could not start {}: {error}", program.display()))?;
    let (send, asked) = mpsc::unbounded_channel();
    let _ = send.send(ToPlugin::Hello {
        protocol: PROTOCOL,
        workspace: workspace.to_path_buf(),
    });
    let (done, ended) = oneshot::channel();
    let errors = Arc::new(Mutex::new(VecDeque::new()));
    let reading = child
        .stderr
        .take()
        .map(|stderr| tokio::spawn(read_errors(name.to_owned(), stderr, Arc::clone(&errors))));
    let speaking = Speaking {
        name: name.to_owned(),
        gate: gate.clone(),
        tell: tell.clone(),
        errors,
        reading,
        given: Vec::new(),
    };
    tokio::spawn(async move {
        speaking.speak(child, asked).await;
        let _ = done.send(());
    });
    Ok(Started {
        name: name.to_owned(),
        send,
        ended: Mutex::new(Some(ended)),
    })
}

/// What a plugin writes on its standard error, to iq's log, its last lines
/// kept.
async fn read_errors(
    name: String,
    stderr: tokio::process::ChildStderr,
    kept: Arc<Mutex<VecDeque<String>>>,
) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::warn!("plugin {name}: {line}");
        if let Ok(mut kept) = kept.lock() {
            kept.push_back(line);
            while kept.len() > TOLD_LINES {
                kept.pop_front();
            }
        }
    }
}

/// One plugin spoken to.
struct Speaking {
    name: String,
    gate: Gate,
    tell: std::sync::mpsc::Sender<Update>,
    /// The last lines of its errors.
    errors: Arc<Mutex<VecDeque<String>>>,
    /// What reads them, waited for a moment once it ended, for its last
    /// words.
    reading: Option<tokio::task::JoinHandle<()>>,
    /// The places it gave variables to, taken back when it stops.
    given: Vec<PathBuf>,
}

impl Speaking {
    /// Speaks with the plugin until it ends, or iq does.
    async fn speak(
        mut self,
        mut child: tokio::process::Child,
        mut asked: mpsc::UnboundedReceiver<ToPlugin>,
    ) {
        let mut stdin = child.stdin.take();
        let Some(stdout) = child.stdout.take() else {
            return;
        };
        let mut said = BufReader::new(stdout).lines();
        let mut greeted = false;
        let mut leaving = false;
        let greeting = tokio::time::sleep(GREETING);
        tokio::pin!(greeting);
        let why = loop {
            tokio::select! {
                message = asked.recv(), if stdin.is_some() => {
                    let bye = matches!(message, None | Some(ToPlugin::Bye));
                    if let (Some(message), Some(input)) = (message, stdin.as_mut())
                        && let Ok(line) = iq_plugin::encode(&message)
                        && (input.write_all(line.as_bytes()).await.is_err()
                            || input.flush().await.is_err())
                    {
                        break None;
                    }
                    if bye {
                        // Its input closed: it ends, as it was told.
                        leaving = true;
                        stdin = None;
                    }
                }
                line = said.next_line() => match line {
                    Ok(Some(line)) => {
                        if let Some(why) = self.take(&line, &mut greeted) {
                            break Some(why);
                        }
                    }
                    _ => break None,
                },
                () = &mut greeting, if !greeted => {
                    break Some(format!("did not answer iq within {}s", GREETING.as_secs()));
                }
            }
        };
        drop(stdin);
        if why.is_some() {
            // Stopped by iq: no reason to wait for it.
            let _ = child.start_kill();
        }
        let status = match tokio::time::timeout(LEAVING, child.wait()).await {
            Ok(status) => status.ok(),
            Err(_) => {
                let _ = child.kill().await;
                None
            }
        };
        // What it said last on its way out, unless what it left running
        // holds its errors open.
        if let Some(reading) = self.reading.take() {
            let _ = tokio::time::timeout(Duration::from_millis(500), reading).await;
        }
        for folder in std::mem::take(&mut self.given) {
            let _ = self.gate.give(&folder, Default::default(), &self.by());
        }
        let _ = self.tell.send(Update::Plugin {
            plugin: self.name.clone(),
            told: Told::Gone,
        });
        if leaving && why.is_none() {
            return;
        }
        let mut text = format!("the plugin {} stopped", self.name);
        match (why, status) {
            (Some(why), _) => text.push_str(&format!(": it {why}")),
            (None, Some(status)) => text.push_str(&format!(" ({status})")),
            (None, None) => {}
        }
        let errors: Vec<String> = self
            .errors
            .lock()
            .map(|kept| kept.iter().cloned().collect())
            .unwrap_or_default();
        if !errors.is_empty() {
            text.push_str(&format!(", saying: {}", errors.join(" / ")));
        }
        text.push_str(". iq goes on without it.");
        let _ = self.tell.send(Update::Said(text));
    }

    /// Who the audit log says gave or ran what.
    fn by(&self) -> String {
        format!("plugin {}", self.name)
    }

    /// Takes a line the plugin said; why it is stopped, when it must be.
    fn take(&mut self, line: &str, greeted: &mut bool) -> Option<String> {
        let message: FromPlugin = match iq_plugin::decode(line) {
            Ok(message) => message,
            Err(error) => {
                tracing::warn!("plugin {}: {error}", self.name);
                return None;
            }
        };
        let told = |told: Told| Update::Plugin {
            plugin: self.name.clone(),
            told,
        };
        let update = match message {
            FromPlugin::Hello { protocol, commands } => {
                if protocol != PROTOCOL {
                    return Some(format!(
                        "speaks the protocol {protocol}, iq {PROTOCOL}: install both from the \
                         same commit"
                    ));
                }
                *greeted = true;
                told(Told::Commands(
                    commands
                        .into_iter()
                        .map(|spec| PluginCommand {
                            name: spec.name,
                            args: spec.args,
                            what: spec.what,
                        })
                        .collect(),
                ))
            }
            _ if !*greeted => return Some("spoke before answering iq's hello".to_owned()),
            FromPlugin::Status { text } => told(Told::Status(text)),
            FromPlugin::Lines { chat, lines } => told(Told::Lines { chat, lines }),
            FromPlugin::Say { text } => Update::Said(format!("{}: {text}", self.name)),
            FromPlugin::Variables {
                folder,
                vars,
                enter,
            } => match self.gate.give(&folder, vars, &self.by()) {
                Ok(()) => {
                    if !self.given.contains(&folder) {
                        self.given.push(folder.clone());
                    }
                    told(Told::Enter {
                        folder,
                        line: enter,
                    })
                }
                Err(error) => Update::Said(format!("{}: {error}", self.name)),
            },
            FromPlugin::Running { line, folder } => {
                if let Err(error) = self.gate.outside(&self.by(), &line, &folder) {
                    tracing::warn!("plugin {}: {error}", self.name);
                }
                return None;
            }
        };
        let _ = self.tell.send(update);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plugin written as a shell script in `folder`: it answers iq's
    /// hello with a command, then says what each line it reads was, and
    /// gives the place it is told of a variable.
    fn script(folder: &Path, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = folder.join(iq_plugin::program(name));
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    const ECHO: &str = r#"read hello
echo '{"kind":"hello","protocol":1,"commands":[{"name":"ping","args":"","what":"answers"}]}'
echo 'not a message'
while read line; do
  case "$line" in
    *'"placed"'*)
      folder=$(echo "$line" | sed 's/.*"folder":"\([^"]*\)".*/\1/')
      echo "{\"kind\":\"variables\",\"folder\":\"$folder\",\"vars\":{\"COMPOSE_PROJECT_NAME\":\"shop-lead\"},\"enter\":\"eval x\"}"
      echo "{\"kind\":\"variables\",\"folder\":\"$folder\",\"vars\":{\"PATH\":\"/x\"}}" ;;
    *'"command"'*) echo '{"kind":"say","text":"pong"}'; echo '{"kind":"status","text":"up"}' ;;
    *'"bye"'*) exit 0 ;;
  esac
done
"#;

    fn updates(
        feed: &std::sync::mpsc::Receiver<Update>,
        until: impl Fn(&Update) -> bool,
    ) -> Vec<Update> {
        let mut got = Vec::new();
        while let Ok(update) = feed.recv_timeout(Duration::from_secs(10)) {
            let last = until(&update);
            got.push(update);
            if last {
                break;
            }
        }
        got
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_plugin_is_greeted_told_and_heard_and_ends_with_iq() {
        let bin = tempfile::tempdir().unwrap();
        let place = tempfile::tempdir().unwrap();
        let audit = tempfile::tempdir().unwrap();
        script(bin.path(), "echo", ECHO);
        let gate = Gate::builder(place.path())
            .audit(audit.path().join("audit.log"))
            .build()
            .unwrap();
        let (tell, feed) = std::sync::mpsc::channel();
        let plugins = Plugins::start(
            &["echo".to_owned(), "absent".to_owned(), "../x".to_owned()],
            place.path(),
            Some(bin.path().as_os_str()),
            &gate,
            &tell,
        );
        let said = updates(&feed, |u| {
            matches!(
                u,
                Update::Plugin {
                    told: Told::Commands(_),
                    ..
                }
            )
        });
        let texts: Vec<&str> = said
            .iter()
            .filter_map(|u| match u {
                Update::Said(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            texts
                .iter()
                .any(|t| t
                    .starts_with("the plugin absent is not started: iq-absent is not on the PATH")),
            "{texts:?}"
        );
        assert!(
            texts
                .iter()
                .any(|t| t.starts_with("the plugin ../x is not started")),
            "{texts:?}"
        );
        let Some(Update::Plugin {
            told: Told::Commands(commands),
            ..
        }) = said.last()
        else {
            panic!("{said:?}");
        };
        assert_eq!(commands[0].name, "ping");
        // The place it gives variables to is told on the board and given
        // to the gate; what the sandbox rests on is refused and said.
        let folder = place.path().to_path_buf();
        plugins.placed("lead", &folder);
        let said = updates(&feed, |u| matches!(u, Update::Said(_)));
        assert!(
            said.iter().any(|u| matches!(
                u,
                Update::Plugin {
                    told: Told::Enter { folder: given, line: Some(line) },
                    ..
                } if *given == folder && line == "eval x"
            )),
            "{said:?}"
        );
        let Some(Update::Said(refused)) = said.last() else {
            panic!("{said:?}");
        };
        assert!(
            refused.starts_with("echo: PATH may not be given"),
            "{refused}"
        );
        plugins.command(
            "echo",
            "ping".into(),
            String::new(),
            Some("lead".into()),
            None,
        );
        let said = updates(&feed, |u| {
            matches!(
                u,
                Update::Plugin {
                    told: Told::Status(_),
                    ..
                }
            )
        });
        assert!(
            said.iter()
                .any(|u| matches!(u, Update::Said(text) if text == "echo: pong")),
            "{said:?}"
        );
        // Ended with iq, it takes what it showed with it, and is not said
        // to have stopped.
        plugins.end().await;
        let said = updates(&feed, |u| {
            matches!(
                u,
                Update::Plugin {
                    told: Told::Gone,
                    ..
                }
            )
        });
        assert!(
            matches!(
                said.last(),
                Some(Update::Plugin {
                    told: Told::Gone,
                    ..
                })
            ),
            "{said:?}"
        );
        assert!(feed.recv_timeout(Duration::from_millis(300)).is_err());
        let log = std::fs::read_to_string(audit.path().join("audit.log")).unwrap();
        assert!(log.contains(r#""by":"plugin echo""#), "{log}");
        assert!(log.contains("--iq-plugin"), "{log}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_plugin_that_stops_or_speaks_another_protocol_is_said_and_iq_goes_on() {
        let bin = tempfile::tempdir().unwrap();
        let place = tempfile::tempdir().unwrap();
        script(
            bin.path(),
            "old",
            "read hello\necho '{\"kind\":\"hello\",\"protocol\":0}'\nexec sleep 30\n",
        );
        script(
            bin.path(),
            "crash",
            "read hello\necho 'it broke' >&2\nexit 3\n",
        );
        let gate = Gate::builder(place.path()).build().unwrap();
        let (tell, feed) = std::sync::mpsc::channel();
        let plugins = Plugins::start(
            &["old".to_owned(), "crash".to_owned()],
            place.path(),
            Some(bin.path().as_os_str()),
            &gate,
            &tell,
        );
        let mut texts = Vec::new();
        while texts.len() < 2 {
            match feed.recv_timeout(Duration::from_secs(10)) {
                Ok(Update::Said(text)) => texts.push(text),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        texts.sort();
        assert_eq!(texts.len(), 2, "{texts:?}");
        assert!(
            texts[0].starts_with("the plugin crash stopped"),
            "{texts:?}"
        );
        assert!(texts[0].contains("it broke"), "{texts:?}");
        assert!(
            texts[1].starts_with("the plugin old stopped: it speaks the protocol 0, iq 1"),
            "{texts:?}"
        );
        plugins.end().await;
    }
}
