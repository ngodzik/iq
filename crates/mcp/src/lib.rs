//! A client of the Model Context Protocol: the servers the person's
//! settings declare, by an address or a command, each started on a task of
//! its own, and their tools offered to the crew's agents as
//! `<server>__<tool>`.
//!
//! Servers are reached two ways: streamable HTTP, a JSON-RPC message per
//! POST answered as JSON or as server-sent events, with the headers the
//! settings give or OAuth when the server asks for it; and stdio, a
//! JSON-RPC message per line, the program run on the machine as the
//! person's own. Nothing waits on a server that does not answer: its tools
//! are offered once it does, and the person is told how each answered.
//!
//! Every call goes through the gate as [`iq_gate::Action::Tool`], and is
//! made only with the permit it gives: a tool its server says only reads is
//! judged as reaching the internet, any other waits for the person.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod oauth;

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use iq_agent::{Called, Crew, Extension};
use iq_core::{ChatModel, ToolSpec};
use iq_gate::{Action, Permit};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};

pub use error::{AuthError, McpError};

use error::said;
use oauth::{Challenge, Grants};

/// The protocol's version this client speaks.
const PROTOCOL: &str = "2025-06-18";

/// How long a server may take to answer one message.
const ANSWER_WITHIN: Duration = Duration::from_secs(60);

/// How long a tool may take to answer its call.
const CALL_WITHIN: Duration = Duration::from_secs(300);

/// How long a program has to end once its input is closed, before it is
/// killed.
const END_WITHIN: Duration = Duration::from_secs(2);

/// Between a server's name and its tool's.
const SEPARATOR: &str = "__";

/// The longest name of a tool providers take.
const NAME_MOST: usize = 64;

/// The most characters of a tool's answer given to the model: as much as a
/// command's output, so that one page fetched does not fill its context.
const ANSWER_CHARS: usize = 30_000;

/// How many of the last lines a program wrote of its errors are said when
/// it stops.
const ERROR_LINES: usize = 5;

/// What the agents are told of the servers' tools.
const INSTRUCTIONS: &str = "Tools named `<server>__<tool>` are those of the servers the person \
connected (MCP servers). One its server says only reads runs at once while the internet is open, \
and is asked as the internet is otherwise; any other waits for the person, since it may change \
what the server keeps, which others may share.";

/// What the person is told of a server, as it happens.
pub type Tell = Arc<dyn Fn(String) + Send + Sync>;

/// How a server is reached.
#[derive(Clone)]
enum Reach {
    Url(String),
    Command(Vec<String>),
}

/// A server as the settings declare it, the secrets its headers and its
/// environment hold read from the person's variables.
#[derive(Clone)]
pub struct Declared {
    reach: Reach,
    headers: BTreeMap<String, String>,
    env: Option<Vec<(OsString, OsString)>>,
}

/// Written by hand: the headers' values and the environment never show.
impl fmt::Debug for Declared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut shown = f.debug_struct("Declared");
        match &self.reach {
            Reach::Url(url) => shown.field("url", url),
            Reach::Command(command) => shown.field("command", command),
        };
        shown
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .field("env", &self.env.as_ref().map(Vec::len))
            .finish()
    }
}

impl Declared {
    /// A server over streamable HTTP at `url`.
    #[must_use]
    pub fn at(url: impl Into<String>) -> Self {
        Self {
            reach: Reach::Url(url.into()),
            headers: BTreeMap::new(),
            env: None,
        }
    }

    /// A server over stdio: `command`'s first word is the program, the
    /// others its arguments.
    #[must_use]
    pub fn run(command: Vec<String>) -> Self {
        Self {
            reach: Reach::Command(command),
            headers: BTreeMap::new(),
            env: None,
        }
    }

    /// Sends `name: value` with every request, for a server over HTTP. A
    /// server given its `Authorization` is never asked for OAuth.
    #[must_use]
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Runs the program of a server over stdio with `vars` alone as its
    /// environment, rather than iq's.
    #[must_use]
    pub fn env(mut self, vars: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        self.env = Some(vars.into_iter().collect());
        self
    }

    /// The address of a server over HTTP.
    fn url(&self) -> Option<&str> {
        match &self.reach {
            Reach::Url(url) => Some(url),
            Reach::Command(_) => None,
        }
    }

    /// Whether the settings give it what authorizes it.
    fn authorized(&self) -> bool {
        self.headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization"))
    }
}

