use super::*;
use iq_gate::{Approver, Gate, Judged};
use tokio::io::AsyncReadExt;

fn quiet() -> Tell {
    Arc::new(|_| {})
}

/// What `tell` was told, kept for a test to read.
fn heard() -> (Tell, Arc<Mutex<Vec<String>>>) {
    let kept = Arc::new(Mutex::new(Vec::new()));
    let keeping = Arc::clone(&kept);
    (
        Arc::new(move |said| keeping.lock().unwrap().push(said)),
        kept,
    )
}

/// A permit for `action`, the internet open and the person approving
/// what is asked.
fn permitted(action: Action) -> (tempfile::TempDir, Permit) {
    let dir = tempfile::tempdir().unwrap();
    let gate = Gate::builder(dir.path()).internet(true).build().unwrap();
    let permit = match gate.judge(action).unwrap() {
        Judged::Allowed(permit) => permit,
        Judged::Asked { pending, .. } => gate.approve(pending, Approver::Person).unwrap(),
        Judged::Denied { reason } => panic!("denied: {reason}"),
    };
    (dir, permit)
}

/// One request read whole: its first line, headers in lower case, body.
async fn read_request(socket: &mut tokio::net::TcpStream) -> (String, String, String) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
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
                let first = head.lines().next().unwrap_or("").to_owned();
                return (
                    first,
                    head.to_lowercase(),
                    body[..length.min(body.len())].to_owned(),
                );
            }
        }
        if n == 0 {
            return (String::new(), String::new(), String::new());
        }
    }
}

/// An MCP server over HTTP, as small as a test needs: it answers
/// `initialize`, `tools/list` and `tools/call`, the list as server-sent
/// events, and checks the session it gave and the version agreed on are
/// sent back.
async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let (first, head, body) = read_request(&mut socket).await;
                if first.starts_with("DELETE") {
                    let _ = socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                        .await;
                    return;
                }
                let message: Value = serde_json::from_str(&body).unwrap();
                let id = message["id"].clone();
                let (content_type, reply) = match message["method"].as_str().unwrap() {
                    "initialize" => (
                        "application/json",
                        json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": "2025-03-26"}})
                            .to_string(),
                    ),
                    "tools/list" => {
                        assert!(head.contains("mcp-session-id: s-1"), "{head}");
                        assert!(head.contains("mcp-protocol-version: 2025-03-26"), "{head}");
                        assert!(head.contains("authorization: bearer t"), "{head}");
                        let long = "x".repeat(70);
                        let list = json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [
                            {"name": "get.issue", "description": "An issue.",
                             "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": true}},
                            {"name": "create_issue", "inputSchema": {"type": "object"}},
                            {"name": long, "inputSchema": {"type": "object"}},
                        ]}});
                        ("text/event-stream", format!("event: message\ndata: {list}\n\n"))
                    }
                    "tools/call" => (
                        "application/json",
                        json!({"jsonrpc": "2.0", "id": id, "result": {"content": [
                            {"type": "text", "text": format!("{} {}", message["params"]["name"], message["params"]["arguments"]["id"])}
                        ]}})
                        .to_string(),
                    ),
                    _ => ("application/json", String::new()),
                };
                let status = if reply.is_empty() {
                    "202 Accepted"
                } else {
                    "200 OK"
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nMcp-Session-Id: s-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });
    url
}

