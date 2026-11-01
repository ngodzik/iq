//! The Mermaid diagrams agents write, drawn as images for Kitty by the
//! program the settings name (`mermaid`), off unless they do. A diagram is
//! written to a folder of its own in iq's cache, with nothing else, and the
//! program run there in iq's sandbox: no network, nothing written but in
//! that folder, stopped past a minute. Its source is model-written, so it
//! is drawn as any command an agent asks for would run, judged by the gate
//! and written to the audit log. Each image is kept, so that the same
//! diagram is drawn once.

use std::path::{Path, PathBuf};
use std::time::Duration;

use iq_gate::{Action, Gate, Helper, Judged};
use iq_tui::{Draw, Drawn};

/// The program `mermaid = "auto"` draws with, and how: a single binary that
/// draws without a browser.
const AUTO: [&str; 7] = ["mmdr", "-i", "{input}", "-o", "{output}", "-e", "png"];

/// What `mermaid = "auto"` says when its program is not installed.
pub(crate) const NO_MMDR: &str = "mermaid = \"auto\" draws diagrams with mmdr, which is not on the PATH: \
                                  cargo install mermaid-rs-renderer";

/// How long a diagram may take to draw.
const DRAW_WITHIN: Duration = Duration::from_secs(60);

/// How long the program may write nothing.
const QUIET_WITHIN: Duration = Duration::from_secs(30);

/// The largest image taken, in bytes.
const MOST_BYTES: u64 = 16 << 20;

/// The source's file and the image's, in a diagram's folder.
const INPUT: &str = "diagram.mmd";
const OUTPUT: &str = "diagram.png";

/// What draws the diagrams: the program and its arguments, `{input}` and
/// `{output}` standing for the files, where the drawn ones are kept, what
/// the gate is given, and the runtime the sandbox runs in.
pub(crate) struct Diagrams {
    command: Vec<String>,
    cache: PathBuf,
    home: PathBuf,
    audit: Option<PathBuf>,
    runtime: tokio::runtime::Handle,
}

/// The command `mermaid` names: its own words, or what `auto` finds on the
/// `PATH` (`installed` says whether a program is); none when `auto` finds
/// nothing.
pub(crate) fn command(
    mermaid: &crate::settings::Mermaid,
    installed: impl Fn(&str) -> bool,
) -> Option<Vec<String>> {
    match mermaid {
        crate::settings::Mermaid::Auto => {
            installed(AUTO[0]).then(|| AUTO.iter().map(|w| (*w).to_owned()).collect())
        }
        crate::settings::Mermaid::Command(words) => Some(words.clone()),
    }
}

/// Whether `program` is a file in one of the folders of the `PATH`.
pub(crate) fn installed(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// `word` as a shell reads it back as one word.
fn quoted(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:={}+,@%".contains(c));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// The key of a diagram: the same for the same source drawn the same way,
/// from one session to the next.
fn key(command: &[String], source: &str) -> u64 {
    // FNV-1a: stable across versions, unlike the standard hasher.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in command.iter().map(String::as_str).chain([source]) {
        for byte in part.bytes().chain([0]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

impl Diagrams {
    /// Draws with `command`, keeping what it drew in `cache`, its runs
    /// judged by a gate of the person's `home`, written to `audit`.
    pub(crate) fn new(
        command: Vec<String>,
        cache: PathBuf,
        home: PathBuf,
        audit: Option<PathBuf>,
        runtime: tokio::runtime::Handle,
    ) -> Diagrams {
        Diagrams {
            command,
            cache,
            home,
            audit,
            runtime,
        }
    }

    /// The command line run in `folder`, its files named there.
    fn line(&self) -> String {
        self.command
            .iter()
            .map(|word| quoted(&word.replace("{input}", INPUT).replace("{output}", OUTPUT)))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Runs the program in `folder`, in the sandbox.
    fn run(&self, folder: &Path) -> Result<(), String> {
        let mut gate = Gate::builder(folder).home(Some(self.home.clone()));
        if let Some(audit) = &self.audit {
            gate = gate.audit(audit);
        }
        let gate = gate.build().map_err(|e| e.to_string())?;
        let action = Action::Run {
            line: self.line(),
            folder: folder.to_path_buf(),
            network: false,
            host: false,
        };
        let permit = match gate.judge(action).map_err(|e| e.to_string())? {
            Judged::Allowed(permit) => permit,
            Judged::Asked { reason, .. } | Judged::Denied { reason } => {
                return Err(format!(
                    "the gate does not let it run in the sandbox: {reason}"
                ));
            }
        };
        let helper = Helper::this_program().map_err(|e| e.to_string())?;
        let sandbox = gate
            .sandbox(helper)
            .with_timeout(DRAW_WITHIN)
            .with_quiet(QUIET_WITHIN);
        let ran = self
            .runtime
            .block_on(sandbox.run(permit))
            .map_err(|e| e.to_string())?;
        if ran.timed_out {
            return Err("it took more than a minute".to_owned());
        }
        if ran.status != Some(0) {
            let said: Vec<&str> = ran
                .output
                .lines()
                .filter(|l| !l.trim().is_empty())
                .collect();
            let last = said[said.len().saturating_sub(3)..].join(" / ");
            return Err(format!(
                "{} ended with status {}: {last}",
                self.command.first().map_or("it", String::as_str),
                ran.status.map_or("none".to_owned(), |s| s.to_string())
            ));
        }
        Ok(())
    }
}

impl Draw for Diagrams {
    fn draw(&self, source: &str) -> Result<Drawn, String> {
        let folder = self
            .cache
            .join(format!("{:016x}", key(&self.command, source)));
        let (input, output) = (folder.join(INPUT), folder.join(OUTPUT));
        if !output.is_file() {
            std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
            std::fs::write(&input, source).map_err(|e| format!("{}: {e}", input.display()))?;
            self.run(&folder)?;
        }
        let size = std::fs::metadata(&output)
            .map_err(|_| format!("it wrote no {OUTPUT}"))?
            .len();
        if size > MOST_BYTES {
            return Err(format!("its image is {size} bytes, more than iq shows"));
        }
        let png = std::fs::read(&output).map_err(|e| format!("{}: {e}", output.display()))?;
        Ok(Drawn {
            png,
            source: Some(input),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Mermaid;

    #[test]
    fn auto_draws_with_mmdr_alone_and_a_command_as_written() {
        let found = command(&Mermaid::Auto, |p| p == "mmdr").unwrap();
        assert_eq!(
            found,
            ["mmdr", "-i", "{input}", "-o", "{output}", "-e", "png"]
        );
        // Nothing heavier is looked for.
        assert_eq!(command(&Mermaid::Auto, |p| p == "mmdc"), None);
        let own = Mermaid::Command(vec!["draw".into(), "{input}".into(), "{output}".into()]);
        assert_eq!(
            command(&own, |_| false).unwrap(),
            ["draw", "{input}", "{output}"]
        );
    }

    #[test]
    fn the_line_names_the_folders_files_and_quotes_what_a_shell_would_split() {
        let diagrams = Diagrams::new(
            vec![
                "draw".into(),
                "-i".into(),
                "{input}".into(),
                "--title".into(),
                "a b".into(),
            ],
            PathBuf::new(),
            PathBuf::new(),
            None,
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .handle()
                .clone(),
        );
        assert_eq!(diagrams.line(), "draw -i diagram.mmd --title 'a b'");
        assert_eq!(quoted("it's"), r"'it'\''s'");
        assert_ne!(key(&["a".into()], "x"), key(&["a".into()], "y"));
    }
}