/// A tool a server offers.
#[derive(Debug, Clone, PartialEq)]
struct RemoteTool {
    /// Its name, as the server knows it.
    name: String,
    /// Its name as offered to models: the server's, then its own, as
    /// providers take names.
    offered: String,
    description: String,
    schema: Value,
    read_only: bool,
}

/// Where a server is.
#[derive(Debug, Clone, PartialEq)]
enum State {
    Connecting,
    /// Waiting for the person's consent, on this page.
    Consenting(String),
    /// Connected, with its tools, and how many it offers that were left out
    /// for names too long for providers.
    Ready(Vec<RemoteTool>, usize),
    Failed(String),
}

/// What reaches a server over HTTP. A clone sends as the original does,
/// so that a call need not hold the others back.
#[derive(Clone)]
struct Http {
    client: reqwest::Client,
    url: String,
    headers: BTreeMap<String, String>,
    /// The OAuth token sent, when the server takes one.
    bearer: Option<String>,
    session: Option<String>,
    /// The protocol's version agreed on, sent once it is.
    protocol: Option<String>,
    /// What its last refusal said, when it refused what was sent.
    challenge: Option<Challenge>,
}

impl Http {
    /// Sends `message` and, for a request, returns the answer of `id`.
    async fn post(&mut self, message: &Value, id: Option<u64>) -> Result<Option<Value>, McpError> {
        let mut post = self
            .client
            .post(self.url.as_str())
            .header("Accept", "application/json, text/event-stream")
            .json(message);
        for (name, value) in &self.headers {
            post = post.header(name.as_str(), value.as_str());
        }
        if let Some(token) = self.bearer.as_deref() {
            post = post.bearer_auth(token);
        }
        if let Some(session) = self.session.as_deref() {
            post = post.header("Mcp-Session-Id", session);
        }
        if let Some(protocol) = self.protocol.as_deref() {
            post = post.header("MCP-Protocol-Version", protocol);
        }
        let response = post.send().await.map_err(|source| McpError::Http {
            url: self.url.clone(),
            source,
        })?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        if let Some(given) = header("Mcp-Session-Id") {
            self.session = Some(given);
        }
        let status = response.status();
        self.challenge = (status == reqwest::StatusCode::UNAUTHORIZED)
            .then(|| Challenge::read(&header("WWW-Authenticate").unwrap_or_default()));
        let events = header("Content-Type").is_some_and(|t| t.starts_with("text/event-stream"));
        let body = response.text().await.map_err(|source| McpError::Http {
            url: self.url.clone(),
            source,
        })?;
        if !status.is_success() {
            return Err(McpError::Status {
                url: self.url.clone(),
                status: status.as_u16(),
                said: first_line(&body),
            });
        }
        let Some(id) = id else {
            return Ok(None);
        };
        let candidates: Vec<Value> = if events {
            sse_data(&body)
                .iter()
                .filter_map(|data| serde_json::from_str(data).ok())
                .collect()
        } else {
            serde_json::from_str(&body).ok().into_iter().collect()
        };
        Ok(candidates.into_iter().find(|a| answers(a, id)))
    }

    /// Ends its session, when it has one, as the protocol asks a client to.
    async fn end(&self) {
        let Some(session) = &self.session else {
            return;
        };
        let mut delete = self
            .client
            .delete(self.url.as_str())
            .header("Mcp-Session-Id", session.as_str());
        for (name, value) in &self.headers {
            delete = delete.header(name.as_str(), value.as_str());
        }
        if let Some(token) = self.bearer.as_deref() {
            delete = delete.bearer_auth(token);
        }
        let _ = tokio::time::timeout(END_WITHIN, delete.send()).await;
    }
}

/// What reaches a server over stdio: its program, kept so that it is
/// stopped with the link.
struct Stdio {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    /// The last lines it wrote of its errors.
    errors: Arc<Mutex<VecDeque<String>>>,
}

impl Stdio {
    async fn write(&mut self, message: &Value) -> Result<(), McpError> {
        let mut text = message.to_string();
        text.push('\n');
        let written = match self.stdin.write_all(text.as_bytes()).await {
            Ok(()) => self.stdin.flush().await,
            Err(error) => Err(error),
        };
        written.map_err(|error| {
            // Its input closed: the program ended.
            if error.kind() == std::io::ErrorKind::BrokenPipe {
                McpError::Stopped {
                    said: last_errors(&self.errors),
                }
            } else {
                McpError::Write(error)
            }
        })
    }