#[tokio::test]
async fn a_servers_tools_are_offered_prefixed_judged_and_called_with_their_permit() {
    let url = serve().await;
    let (tell, told) = heard();
    let declared = BTreeMap::from([(
        "tracker".to_owned(),
        Declared::at(url).header("Authorization", "Bearer t"),
    )]);
    let mcp = Mcp::start(declared, None, tell);
    mcp.settled(Duration::from_secs(5)).await;
    let said = "2 tools, 1 only reading, 1 left out, their names too long for the providers";
    assert_eq!(mcp.status(), [("tracker".to_owned(), said.to_owned())]);
    let names: Vec<String> = mcp.specs().into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["tracker__get_issue", "tracker__create_issue"]);
    // The action names the tool as the server knows it, for the gate.
    let action = mcp
        .action("tracker__get_issue", &json!({"id": "ENG-1"}))
        .unwrap();
    assert_eq!(
        action,
        Action::Tool {
            server: "tracker".into(),
            tool: "get.issue".into(),
            arguments: r#"{"id":"ENG-1"}"#.into(),
            reads: true,
        }
    );
    let writes = mcp.action("tracker__create_issue", &Value::Null).unwrap();
    assert!(matches!(&writes, Action::Tool { reads: false, arguments, .. } if arguments == "{}"));
    assert!(matches!(
        mcp.action("other__x", &json!({})),
        Err(McpError::NoTool(_))
    ));
    let (_dir, permit) = permitted(action);
    assert_eq!(
        mcp.call(&permit).await.unwrap(),
        "\"get.issue\" \"ENG-1\"".to_owned()
    );
    // A permit for something else calls nothing.
    let (_other, read) = permitted(Action::Read {
        path: "notes.txt".into(),
    });
    assert!(matches!(mcp.call(&read).await, Err(McpError::NotATool)));
    assert_eq!(
        told.lock().unwrap().as_slice(),
        [format!("the MCP server tracker: {said}")]
    );
    mcp.end().await;
}

/// How many times a test's consent page was opened.
static CONSENTS: AtomicU64 = AtomicU64::new(0);

/// The person consenting at once: the browser goes back to the callback
/// with the PKCE challenge as its code, for the token endpoint to check
/// against the verifier. The page asks for the scope the refusal named.
fn consent_at_once(page: &str) -> bool {
    CONSENTS.fetch_add(1, Ordering::SeqCst);
    let page = reqwest::Url::parse(page).unwrap();
    let param = |name: &str| {
        page.query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
            .unwrap()
    };
    assert_eq!(param("scope"), "connect");
    let redirect = reqwest::Url::parse(&param("redirect_uri")).unwrap();
    let target = format!(
        "{}?code={}&state={}",
        redirect.path(),
        param("code_challenge"),
        param("state")
    );
    let address = format!("127.0.0.1:{}", redirect.port().unwrap());
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let mut socket = std::net::TcpStream::connect(address).unwrap();
        write!(socket, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        let mut answer = String::new();
        let _ = socket.read_to_string(&mut answer);
    });
    true
}

