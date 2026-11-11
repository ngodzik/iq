//! Commands run held by the operating system: they read the system, the
//! toolchains and the workspace, write only the workspace, a folder of
//! their own and the toolchains' caches, and reach no network unless their
//! permit says. Each starts through a helper, iq itself, that confines its
//! own process and then becomes the command, so that what the command
//! starts is held as it is. A command run on the machine itself goes
//! through the helper too, which only detaches it: no command ever has the
//! person's terminal, to write over iq's screen or wait there for an
//! answer.

use std::ffi::OsString;
use std::io::BufRead;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::action::{Action, Permit};
use crate::audit::Audit;
use crate::error::GateError;
use crate::given::Given;
use crate::redact::{Redactor, secret_name};
use crate::watch::Watch;

/// The flag the iq command takes to run as the sandbox's helper.
pub const HELPER_FLAG: &str = "--iq-sandbox";

/// The flag that makes the helper hold a network of the sandbox's own,
/// where only localhost answers, for its commands to share.
pub const HOLD_FLAG: &str = "--iq-network";

/// The flag that makes the helper only detach the command from iq's
/// terminal, without confining it: a command run on the machine itself.
pub const DETACH_FLAG: &str = "--iq-detach";

/// How git reaches a remote over ssh unless the person named their own
/// way: never asking, so that what would wait for an answer fails at once
/// with a message.
const SSH_ASKING_NOTHING: &str = "ssh -o BatchMode=yes";

/// What the holder of that network says once it holds it.
pub(crate) const HOLDING: &str = "holding";

/// How the helper's own failures are told apart from the command's output.
const HELPER_PREFIX: &str = "iq-sandbox: ";

/// The most a command prints that is kept: its end, where errors are.
const OUTPUT_BYTES: usize = 30_000;

/// The longest a command runs by default.
const TIMEOUT: Duration = Duration::from_secs(600);

/// How long a command may write nothing before it is stopped, unless the
/// sandbox is told otherwise: one that waits for a login in a browser, or
/// for an answer at a terminal it does not have, would wait until its time
/// ran out, holding its agent.
const QUIET: Duration = Duration::from_secs(180);

/// How long a command's output is waited for once it ended: what it left
/// running in the background holds its output open, and is not waited for.
const DRAIN: Duration = Duration::from_secs(1);

/// The system's folders a command reads and runs programs from, those of
/// Linux and of macOS.
const SYSTEM: &[&str] = &[
    "/usr",
    "/System",
    "/Library",
    "/Applications",
    "/private/etc",
    "/private/var/db",
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    "/etc",
    "/opt",
    "/proc",
    "/sys",
    "/nix",
    "/snap",
    "/home/linuxbrew/.linuxbrew",
];

/// The folders and files of the home a command reads and runs programs
/// from: the toolchains, their managers, and git's own settings.
const HOME_READ: &[&str] = &[
    ".cargo",
    ".rustup",
    ".local/bin",
    ".local/lib",
    ".local/share/pnpm",
    ".local/share/mise",
    ".local/share/fnm",
    // The Pythons uv installs, which the environments it makes run on, and
    // the tools it installs, linked from `~/.local/bin`; never all of its
    // folder, which keeps the credentials it logs in with.
    ".local/share/uv/python",
    ".local/share/uv/tools",
    ".linuxbrew",
    ".npm",
    ".nvm",
    ".volta",
    ".asdf",
    ".pyenv",
    ".rbenv",
    ".bun",
    ".deno",
    ".sdkman",
    "go",
    ".gitconfig",
    ".config/git",
];

/// The package managers' caches a command writes, to install and build
/// from what they hold: each keeps what it fetched by its checksum.
const HOME_WRITE: &[&str] = &[
    ".cargo/registry",
    ".cargo/git",
    ".npm/_cacache",
    ".bun/install/cache",
    ".local/share/pnpm/store",
    ".cache/pip",
    ".cache/uv",
    ".cache/yarn",
    ".cache/go-build",
    "go/pkg/mod",
    "Library/Caches/pip",
    "Library/Caches/uv",
    "Library/Caches/Yarn",
    "Library/Caches/go-build",
    "Library/pnpm/store",
];

