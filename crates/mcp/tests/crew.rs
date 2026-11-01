//! A server's tools as the crew's agents call them: offered with the
//! crew's own, each call judged by the gate, answered by the reviewer or
//! the person, then made. The model is scripted and the server a stand-in
//! on localhost.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iq_agent::{Crew, Event, PERSON, Settings};
use iq_core::{
    ChatModel, ChatRequest, ChatResponse, Message, ModelId, TokenCount, ToolCall, Usage,
};
use iq_gate::{Gate, Helper};
use iq_mcp::{Declared, Mcp};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Answers each agent from its own script, the agent known by the name its
/// instructions give it, and keeps every request.
#[derive(Default)]
struct Scripted {
    scripts: Mutex<HashMap<String, VecDeque<ChatResponse>>>,
    requests: Mutex<Vec<(String, ChatRequest)>>,
}

impl Scripted {
    fn seen_by(&self, agent: &str) -> String {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == agent)
            .map(|(_, request)| format!("{:?}", request.messages))
            .collect()
    }

    fn first_of(&self, agent: &str) -> ChatRequest {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .find(|(name, _)| name == agent)
            .map(|(_, request)| request.clone())
            .unwrap()
    }
}

#[derive(Debug)]
struct Never;

impl std::fmt::Display for Never {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("never")
    }
}

impl std::error::Error for Never {}

#[derive(Clone)]
struct Model(Arc<Scripted>);

impl ChatModel for Model {
    type Error = Never;

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, Never> {
        let name = match request.messages.first() {
            Some(Message::System(text)) => text
                .split_once("you are listed as ")
                .and_then(|(_, rest)| rest.split(':').next())
                .unwrap_or("reviewer")
                .to_owned(),
            _ => "?".to_owned(),
        };
        self.0
            .requests
            .lock()
            .unwrap()
            .push((name.clone(), request.clone()));
        let next = self
            .0
            .scripts
            .lock()
            .unwrap()
            .get_mut(&name)
            .and_then(VecDeque::pop_front);
        Ok(next.unwrap_or_else(|| say("nothing more")))
    }
}

fn say(text: &str) -> ChatResponse {
    ChatResponse {
        content: Some(text.to_owned()),
        tool_calls: Vec::new(),
        usage: Usage {
            input: TokenCount(100),
            output: TokenCount(10),
        },
        cost: None,
        cache: None,
    }
}

fn calls(tool: &str, arguments: Value) -> ChatResponse {
    ChatResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call-{tool}"),
            name: tool.to_owned(),
            arguments: arguments.to_string(),
        }],
        ..say("")
    }
}

/// A server with a tool that reads an issue and one that closes it, which
/// counts the calls it is made.
async fn serve(made: Arc<Mutex<Vec<String>>>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let made = Arc::clone(&made);
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 4096];
                let body = loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    buffer.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buffer).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length: usize = head
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse().unwrap())
                            })
                            .unwrap_or(0);
                        if body.len() >= length || n == 0 {
                            break body[..length.min(body.len())].to_owned();
                        }
                    }
                };
                let message: Value = serde_json::from_str(&body).unwrap_or_default();
                let id = message["id"].clone();
                let result = match message["method"].as_str() {
                    Some("initialize") => Some(json!({"protocolVersion": "2025-06-18"})),
                    Some("tools/list") => Some(json!({"tools": [
                        {"name": "get_issue", "description": "Reads an issue.",
                         "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": true}},
                        {"name": "close_issue", "description": "Closes an issue.",
                         "inputSchema": {"type": "object"}},
                    ]})),
                    Some("tools/call") => {
                        let name = message["params"]["name"].as_str().unwrap().to_owned();
                        made.lock().unwrap().push(name.clone());
                        let issue = &message["params"]["arguments"]["id"];
                        Some(
                            json!({"content": [{"type": "text", "text": format!("{name} {issue}")}]}),
                        )
                    }
                    _ => None,
                };
                let reply = result
                    .map(|result| json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string())
                    .unwrap_or_default();
                let status = if reply.is_empty() {
                    "202 Accepted"
                } else {
                    "200 OK"
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });
    url
}

#[tokio::test]
async fn an_agent_reads_through_the_reviewer_and_changes_only_once_the_person_says() {
    let made = Arc::new(Mutex::new(Vec::new()));
    let url = serve(Arc::clone(&made)).await;
    let mcp = Mcp::start(
        BTreeMap::from([("tracker".to_owned(), Declared::at(url))]),
        None,
        Arc::new(|_| {}),
    );
    mcp.settled(Duration::from_secs(5)).await;

    let dir = tempfile::tempdir().unwrap();
    let gate = Gate::builder(dir.path().canonicalize().unwrap())
        .build()
        .unwrap();
    let model = Arc::new(Scripted::default());
    *model.scripts.lock().unwrap() = HashMap::from([
        (
            "lead".to_owned(),
            VecDeque::from([
                calls("tracker__get_issue", json!({"id": "A-1"})),
                calls("tracker__close_issue", json!({"id": "A-1"})),
                say("Closed."),
            ]),
        ),
        (
            "reviewer".to_owned(),
            VecDeque::from([say("APPROVE: the task reads the issue.")]),
        ),
    ]);
    let events = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&events);
    let crew = Crew::new(
        Model(Arc::clone(&model)),
        gate,
        Helper::new("/nonexistent", Vec::<String>::new()),
        Settings {
            reviewer: Some(ModelId::new("cheap/reviewer").unwrap()),
            ..Settings::default()
        },
        Arc::new(move |event| kept.lock().unwrap().push(event)),
    );
    crew.extend(Arc::new(mcp.clone()));
    crew.set_autopilot(true);
    crew.set_autopilot_internet(true);
    crew.start(
        "lead",
        &ModelId::new("cheap/model").unwrap(),
        "close issue A-1".into(),
        PERSON,
    );

    // The read, the internet closed, goes to the reviewer, who may give
    // what only reaches the internet; the change waits for the person.
    let asked = loop {
        let found = events.lock().unwrap().iter().find_map(|event| match event {
            Event::Asked { id, action, .. } => Some((*id, action.clone())),
            _ => None,
        });
        if let Some(found) = found {
            break found;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(made.lock().unwrap().as_slice(), ["get_issue"]);
    assert!(
        asked
            .1
            .starts_with(r#"call close_issue of the server tracker, with {"id":"A-1"}"#),
        "{}",
        asked.1
    );
    crew.answer(asked.0, true);
    tokio::time::timeout(Duration::from_secs(10), crew.settled())
        .await
        .unwrap();
    assert_eq!(
        made.lock().unwrap().as_slice(),
        ["get_issue", "close_issue"]
    );

    let reviewer = model.seen_by("reviewer");
    assert!(
        reviewer.contains("call get_issue of the server tracker, which it says only reads"),
        "{reviewer}"
    );
    assert!(
        reviewer.contains("on a server the person connected"),
        "{reviewer}"
    );
    assert!(!reviewer.contains("close_issue"), "{reviewer}");
    let lead = model.seen_by("lead");
    assert!(lead.contains(r#"get_issue \"A-1\""#), "{lead}");
    assert!(lead.contains(r#"close_issue \"A-1\""#), "{lead}");
    // The lead was offered the server's tools beside the crew's, and told
    // how they are judged.
    let first = model.first_of("lead");
    let offered: Vec<&str> = first.tools.iter().map(|t| t.name.as_str()).collect();
    assert!(offered.contains(&"tracker__get_issue") && offered.contains(&"tracker__close_issue"));
    assert!(lead.contains("Tools named `<server>__<tool>`"), "{lead}");
    mcp.end().await;
}