/// An MCP server that takes OAuth only, and its authorization server, at
/// one address: its refusal pointing to its metadata where nobody would
/// look for it, registration, the token endpoint, and the MCP endpoint,
/// which takes the tokens in `accepted`.
async fn serve_oauth(accepted: &'static [&'static str]) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let origin = base.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let base = origin.clone();
            tokio::spawn(async move {
                let (first, head, body) = read_request(&mut socket).await;
                let target = first.split(' ').nth(1).unwrap_or("").to_owned();
                let field = |name: &str| {
                    body.split('&')
                        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
                        .map(|v| v.replace("%3A", ":").replace("%2F", "/"))
                };
                let json =
                    |value: Value| (200, "application/json", value.to_string(), String::new());
                let (status, content_type, reply, extra) = match target.as_str() {
                    "/meta/resource" => json(json!({
                        "resource": format!("{base}/mcp"),
                        "authorization_servers": [base],
                        "scopes_supported": ["everything"],
                    })),
                    "/.well-known/oauth-authorization-server" => json(json!({
                        "issuer": base,
                        "authorization_endpoint": format!("{base}/authorize"),
                        "token_endpoint": format!("{base}/token"),
                        "registration_endpoint": format!("{base}/register"),
                        "code_challenge_methods_supported": ["S256"],
                        "token_endpoint_auth_methods_supported": ["none"],
                    })),
                    "/register" => {
                        let asked: Value = serde_json::from_str(&body).unwrap();
                        assert_eq!(asked["token_endpoint_auth_method"], "none");
                        assert_eq!(asked["client_name"], "iq");
                        json(json!({"client_id": "c-1"}))
                    }
                    "/token" => match field("grant_type").as_deref() {
                        Some("authorization_code") => {
                            let verifier = field("code_verifier").unwrap();
                            assert_eq!(field("code"), Some(oauth::challenge_of(&verifier)));
                            assert_eq!(field("client_id").as_deref(), Some("c-1"));
                            assert_eq!(field("resource"), Some(format!("{base}/mcp")));
                            json(
                                json!({"access_token": "t-1", "refresh_token": "r-1", "expires_in": 3600}),
                            )
                        }
                        Some("refresh_token") => {
                            assert_eq!(field("refresh_token").as_deref(), Some("r-old"));
                            json(json!({"access_token": "t-2", "expires_in": 3600}))
                        }
                        other => panic!("grant {other:?}"),
                    },
                    "/mcp" => {
                        let taken = accepted
                            .iter()
                            .any(|t| head.contains(&format!("authorization: bearer {t}")));
                        if taken {
                            let message: Value = serde_json::from_str(&body).unwrap();
                            let id = message["id"].clone();
                            match message["method"].as_str().unwrap() {
                                "initialize" => {
                                    json(json!({"jsonrpc": "2.0", "id": id, "result": {}}))
                                }
                                "tools/list" => {
                                    json(json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [
                                        {"name": "fetch", "inputSchema": {"type": "object"},
                                         "annotations": {"readOnlyHint": true}}
                                    ]}}))
                                }
                                "tools/call" => {
                                    json(json!({"jsonrpc": "2.0", "id": id, "result": {
                                        "content": [{"type": "text", "text": "a page"}]
                                    }}))
                                }
                                _ => (202, "application/json", String::new(), String::new()),
                            }
                        } else {
                            (
                                401,
                                "application/json",
                                String::new(),
                                format!(
                                    "WWW-Authenticate: Bearer resource_metadata=\"{base}/meta/resource\", scope=\"connect\"\r\n"
                                ),
                            )
                        }
                    }
                    _ => (404, "text/plain", String::new(), String::new()),
                };
                let reason = match status {
                    200 => "OK",
                    202 => "Accepted",
                    401 => "Unauthorized",
                    _ => "Not Found",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });
    base
}