/// The variables a command gets from iq's environment, beside those of
/// the locale (`LC_*`): none that may hold a secret.
const PASSED: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LANGUAGE",
    "TERM",
    "COLORTERM",
    "TZ",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "GOPATH",
    "GOROOT",
    "NVM_DIR",
    "PYENV_ROOT",
    "JAVA_HOME",
    "PKG_CONFIG",
    "PKG_CONFIG_PATH",
];

/// Where macOS keeps the person's data, the homes and other disks among
/// it, which their own paths (`/Users`, `/Volumes`, `/home`) reach too: a
/// folder of the system's never stands for what is there.
pub(crate) const DATA_VOLUME: &str = "/System/Volumes/Data";

/// What the sandbox lets a command reach, beside the workspace.
#[derive(Debug, Clone)]
pub(crate) struct Bounds {
    pub(crate) workspace: PathBuf,
    pub(crate) home: Option<PathBuf>,
    pub(crate) readable: Vec<PathBuf>,
    pub(crate) writable: Vec<PathBuf>,
    /// The agents' folder of scratch, also among `writable`.
    pub(crate) scratch: Option<PathBuf>,
}

impl Bounds {
    /// The bounds of a workspace, the person's own additions after the
    /// defaults.
    pub(crate) fn new(
        workspace: PathBuf,
        home: Option<PathBuf>,
        readable: Vec<PathBuf>,
        writable: Vec<PathBuf>,
    ) -> Self {
        let in_home = |names: &[&str]| -> Vec<PathBuf> {
            home.iter()
                .flat_map(|home| names.iter().map(move |name| home.join(name)))
                .collect()
        };
        let mut read: Vec<PathBuf> = SYSTEM.iter().map(PathBuf::from).collect();
        read.extend(in_home(HOME_READ));
        read.extend(readable);
        let mut write = in_home(HOME_WRITE);
        write.extend(writable);
        Self {
            workspace,
            home,
            readable: read,
            writable: write,
            scratch: None,
        }
    }

    /// Whether a command may read `path`. `/System` lets it read the
    /// system, not the data volume under it.
    pub(crate) fn reads(&self, path: &Path) -> bool {
        let data = Path::new(DATA_VOLUME);
        path.starts_with(&self.workspace)
            || self.readable.iter().chain(&self.writable).any(|root| {
                path.starts_with(root) && !(data.starts_with(root) && path.starts_with(data))
            })
    }
}

/// The policy the helper confines itself to.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Policy {
    pub(crate) read: Vec<PathBuf>,
    pub(crate) write: Vec<PathBuf>,
    /// Whether the command reaches the internet.
    pub(crate) network: bool,
    /// Without it, the process holding the network of the sandbox's own,
    /// to join; where there is none, the command has one of its own.
    #[serde(default)]
    pub(crate) shared: Option<u32>,
}

/// The program that confines itself and becomes the command: the iq
/// command with [`HELPER_FLAG`], or a program of its own in tests.
#[derive(Debug, Clone)]
pub struct Helper {
    program: PathBuf,
    args: Vec<OsString>,
}