    /// Sends `message` and, for a request, reads until the answer of `id`.
    /// What else it says meanwhile is passed over, but its own requests,
    /// which are answered: a ping as it asks, any other as unknown.
    async fn exchange(
        &mut self,
        message: &Value,
        id: Option<u64>,
    ) -> Result<Option<Value>, McpError> {
        self.write(message).await?;
        let Some(id) = id else {
            return Ok(None);
        };
        loop {
            let Some(line) = self.lines.next_line().await.map_err(McpError::Read)? else {
                return Err(McpError::Stopped {
                    said: last_errors(&self.errors),
                });
            };
            let Ok(heard) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if answers(&heard, id) {
                return Ok(Some(heard));
            }
            let asked = heard.get("result").is_none() && heard.get("error").is_none();
            if let (Some(method), Some(their), true) =
                (heard["method"].as_str(), heard.get("id"), asked)
            {
                let reply = if method == "ping" {
                    json!({"jsonrpc": "2.0", "id": their, "result": {}})
                } else {
                    json!({"jsonrpc": "2.0", "id": their,
                           "error": {"code": -32601, "message": "Method not found"}})
                };
                self.write(&reply).await?;
            }
        }
    }

    /// Closes its input, as the protocol ends a program, and kills it when
    /// it does not end by itself.
    async fn end(self) {
        let Self {
            mut child, stdin, ..
        } = self;
        drop(stdin);
        if tokio::time::timeout(END_WITHIN, child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
        }
    }
}

/// The way to a server.
enum Link {
    Stdio(Box<Stdio>),
    Http(Http),
}

impl Link {
    /// Starts the program or prepares the HTTP client.
    fn open(config: &Declared) -> Result<Self, McpError> {
        let command = match &config.reach {
            Reach::Url(url) => {
                return Ok(Self::Http(Http {
                    client: reqwest::Client::new(),
                    url: url.clone(),
                    headers: config.headers.clone(),
                    bearer: None,
                    session: None,
                    protocol: None,
                    challenge: None,
                }));
            }
            Reach::Command(command) => command,
        };
        let (program, args) = command.split_first().ok_or(McpError::Unreachable)?;
        let mut run = tokio::process::Command::new(program);
        run.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        if let Some(env) = &config.env {
            run.env_clear().envs(env.iter().cloned());
        }
        let mut child = run.spawn().map_err(|source| McpError::Start {
            program: program.clone(),
            source,
        })?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(McpError::Stopped {
                said: String::new(),
            });
        };
        let errors = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(stderr) = child.stderr.take() {
            let kept = Arc::clone(&errors);
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
                    if kept.len() == ERROR_LINES {
                        kept.pop_front();
                    }
                    kept.push_back(line);
                }
            });
        }
        Ok(Self::Stdio(Box::new(Stdio {
            child,
            stdin,
            lines: BufReader::new(stdout).lines(),
            errors,
        })))
    }

    fn set_bearer(&mut self, token: String) {
        if let Self::Http(http) = self {
            http.bearer = Some(token);
        }
    }

    /// What the server's last refusal said, when it refused what was sent.
    fn challenge(&self) -> Option<Challenge> {
        match self {
            Self::Http(http) => http.challenge.clone(),
            Self::Stdio(_) => None,
        }
    }

    /// Whether the server gave it a session.
    fn in_session(&self) -> bool {
        matches!(
            self,
            Self::Http(Http {
                session: Some(_),
                ..
            })
        )
    }

    /// Sends a request and waits `within` at most for its answer's result.
    async fn request(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
        within: Duration,
    ) -> Result<Value, McpError> {
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let exchange = async {
            match self {
                Self::Stdio(stdio) => stdio.exchange(&message, Some(id)).await,
                Self::Http(http) => http.post(&message, Some(id)).await,
            }
        };
        let answer =
            tokio::time::timeout(within, exchange)
                .await
                .map_err(|_| McpError::Timeout {
                    method: method.to_owned(),
                    seconds: within.as_secs(),
                })??;
        result_of(method, answer)
    }

    /// Sends a notification, which has no answer.
    async fn notify(&mut self, method: &str) -> Result<(), McpError> {
        let message = json!({"jsonrpc": "2.0", "method": method});
        let exchange = async {
            match self {
                Self::Stdio(stdio) => stdio.exchange(&message, None).await,
                Self::Http(http) => http.post(&message, None).await,
            }
        };
        tokio::time::timeout(ANSWER_WITHIN, exchange)
            .await
            .map_err(|_| McpError::Timeout {
                method: method.to_owned(),
                seconds: ANSWER_WITHIN.as_secs(),
            })?
            .map(|_| ())
    }

    async fn end(self) {
        match self {
            Self::Stdio(stdio) => stdio.end().await,
            Self::Http(http) => http.end().await,
        }
    }
}

