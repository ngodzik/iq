//! The commands the plugin runs on the machine, as the person would: git,
//! docker's compose, and the line that starts a worktree's stack, each in
//! the worktree with what makes its compose its own beside the person's
//! environment.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use crate::PortsError;

/// How long a command that reads may run.
pub const READING: Duration = Duration::from_secs(60);

/// How long the line that starts a stack may run: it may build images.
pub const STARTING: Duration = Duration::from_secs(30 * 60);

/// The most of what a failed command said that is kept: its end.
const SAID: usize = 600;

/// What a command did.
#[derive(Debug, Clone)]
pub struct Output {
    /// Whether it succeeded.
    pub ok: bool,
    /// What it printed.
    pub stdout: String,
    /// What it printed as errors.
    pub stderr: String,
}

impl Output {
    /// What it said, its end, for the person to read why it failed.
    #[must_use]
    pub fn said(&self) -> String {
        let all = format!("{}\n{}", self.stdout.trim(), self.stderr.trim());
        let all = all.trim();
        let start = all.char_indices().rev().nth(SAID).map_or(0, |(at, _)| at);
        all[start..].trim().to_owned()
    }
}

/// Runs `program` with `args` in `folder`, `vars` beside the person's
/// environment, stopped after `most`.
///
/// # Errors
///
/// It could not be started, or ran out of time.
pub async fn run(
    program: &str,
    args: &[&str],
    folder: &Path,
    vars: &BTreeMap<String, String>,
    most: Duration,
) -> Result<Output, PortsError> {
    let line = std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .current_dir(folder)
        .envs(vars)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let failed = |said: String| PortsError::Command {
        command: line.clone(),
        said,
    };
    let output = tokio::time::timeout(most, command.output())
        .await
        .map_err(|_| failed(format!("it ran for more than {}s", most.as_secs())))?
        .map_err(|error| failed(error.to_string()))?;
    Ok(Output {
        ok: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Runs `line` as a shell reads it, as [`run`] does.
///
/// # Errors
///
/// As [`run`]'s.
pub async fn shell(
    line: &str,
    folder: &Path,
    vars: &BTreeMap<String, String>,
    most: Duration,
) -> Result<Output, PortsError> {
    run("/bin/sh", &["-c", line], folder, vars, most).await
}

/// What git says of `folder`, trimmed; none when it fails.
pub async fn git(folder: &Path, args: &[&str]) -> Option<String> {
    let output = run("git", args, folder, &BTreeMap::new(), READING)
        .await
        .ok()?;
    output.ok.then(|| output.stdout.trim().to_owned())
}

/// Runs `docker compose` with `args` in `folder`, `vars` making the
/// worktree's compose its own.
///
/// # Errors
///
/// As [`run`]'s.
pub async fn compose(
    folder: &Path,
    vars: &BTreeMap<String, String>,
    args: &[&str],
) -> Result<Output, PortsError> {
    let mut all = vec!["compose"];
    all.extend_from_slice(args);
    run("docker", &all, folder, vars, READING).await
}
