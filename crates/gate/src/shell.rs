//! A command line read as a shell splits it: the commands it runs, each as
//! its words, for rules to match.
//!
//! What a program does once started is not read here: the sandbox holds
//! that. Only the line's own syntax is, so that `cd $(git rev-parse
//! --show-toplevel) && git reset --hard` shows its `git reset --hard`.

/// What a line runs, as far as it can be read.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Line {
    /// Each command, its words unquoted, assignments and wrappers (`env`,
    /// `timeout 5`, `xargs`) left out, its program by its file's name.
    pub(crate) commands: Vec<Vec<String>>,
    /// Whether all of it was read: not when its syntax leaves to running it
    /// what runs (a variable as the program, `eval`, a script fed to a
    /// shell on its input).
    pub(crate) whole: bool,
}

/// How deep substitutions and `sh -c` are followed: past it, a line is not
/// read whole.
const DEEPEST: usize = 8;

/// Reads `line` into the commands it runs.
pub(crate) fn read(line: &str) -> Line {
    let mut out = Line {
        commands: Vec::new(),
        whole: true,
    };
    read_into(line, &mut out, 0);
    out
}

/// A word being read: its text, and whether an expansion makes it what
/// only running the line tells.
#[derive(Default)]
struct Word {
    text: String,
    started: bool,
    dynamic: bool,
}

/// The state of one command as it is read.
#[derive(Default)]
struct Command {
    words: Vec<(String, bool)>,
    word: Word,
    /// A here-document's delimiter, its body to skip at the next line.
    heredocs: Vec<(String, bool)>,
}

impl Command {
    fn end_word(&mut self) {
        let word = std::mem::take(&mut self.word);
        if word.started {
            self.words.push((word.text, word.dynamic));
        }
    }
}

fn read_into(text: &str, out: &mut Line, depth: usize) {
    if depth > DEEPEST {
        out.whole = false;
        return;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut command = Command::default();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => command.end_word(),
            '\\' => {
                // A newline escaped joins two lines.
                if chars.get(i + 1) == Some(&'\n') {
                    i += 1;
                } else if let Some(next) = chars.get(i + 1) {
                    command.push(*next);
                    i += 1;
                }
            }
            '\'' => {
                command.word.started = true;
                let end = find(&chars, i + 1, '\'');
                command.word.text.extend(&chars[i + 1..end]);
                i = end;
            }
            '"' => {
                command.word.started = true;
                i = double_quoted(&chars, i + 1, &mut command.word, out, depth);
            }
            '$' => i = dollar(&chars, i, &mut command.word, out, depth),
            '`' => {
                let end = backquoted_end(&chars, i + 1);
                let inner: String = chars[i + 1..end].iter().collect();
                read_into(&inner, out, depth + 1);
                command.word.started = true;
                command.word.dynamic = true;
                i = end;
            }
            '<' | '>' if chars.get(i + 1) == Some(&'(') => {
                let end = closing(&chars, i + 2);
                let inner: String = chars[i + 2..end.min(chars.len())].iter().collect();
                read_into(&inner, out, depth + 1);
                command.word.started = true;
                command.word.dynamic = true;
                i = end;
            }
            '<' if chars.get(i + 1) == Some(&'<') && chars.get(i + 2) != Some(&'<') => {
                command.end_word();
                i = heredoc(&chars, i + 2, &mut command);
                continue;
            }
            // `2>&1` and `&>file` redirect; they do not end the command.
            '&' if command.word.text.ends_with(['>', '<']) || chars.get(i + 1) == Some(&'>') => {
                command.push('&');
            }
            '#' if !command.word.started => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '\n' => {
                finish(&mut command, out, depth);
                i = skip_heredocs(&chars, i + 1, &mut command);
                continue;
            }
            ';' | '&' | '|' | '(' | ')' => finish(&mut command, out, depth),
            // A group, `{ make; }`, rather than a brace in a word.
            '{' if !command.word.started && chars.get(i + 1).is_some_and(|c| c.is_whitespace()) => {
                finish(&mut command, out, depth);
            }
            '}' if !command.word.started && command.words.is_empty() => {
                finish(&mut command, out, depth);
            }
            _ => command.push(c),
        }
        i += 1;
    }
    finish(&mut command, out, depth);
}

