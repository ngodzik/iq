//! iq's program as an MCP server, over its input and output: the board's
//! tools offered to an agent with tools of its own, each call passed to
//! the running iq through the bridge.
//!
//! Only what the board needs of the protocol is spoken: starting, the
//! list of tools, calling one. A request for anything else is answered
//! that the method is not there; notifications are read and let be.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::bridge::{Request, agent, ask};

/// The protocol's version answered when the client names none.
const VERSION: &str = "2025-06-18";

/// Serves until the input ends. Returns the exit status.
pub fn mcp_main() -> i32 {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let agent = agent();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            return 1;
        };
        let Some(answer) = answer(&line, &agent, &mut |request| ask(request)) else {
            continue;
        };
        if writeln!(stdout, "{answer}")
            .and_then(|()| stdout.flush())
            .is_err()
        {
            return 1;
        }
    }
    0
}

/// The answer to one line of the client's, if it asks for one.
fn answer(
    line: &str,
    agent: &str,
    bridge: &mut dyn FnMut(&Request) -> std::io::Result<crate::bridge::Reply>,
) -> Option<Value> {
    let message: Value = serde_json::from_str(line).ok()?;
    // A notification, or an answer to a request of ours: there are none.
    let id = message.get("id").cloned()?;
    let method = message["method"].as_str()?;
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": message["params"]["protocolVersion"].as_str().unwrap_or(VERSION),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "iq", "version": env!("CARGO_PKG_VERSION")},
        })),
        "ping" => Ok(json!({})),
        "tools/list" => {
            // The running iq says which tools this agent has; without it,
            // the board's are listed, and their calls say iq is not there.
            let asked = bridge(&Request::Tools {
                agent: agent.to_owned(),
            });
            let tools = asked
                .ok()
                .filter(|reply| reply.ok)
                .and_then(|reply| serde_json::from_str::<Value>(&reply.text).ok())
                .filter(Value::is_array)
                .unwrap_or_else(|| crate::tools::specs_json(&crate::tools::board()));
            Ok(json!({ "tools": tools }))
        }
        "tools/call" => {
            let request = Request::Tool {
                agent: agent.to_owned(),
                name: message["params"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                arguments: message["params"]["arguments"].clone(),
            };
            let (text, ok) = match bridge(&request) {
                Ok(reply) => (reply.text, reply.ok),
                Err(error) => (format!("iq is not there to answer: {error}"), false),
            };
            Ok(json!({
                "content": [{"type": "text", "text": text}],
                "isError": !ok,
            }))
        }
        _ => Err(json!({"code": -32601, "message": format!("no method {method}")})),
    };
    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::Reply;

    #[test]
    fn the_board_is_offered_and_its_calls_passed_on() {
        let asked = std::cell::RefCell::new(Vec::new());
        let mut bridge = |request: &Request| {
            asked.borrow_mut().push(format!("{request:?}"));
            // The running iq lists the board's tools and those an
            // extension gives this agent.
            let text = match request {
                Request::Tools { .. } => {
                    let mut tools = crate::tools::board();
                    tools.push(iq_core::ToolSpec {
                        name: "assay_run".into(),
                        description: "Runs.".into(),
                        parameters: json!({"type": "object"}),
                    });
                    crate::tools::specs_json(&tools).to_string()
                }
                _ => "Sent.".into(),
            };
            Ok(Reply { text, ok: true })
        };
        let start = answer(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
            "tester",
            &mut bridge,
        )
        .unwrap();
        assert_eq!(start["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(
            answer(
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                "tester",
                &mut bridge
            ),
            None
        );
        let list = answer(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "tester",
            &mut bridge,
        )
        .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "start_agent",
                "send_message",
                "wait_for_messages",
                "list_agents",
                "assay_run"
            ]
        );
        let call = answer(
            r#"{"jsonrpc":"2.0","id":"c","method":"tools/call","params":{"name":"send_message","arguments":{"to":"lead","text":"hi"}}}"#,
            "tester",
            &mut bridge,
        )
        .unwrap();
        assert_eq!(call["id"], "c");
        assert_eq!(call["result"]["content"][0]["text"], "Sent.");
        assert_eq!(call["result"]["isError"], false);
        assert!(asked.borrow()[0].contains("Tools") && asked.borrow()[0].contains("tester"));
        assert!(asked.borrow()[1].contains("tester") && asked.borrow()[1].contains("send_message"));
        // Without the running iq, the board's tools are listed still.
        let mut gone = |_: &Request| Err(std::io::Error::other("no iq"));
        let alone = answer(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/list"}"#,
            "tester",
            &mut gone,
        )
        .unwrap();
        assert_eq!(alone["result"]["tools"].as_array().unwrap().len(), 4);
        let unknown = answer(
            r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#,
            "tester",
            &mut bridge,
        )
        .unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
    }
}