impl Helper {
    /// `program` run with `args` before the policy.
    pub fn new(
        program: impl Into<PathBuf>,
        args: impl IntoIterator<Item = impl Into<OsString>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    /// The running program, as the iq command is, with [`HELPER_FLAG`].
    pub fn this_program() -> Result<Self, GateError> {
        let program = std::env::current_exe().map_err(GateError::Spawn)?;
        Ok(Self::new(program, [HELPER_FLAG]))
    }
}

/// What a command did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// Its exit status, none when a signal ended it.
    pub status: Option<i32>,
    /// What it printed, errors with the rest in order, its secrets taken
    /// out, its end kept when it is long.
    pub output: String,
    /// Whether it ran out of time, or wrote nothing for too long, and was
    /// stopped.
    pub timed_out: bool,
    /// What it wrote where code runs later on its own and was put back:
    /// git's hooks, the settings that make git run a program.
    pub restored: Vec<PathBuf>,
    /// What it changed where code runs later and was left, for the person
    /// to see: CI, the editor's tasks, direnv's file.
    pub touched: Vec<PathBuf>,
    /// The files of its folder of scratch taken away after it, the oldest
    /// first, so that the folder keeps within its size.
    pub cleared: Vec<PathBuf>,
}

/// Runs the commands a [`Permit`] allows, held by the operating system.
#[derive(Debug, Clone)]
pub struct Sandbox {
    helper: Helper,
    bounds: Bounds,
    audit: Option<Audit>,
    timeout: Duration,
    /// How long a command may write nothing before it is stopped.
    quiet: Duration,
    /// The process holding the commands' own network, started with the
    /// first command without the internet, on Linux.
    holder: Arc<Mutex<Option<Holder>>>,
    /// The process groups of commands still running, and of those that
    /// left something running when they ended, as a server started in the
    /// background: stopped when the sandbox ends, whether their turn ended
    /// or was cut short.
    leftovers: Arc<Mutex<Vec<u32>>>,
    /// The variables the person's programs gave each place's commands.
    given: Given,
    /// The folder of scratch its commands are given, within the gate's,
    /// and the most it holds, in bytes.
    scratch: Option<(PathBuf, u64)>,
}

/// The process holding a network of the sandbox's own: it lets it go when
/// its input ends, as when iq ends, or when it is dropped.
#[derive(Debug)]
struct Holder(std::process::Child);

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Event<'a> {
    Run {
        line: &'a str,
        folder: &'a Path,
        network: bool,
        host: bool,
    },
    Ran {
        status: Option<i32>,
        timed_out: bool,
        restored: &'a [PathBuf],
        touched: &'a [PathBuf],
    },
}

impl Sandbox {
    pub(crate) fn new(helper: Helper, bounds: Bounds, audit: Option<Audit>, given: Given) -> Self {
        Self {
            helper,
            bounds,
            audit,
            timeout: TIMEOUT,
            quiet: QUIET,
            holder: Arc::default(),
            leftovers: Arc::default(),
            given,
            scratch: None,
        }
    }

    /// Ends the sandbox: what its commands left running is stopped, and
    /// its network let go.
    pub async fn end(&self) {
        let groups = self
            .leftovers
            .lock()
            .map(|mut groups| std::mem::take(&mut *groups))
            .unwrap_or_default();
        for group in groups {
            stop_group(group).await;
        }
        if let Ok(mut holder) = self.holder.lock() {
            holder.take();
        }
    }