impl Command {
    fn push(&mut self, c: char) {
        self.word.started = true;
        self.word.text.push(c);
    }
}

/// The index of the next `quote` from `from`, else the end.
fn find(chars: &[char], from: usize, quote: char) -> usize {
    (from..chars.len())
        .find(|&j| chars[j] == quote)
        .unwrap_or(chars.len())
}

/// Reads a double-quoted string from `from`, past its opening quote, into
/// `word`; returns the index of its closing quote.
fn double_quoted(
    chars: &[char],
    from: usize,
    word: &mut Word,
    out: &mut Line,
    depth: usize,
) -> usize {
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            '"' => return i,
            '\\' if matches!(chars.get(i + 1), Some('"' | '\\' | '$' | '`')) => {
                word.text.push(chars[i + 1]);
                i += 1;
            }
            '$' => {
                i = dollar(chars, i, word, out, depth);
            }
            '`' => {
                let end = backquoted_end(chars, i + 1);
                let inner: String = chars[i + 1..end].iter().collect();
                read_into(&inner, out, depth + 1);
                word.dynamic = true;
                i = end;
            }
            c => word.text.push(c),
        }
        i += 1;
    }
    chars.len()
}

/// Reads what a `$` at `at` starts: a substitution, read as commands of its
/// own, or a variable; returns the index of its last character.
fn dollar(chars: &[char], at: usize, word: &mut Word, out: &mut Line, depth: usize) -> usize {
    word.started = true;
    match chars.get(at + 1) {
        Some('(') if chars.get(at + 2) == Some(&'(') => {
            // Arithmetic: a number, no command.
            word.dynamic = true;
            closing(chars, at + 3).saturating_add(1).min(chars.len())
        }
        Some('(') => {
            let end = closing(chars, at + 2);
            let inner: String = chars[at + 2..end.min(chars.len())].iter().collect();
            read_into(&inner, out, depth + 1);
            word.dynamic = true;
            end
        }
        Some('{') => {
            word.dynamic = true;
            find(chars, at + 2, '}')
        }
        Some(&c)
            if c.is_ascii_alphanumeric()
                || matches!(c, '_' | '@' | '*' | '#' | '?' | '$' | '!' | '-') =>
        {
            word.dynamic = true;
            let mut end = at + 1;
            if c.is_ascii_alphabetic() || c == '_' {
                while chars
                    .get(end + 1)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
                {
                    end += 1;
                }
            }
            end
        }
        _ => {
            word.text.push('$');
            at
        }
    }
}

/// The index of the `)` that closes a parenthesis opened before `from`,
/// quotes skipped, else the end.
fn closing(chars: &[char], from: usize) -> usize {
    let mut depth = 1;
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '\'' => i = find(chars, i + 1, '\''),
            '"' => {
                let mut j = i + 1;
                while j < chars.len() && chars[j] != '"' {
                    if chars[j] == '\\' {
                        j += 1;
                    }
                    j += 1;
                }
                i = j;
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    chars.len()
}

/// The index of the backquote that closes one opened before `from`.
fn backquoted_end(chars: &[char], from: usize) -> usize {
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '`' => return i,
            _ => {}
        }
        i += 1;
    }
    chars.len()
}

/// Reads a here-document's delimiter after `<<` at `from`, keeping it for
/// its body to be skipped; returns where the line goes on.
fn heredoc(chars: &[char], from: usize, command: &mut Command) -> usize {
    let mut i = from;
    let tabs = chars.get(i) == Some(&'-');
    if tabs {
        i += 1;
    }
    while chars.get(i) == Some(&' ') {
        i += 1;
    }
    let mut delimiter = String::new();
    while let Some(&c) = chars.get(i) {
        if c.is_whitespace() || matches!(c, ';' | '&' | '|' | ')' | '<' | '>') {
            break;
        }
        if !matches!(c, '\'' | '"' | '\\') {
            delimiter.push(c);
        }
        i += 1;
    }
    command.heredocs.push((delimiter, tabs));
    i
}