/// Waits until the server is ready or failed, past the consent that
/// `settled` does not wait for.
async fn ready(mcp: &Mcp) -> String {
    for _ in 0..500 {
        let how = mcp.status()[0].1.clone();
        if !how.starts_with("waiting") && how != "connecting" {
            return how;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    mcp.status()[0].1.clone()
}

fn oauth_server(url: String) -> BTreeMap<String, Declared> {
    BTreeMap::from([("docs".to_owned(), Declared::at(url))])
}

#[tokio::test]
async fn a_server_that_takes_oauth_only_gets_a_grant_kept_renewed_and_asked_again() {
    let dir = tempfile::tempdir().unwrap();
    let grants = dir.path().join("mcp-auth");
    let url = |base: &str| format!("{base}/mcp");

    // No grant yet: the person consents once, and the grant is kept.
    let base = serve_oauth(&["t-1", "t-2"]).await;
    let before = CONSENTS.load(Ordering::SeqCst);
    let (tell, told) = heard();
    let mcp = Mcp::start_with(
        oauth_server(url(&base)),
        Some(grants.clone()),
        tell,
        consent_at_once,
    );
    assert_eq!(ready(&mcp).await, "1 tool, 1 only reading");
    assert_eq!(CONSENTS.load(Ordering::SeqCst), before + 1);
    // Said once, for the person to open the page if no browser did.
    for _ in 0..100 {
        if told.lock().unwrap().len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let told = told.lock().unwrap().clone();
    assert_eq!(told.len(), 2, "{told:?}");
    assert!(told[0].starts_with("the MCP server docs waits for your consent"));
    assert_eq!(told[1], "the MCP server docs: 1 tool, 1 only reading");
    let (_gate, permit) = permitted(mcp.action("docs__fetch", &json!({})).unwrap());
    assert_eq!(mcp.call(&permit).await.unwrap(), "a page");
    let kept = Grants::new(grants.clone())
        .load("docs", &url(&base))
        .unwrap();
    assert_eq!(kept.access_token, "t-1");

    // A grant about to end is renewed, without asking.
    let ending = oauth::Grant::for_test(&base, "t-old", "r-old", oauth::now() + 10);
    Grants::new(grants.clone()).save("docs", &ending).unwrap();
    let before = CONSENTS.load(Ordering::SeqCst);
    let mcp = Mcp::start_with(
        oauth_server(url(&base)),
        Some(grants.clone()),
        quiet(),
        consent_at_once,
    );
    assert_eq!(ready(&mcp).await, "1 tool, 1 only reading");
    assert_eq!(CONSENTS.load(Ordering::SeqCst), before);
    let kept = Grants::new(grants.clone())
        .load("docs", &url(&base))
        .unwrap();
    assert_eq!(kept.access_token, "t-2");

    // A grant the server no longer takes is forgotten, and the person
    // asked once more.
    let refused = oauth::Grant::for_test(&base, "t-revoked", "r-x", oauth::now() + 3600);
    Grants::new(grants.clone()).save("docs", &refused).unwrap();
    let before = CONSENTS.load(Ordering::SeqCst);
    let mcp = Mcp::start_with(
        oauth_server(url(&base)),
        Some(grants.clone()),
        quiet(),
        consent_at_once,
    );
    assert_eq!(ready(&mcp).await, "1 tool, 1 only reading");
    assert_eq!(CONSENTS.load(Ordering::SeqCst), before + 1);
    let kept = Grants::new(grants).load("docs", &url(&base)).unwrap();
    assert_eq!(kept.access_token, "t-1");
}

#[tokio::test]
async fn a_server_given_its_authorization_is_never_asked_for_oauth() {
    let dir = tempfile::tempdir().unwrap();
    let base = serve_oauth(&["t-1"]).await;
    let before = CONSENTS.load(Ordering::SeqCst);
    let declared = BTreeMap::from([(
        "docs".to_owned(),
        Declared::at(format!("{base}/mcp")).header("authorization", "Bearer wrong"),
    )]);
    let mcp = Mcp::start_with(
        declared,
        Some(dir.path().join("mcp-auth")),
        quiet(),
        consent_at_once,
    );
    mcp.settled(Duration::from_secs(5)).await;
    assert!(
        mcp.status()[0].1.starts_with("failed: ") && mcp.status()[0].1.contains("HTTP 401"),
        "{:?}",
        mcp.status()
    );
    assert_eq!(CONSENTS.load(Ordering::SeqCst), before);
}

/// A server over stdio, in the shell: it answers by the method each line
/// names, with the id it was asked with, writes a log and asks for a ping
/// and for what it does not know before the list, keeps what it is sent in
/// `seen`, and ends after a call.
#[cfg(unix)]
fn stdio_server(seen: &std::path::Path) -> Vec<String> {
    let script = r#"while IFS= read -r line; do
  printf '%s\n' "$line" >> 'SEEN'
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*) echo '{"jsonrpc":"2.0","id":'"$id"',"result":{"protocolVersion":"2025-06-18"}}' ;;
    *'"tools/list"'*)
      echo 'starting up' >&2
      echo '{"jsonrpc":"2.0","method":"notifications/message","params":{"data":"listing"}}'
      echo '{"jsonrpc":"2.0","id":"p-1","method":"ping"}'
      echo '{"jsonrpc":"2.0","id":"r-1","method":"roots/list"}'
      echo '{"jsonrpc":"2.0","id":'"$id"',"result":{"tools":[{"name":"note","inputSchema":{"type":"object"}}]}}' ;;
    *'"tools/call"'*) echo '{"jsonrpc":"2.0","id":'"$id"',"result":{"content":[{"type":"text","text":"noted"}]}}'; exit 0 ;;
  esac
done"#;
    vec![
        "sh".into(),
        "-c".into(),
        script.replace("SEEN", &seen.display().to_string()),
    ]
}