    /// The process holding the commands' own network, started if it is not
    /// there; `None` where none can be held, each command then having one
    /// of its own.
    fn shared_network(&self) -> Option<u32> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let mut holder = self.holder.lock().ok()?;
        if let Some(Holder(child)) = holder.as_mut()
            && matches!(child.try_wait(), Ok(None))
        {
            return Some(child.id());
        }
        let mut child = std::process::Command::new(&self.helper.program)
            .arg(HOLD_FLAG)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut said = String::new();
        let read = child
            .stdout
            .take()
            .map(|out| std::io::BufReader::new(out).read_line(&mut said));
        let started = Holder(child);
        if !matches!(read, Some(Ok(_))) || said.trim() != HOLDING {
            return None;
        }
        let id = started.0.id();
        *holder = Some(started);
        Some(id)
    }

    /// The same sandbox, its commands stopped after `timeout`.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The same sandbox, its commands stopped once they wrote nothing for
    /// `quiet`: one waiting for an answer nobody can give it.
    #[must_use]
    pub fn with_quiet(mut self, quiet: Duration) -> Self {
        self.quiet = quiet;
        self
    }

    /// The same sandbox, its commands given `folder` as `IQ_SCRATCH`: the
    /// folder of scratch of their chat, within the gate's (see
    /// [`GateBuilder::scratch`]), made when it is not there. Once a command
    /// ends, the oldest files there are taken away until it holds no more
    /// than `most` bytes. A folder outside the gate's is not given.
    ///
    /// [`GateBuilder::scratch`]: crate::GateBuilder::scratch
    #[must_use]
    pub fn with_scratch(mut self, folder: impl Into<PathBuf>, most: u64) -> Self {
        let folder: PathBuf = folder.into();
        let within = self.bounds.scratch.as_ref().is_some_and(|scratch| {
            folder.starts_with(scratch)
                && folder
                    .components()
                    .all(|part| !matches!(part, std::path::Component::ParentDir))
        });
        self.scratch = within.then_some((folder, most));
        self
    }

    /// Runs the command `permit` allows: in the sandbox, or on the machine
    /// itself when the permit says so, as the person would run it, with
    /// their environment but iq's own variables. Either way it gets what
    /// the person's programs gave its place (see [`Gate::give`]), its
    /// output is cleared of secrets, and what it wrote where code runs
    /// later is put back.
    ///
    /// [`Gate::give`]: crate::Gate::give
    pub async fn run(&self, permit: Permit) -> Result<Ran, GateError> {
        self.run_with(permit, &Live::default()).await
    }

    /// Runs the command `permit` allows as [`Sandbox::run`] does, what it
    /// writes readable in `live` as it comes: a command left to run in the
    /// background, read before it ends.
    ///
    /// # Errors
    ///
    /// As [`Sandbox::run`].
    pub async fn run_live(&self, permit: Permit, live: &Live) -> Result<Ran, GateError> {
        self.run_with(permit, live).await
    }

    async fn run_with(&self, permit: Permit, live: &Live) -> Result<Ran, GateError> {
        let Action::Run {
            line,
            folder,
            network,
            host,
        } = permit.action
        else {
            return Err(GateError::NotACommand);
        };
        if !host && !cfg!(any(target_os = "linux", target_os = "macos")) {
            return Err(GateError::Unsupported(format!(
                "iq holds no command in a sandbox on {} yet",
                std::env::consts::OS
            )));
        }
        let own = tempfile::Builder::new()
            .prefix("iq-sandbox-")
            .tempdir()
            .map_err(GateError::Spawn)?;
        let workspace = &self.bounds.workspace;
        let mut read = self.bounds.readable.clone();
        read.push(workspace.clone());
        let mut write = self.bounds.writable.clone();
        write.extend([
            workspace.clone(),
            own.path().to_path_buf(),
            PathBuf::from("/dev"),
        ]);
        let shared = if network { None } else { self.shared_network() };
        let policy = Policy {
            read,
            write,
            network,
            shared,
        };
        let policy =
            serde_json::to_string(&policy).map_err(|e| GateError::Confine(e.to_string()))?;
        if let Some(audit) = &self.audit {
            audit.write(&Event::Run {
                line: &line,
                folder: &folder,
                network,
                host,
            })?;
        }
        // Errors land with the output, in order.
        let shell = ["/bin/sh", "-c", &format!("exec 2>&1; {line}")];
        let mut command = if host {
            let mut command = tokio::process::Command::new(&self.helper.program);
            command
                .arg(DETACH_FLAG)
                .arg("--")
                .args(shell)
                .env_clear()
                .envs(kept(std::env::vars_os()));
            command
        } else {
            let mut command = tokio::process::Command::new(&self.helper.program);
            command
                .args(&self.helper.args)
                .arg(policy)
                .arg("--")
                .args(shell)
                .env_clear()
                .envs(passed(std::env::vars_os()))
                .env("TMPDIR", own.path())
                .env("IQ_SANDBOX", "1");
            if !network {
                // Without it cargo tries the network, and fails, on each build.
                command.env("CARGO_NET_OFFLINE", "true");
            }
            command
        };
        // Nobody can answer what git or ssh would ask at a terminal, which
        // the command has not: they fail at once instead of waiting.
        command.env("GIT_TERMINAL_PROMPT", "0");
        if !ssh_named(std::env::vars_os()) && !ssh_set(&folder).await {
            command.env("GIT_SSH_COMMAND", SSH_ASKING_NOTHING);
        }
        // What the person's programs gave this place, the plugin that keeps
        // its containers apart among them, either way.
        command.envs(self.given.of(workspace));
        if let Some((scratch, _)) = &self.scratch
            && std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(scratch)
                .is_ok()
        {
            command.env("IQ_SCRATCH", scratch);
        }
        command
            .current_dir(&folder)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let watch = Watch::take(workspace);
        let mut child = command.spawn().map_err(GateError::Spawn)?;
        // The helper leads a session of its own, its group the same.
        let group = child.id();
        // Kept from the start: a turn stopped while the command runs drops
        // this wait, which stops the group, and the sandbox's end stops
        // what it may have left.
        if let Some(group) = group
            && let Ok(mut leftovers) = self.leftovers.lock()
        {
            leftovers.push(group);
        }
        let mut stopping = Stopping(group);
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
        if let Ok(mut redactor) = live.redactor.lock() {
            *redactor = Some(Redactor::new(std::env::vars_os(), workspace));
        }
        let ended = waited(
            &mut child,
            stdout,
            stderr,
            self.timeout,
            self.quiet,
            Arc::clone(&live.out),
        )
        .await;
        stopping.0 = None;
        if let Some(group) = group {
            if ended.stopped.is_some() {
                stop_group(group).await;
                let _ = child.wait().await;
            }
            if (ended.stopped.is_some() || !alive(group).await)
                && let Ok(mut leftovers) = self.leftovers.lock()
            {
                leftovers.retain(|kept| *kept != group);
            }
        }
        let settled = watch.settle();
        let helper = String::from_utf8_lossy(&ended.stderr);
        if let Some(failure) = helper.strip_prefix(HELPER_PREFIX) {
            return Err(GateError::Confine(failure.trim().to_owned()));
        }
        // What the shell says before its errors join the output, such as a
        // line it cannot parse, comes after.
        let mut output = format!("{}{helper}", String::from_utf8_lossy(&ended.stdout));
        let (status, timed_out) = match ended.stopped {
            None => (ended.status.and_then(|status| status.code()), false),
            Some(why) => {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&why);
                (None, true)
            }
        };
        let redactor = Redactor::new(std::env::vars_os(), workspace);
        let cleared = match &self.scratch {
            Some((scratch, most)) => clear(scratch, *most),
            None => Vec::new(),
        };
        let ran = Ran {
            status,
            output: tail(&redactor.redact(&output), OUTPUT_BYTES),
            timed_out,
            restored: settled.restored,
            touched: settled.touched,
            cleared,
        };
        if let Some(audit) = &self.audit {
            audit.write(&Event::Ran {
                status: ran.status,
                timed_out: ran.timed_out,
                restored: &ran.restored,
                touched: &ran.touched,
            })?;
        }
        Ok(ran)
    }
}