/// Skips the bodies of the here-documents opened on the line that ended
/// before `from`; returns where the next line starts.
fn skip_heredocs(chars: &[char], from: usize, command: &mut Command) -> usize {
    let mut i = from;
    for (delimiter, tabs) in std::mem::take(&mut command.heredocs) {
        loop {
            if i >= chars.len() {
                return i;
            }
            let end = find(chars, i, '\n');
            let line: String = chars[i..end].iter().collect();
            i = end + 1;
            let line = if tabs {
                line.trim_start_matches('\t')
            } else {
                &line
            };
            if line == delimiter {
                break;
            }
        }
    }
    i
}

/// Ends the command being read: its words, assignments and wrappers left
/// out, go to `out`, and what it runs in turn (`sh -c '...'`) is read.
fn finish(command: &mut Command, out: &mut Line, depth: usize) {
    command.end_word();
    let fed = !command.heredocs.is_empty();
    let words = std::mem::take(&mut command.words);
    let words = unwrapped(&words);
    let Some((program, dynamic)) = words.first() else {
        return;
    };
    if *dynamic {
        out.whole = false;
    }
    let program = program.rsplit('/').next().unwrap_or(program).to_owned();
    let mut plain: Vec<String> = Vec::with_capacity(words.len());
    plain.push(program.clone());
    let args = if program == "git" {
        git_options(&words[1..])
    } else {
        &words[1..]
    };
    plain.extend(args.iter().map(|(word, _)| word.clone()));
    match program.as_str() {
        "eval" | "source" | "." => out.whole = false,
        "sh" | "bash" | "zsh" | "dash" | "ksh" => match script(&words[1..]) {
            Some(Some(script)) => read_into(&script, out, depth + 1),
            // A script made as the line runs, in a file, or on the input.
            _ => out.whole = false,
        },
        _ if fed
            && matches!(
                program.as_str(),
                "python" | "python3" | "node" | "ruby" | "perl"
            ) =>
        {
            out.whole = false;
        }
        _ => {}
    }
    out.commands.push(plain);
}

/// What a shell's `-c` gives it to run: `Some(Some(text))` when it is read,
/// `Some(None)` when only running tells, `None` when there is no `-c`.
fn script(args: &[(String, bool)]) -> Option<Option<String>> {
    let at = args.iter().position(|(word, _)| {
        word.starts_with('-') && !word.starts_with("--") && word.contains('c')
    })?;
    Some(
        args.get(at + 1)
            .filter(|(_, dynamic)| !dynamic)
            .map(|(word, _)| word.clone()),
    )
}

/// `words` without the assignments before the program and the wrappers
/// that only run the program after them.
fn unwrapped(words: &[(String, bool)]) -> &[(String, bool)] {
    let mut rest = words;
    loop {
        let Some(((first, _), tail)) = rest.split_first() else {
            return rest;
        };
        if assignment(first) {
            rest = tail;
            continue;
        }
        let valued: &[&str] = match first.as_str() {
            "env" => &["-u", "-C", "-S"],
            "nice" => &["-n"],
            "timeout" => &["-s", "-k"],
            "xargs" => &["-n", "-I", "-P", "-L", "-s", "-d", "-E", "-a"],
            "stdbuf" => &["-i", "-o", "-e"],
            "command" | "builtin" | "exec" | "nohup" | "time" => &[],
            _ => return rest,
        };
        let mut i = 1;
        while let Some((word, _)) = tail.get(i - 1) {
            if valued.contains(&word.as_str()) {
                i += 2;
            } else if word.starts_with('-') || (first == "env" && assignment(word)) {
                i += 1;
            } else {
                break;
            }
        }
        // `timeout 5 cargo test`: its duration.
        if first == "timeout"
            && tail
                .get(i - 1)
                .is_some_and(|(word, _)| word.starts_with(|c: char| c.is_ascii_digit()))
        {
            i += 1;
        }
        rest = tail.get(i - 1..).unwrap_or_default();
    }
}

