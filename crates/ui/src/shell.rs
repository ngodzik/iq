//! What a command line does, said in a few words read from the line
//! itself: the programs it runs and the files it names, never the body of
//! a script it feeds them. A long command shows in the conversation as
//! this, the line whole on the steps' page.

/// What a word of a command runs with the word after it, said together:
/// `cargo test`, `git commit`.
const WITH_THEIR_FIRST: [&str; 22] = [
    "cargo", "git", "npm", "pnpm", "yarn", "bun", "deno", "uv", "poetry", "go", "make", "docker",
    "podman", "kubectl", "helm", "gh", "dotnet", "mvn", "gradle", "pip", "pip3", "just",
];

/// What only sets up the commands after it, said through.
const SETTING_UP: [&str; 7] = ["sudo", "env", "time", "nice", "nohup", "exec", "command"];

/// The shell's own words before a command, said through: `do sleep 1`.
const LEADING: [&str; 9] = [
    "do", "then", "else", "elif", "if", "while", "until", "!", "time",
];

/// The shell's own words that make a whole part, said nothing of: a loop's
/// head, a block's end.
const SHELL_ONLY: [&str; 8] = ["for", "case", "select", "done", "fi", "esac", "}", ")"];

/// Commands that say nothing of what the line does.
const SAYING_NOTHING: [&str; 15] = [
    "cd", "set", "export", "echo", "printf", "true", "false", ":", "exit", "pushd", "popd",
    "source", "[", "[[", "test",
];

/// The endings of a word that names a file.
const FILES: [&str; 26] = [
    "rs", "py", "ts", "tsx", "js", "jsx", "go", "toml", "json", "yaml", "yml", "md", "sh", "txt",
    "lock", "cfg", "ini", "sql", "html", "css", "c", "h", "cpp", "java", "kt", "rb",
];

/// What `line` does, in at most `most` characters: the programs it runs,
/// in order, each once, then the files it names that none of them is;
/// cut at a word, `…` saying so.
///
/// # Examples
///
/// ```
/// let line = "cd crates/x && cat > src/hook.py <<'EOF'\nimport os\nEOF\ncargo fmt && cargo test -q";
/// assert_eq!(
///     iq_ui::shell::few_words(line, 80),
///     "cat, cargo fmt, cargo test · src/hook.py"
/// );
/// ```
#[must_use]
pub fn few_words(line: &str, most: usize) -> String {
    let mut programs: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    for command in commands(&without_heredocs(line)) {
        let words: Vec<String> = command.split_whitespace().map(unquoted).collect();
        let mut words = words.iter().map(String::as_str).peekable();
        // What sets a variable or only sets up the rest.
        while let Some(word) = words.peek() {
            let assigns = word.contains('=') && !word.starts_with('-') && !word.starts_with('=');
            if LEADING.contains(word) {
                words.next();
            } else if *word == "timeout" {
                // With how long it gives.
                words.next();
                words.next_if(|word| word.starts_with(|c: char| c.is_ascii_digit()));
            } else if assigns || SETTING_UP.contains(word) {
                words.next();
            } else {
                break;
            }
        }
        let Some(program) = words.next() else {
            continue;
        };
        if SAYING_NOTHING.contains(&program) || SHELL_ONLY.contains(&program) {
            continue;
        }
        let rest: Vec<&str> = words.collect();
        let mut said = program_name(program);
        if WITH_THEIR_FIRST.contains(&said.as_str())
            && let Some(first) = rest.iter().find(|word| !word.starts_with('-'))
        {
            said = format!("{said} {first}");
        } else if (said == "python" || said == "python3")
            && let Some(at) = rest.iter().position(|word| *word == "-m")
            && let Some(module) = rest.get(at + 1)
        {
            said = format!("{said} -m {module}");
        }
        if !programs.contains(&said) {
            programs.push(said);
        }
        for word in rest {
            let word = word.trim_start_matches(['<', '>']);
            if names_a_file(word) && !files.iter().any(|file| file == word) {
                files.push(word.to_owned());
            }
        }
    }
    files.retain(|file| {
        !programs
            .iter()
            .any(|program| program.ends_with(file.as_str()))
    });
    let mut said = programs.join(", ");
    if !files.is_empty() {
        if !said.is_empty() {
            said.push_str(" · ");
        }
        said.push_str(&files.join(", "));
    }
    cut_at_word(&said, most)
}