/// Takes the oldest files of `folder` away until it holds no more than
/// `most` bytes, and says which went. Links are taken as files, never
/// followed.
fn clear(folder: &Path, most: u64) -> Vec<PathBuf> {
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(at) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            if meta.is_dir() {
                folders.push(entry.path());
            } else {
                let changed = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                files.push((changed, meta.len(), entry.path()));
            }
        }
    }
    let mut held: u64 = files.iter().map(|(_, size, _)| size).sum();
    files.sort();
    let mut cleared = Vec::new();
    for (_, size, path) in files {
        if held <= most {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            held = held.saturating_sub(size);
            cleared.push(path);
        }
    }
    cleared
}

/// What a command run in the background has written so far, readable as
/// it runs, its secrets taken out as its output's are.
#[derive(Debug, Clone, Default)]
pub struct Live {
    out: Arc<Mutex<Vec<u8>>>,
    redactor: Arc<Mutex<Option<Redactor>>>,
}

impl Live {
    /// The end of what it wrote so far, as many as `bytes`, its secrets
    /// taken out.
    #[must_use]
    pub fn so_far(&self, bytes: usize) -> String {
        let written = self
            .out
            .lock()
            .map(|out| String::from_utf8_lossy(&out).into_owned())
            .unwrap_or_default();
        let redacted = match self.redactor.lock().ok().and_then(|r| r.clone()) {
            Some(redactor) => redactor.redact(&written),
            None => written,
        };
        tail(&redacted, bytes)
    }
}

