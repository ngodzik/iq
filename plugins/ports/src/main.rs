//! The `iq-ports` command: the plugin iq starts with `--iq-plugin`, and
//! what the person runs in a worktree themselves.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use iq_ports::config::Config;
use iq_ports::relay::Relay;
use iq_ports::stack::Stack;
use iq_ports::store::Store;

/// Each worktree's docker compose stack apart from the others' (its
/// containers, its data, ports of its own), the usual ports of localhost
/// led to the one chosen. Nothing of the project is changed.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Prints what makes compose the worktree's in this shell:
    /// `eval "$(iq-ports env)"`.
    Env {
        /// A folder of the worktree; this one unless given.
        folder: Option<PathBuf>,
    },
    /// Starts the worktree's stack apart, as ~/.iq/ports.toml says it
    /// starts (`docker compose up -d` unless it says).
    Up {
        /// A folder of the worktree; this one unless given.
        folder: Option<PathBuf>,
    },
    /// Leads the usual ports of localhost to the worktree's stack, started
    /// if it is not, until stopped with Ctrl-C.
    Serve {
        /// A folder of the worktree; this one unless given.
        folder: Option<PathBuf>,
    },
    /// Stops the worktree's stack; its data is kept.
    Down {
        /// A folder of the worktree; this one unless given.
        folder: Option<PathBuf>,
    },
    /// The worktrees kept apart: their stacks, their ports.
    Ps,
}

fn main() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new().context("starting the runtime")?;
    if std::env::args_os()
        .nth(1)
        .is_some_and(|flag| flag == iq_plugin::FLAG)
    {
        return runtime.block_on(as_plugin());
    }
    let cli = Cli::parse();
    runtime.block_on(run(cli.command.unwrap_or(Command::Ps)))
}

/// The person's home.
fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

/// The person's own `COMPOSE_FILE`, when they set one.
fn listed() -> Option<String> {
    std::env::var("COMPOSE_FILE").ok()
}

/// Spoken to by iq, over the standard input and output.
async fn as_plugin() -> Result<()> {
    let home = home()?;
    let config = Config::load(&home)?;
    iq_ports::plugin::speak(
        tokio::io::BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
        Store::of_machine(&home),
        config,
        listed(),
    )
    .await?;
    Ok(())
}

/// The stack of the worktree `folder` is in, this folder's unless given,
/// kept as it now is.
async fn stack_of(folder: Option<&Path>, store: &Store) -> Result<Stack> {
    let here = match folder {
        Some(folder) => folder.to_path_buf(),
        None => std::env::current_dir().context("reading the current folder")?,
    };
    let mut kept = store.load();
    let stack = Stack::prepare(&here, listed().as_deref(), store, &kept).await?;
    kept.places.insert(stack.folder.clone(), stack.place());
    store.save(&kept)?;
    for note in &stack.notes {
        eprintln!("iq-ports: {note}");
    }
    Ok(stack)
}

/// Starts `stack` when it does not run.
async fn started(stack: &Stack, config: &Config) -> Result<()> {
    if stack.running().await {
        return Ok(());
    }
    let up = config.up(&stack.repository);
    if let Some(note) = stack.unmoved_by(up).await {
        eprintln!("iq-ports: {note}");
    }
    eprintln!("iq-ports: {} starts: `{up}`…", stack.project);
    stack.start(up).await?;
    Ok(())
}

/// The ports of `stack` as the person reads them.
fn ports(stack: &Stack) -> String {
    stack
        .moved
        .iter()
        .map(|m| format!("{}:{} → localhost:{}", m.service, m.usual, m.hidden))
        .collect::<Vec<_>>()
        .join(", ")
}

async fn run(command: Command) -> Result<()> {
    let home = home()?;
    let store = Store::of_machine(&home);
    let config = Config::load(&home)?;
    match command {
        Command::Env { folder } => {
            println!("{}", stack_of(folder.as_deref(), &store).await?.exports());
        }
        Command::Up { folder } => {
            let stack = stack_of(folder.as_deref(), &store).await?;
            started(&stack, &config).await?;
            println!("{} runs: {}", stack.project, ports(&stack));
        }
        Command::Serve { folder } => {
            let stack = stack_of(folder.as_deref(), &store).await?;
            started(&stack, &config).await?;
            let mut relay = Relay::default();
            let routes: HashMap<u16, u16> = stack.relayed();
            for (port, error) in relay.lead(routes).await {
                eprintln!("iq-ports: localhost:{port} is held by another program: {error}");
            }
            if relay.led().is_empty() {
                bail!(
                    "no usual port of localhost could be led to {}",
                    stack.project
                );
            }
            let led: Vec<String> = relay.led().iter().map(|p| format!(":{p}")).collect();
            println!(
                "localhost {} → {}, until Ctrl-C",
                led.join(", "),
                stack.project
            );
            tokio::signal::ctrl_c()
                .await
                .context("waiting for Ctrl-C")?;
        }
        Command::Down { folder } => {
            let stack = stack_of(folder.as_deref(), &store).await?;
            let down = iq_ports::docker::compose(&stack.folder, &stack.vars(), &["down"]).await?;
            if !down.ok {
                bail!("`docker compose down` failed: {}", down.said());
            }
            println!("{} is stopped; its data is kept.", stack.project);
        }
        Command::Ps => {
            let kept = store.load();
            if kept.places.is_empty() {
                println!("No worktree is kept apart yet.");
            }
            for (folder, place) in &kept.places {
                let served = if kept.served.as_ref() == Some(folder) {
                    "  ← localhost leads here"
                } else {
                    ""
                };
                println!("{}  {}{served}", place.project, folder.display());
                for m in &place.moved {
                    println!("    {}:{} → localhost:{}", m.service, m.usual, m.hidden);
                }
            }
        }
    }
    Ok(())
}
