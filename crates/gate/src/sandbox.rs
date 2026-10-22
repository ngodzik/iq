//! Commands run held by the operating system: they read the system, the
//! toolchains and the workspace, write only the workspace, a folder of
//! their own and the toolchains' caches, and reach no network unless their
//! permit says. Each starts through a helper, iq itself, that confines its
//! own process and then becomes the command, so that what the command
//! starts is held as it is.

use std::ffi::OsString;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::action::{Action, Permit};
use crate::audit::Audit;
use crate::error::GateError;
use crate::redact::{Redactor, secret_name};
use crate::watch::Watch;

/// The flag the iq command takes to run as the sandbox's helper.
pub const HELPER_FLAG: &str = "--iq-sandbox";

/// The flag that makes the helper hold a network of the sandbox's own,
/// where only localhost answers, for its commands to share.
pub const HOLD_FLAG: &str = "--iq-network";

/// What the holder of that network says once it holds it.
pub(crate) const HOLDING: &str = "holding";

/// How the helper's own failures are told apart from the command's output.
const HELPER_PREFIX: &str = "iq-sandbox: ";

/// The most a command prints that is kept: its end, where errors are.
const OUTPUT_BYTES: usize = 30_000;

/// The longest a command runs by default.
const TIMEOUT: Duration = Duration::from_secs(600);

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

/// What the sandbox lets a command reach, beside the workspace.
#[derive(Debug, Clone)]
pub(crate) struct Bounds {
    pub(crate) workspace: PathBuf,
    pub(crate) home: Option<PathBuf>,
    pub(crate) readable: Vec<PathBuf>,
    pub(crate) writable: Vec<PathBuf>,
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
        }
    }

    /// Whether a command may read `path`.
    pub(crate) fn reads(&self, path: &Path) -> bool {
        path.starts_with(&self.workspace)
            || self
                .readable
                .iter()
                .chain(&self.writable)
                .any(|root| path.starts_with(root))
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
    /// Whether it ran out of time and was stopped.
    pub timed_out: bool,
    /// What it wrote where code runs later on its own and was put back:
    /// git's hooks, the settings that make git run a program.
    pub restored: Vec<PathBuf>,
    /// What it changed where code runs later and was left, for the person
    /// to see: CI, the editor's tasks, direnv's file.
    pub touched: Vec<PathBuf>,
}

/// Runs the commands a [`Permit`] allows, held by the operating system.
#[derive(Debug, Clone)]
pub struct Sandbox {
    helper: Helper,
    bounds: Bounds,
    audit: Option<Audit>,
    timeout: Duration,
    /// The process holding the commands' own network, started with the
    /// first command without the internet, on Linux.
    holder: Arc<Mutex<Option<Holder>>>,
    /// The process groups of commands that left something running when
    /// they ended, as a server started in the background: stopped when the
    /// sandbox ends.
    leftovers: Arc<Mutex<Vec<u32>>>,
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
    },
    Ran {
        status: Option<i32>,
        timed_out: bool,
        restored: &'a [PathBuf],
        touched: &'a [PathBuf],
    },
}

impl Sandbox {
    pub(crate) fn new(helper: Helper, bounds: Bounds, audit: Option<Audit>) -> Self {
        Self {
            helper,
            bounds,
            audit,
            timeout: TIMEOUT,
            holder: Arc::default(),
            leftovers: Arc::default(),
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

    /// Runs the command `permit` allows.
    pub async fn run(&self, permit: Permit) -> Result<Ran, GateError> {
        let Action::Run {
            line,
            folder,
            network,
        } = permit.action
        else {
            return Err(GateError::NotACommand);
        };
        if !cfg!(any(target_os = "linux", target_os = "macos")) {
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
            })?;
        }
        let mut command = tokio::process::Command::new(&self.helper.program);
        command
            .args(&self.helper.args)
            .arg(policy)
            .arg("--")
            .args(["/bin/sh", "-c"])
            // Errors land with the output, in order.
            .arg(format!("exec 2>&1; {line}"))
            .current_dir(&folder)
            .env_clear()
            .envs(passed(std::env::vars_os()))
            .env("TMPDIR", own.path())
            .env("IQ_SANDBOX", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        if !network {
            // Without it cargo tries the network, and fails, on each build.
            command.env("CARGO_NET_OFFLINE", "true");
        }
        let watch = Watch::take(workspace);
        let child = command.spawn().map_err(GateError::Spawn)?;
        let group = child.id();
        let waited = tokio::time::timeout(self.timeout, child.wait_with_output()).await;
        if let Some(group) = group {
            if waited.is_err() {
                stop_group(group).await;
            } else if alive(group).await
                && let Ok(mut leftovers) = self.leftovers.lock()
            {
                leftovers.push(group);
            }
        }
        let settled = watch.settle();
        let (status, output, timed_out) = match waited {
            Ok(output) => {
                let output = output.map_err(GateError::Spawn)?;
                let helper = String::from_utf8_lossy(&output.stderr);
                if let Some(failure) = helper.strip_prefix(HELPER_PREFIX) {
                    return Err(GateError::Confine(failure.trim().to_owned()));
                }
                // What the shell says before its errors join the output, such
                // as a line it cannot parse, comes after.
                let printed = format!("{}{helper}", String::from_utf8_lossy(&output.stdout));
                (output.status.code(), printed, false)
            }
            Err(_) => (
                None,
                format!("stopped after {} seconds", self.timeout.as_secs()),
                true,
            ),
        };
        let redactor = Redactor::new(std::env::vars_os(), workspace);
        let ran = Ran {
            status,
            output: tail(&redactor.redact(&output), OUTPUT_BYTES),
            timed_out,
            restored: settled.restored,
            touched: settled.touched,
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

/// The variables of `env` a command gets.
fn passed(
    env: impl IntoIterator<Item = (OsString, OsString)>,
) -> impl Iterator<Item = (OsString, OsString)> {
    env.into_iter().filter(|(name, _)| {
        name.to_str().is_some_and(|name| {
            (PASSED.contains(&name) || name.starts_with("LC_")) && !secret_name(name)
        })
    })
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
    let (program, args) = match crate::confine::confine(&policy, program, args.collect()) {
        Ok(command) => command,
        Err(error) => return error,
    };
    GateError::Spawn(std::process::Command::new(program).args(args).exec())
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