/// Git's arguments without its own options before its command, which
/// change where or how it works but not what it does: `git -C dir reset
/// --hard` resets as `git reset --hard` does, and is judged so.
fn git_options(args: &[(String, bool)]) -> &[(String, bool)] {
    let mut i = 0;
    while let Some((word, _)) = args.get(i) {
        i += match word.as_str() {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--config-env" => 2,
            option if option.starts_with('-') => 1,
            _ => break,
        };
    }
    args.get(i..).unwrap_or_default()
}

/// Whether `word` sets a variable for the command, `NAME=value`.
fn assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each command as a line, the words only running tells left out.
    fn commands(line: &str) -> Vec<String> {
        read(line)
            .commands
            .iter()
            .map(|words| {
                let words: Vec<&str> = words
                    .iter()
                    .map(String::as_str)
                    .filter(|word| !word.is_empty())
                    .collect();
                words.join(" ")
            })
            .collect()
    }

    #[test]
    fn a_line_is_split_into_its_commands() {
        assert_eq!(
            commands("cargo test && git status; ls -la | head -5 || true"),
            ["cargo test", "git status", "ls -la", "head -5", "true"]
        );
        assert_eq!(
            commands("cargo build 2>&1 | tail -3 &> /dev/null"),
            ["cargo build 2>&1", "tail -3 &> /dev/null"]
        );
        assert_eq!(commands("echo 'a && b' \"c; d\""), ["echo a && b c; d"]);
        assert_eq!(commands("(cd sub && make) # done"), ["cd sub", "make"]);
        assert!(read("cargo test && git status").whole);
    }

    #[test]
    fn what_runs_within_is_read_too() {
        assert_eq!(
            commands("cd $(git rev-parse --show-toplevel) && git reset --hard"),
            ["git rev-parse --show-toplevel", "cd", "git reset --hard"]
        );
        assert_eq!(commands("echo \"`git push`\""), ["git push", "echo"]);
        assert_eq!(
            commands("diff <(git show HEAD:a) a"),
            ["git show HEAD:a", "diff a"]
        );
        assert_eq!(
            commands("bash -lc 'git push origin main'"),
            ["git push origin main", "bash -lc git push origin main"]
        );
        assert_eq!(commands("echo $((1 + 2))"), ["echo"]);
    }

    #[test]
    fn wrappers_and_assignments_are_left_out() {
        assert_eq!(
            commands("RUST_LOG=debug timeout 30 env -u X FOO=1 /usr/bin/git push"),
            ["git push"]
        );
        assert_eq!(
            commands("find . -name x | xargs -n 1 rm"),
            ["find . -name x", "rm"]
        );
        assert_eq!(commands("sudo rm -rf /"), ["sudo rm -rf /"]);
        // Git's own options do not hide its command.
        assert_eq!(
            commands("git -C /w -c user.name=x --no-pager --git-dir=.git reset --hard"),
            ["git reset --hard"]
        );
        assert_eq!(commands("git --work-tree w push"), ["git push"]);
    }

    #[test]
    fn a_here_document_is_data() {
        let line = "cat <<'EOF' > notes.md\ngit push\nEOF\ngit status";
        assert_eq!(commands(line), ["cat > notes.md", "git status"]);
        assert!(read(line).whole);
        assert!(!read("python3 <<EOF\nimport os\nEOF").whole);
    }

    #[test]
    fn what_only_running_tells_is_not_read_whole() {
        assert!(!read("$TOOL --version").whole);
        assert!(!read("eval \"$(ssh-agent)\"").whole);
        assert!(!read("bash setup.sh").whole);
        assert!(!read("curl -s x | sh").whole);
        assert!(!read("sh -c \"$SCRIPT\"").whole);
        assert!(read("echo $HOME").whole);
    }
}