/// Whether `message` answers the request `id`.
fn answers(message: &Value, id: u64) -> bool {
    message["id"].as_u64() == Some(id)
        && (message.get("result").is_some() || message.get("error").is_some())
}

/// The result of the answer to `method`, or its error said.
fn result_of(method: &str, answer: Option<Value>) -> Result<Value, McpError> {
    let answer = answer.ok_or_else(|| McpError::NoAnswer {
        method: method.to_owned(),
    })?;
    if let Some(error) = answer.get("error") {
        return Err(McpError::Refused {
            method: method.to_owned(),
            message: error["message"]
                .as_str()
                .map_or_else(|| error.to_string(), str::to_owned),
        });
    }
    Ok(answer["result"].clone())
}

/// The data of each server-sent event of `body`, its lines joined.
fn sse_data(body: &str) -> Vec<String> {
    let mut events = Vec::new();
    let mut data: Vec<&str> = Vec::new();
    for line in body.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if !data.is_empty() {
                events.push(data.join("\n"));
                data.clear();
            }
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    events
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

/// The last lines a program wrote of its errors, as said after why it
/// stopped.
fn last_errors(errors: &Mutex<VecDeque<String>>) -> String {
    let errors = errors.lock().unwrap_or_else(PoisonError::into_inner);
    if errors.is_empty() {
        return String::new();
    }
    let lines: Vec<&str> = errors.iter().map(String::as_str).collect();
    format!(", saying: {}", lines.join(" / "))
}

/// A tool call's result as text: its text parts, what else it holds named;
/// as much as a model is given of it.
fn call_text(result: &Value) -> Result<String, McpError> {
    let parts: Vec<String> = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|part| match part["type"].as_str() {
            Some("text") => part["text"].as_str().unwrap_or("").to_owned(),
            Some("resource") => part["resource"]["text"]
                .as_str()
                .map_or_else(|| "[a resource]".to_owned(), str::to_owned),
            Some(other) => format!("[{other}]"),
            None => String::new(),
        })
        .collect();
    let mut text = parts.join("\n");
    if text.is_empty()
        && let Some(structured) = result.get("structuredContent")
    {
        text = structured.to_string();
    }
    let total = text.chars().count();
    if total > ANSWER_CHARS {
        text = text.chars().take(ANSWER_CHARS).collect();
        text.push_str(&format!(
            "\n[cut: the tool answered {total} characters, of which these are the first \
             {ANSWER_CHARS}; ask it for less]"
        ));
    }
    if result["isError"].as_bool() == Some(true) {
        Err(McpError::Failed(text))
    } else {
        Ok(text)
    }
}

