//! Two worktrees of one project run the same stack at once, for real: the
//! same project name, the same container name, the same port, each kept
//! apart, and the usual port led to one, then to the other.
//!
//! It needs docker and the image `python:3.13-alpine`, so it is run by
//! hand: `cargo test -p iq-ports -- --ignored`.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use iq_ports::relay::Relay;
use iq_ports::stack::Stack;
use iq_ports::store::{Kept, Store};

/// The usual port the project's file publishes: one nothing else uses.
const USUAL: u16 = 18765;

const COMPOSE: &str = "name: shop
services:
  web:
    image: python:3.13-alpine
    container_name: shop-web
    command: [\"python\", \"-m\", \"http.server\", \"8000\"]
    working_dir: /srv
    volumes: [\"./:/srv:ro\"]
    ports: [\"18765:8000\"]
";

fn git(folder: &Path, args: &[&str]) {
    let done = std::process::Command::new("git")
        .args(args)
        .current_dir(folder)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .unwrap();
    assert!(done.success(), "git {args:?}");
}

/// What localhost's `port` answers, asked again until it does.
fn get(port: u16) -> String {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let said = std::net::TcpStream::connect(("127.0.0.1", port)).and_then(|mut stream| {
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            stream.write_all(b"GET /index.html HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
            let mut said = String::new();
            stream.read_to_string(&mut said)?;
            Ok(said)
        });
        match said {
            Ok(said) if said.starts_with("HTTP/1.0 200") => {
                return said
                    .rsplit("\r\n\r\n")
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
            }
            _ if Instant::now() > until => panic!("localhost:{port} did not answer"),
            _ => std::thread::sleep(Duration::from_millis(250)),
        }
    }
}

/// Stops the stacks, whatever the test came to.
struct Down(Vec<(std::path::PathBuf, BTreeMap<String, String>)>);

impl Drop for Down {
    fn drop(&mut self) {
        for (folder, vars) in &self.0 {
            let _ = std::process::Command::new("docker")
                .args(["compose", "down", "--volumes"])
                .current_dir(folder)
                .envs(vars)
                .status();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs docker and python:3.13-alpine"]
async fn two_worktrees_run_the_same_stack_apart_and_localhost_leads_to_either() {
    let root = tempfile::tempdir().unwrap();
    let repository = root.path().join("shop");
    std::fs::create_dir(&repository).unwrap();
    std::fs::write(repository.join("compose.yaml"), COMPOSE).unwrap();
    git(&repository, &["init", "-q", "-b", "main"]);
    git(&repository, &["add", "."]);
    git(&repository, &["commit", "-q", "-m", "stack"]);
    for name in ["lead", "fix"] {
        git(
            &repository,
            &["worktree", "add", "-q", "-b", name, &format!("../{name}")],
        );
        std::fs::write(root.path().join(name).join("index.html"), name).unwrap();
    }
    let store = Store::at(root.path().join("state"));
    let mut kept = Kept::default();
    let mut stacks = Vec::new();
    for name in ["lead", "fix"] {
        let stack = Stack::prepare(&root.path().join(name), None, &store, &kept)
            .await
            .unwrap();
        kept.places.insert(stack.folder.clone(), stack.place());
        stacks.push(stack);
    }
    let _down = Down(
        stacks
            .iter()
            .map(|s| (s.folder.clone(), s.vars()))
            .collect(),
    );
    assert_eq!(stacks[0].project, "shop-lead");
    assert_eq!(stacks[1].project, "shop-fix");
    let hidden: Vec<u16> = stacks.iter().map(|s| s.relayed()[&USUAL]).collect();
    assert_ne!(hidden[0], hidden[1]);
    // Both run at once: neither the name nor the port is in the way.
    for stack in &stacks {
        stack.start("docker compose up -d").await.unwrap();
        assert!(stack.running().await, "{}", stack.project);
    }
    assert_eq!(get(hidden[0]), "lead");
    assert_eq!(get(hidden[1]), "fix");
    // The usual port leads to one, then to the other at once.
    let mut relay = Relay::default();
    assert!(relay.lead(stacks[0].relayed()).await.is_empty());
    let asked = tokio::task::spawn_blocking(|| get(USUAL)).await.unwrap();
    assert_eq!(asked, "lead");
    assert!(relay.lead(stacks[1].relayed()).await.is_empty());
    let asked = tokio::task::spawn_blocking(|| get(USUAL)).await.unwrap();
    assert_eq!(asked, "fix");
}