#[cfg(unix)]
#[tokio::test]
async fn a_server_over_stdio_is_answered_its_own_requests_and_called() {
    let dir = tempfile::tempdir().unwrap();
    let seen = dir.path().join("seen");
    let declared = BTreeMap::from([("notes".to_owned(), Declared::run(stdio_server(&seen)))]);
    let mcp = Mcp::start(declared, None, quiet());
    mcp.settled(Duration::from_secs(5)).await;
    assert_eq!(mcp.status()[0].1, "1 tool, 0 only reading");
    let (_gate, permit) = permitted(mcp.action("notes__note", &json!({"text": "hi"})).unwrap());
    assert_eq!(mcp.call(&permit).await.unwrap(), "noted");
    let sent: Vec<Value> = std::fs::read_to_string(&seen)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let answered = |id: &str| sent.iter().find(|m| m["id"] == id).cloned().unwrap();
    assert_eq!(answered("p-1")["result"], json!({}));
    assert_eq!(answered("r-1")["error"]["code"], -32601);
    // It ended after the call: the next one says so, with what it wrote
    // of its errors, and it is started again for the one after.
    let failed = mcp.call(&permit).await.unwrap_err();
    assert_eq!(said(&failed), "the server stopped, saying: starting up");
    assert_eq!(mcp.status()[0].1, "1 tool, 0 only reading");
    assert_eq!(mcp.call(&permit).await.unwrap(), "noted");
    mcp.end().await;
}

#[tokio::test]
async fn a_server_that_cannot_start_says_why() {
    let declared = BTreeMap::from([(
        "broken".to_owned(),
        Declared::run(vec!["/no/such/program".into()]),
    )]);
    let mcp = Mcp::start(declared, None, quiet());
    mcp.settled(Duration::from_secs(5)).await;
    let status = mcp.status();
    assert!(
        status[0]
            .1
            .starts_with("failed: /no/such/program could not be started: "),
        "{status:?}"
    );
    assert!(mcp.specs().is_empty());
}

#[test]
fn events_are_read_from_their_data_lines_and_names_made_plain() {
    let body = "event: message\ndata: {\"a\":1}\n\ndata: line one\ndata: line two\n\n";
    assert_eq!(sse_data(body), ["{\"a\":1}", "line one\nline two"]);
    assert_eq!(plain("my.server"), "my_server");
}

#[test]
fn an_answer_too_long_is_cut_and_one_that_failed_is_an_error() {
    let long = "a".repeat(ANSWER_CHARS + 10);
    let text = call_text(&json!({"content": [{"type": "text", "text": long}]})).unwrap();
    assert!(text.starts_with(&"a".repeat(ANSWER_CHARS)));
    assert!(
        text.ends_with("ask it for less]"),
        "{}",
        &text[ANSWER_CHARS..]
    );
    let failed = call_text(
        &json!({"content": [{"type": "text", "text": "no such issue"}],
                                   "isError": true}),
    );
    assert!(matches!(failed, Err(McpError::Failed(said)) if said == "no such issue"));
    let structured = call_text(&json!({"structuredContent": {"n": 1}})).unwrap();
    assert_eq!(structured, r#"{"n":1}"#);
}

#[test]
fn a_declared_server_never_shows_its_secrets() {
    let declared = Declared::at("https://mcp.example.com/mcp")
        .header("Authorization", "Bearer secret-token")
        .env([("TOKEN".into(), "secret-env".into())]);
    let shown = format!("{declared:?}");
    assert!(
        !shown.contains("secret-token") && !shown.contains("secret-env"),
        "{shown}"
    );
    assert!(shown.contains("Authorization"), "{shown}");
}