/// A name as model providers take it: letters, digits, `_` and `-`.
fn plain(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// One server declared in the settings.
struct Server {
    name: String,
    config: Declared,
    state: Mutex<State>,
    link: tokio::sync::Mutex<Option<Link>>,
    next_id: AtomicU64,
    /// Where its OAuth grant is kept, when it takes OAuth.
    grants: Option<Grants>,
    /// What opens the consent page: the browser, but in tests.
    open: fn(&str) -> bool,
    tell: Tell,
}

impl Server {
    fn set(&self, state: State) {
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = state;
    }

    fn state(&self) -> State {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Whether it is asked for OAuth when it refuses: a server over HTTP
    /// that the settings give no authorization, with a folder to keep its
    /// grant in.
    fn takes_oauth(&self) -> bool {
        self.config.url().is_some() && !self.config.authorized() && self.grants.is_some()
    }

    /// How it is, in words.
    fn said(&self) -> String {
        match self.state() {
            State::Connecting => "connecting".to_owned(),
            State::Consenting(page) => {
                format!("waiting for your consent in the browser; if it did not open: {page}")
            }
            State::Ready(tools, left_out) => {
                let read = tools.iter().filter(|t| t.read_only).count();
                let mut said = format!(
                    "{} tool{}, {read} only reading",
                    tools.len(),
                    if tools.len() == 1 { "" } else { "s" }
                );
                if left_out > 0 {
                    said.push_str(&format!(
                        ", {left_out} left out, their names too long for the providers"
                    ));
                }
                said
            }
            State::Failed(why) => format!("failed: {why}"),
        }
    }

    /// Opens the link, says hello, and lists the tools; the link it had is
    /// ended.
    async fn connect(&self) {
        self.set(State::Connecting);
        match self.connected().await {
            Ok((link, tools, left_out)) => {
                let before = self.link.lock().await.replace(link);
                if let Some(before) = before {
                    before.end().await;
                }
                self.set(State::Ready(tools, left_out));
            }
            Err(why) => self.set(State::Failed(said(&why))),
        }
    }

    /// A link said hello to, and the tools it lists. A server that refuses
    /// is asked for OAuth when it takes it: the grant kept, else the
    /// person's consent, once more when it no longer takes the one kept.
    async fn connected(&self) -> Result<(Link, Vec<RemoteTool>, usize), McpError> {
        let mut link = Link::open(&self.config)?;
        let oauth = self.takes_oauth();
        let mut sent = false;
        if oauth && let Some(token) = self.kept_token().await {
            link.set_bearer(token);
            sent = true;
        }
        let (tools, left_out) = match self.hello(&mut link).await {
            Err(McpError::Status { status: 401, .. }) if oauth => {
                if sent && let Some(grants) = &self.grants {
                    grants.forget(&self.name);
                }
                let challenge = link.challenge().unwrap_or_default();
                link.set_bearer(self.consented(&challenge).await?);
                self.hello(&mut link).await?
            }
            other => other?,
        };
        Ok((link, tools, left_out))
    }

    /// The token of the grant kept for its address, renewed when it is
    /// about to end; none when there is none or it cannot be renewed.
    async fn kept_token(&self) -> Option<String> {
        let grants = self.grants.as_ref()?;
        let grant = grants.load(&self.name, self.config.url()?)?;
        if !grant.is_ending(oauth::now()) {
            return Some(grant.access_token);
        }
        let renewed = oauth::renew(&reqwest::Client::new(), &grant).await.ok()?;
        grants.save(&self.name, &renewed).ok()?;
        Some(renewed.access_token)
    }

    /// The person's consent, asked in the browser, its grant kept.
    async fn consented(&self, challenge: &Challenge) -> Result<String, McpError> {
        let grants = self.grants.as_ref().ok_or(AuthError::NoFolder)?;
        let url = self.config.url().ok_or(McpError::Unreachable)?;
        let grant = oauth::consent(
            &reqwest::Client::new(),
            url,
            challenge,
            |page| {
                self.set(State::Consenting(page.to_owned()));
                (self.tell)(format!(
                    "the MCP server {} waits for your consent in the browser; if it did not \
                     open: {page}",
                    self.name
                ));
            },
            self.open,
        )
        .await?;
        self.set(State::Connecting);
        grants.save(&self.name, &grant)?;
        Ok(grant.access_token)
    }

    /// Says hello on `link` and lists the tools, those whose names are too
    /// long for providers counted apart.
    async fn hello(&self, link: &mut Link) -> Result<(Vec<RemoteTool>, usize), McpError> {
        let hello = link
            .request(
                self.id(),
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL,
                    "capabilities": {},
                    "clientInfo": {"name": "iq", "version": env!("CARGO_PKG_VERSION")},
                }),
                ANSWER_WITHIN,
            )
            .await?;
        if let Link::Http(http) = link {
            http.protocol = Some(
                hello["protocolVersion"]
                    .as_str()
                    .unwrap_or(PROTOCOL)
                    .to_owned(),
            );
        }
        link.notify("notifications/initialized").await?;
        let mut tools: Vec<RemoteTool> = Vec::new();
        let mut left_out = 0;
        let mut cursor: Option<String> = None;
        loop {
            let params = cursor.map_or_else(|| json!({}), |c| json!({"cursor": c}));
            let page = link
                .request(self.id(), "tools/list", params, ANSWER_WITHIN)
                .await?;
            for tool in page["tools"].as_array().into_iter().flatten() {
                let Some(name) = tool["name"].as_str() else {
                    continue;
                };
                let offered = format!("{}{SEPARATOR}{}", self.name, plain(name));
                if offered.len() > NAME_MOST {
                    left_out += 1;
                    continue;
                }
                if tools.iter().any(|t| t.offered == offered) {
                    continue;
                }
                tools.push(RemoteTool {
                    name: name.to_owned(),
                    offered,
                    description: tool["description"].as_str().unwrap_or("").to_owned(),
                    schema: tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object"})),
                    read_only: tool["annotations"]["readOnlyHint"].as_bool() == Some(true),
                });
            }
            cursor = page["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        Ok((tools, left_out))
    }

    /// Calls `tool` with `arguments`. A call the server refused before
    /// taking it (its grant no longer taken, its session gone) connects
    /// again and is made once more; a program that stopped is started
    /// again for the next call.
    async fn call(&self, tool: &str, arguments: &Value) -> Result<String, McpError> {
        let first = self.call_once(tool, arguments).await;
        let again = match &first {
            Err(McpError::Status { status: 401, .. }) => self.takes_oauth(),
            // A session the server no longer knows (its restart, its
            // timeout) is answered as not found.
            Err(McpError::Status { status: 404, .. }) => self
                .link
                .lock()
                .await
                .as_ref()
                .is_some_and(Link::in_session),
            Err(McpError::Stopped { .. }) => {
                self.connect().await;
                false
            }
            _ => false,
        };
        if !again {
            return first;
        }
        self.connect().await;
        self.call_once(tool, arguments).await
    }

    async fn call_once(&self, tool: &str, arguments: &Value) -> Result<String, McpError> {
        let mut link = self.link.lock().await;
        let open = link
            .as_mut()
            .ok_or_else(|| McpError::NotConnected(self.name.clone()))?;
        if self.takes_oauth()
            && let Some(token) = self.renewed().await
        {
            open.set_bearer(token);
        }
        let id = self.id();
        let params = json!({"name": tool, "arguments": arguments});
        let result = match open {
            // Sent by a copy, so that another call need not wait for this
            // one: only a refusal or a session gone matters of what it
            // hears, and either connects again.
            Link::Http(http) => {
                let mut http = http.clone();
                drop(link);
                let message =
                    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": params});
                let answer = tokio::time::timeout(CALL_WITHIN, http.post(&message, Some(id)))
                    .await
                    .map_err(|_| McpError::Timeout {
                        method: "tools/call".to_owned(),
                        seconds: CALL_WITHIN.as_secs(),
                    })?;
                result_of("tools/call", answer?)?
            }
            Link::Stdio(_) => open.request(id, "tools/call", params, CALL_WITHIN).await?,
        };
        call_text(&result)
    }

    /// The token renewed, when the grant kept is about to end.
    async fn renewed(&self) -> Option<String> {
        let grants = self.grants.as_ref()?;
        let grant = grants.load(&self.name, self.config.url()?)?;
        if !grant.is_ending(oauth::now()) {
            return None;
        }
        let renewed = oauth::renew(&reqwest::Client::new(), &grant).await.ok()?;
        grants.save(&self.name, &renewed).ok()?;
        Some(renewed.access_token)
    }

    /// Ends its link.
    async fn end(&self) {
        if let Some(link) = self.link.lock().await.take() {
            link.end().await;
        }
    }
}

/// The servers the settings declare, and their tools, offered to every
/// agent of the crew.
#[derive(Clone)]
pub struct Mcp {
    servers: Arc<Vec<Arc<Server>>>,
}

impl fmt::Debug for Mcp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.servers.iter().map(|s| (&s.name, &s.config)))
            .finish()
    }
}