/// `line` without the bodies of the here-documents it feeds commands: what
/// they write is not what the line runs.
fn without_heredocs(line: &str) -> String {
    let mut kept = Vec::new();
    let mut until: Option<String> = None;
    for row in line.lines() {
        if let Some(end) = &until {
            if row.trim() == end {
                until = None;
            }
            continue;
        }
        kept.push(row);
        if let Some((_, after)) = row.split_once("<<") {
            let after = after.trim_start_matches('-').trim_start();
            let end: String = after
                .trim_start_matches(['\'', '"'])
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !end.is_empty() {
                until = Some(end);
            }
        }
    }
    kept.join("\n")
}

/// The simple commands of `line`, apart where the shell parts them: `&&`,
/// `||`, `;`, `|`, a new line.
fn commands(line: &str) -> Vec<String> {
    line.replace("&&", "\n")
        .replace("||", "\n")
        .replace([';', '|'], "\n")
        .lines()
        .map(|command| {
            command
                .trim()
                .trim_start_matches(['(', '{'])
                .trim()
                .to_owned()
        })
        .filter(|command| !command.is_empty())
        .collect()
}

/// `word` without the quotes around it.
fn unquoted(word: &str) -> String {
    word.trim_matches(|c| c == '\'' || c == '"').to_owned()
}

/// How a program is named: by its file alone, or, for a script of the
/// project's own, by its folder and file (`scripts/check.sh`).
fn program_name(program: &str) -> String {
    let program = program.trim_start_matches("./");
    let parts: Vec<&str> = program.split('/').filter(|part| !part.is_empty()).collect();
    match parts.as_slice() {
        [] => program.to_owned(),
        [one] => (*one).to_owned(),
        _ if program.starts_with('/') => parts[parts.len() - 1].to_owned(),
        _ => parts[parts.len() - 2..].join("/"),
    }
}

/// Whether `word` names a file, by its ending.
fn names_a_file(word: &str) -> bool {
    !word.starts_with('-')
        && !word.contains("://")
        && word
            .rsplit_once('.')
            .is_some_and(|(stem, ending)| !stem.is_empty() && FILES.contains(&ending))
}

/// `text` within `most` characters, cut at a word, `…` saying so.
fn cut_at_word(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        return text.to_owned();
    }
    let kept: String = text.chars().take(most.saturating_sub(1)).collect();
    let at = kept
        .rfind([' ', ','])
        .filter(|at| *at > most / 2)
        .unwrap_or(kept.len());
    format!("{}…", kept[..at].trim_end_matches([',', ' ', '·']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_script_is_said_by_what_it_runs_never_by_its_body() {
        let script = "set -e\ncd /work/repo\ncat > src/iq/hooks.py <<'PY'\nimport json\ndef hook(): pass\nPY\npython3 - <<EOF\nprint('patched')\nEOF\nuv run ruff format src && uv run pytest -q tests/test_hooks.py | tail -5\nscripts/check.sh";
        assert_eq!(
            few_words(script, 200),
            "cat, python3, uv run, tail, scripts/check.sh · src/iq/hooks.py, tests/test_hooks.py"
        );
        // Variables and what only sets up the command are not what it runs.
        assert_eq!(
            few_words("RUST_LOG=debug timeout 60 cargo test -p iq-ui", 80),
            "cargo test"
        );
        assert_eq!(
            few_words("sudo env A=1 python -m pytest -x", 80),
            "python -m pytest"
        );
        assert_eq!(few_words("echo done", 80), "");
        // The shell's own words are not programs.
        assert_eq!(
            few_words("for i in 1 2 3; do echo step $i; sleep 1; done", 80),
            "sleep"
        );
        assert_eq!(
            few_words("if [ -f x.toml ]; then cargo build; else make; fi", 80),
            "cargo build, make"
        );
    }

    #[test]
    fn what_is_too_long_is_cut_at_a_word() {
        let said = few_words("cargo fmt && cargo clippy && cargo test && cargo doc", 30);
        assert_eq!(said, "cargo fmt, cargo clippy…");
        assert!(said.chars().count() <= 30);
    }
}