/// How a command ended: its status when it ended on its own, else why it
/// was stopped; and what it wrote.
struct Ended {
    status: Option<std::process::ExitStatus>,
    stopped: Option<String>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Waits for `child` to end, reading what it writes as it comes, and stops
/// waiting after `timeout`, or once it wrote nothing for `quiet`. Its
/// output is read for [`DRAIN`] more once it ended: what it left running
/// holds its output open.
async fn waited(
    child: &mut tokio::process::Child,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    timeout: Duration,
    quiet: Duration,
    out: Arc<Mutex<Vec<u8>>>,
) -> Ended {
    let started = Instant::now();
    let last = Arc::new(Mutex::new(Instant::now()));
    let err = Arc::new(Mutex::new(Vec::new()));
    let readers = [
        stdout.map(|pipe| tokio::spawn(reading(pipe, Arc::clone(&out), Arc::clone(&last)))),
        stderr.map(|pipe| tokio::spawn(reading(pipe, Arc::clone(&err), Arc::clone(&last)))),
    ];
    let mut status = None;
    let stopped = loop {
        tokio::select! {
            ended = child.wait() => {
                status = ended.ok();
                break None;
            }
            () = tokio::time::sleep(Duration::from_millis(250)) => {
                if started.elapsed() >= timeout {
                    break Some(format!("stopped after {} seconds", timeout.as_secs()));
                }
                let silent = last.lock().map(|last| last.elapsed()).unwrap_or_default();
                if silent >= quiet {
                    break Some(format!(
                        "stopped after {} seconds without output: it was waiting for something \
                         nobody can give it (an answer at a terminal, a login in a browser), or \
                         it is slow; run it again with output, or in the background with its \
                         output in a file",
                        quiet.as_secs()
                    ));
                }
            }
        }
    };
    for reader in readers.into_iter().flatten() {
        if tokio::time::timeout(DRAIN, reader).await.is_err() {
            // What it left running holds the pipe: what was read is kept.
        }
    }
    let taken = |buffer: &Arc<Mutex<Vec<u8>>>| buffer.lock().map(|b| b.clone()).unwrap_or_default();
    Ended {
        status,
        stopped,
        stdout: taken(&out),
        stderr: taken(&err),
    }
}

/// Reads `pipe` into `into` as it comes, noting in `last` when it last
/// came, until it closes.
async fn reading(
    mut pipe: impl tokio::io::AsyncRead + Unpin,
    into: Arc<Mutex<Vec<u8>>>,
    last: Arc<Mutex<Instant>>,
) {
    use tokio::io::AsyncReadExt;
    let mut chunk = [0u8; 8192];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if let Ok(mut into) = into.lock() {
                    into.extend_from_slice(&chunk[..n]);
                }
                if let Ok(mut last) = last.lock() {
                    *last = Instant::now();
                }
            }
        }
    }
}