impl Mcp {
    /// Starts every server of `declared`, each on a task of its own: their
    /// tools are offered as each answers, and `tell` is told how each did,
    /// and when one waits for the person's consent. The OAuth grants of
    /// those that ask for it are kept in `grants`, one file each, readable
    /// by their owner only; without it, none is asked. Needs a Tokio
    /// runtime.
    #[must_use]
    pub fn start(
        declared: BTreeMap<String, Declared>,
        grants: Option<PathBuf>,
        tell: Tell,
    ) -> Self {
        Self::start_with(declared, grants, tell, oauth::open_browser)
    }

    /// As [`Mcp::start`], the consent page opened by `open`.
    fn start_with(
        declared: BTreeMap<String, Declared>,
        grants: Option<PathBuf>,
        tell: Tell,
        open: fn(&str) -> bool,
    ) -> Self {
        let servers: Vec<Arc<Server>> = declared
            .into_iter()
            .map(|(name, config)| {
                Arc::new(Server {
                    name: plain(&name),
                    config,
                    state: Mutex::new(State::Connecting),
                    link: tokio::sync::Mutex::new(None),
                    next_id: AtomicU64::new(1),
                    grants: grants.clone().map(Grants::new),
                    open,
                    tell: Arc::clone(&tell),
                })
            })
            .collect();
        for server in &servers {
            let server = Arc::clone(server);
            tokio::spawn(async move {
                server.connect().await;
                (server.tell)(format!("the MCP server {}: {}", server.name, server.said()));
            });
        }
        Self {
            servers: Arc::new(servers),
        }
    }

