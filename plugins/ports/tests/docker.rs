//! Two worktrees of one project run the same stack at once, for real: the
//! same project name, the same container name, the same port, each kept
//! apart, and the usual port led to one, then to the other; and the same
//! when a `make` target hands compose its files and its project itself,
//! beside the person's own stack.
//!
//! It needs docker, make and the image `python:3.13-alpine`, so it is run
//! by hand: `cargo test -p iq-ports -- --ignored`.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use iq_ports::relay::Relay;
use iq_ports::shim::Shim;
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

/// A repository named `name` whose compose file is `compose`, with
/// `extra` files beside it, and worktrees `lead` and `fix` of it, each
/// serving its own name.
fn repository(
    root: &Path,
    name: &str,
    compose: &str,
    extra: &[(&str, &str)],
) -> std::path::PathBuf {
    let repository = root.join(name);
    std::fs::create_dir(&repository).unwrap();
    std::fs::write(repository.join("compose.yaml"), compose).unwrap();
    for (file, text) in extra {
        std::fs::write(repository.join(file), text).unwrap();
    }
    git(&repository, &["init", "-q", "-b", "main"]);
    git(&repository, &["add", "."]);
    git(&repository, &["commit", "-q", "-m", "stack"]);
    for name in ["lead", "fix"] {
        git(
            &repository,
            &["worktree", "add", "-q", "-b", name, &format!("../{name}")],
        );
        std::fs::write(root.join(name).join("index.html"), name).unwrap();
    }
    repository
}

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
    repository(root.path(), "shop", COMPOSE, &[]);
    let store = Store::at(root.path().join("state"));
    let mut kept = Kept::default();
    let mut stacks = Vec::new();
    for name in ["lead", "fix"] {
        let stack = Stack::prepare(&root.path().join(name), None, &store, &kept, None)
            .await
            .unwrap();
        kept.keep(stack.folder.clone(), stack.place());
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

/// Another stack's port, so that both tests run at once.
const CART: u16 = 18766;

const CART_COMPOSE: &str = "name: cart
services:
  web:
    image: python:3.13-alpine
    container_name: cart-web
    command: [\"python\", \"-m\", \"http.server\", \"8000\"]
    working_dir: /srv
    volumes: [\"./:/srv:ro\"]
    ports: [\"18766:8000\"]
";

/// A target that hands compose its file and its project itself.
const MAKEFILE: &str = "up:\n\tdocker compose --project-name cart --file compose.yaml up -d\n";

/// The names of the containers of the compose project `project`.
fn containers(project: &str) -> String {
    let listed = std::process::Command::new("docker")
        .args(["ps", "--all", "--format", "{{.Names}}", "--filter"])
        .arg(format!("label=com.docker.compose.project={project}"))
        .output()
        .unwrap();
    String::from_utf8_lossy(&listed.stdout).trim().to_owned()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs docker, make and python:3.13-alpine"]
async fn a_target_handing_compose_its_files_keeps_each_worktree_apart_beside_the_persons_own() {
    let root = tempfile::tempdir().unwrap();
    let repository = repository(root.path(), "cart", CART_COMPOSE, &[("Makefile", MAKEFILE)]);
    std::fs::write(repository.join("index.html"), "main").unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let persons = std::env::var_os("DOCKER_CONFIG")
        .map_or_else(|| home.join(".docker"), std::path::PathBuf::from);
    let shim = Shim::new(env!("CARGO_BIN_EXE_iq-ports"), persons);
    let store = Store::at(root.path().join("state"));
    let mut stacks = Vec::new();
    for name in ["lead", "fix"] {
        let mut kept = store.load();
        let stack = Stack::prepare(&root.path().join(name), None, &store, &kept, Some(&shim))
            .await
            .unwrap();
        assert!(stack.docker.is_some(), "{:?}", stack.notes);
        kept.keep(stack.folder.clone(), stack.place());
        store.save(&kept).unwrap();
        stacks.push(stack);
    }
    let mut down: Vec<_> = stacks
        .iter()
        .map(|s| (s.folder.clone(), s.vars()))
        .collect();
    down.push((repository.clone(), BTreeMap::new()));
    let _down = Down(down);
    // The person's own stack, started from their checkout as they do.
    let own = iq_ports::docker::shell(
        "make up",
        &repository,
        &BTreeMap::new(),
        iq_ports::docker::STARTING,
    )
    .await
    .unwrap();
    assert!(own.ok, "{}", own.said());
    // Each worktree's, through the same target.
    for stack in &stacks {
        stack.start("make up").await.unwrap();
        assert!(stack.running().await, "{}", stack.project);
    }
    assert_eq!(containers("cart"), "cart-web");
    assert_eq!(containers("cart-lead"), "cart-lead-web-1");
    assert_eq!(containers("cart-fix"), "cart-fix-web-1");
    let hidden: Vec<u16> = stacks.iter().map(|s| s.relayed()[&CART]).collect();
    assert_ne!(hidden[0], hidden[1]);
    assert_eq!(get(CART), "main");
    assert_eq!(get(hidden[0]), "lead");
    assert_eq!(get(hidden[1]), "fix");
    // Read again, each keeps the ports its files were moved to.
    for stack in &stacks {
        let again = Stack::prepare(&stack.folder, None, &store, &store.load(), Some(&shim))
            .await
            .unwrap();
        assert_eq!(again.relayed(), stack.relayed());
    }
}