/// Stops a command's group when the wait for it is dropped before it
/// ended: a turn stopped by the person while its command runs. Once the
/// command ended, there is nothing to stop.
struct Stopping(Option<u32>);

impl Drop for Stopping {
    fn drop(&mut self) {
        if let Some(group) = self.0.take()
            && group >= 2
        {
            let _ = std::process::Command::new("kill")
                .args(["-s", "KILL", "--", &format!("-{group}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// The variables of `env` a command on the machine itself gets: the
/// person's own, as they would run it, but iq's, which are of no use to
/// it, its provider's key among them.
fn kept(
    env: impl IntoIterator<Item = (OsString, OsString)>,
) -> impl Iterator<Item = (OsString, OsString)> {
    env.into_iter().filter(|(name, _)| {
        name.to_str()
            .is_none_or(|name| !name.starts_with("IQ_") && !name.starts_with("REQUESTY_"))
    })
}

/// The variables of `env` a command in the sandbox gets.
fn passed(
    env: impl IntoIterator<Item = (OsString, OsString)>,
) -> impl Iterator<Item = (OsString, OsString)> {
    env.into_iter().filter(|(name, _)| {
        name.to_str().is_some_and(|name| {
            (PASSED.contains(&name) || name.starts_with("LC_")) && !secret_name(name)
        })
    })
}

/// Whether the person named in their environment, `env`, how git reaches
/// a remote over ssh, which [`SSH_ASKING_NOTHING`] would override.
fn ssh_named(env: impl IntoIterator<Item = (OsString, OsString)>) -> bool {
    env.into_iter()
        .any(|(name, value)| (name == "GIT_SSH_COMMAND" || name == "GIT_SSH") && !value.is_empty())
}

/// Whether git's settings, as `folder` reads them, name how it reaches a
/// remote over ssh (`core.sshCommand`), which [`SSH_ASKING_NOTHING`]
/// would override.
async fn ssh_set(folder: &Path) -> bool {
    tokio::process::Command::new("git")
        .args(["config", "--get", "core.sshCommand"])
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .is_ok_and(|said| said.status.success() && !said.stdout.trim_ascii().is_empty())
}

/// Stops every process of the command's group, the command's own children
/// with it. A group below 2 would name every process the person may
/// signal, and is never signalled.
async fn stop_group(group: u32) {
    if group < 2 {
        return;
    }
    let _ = tokio::process::Command::new("kill")
        .args(["-s", "KILL", "--", &format!("-{group}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

/// Whether a process of `group` still runs. A group below 2 names none.
async fn alive(group: u32) -> bool {
    group >= 2
        && tokio::process::Command::new("kill")
            .args(["-s", "0", "--", &format!("-{group}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok_and(|status| status.success())
}

/// The last `bytes` of `text`, at a character, saying what was cut.
fn tail(text: &str, bytes: usize) -> String {
    if text.len() <= bytes {
        return text.to_owned();
    }
    let mut from = text.len() - bytes;
    while !text.is_char_boundary(from) {
        from += 1;
    }
    format!("[{from} bytes cut]\n{}", &text[from..])
}

/// Confines this process to the policy its first argument gives, then
/// becomes the command after `--`. Returns only when that fails; the
/// helper prints it as [`helper_failure`] writes it and exits, so that
/// nothing runs unconfined.
pub fn sandboxed(args: impl IntoIterator<Item = OsString>) -> GateError {
    use std::os::unix::process::CommandExt;
    let mut args = args.into_iter();
    let policy: Policy = match args
        .next()
        .and_then(|policy| serde_json::from_str(policy.to_str()?).ok())
    {
        Some(policy) => policy,
        None => return GateError::Confine("no policy was given".to_owned()),
    };
    if args.next().as_deref() != Some("--".as_ref()) {
        return GateError::Confine("no command was given".to_owned());
    }
    let Some(program) = args.next() else {
        return GateError::Confine("no command was given".to_owned());
    };
    if let Err(error) = own_session() {
        return error;
    }
    let (program, args) = match crate::confine::confine(&policy, program, args.collect()) {
        Ok(command) => command,
        Err(error) => return error,
    };
    GateError::Spawn(std::process::Command::new(program).args(args).exec())
}

/// Becomes the command after `--` in a session of its own, with no
/// terminal, without confining it: what the helper does with
/// [`DETACH_FLAG`], for a command run on the machine itself. Returns only
/// when that fails; the helper prints it as [`helper_failure`] writes it
/// and exits.
pub fn detached(args: impl IntoIterator<Item = OsString>) -> GateError {
    use std::os::unix::process::CommandExt;
    let mut args = args.into_iter();
    if args.next().as_deref() != Some("--".as_ref()) {
        return GateError::Confine("no command was given".to_owned());
    }
    let Some(program) = args.next() else {
        return GateError::Confine("no command was given".to_owned());
    };
    if let Err(error) = own_session() {
        return error;
    }
    GateError::Spawn(std::process::Command::new(program).args(args).exec())
}

/// Leaves iq's session for one of this process's own, with no terminal:
/// what the command would ask at one (ssh, sudo, a pinentry) fails at
/// once instead of writing over iq's screen and waiting there for an
/// answer. The session's group, the one the sandbox stops, is this
/// process's.
fn own_session() -> Result<(), GateError> {
    nix::unistd::setsid().map(drop).map_err(|error| {
        GateError::Confine(format!(
            "the command could not leave iq's terminal: {error}"
        ))
    })
}

/// Holds a network of the sandbox's own, where only localhost answers, for
/// its commands to share, until the input ends: what the helper does with
/// [`HOLD_FLAG`]. Returns why it stopped.
pub fn hold_network() -> GateError {
    crate::confine::hold_network()
}

/// What the helper prints when it fails, before exiting: the sandbox
/// reads it as its own failure, not the command's.
pub fn helper_failure(error: &GateError) -> String {
    format!("{HELPER_PREFIX}{error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_reaches_ssh_the_persons_way_when_they_named_one() {
        let env = |pairs: &[(&str, &str)]| -> Vec<(OsString, OsString)> {
            pairs
                .iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value)))
                .collect()
        };
        assert!(ssh_named(env(&[("GIT_SSH_COMMAND", "ssh -i ~/.ssh/work")])));
        assert!(ssh_named(env(&[("GIT_SSH", "plink")])));
        assert!(!ssh_named(env(&[
            ("GIT_SSH_COMMAND", ""),
            ("PATH", "/bin")
        ])));
    }

    #[tokio::test]
    async fn git_s_own_settings_name_how_it_reaches_ssh() {
        let repository = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(repository.path())
                .output()
                .is_ok_and(|done| done.status.success())
        };
        if !git(&["init", "-q"]) {
            return;
        }
        assert!(git(&["config", "core.sshCommand", "ssh -i ~/.ssh/work"]));
        assert!(ssh_set(repository.path()).await);
    }

    #[test]
    fn the_data_volume_is_read_only_where_the_policy_names_it() {
        let home = PathBuf::from("/System/Volumes/Data/home/me");
        let bounds = Bounds::new(home.join("project"), Some(home), Vec::new(), Vec::new());
        assert!(bounds.reads(Path::new("/System/Library/Frameworks/x")));
        assert!(bounds.reads(Path::new("/usr/bin/git")));
        assert!(!bounds.reads(Path::new("/System/Volumes/Data/Users/me/.ssh/config")));
        assert!(!bounds.reads(Path::new("/System/Volumes/Data/home/me/notes.txt")));
        assert!(bounds.reads(Path::new(
            "/System/Volumes/Data/home/me/project/src/main.rs"
        )));
        assert!(bounds.reads(Path::new("/System/Volumes/Data/home/me/.cargo/bin/cargo")));
    }
}