    /// Whether the settings declare any server.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    /// Waits until every server answered or failed, `within` at most; one
    /// waiting for the person's consent is not waited for.
    pub async fn settled(&self, within: Duration) {
        let _ = tokio::time::timeout(within, async {
            while self.servers.iter().any(|s| s.state() == State::Connecting) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
    }

    /// How each server is, by its name.
    #[must_use]
    pub fn status(&self) -> Vec<(String, String)> {
        self.servers
            .iter()
            .map(|server| (server.name.clone(), server.said()))
            .collect()
    }

    /// The tools of the servers connected, as models are offered them.
    #[must_use]
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.servers
            .iter()
            .flat_map(|server| match server.state() {
                State::Ready(tools, _) => tools
                    .into_iter()
                    .map(|tool| ToolSpec {
                        name: tool.offered,
                        description: if tool.description.is_empty() {
                            format!("A tool of the MCP server {}.", server.name)
                        } else {
                            tool.description
                        },
                        parameters: tool.schema,
                    })
                    .collect(),
                _ => Vec::new(),
            })
            .collect()
    }

    fn find(&self, offered: &str) -> Option<(&Arc<Server>, RemoteTool)> {
        self.servers.iter().find_map(|server| match server.state() {
            State::Ready(tools, _) => tools
                .into_iter()
                .find(|tool| tool.offered == offered)
                .map(|tool| (server, tool)),
            _ => None,
        })
    }

    /// The action of calling the tool offered as `offered` with
    /// `arguments`, for the gate to judge.
    ///
    /// # Errors
    ///
    /// No server connected offers it.
    pub fn action(&self, offered: &str, arguments: &Value) -> Result<Action, McpError> {
        let (server, tool) = self
            .find(offered)
            .ok_or_else(|| McpError::NoTool(offered.to_owned()))?;
        let arguments = if arguments.is_null() {
            json!({})
        } else {
            arguments.clone()
        };
        Ok(Action::Tool {
            server: server.name.clone(),
            tool: tool.name,
            arguments: arguments.to_string(),
            reads: tool.read_only,
        })
    }

    /// Makes the call `permit` allows, with the arguments it was judged
    /// with, and returns what the tool answered.
    ///
    /// # Errors
    ///
    /// The permit is not for a server's tool, its server is not connected,
    /// cannot be reached or refuses, or the tool failed.
    pub async fn call(&self, permit: &Permit) -> Result<String, McpError> {
        let Action::Tool {
            server,
            tool,
            arguments,
            ..
        } = permit.action()
        else {
            return Err(McpError::NotATool);
        };
        let found = self
            .servers
            .iter()
            .find(|s| &s.name == server)
            .ok_or_else(|| McpError::NotConnected(server.clone()))?;
        let arguments: Value = serde_json::from_str(arguments).map_err(McpError::Arguments)?;
        found.call(tool, &arguments).await
    }

    /// Ends every server's link: a program's input closed, then the
    /// program killed if it does not end; a session over HTTP ended.
    pub async fn end(&self) {
        for server in self.servers.iter() {
            server.end().await;
        }
    }
}

impl<M> Extension<M> for Mcp
where
    M: ChatModel + 'static,
{
    fn tools(&self, _agent: &str) -> Vec<ToolSpec> {
        self.specs()
    }

    fn instructions(&self, _agent: &str) -> Option<String> {
        (!self.is_empty()).then(|| INSTRUCTIONS.to_owned())
    }

    fn call(&self, crew: Crew<M>, agent: String, name: String, args: Value) -> Called {
        let mcp = self.clone();
        Box::pin(async move {
            let action = mcp.action(&name, &args).map_err(|e| said(&e))?;
            let permit = crew.act(&agent, action).await?;
            mcp.call(&permit).await.map_err(|e| said(&e))
        })
    }
}

#[cfg(test)]
mod tests;
