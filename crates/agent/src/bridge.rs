//! The bridge: how iq's program, run by an agent with tools of its own,
//! speaks to the running iq, over a socket of iq's folder that only its
//! owner reaches. One request, one answer, as lines of JSON: a tool call
//! for the gate to judge (Claude Code's hook), a tool of the board's (the
//! MCP server), a command the gate allowed, to run in its sandbox.
//!
//! The bridge acts only through the crew, so through the gate: a process
//! that reaches it can ask what an agent may ask, never more.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use iq_core::{ChatModel, Delegate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

use crate::crew::{AGENT_VAR, BRIDGE_VAR, Bridge, Crew};
use crate::tools::Args;

/// The longest request read: a hook's input holds a whole file written.
const LONGEST: u64 = 16 << 20;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Request {
    /// One of Claude Code's tool calls, as its hook is given it.
    Hook { agent: String, input: Value },
    /// One of the board's tools.
    Tool {
        agent: String,
        name: String,
        arguments: Value,
    },
    /// The command the gate allowed under `id`.
    Exec { id: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Reply {
    pub(crate) text: String,
    pub(crate) ok: bool,
}

/// A name no one guesses: 128 bits from the system's generator.
pub(crate) fn token() -> String {
    let mut bytes = [0u8; 16];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    if read.is_err() {
        // Never reached on a Unix system; the time still differs per call.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        bytes = nanos.to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl<M, D> Crew<M, D>
where
    M: ChatModel + 'static,
    D: Delegate + 'static,
{
    /// Listens for iq's program on a new socket in `folder`, a folder of
    /// iq's own that only its owner reaches, and has the agents with tools
    /// of their own reach the crew through it, `program` being iq's.
    /// Sockets left by an iq that is gone are removed.
    pub fn serve(&self, program: PathBuf, folder: PathBuf) -> std::io::Result<JoinHandle<()>> {
        std::fs::create_dir_all(&folder)?;
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700))?;
        remove_stale(&folder);
        let socket = folder.join(format!("{}.sock", token()));
        let listener = tokio::net::UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        self.reach_through(Bridge {
            program,
            socket,
            folder,
        });
        let crew = self.clone();
        Ok(tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let crew = crew.clone();
                tokio::spawn(async move { crew.answer_on(stream).await });
            }
        }))
    }

    /// Stops listening for iq's program: its socket goes.
    pub fn close(&self) {
        if let Some(bridge) = self.take_bridge() {
            let _ = std::fs::remove_file(bridge.socket);
        }
    }

    /// Ends the crew: every agent stopped, the bridge closed, and what
    /// their commands left running stopped with the sandbox.
    pub async fn end(&self) {
        self.stop_all();
        self.close();
        self.sandbox().end().await;
    }

    async fn answer_on(&self, stream: tokio::net::UnixStream) {
        let (read, mut write) = stream.into_split();
        let mut line = String::new();
        let mut reader = tokio::io::BufReader::new(tokio::io::AsyncReadExt::take(read, LONGEST));
        if reader.read_line(&mut line).await.is_err() {
            return;
        }
        let reply = match serde_json::from_str::<Request>(&line) {
            Ok(request) => self.answer_request(request).await,
            Err(error) => Reply {
                text: format!("iq could not read the request: {error}"),
                ok: false,
            },
        };
        if let Ok(mut text) = serde_json::to_string(&reply) {
            text.push('\n');
            let _ = write.write_all(text.as_bytes()).await;
        }
    }

    async fn answer_request(&self, request: Request) -> Reply {
        match request {
            Request::Hook { agent, input } => Reply {
                text: self.hook(&agent, &input).await,
                ok: true,
            },
            Request::Tool {
                agent,
                name,
                arguments,
            } => {
                let result = match Args::from_value(arguments) {
                    Ok(args) => self.board_tool(&agent, &name, &args).await,
                    Err(error) => Err(error),
                };
                match result {
                    Ok(text) => Reply { text, ok: true },
                    Err(text) => Reply { text, ok: false },
                }
            }
            Request::Exec { id } => match self.exec(&id).await {
                Ok((text, ok)) => Reply { text, ok },
                Err(text) => Reply { text, ok: false },
            },
        }
    }
}

/// Removes the sockets of `folder` nobody listens on any more.
fn remove_stale(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "sock")
            && std::os::unix::net::UnixStream::connect(&path).is_err()
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Sends `request` to the iq the environment names, and waits for its
/// answer.
pub(crate) fn ask(request: &Request) -> std::io::Result<Reply> {
    let socket = std::env::var_os(BRIDGE_VAR)
        .ok_or_else(|| std::io::Error::other(format!("{BRIDGE_VAR} is not set")))?;
    ask_at(Path::new(&socket), request)
}

/// Sends `request` to the iq listening on `socket`, and waits for its
/// answer.
pub(crate) fn ask_at(socket: &Path, request: &Request) -> std::io::Result<Reply> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket)?;
    let mut line = serde_json::to_string(request).map_err(std::io::Error::other)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut answer = String::new();
    BufReader::new(stream.take(LONGEST)).read_line(&mut answer)?;
    serde_json::from_str(&answer).map_err(std::io::Error::other)
}

/// The agent iq's program speaks for, as the environment names it.
pub(crate) fn agent() -> String {
    std::env::var(AGENT_VAR).unwrap_or_else(|_| "agent".to_owned())
}

/// iq's program as Claude Code's hook: judges the tool call on its input
/// and prints the answer. Fails closed: when iq cannot be reached, the
/// call is refused. Returns the exit status.
pub fn hook_main() -> i32 {
    let mut input = String::new();
    let answer = match std::io::stdin().take(LONGEST).read_to_string(&mut input) {
        Ok(_) => match serde_json::from_str::<Value>(&input) {
            Ok(input) => match ask(&Request::Hook {
                agent: agent(),
                input,
            }) {
                Ok(reply) => reply.text,
                Err(error) => {
                    crate::hook::deny(&format!("iq is not there to judge it ({error})"), None)
                }
            },
            Err(error) => {
                crate::hook::deny(&format!("the hook's input is not JSON ({error})"), None)
            }
        },
        Err(error) => crate::hook::deny(
            &format!("the hook's input could not be read ({error})"),
            None,
        ),
    };
    println!("{answer}");
    0
}

/// iq's program run in a command's stead: has iq run the command it
/// allowed under `id`, prints what it printed, and returns its status.
pub fn exec_main(id: Option<std::ffi::OsString>) -> i32 {
    let Some(id) = id.and_then(|id| id.into_string().ok()) else {
        eprintln!("iq: no command was named");
        return 2;
    };
    match ask(&Request::Exec { id }) {
        Ok(reply) => {
            println!("{}", reply.text);
            if reply.ok { 0 } else { 1 }
        }
        Err(error) => {
            eprintln!("iq is not there to run it: {error}");
            1
        }
    }
}
