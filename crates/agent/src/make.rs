//! What a `make` target does, read from the project's Makefiles for the
//! reviewer: a target is named for what it means, and judged by its name
//! alone, `make down` stops the services and `make clean` tidies up; its
//! recipe says what it does, which may be to delete the database's data.
//!
//! In a project of several parts, make runs in a folder of its own (`cd
//! backend && make migrate`, `make -C backend migrate`), and a recipe
//! hands its work to a folder's Makefile (`$(MAKE) -C backend migrate`):
//! each is followed into the Makefile that makes the target.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Component, Path, PathBuf};

/// The most targets whose recipes are given, those asked and what they
/// depend on or hand their work to, whatever Makefile has them.
const MOST: usize = 8;

/// The most lines of one recipe given.
const LINES: usize = 20;

/// The Makefiles make looks for in a folder, in its order.
const NAMES: [&str; 3] = ["GNUmakefile", "makefile", "Makefile"];

/// The words a command may start with before the program it runs.
const BEFORE: [&str; 5] = ["env", "exec", "command", "time", "nohup"];

/// The options of make that take the next word as their value.
const VALUED: [&str; 8] = [
    "-I",
    "-o",
    "-W",
    "--include-dir",
    "--old-file",
    "--assume-old",
    "--new-file",
    "--what-if",
];

/// One `make` a line runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Make {
    /// The folder it runs in, relative to where the line runs.
    pub(crate) folder: PathBuf,
    /// The Makefile it is told to read, relative to where the line runs;
    /// when none, the first of [`NAMES`] in its folder.
    pub(crate) file: Option<PathBuf>,
    /// The targets it makes; none for its Makefile's first.
    pub(crate) targets: Vec<String>,
}

/// The `make` commands of `line`, each with the folder it runs in: the
/// line's commands read in turn, the variables set before one skipped, a
/// `cd` moving where those after it run (within its brackets, for a
/// subshell's), and make's `-C` and `-f` read as where it runs and what it
/// reads. A make whose folder or file leaves where the line runs, or names
/// what only the shell knows (a variable, `~`), is left out: its Makefile
/// is none of the project's to read.
pub(crate) fn makes(line: &str) -> Vec<Make> {
    makes_from(line, Path::new(""))
}

/// The `make` commands of `line`, run from `here`, as [`makes`] reads
/// them.
fn makes_from(line: &str, here: &Path) -> Vec<Make> {
    let mut here = Some(here.to_path_buf());
    let mut outer: Vec<Option<PathBuf>> = Vec::new();
    let mut makes = Vec::new();
    for piece in pieces(line) {
        let command = match piece {
            Piece::Open => {
                outer.push(here.clone());
                continue;
            }
            Piece::Close => {
                if let Some(before) = outer.pop() {
                    here = before;
                }
                continue;
            }
            Piece::Command(words) => words,
        };
        let mut words = command
            .iter()
            .map(String::as_str)
            .skip_while(|word| BEFORE.contains(word) || assignment(word));
        let Some(program) = words.next() else {
            continue;
        };
        let rest: Vec<&str> = words.collect();
        if program == "cd" {
            here = match rest.first() {
                Some(folder) => here.as_deref().and_then(|here| joined(here, folder)),
                // Home, which is no folder of the project's.
                None => None,
            };
        } else if is_make(program)
            && let Some(from) = &here
            && let Some(make) = read_make(from, &rest)
        {
            makes.push(make);
        }
    }
    makes
}

/// Whether `program` is make: by its name, wherever it lies, or as a
/// recipe calls it again.
fn is_make(program: &str) -> bool {
    program == "$(MAKE)"
        || program == "${MAKE}"
        || Path::new(program)
            .file_name()
            .is_some_and(|name| name == "make" || name == "gmake")
}

/// What a make run from `here` with `args` does: where it runs, what it
/// reads and what it makes. None when it runs or reads out of reach.
fn read_make(here: &Path, args: &[&str]) -> Option<Make> {
    let mut folder = here.to_path_buf();
    let mut file: Option<String> = None;
    let mut targets = Vec::new();
    let mut words = args.iter().copied().peekable();
    while let Some(word) = words.next() {
        let attached = |long: &str, short: &str| {
            word.strip_prefix(long)
                .or_else(|| word.strip_prefix(short).filter(|v| !v.is_empty()))
                .map(str::to_owned)
        };
        if word == "-C" || word == "--directory" {
            folder = joined(&folder, words.next()?)?;
        } else if let Some(value) = attached("--directory=", "-C") {
            folder = joined(&folder, &value)?;
        } else if ["-f", "--file", "--makefile"].contains(&word) {
            file = Some(words.next()?.to_owned());
        } else if let Some(value) = attached("--file=", "-f")
            .or_else(|| word.strip_prefix("--makefile=").map(str::to_owned))
        {
            file = Some(value);
        } else if VALUED.contains(&word) {
            words.next();
        } else if ["-j", "--jobs", "-l", "--load-average"].contains(&word) {
            // Their value is optional: a number, when one follows.
            words.next_if(|next| next.parse::<f64>().is_ok());
        } else if word
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .starts_with(['>', '<'])
        {
            // A redirection, its file after it when not in the same word.
            if word
                .trim_start_matches(|c: char| c.is_ascii_digit() || "<>&".contains(c))
                .is_empty()
            {
                words.next();
            }
        } else if !word.starts_with('-') && !word.contains('=') {
            targets.push(word.to_owned());
        }
    }
    // Read once make is in its folder, as make reads it.
    let file = match file {
        Some(file) => Some(joined(&folder, &file)?),
        None => None,
    };
    Some(Make {
        folder,
        file,
        targets,
    })
}

/// A piece of a shell line.
#[derive(Debug, PartialEq, Eq)]
enum Piece {
    /// A command, as its words.
    Command(Vec<String>),
    /// A subshell opens: what moves within it stays within it.
    Open,
    /// It closes.
    Close,
}

/// The pieces of `line`: its commands, split where the shell runs another
/// (`&&`, `||`, `;`, `|`, `&`, a new line), and its subshells' brackets;
/// quotes taken off the words they keep whole, and a variable kept whole
/// with its brackets.
fn pieces(line: &str) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut quoted: Option<char> = None;
    let mut started = false;
    let end_word = |word: &mut String, started: &mut bool, words: &mut Vec<String>| {
        if *started {
            words.push(std::mem::take(word));
            *started = false;
        }
    };
    let end_command = |words: &mut Vec<String>, pieces: &mut Vec<Piece>| {
        if !words.is_empty() {
            pieces.push(Piece::Command(std::mem::take(words)));
        }
    };
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match quoted {
            Some(quote) if c == quote => quoted = None,
            Some(_) => word.push(c),
            // A variable or a command's output, `$(MAKE)`, `${DIR}`, is one
            // word with its brackets.
            None if c == '$' && chars.peek().is_some_and(|next| "({".contains(*next)) => {
                let mut depth = 0;
                word.push(c);
                for c in chars.by_ref() {
                    word.push(c);
                    match c {
                        '(' | '{' => depth += 1,
                        ')' | '}' => depth -= 1,
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                }
                started = true;
            }
            None if c == '\'' || c == '"' => {
                quoted = Some(c);
                started = true;
            }
            None if c.is_whitespace() && c != '\n' => {
                end_word(&mut word, &mut started, &mut words);
            }
            None if ";&|()\n".contains(c) => {
                end_word(&mut word, &mut started, &mut words);
                end_command(&mut words, &mut pieces);
                match c {
                    '(' => pieces.push(Piece::Open),
                    ')' => pieces.push(Piece::Close),
                    _ => {}
                }
            }
            None => {
                word.push(c);
                started = true;
            }
        }
    }
    end_word(&mut word, &mut started, &mut words);
    end_command(&mut words, &mut pieces);
    pieces
}

/// Whether `word` sets a variable for the command after it: `NAME=value`.
fn assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// `more` taken from `base`, both relative to where the line runs; none
/// when it leaves it (an absolute path, `..` past it) or names what only
/// the shell knows.
fn joined(base: &Path, more: &str) -> Option<PathBuf> {
    if more.is_empty() || more.contains('$') || more.starts_with('~') || more == "-" {
        return None;
    }
    let mut out = base.to_path_buf();
    for part in Path::new(more).components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(name) => out.push(name),
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

/// The recipes of the targets `line` makes, each from the Makefile that
/// makes it, under that Makefile's path, with what it depends on and
/// those of the Makefiles its recipe hands its work to, as far as
/// [`MOST`] targets; `None` when no Makefile has them. Each Makefile is
/// read by `read`, from a path relative to where the line runs, which
/// gives its text when it is there and may be read.
pub(crate) fn recipes(line: &str, read: impl Fn(&Path) -> Option<String>) -> Option<String> {
    let mut queue: VecDeque<(Make, Option<String>)> = VecDeque::new();
    for make in makes(line) {
        queue_make(&mut queue, make);
    }
    let mut files: BTreeMap<PathBuf, Option<Makefile>> = BTreeMap::new();
    let mut given: Vec<(PathBuf, String)> = Vec::new();
    let mut out = String::new();
    let mut last: Option<PathBuf> = None;
    while let Some((make, target)) = queue.pop_front() {
        if given.len() >= MOST {
            break;
        }
        let candidates: Vec<PathBuf> = match &make.file {
            Some(file) => vec![file.clone()],
            None => NAMES.iter().map(|name| make.folder.join(name)).collect(),
        };
        let mut found = None;
        for path in candidates {
            let makefile = files
                .entry(path.clone())
                .or_insert_with(|| read(&path).map(|text| parse(&text)));
            if makefile.is_some() {
                found = Some(path);
                break;
            }
        }
        let Some(path) = found else {
            continue;
        };
        let Some(Some(makefile)) = files.get(&path) else {
            continue;
        };
        let Some(target) = target.or_else(|| makefile.first.clone()) else {
            continue;
        };
        if given.iter().any(|(p, t)| *p == path && *t == target) {
            continue;
        }
        let Some((needs, recipe)) = makefile.rules.get(&target) else {
            continue;
        };
        given.push((path.clone(), target.clone()));
        if last.as_ref() != Some(&path) {
            out.push_str(&format!("# {}\n", path.display()));
            last = Some(path.clone());
        }
        let needed = if needs.is_empty() {
            String::new()
        } else {
            format!(" {}", needs.join(" "))
        };
        out.push_str(&format!("{target}:{needed}\n"));
        for line in recipe.iter().take(LINES) {
            out.push_str(&format!("    {line}\n"));
        }
        if recipe.len() > LINES {
            out.push_str(&format!("    [{} more lines]\n", recipe.len() - LINES));
        }
        for need in needs {
            queue.push_back((make.clone(), Some(need.clone())));
        }
        // A recipe runs where its make does; one that runs make again
        // hands it the work.
        for line in recipe.iter().take(LINES) {
            let command = makefile.expand(line.trim_start_matches(['@', '-', '+', ' ', '\t']));
            for handed in makes_from(&command, &make.folder) {
                queue_make(&mut queue, handed);
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Queues the targets `make` makes, or its Makefile's first when it names
/// none.
fn queue_make(queue: &mut VecDeque<(Make, Option<String>)>, make: Make) {
    if make.targets.is_empty() {
        queue.push_back((make, None));
        return;
    }
    for target in &make.targets {
        queue.push_back((make.clone(), Some(target.clone())));
    }
}

/// A Makefile as read for its recipes.
#[derive(Debug, Default)]
struct Makefile {
    /// Each target, with what it depends on and its recipe.
    rules: BTreeMap<String, (Vec<String>, Vec<String>)>,
    /// The target make makes when it is named none.
    first: Option<String>,
    /// The variables set to a plain value, which a recipe's folder may
    /// name.
    variables: BTreeMap<String, String>,
}

impl Makefile {
    /// `command` with the plain variables it names written out, and
    /// `$(CURDIR)` as the folder its make runs in.
    fn expand(&self, command: &str) -> String {
        let mut out = command.replace("$(CURDIR)", ".").replace("${CURDIR}", ".");
        for (name, value) in &self.variables {
            out = out
                .replace(&format!("$({name})"), value)
                .replace(&format!("${{{name}}}"), value);
        }
        out
    }
}

/// `makefile` read: each target with what it depends on and its recipe,
/// the first target, and the variables set to a plain value. Other
/// variables are left as written: what a recipe does is read from it as
/// it is.
fn parse(makefile: &str) -> Makefile {
    let mut read = Makefile::default();
    let mut current: Vec<String> = Vec::new();
    for line in makefile.lines() {
        if let Some(command) = line.strip_prefix('\t') {
            let command = command.trim_end();
            if !command.is_empty() {
                for target in &current {
                    if let Some((_, recipe)) = read.rules.get_mut(target) {
                        recipe.push(command.to_owned());
                    }
                }
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((name, value)) = variable(trimmed) {
            if let Some(value) = value.filter(|value| !value.contains('$')) {
                read.variables.insert(name.to_owned(), value.to_owned());
            }
            current.clear();
            continue;
        }
        // A rule: targets, a colon, what they depend on. Not a target with
        // a pattern or a variable, which names no one target.
        let Some((heads, rest)) = trimmed.split_once(':') else {
            current.clear();
            continue;
        };
        if rest.starts_with('=') || heads.contains(['=', '%', '$']) {
            current.clear();
            continue;
        }
        let rest = rest.strip_prefix(':').unwrap_or(rest);
        let needs: Vec<String> = rest
            .split('#')
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        current = heads
            .split_whitespace()
            .filter(|head| !head.starts_with('.'))
            .map(str::to_owned)
            .collect();
        if read.first.is_none() {
            read.first = current.first().cloned();
        }
        for target in &current {
            read.rules
                .entry(target.clone())
                .or_insert_with(|| (needs.clone(), Vec::new()));
        }
    }
    read
}

/// The variable `line` sets, when it sets one, and its value when it is
/// set to one (`NAME = value`, `NAME := value`, `NAME ?= value`), not added
/// to (`NAME += value`).
fn variable(line: &str) -> Option<(&str, Option<&str>)> {
    let at = line.find(['=', ':', '?', '+', ' ', '\t'])?;
    let (name, rest) = line.split_at(at);
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
    {
        return None;
    }
    let rest = rest.trim_start();
    if rest.starts_with("+=") {
        return Some((name, None));
    }
    let value = ["::=", ":=", "?=", "="]
        .iter()
        .find_map(|op| rest.strip_prefix(op))?;
    Some((name, Some(value.trim())))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAKEFILE: &str = "\
# The project's services.
COMPOSE := docker compose
.PHONY: up down up-db

up: ## start
\t$(COMPOSE) up -d

down: ## stop
\t$(COMPOSE) down --volumes --remove-orphans

up-db: down
\t$(COMPOSE) up -d db
\t./scripts/wait-db.sh

%.o: %.c
\tcc -c $<
";

    /// What `line` is told to do, with these Makefiles, by their paths.
    fn told(line: &str, files: &[(&str, &str)]) -> Option<String> {
        recipes(line, |path| {
            files
                .iter()
                .find(|(name, _)| Path::new(name) == path)
                .map(|(_, text)| (*text).to_owned())
        })
    }

    /// The folder and the targets of each make of `line`.
    fn targets(line: &str) -> Vec<(String, Vec<String>)> {
        makes(line)
            .into_iter()
            .map(|make| (make.folder.display().to_string(), make.targets))
            .collect()
    }

    fn one(folder: &str, targets: &[&str]) -> (String, Vec<String>) {
        (
            folder.to_owned(),
            targets.iter().map(|t| (*t).to_owned()).collect(),
        )
    }

    #[test]
    fn each_make_of_a_line_is_read_with_the_folder_it_runs_in() {
        assert_eq!(
            targets("make down && make up-db"),
            [one("", &["down"]), one("", &["up-db"])]
        );
        assert_eq!(
            targets("make -j4 up-db VERBOSE=1 test; echo ok"),
            [one("", &["up-db", "test"])]
        );
        assert_eq!(targets("make -j 4 up > log 2>&1"), [one("", &["up"])]);
        assert_eq!(targets("/usr/bin/gmake up"), [one("", &["up"])]);
        assert_eq!(
            targets("cd backend && make migrate"),
            [one("backend", &["migrate"])]
        );
        assert_eq!(
            targets("UV_NO_SYNC=1 make migrate"),
            [one("", &["migrate"])]
        );
        assert_eq!(
            targets("cd backend; env UV_NO_SYNC=1 make -C db --directory=up gen"),
            [one("backend/db/up", &["gen"])]
        );
        assert_eq!(
            targets("make -Cbackend -s generate | tee log"),
            [one("backend", &["generate"])]
        );
        // What moves in a subshell stays there.
        assert_eq!(
            targets("(cd 'front end' && make build)&&make"),
            [one("front end", &["build"]), one("", &[])]
        );
        // Out of the line's folder, or where only the shell knows.
        assert!(targets("cd .. && make up").is_empty());
        assert!(targets("make -C /etc up").is_empty());
        assert!(targets("cd $DIR && make up").is_empty());
        assert!(targets("cd && make up").is_empty());
        assert!(targets("cargo make down").is_empty());
        let file = &makes("make -C ops -f ci/Makefile.ci check")[0];
        assert_eq!(file.file.as_deref(), Some(Path::new("ops/ci/Makefile.ci")));
    }

    #[test]
    fn a_target_is_given_with_its_recipe_and_what_it_depends_on() {
        let given = told("make up-db", &[("Makefile", MAKEFILE)]).unwrap();
        assert_eq!(
            given,
            "# Makefile\nup-db: down\n    $(COMPOSE) up -d db\n    ./scripts/wait-db.sh\ndown:\n    \
             $(COMPOSE) down --volumes --remove-orphans\n"
        );
        assert!(told("make nothing", &[("Makefile", MAKEFILE)]).is_none());
        // A pattern rule names no target.
        assert!(told("make x.o", &[("Makefile", MAKEFILE)]).is_none());
        // Named none, the first is made.
        assert!(
            told("make", &[("Makefile", MAKEFILE)])
                .unwrap()
                .starts_with("# Makefile\nup:")
        );
        // make reads GNUmakefile before Makefile.
        let gnu = "up:\n\techo gnu\n";
        assert!(
            told("make up", &[("Makefile", MAKEFILE), ("GNUmakefile", gnu)])
                .unwrap()
                .contains("echo gnu")
        );
    }

    #[test]
    fn a_recipe_handing_its_work_to_a_folder_is_followed_there() {
        let top = "\
BACKEND := backend
generate:
\t@$(MAKE) -C backend generate
migrate:
\tcd $(BACKEND) && $(MAKE) migrate
seed:
\t$(MAKE) --directory=$(CURDIR)/backend seed
";
        let backend = "\
generate:
\tuv run python -m app.generate > openapi.json
migrate: generate
\tuv run alembic upgrade head
seed:
\t$(MAKE) -C ../.. seed
";
        let files = [("Makefile", top), ("backend/Makefile", backend)];
        assert_eq!(
            told("make generate", &files).unwrap(),
            "# Makefile\ngenerate:\n    @$(MAKE) -C backend generate\n# backend/Makefile\n\
             generate:\n    uv run python -m app.generate > openapi.json\n"
        );
        let given = told("make migrate", &files).unwrap();
        assert!(
            given.contains("# backend/Makefile\nmigrate: generate\n"),
            "{given}"
        );
        assert!(given.contains("app.generate"), "{given}");
        assert!(
            told("make seed", &files)
                .unwrap()
                .contains("# backend/Makefile\nseed:")
        );
        // A folder out of the project's reach is not read.
        assert_eq!(
            told("cd backend && make seed", &files).unwrap(),
            "# backend/Makefile\nseed:\n    $(MAKE) -C ../.. seed\n"
        );
        assert!(told("cd backend && make up", &files).is_none());
    }

    #[test]
    fn as_many_targets_are_given_as_the_most_whatever_makefile_has_them() {
        let mut chain = String::new();
        for at in 0..MOST + 4 {
            chain.push_str(&format!("t{at}: t{}\n\techo {at}\n", at + 1));
        }
        let given = told("make t0", &[("Makefile", &chain)]).unwrap();
        assert_eq!(given.matches("echo").count(), MOST);
        // A loop of recipes ends.
        let looped = "a:\n\t$(MAKE) b\nb:\n\t$(MAKE) a\n";
        assert_eq!(
            told("make a", &[("Makefile", looped)])
                .unwrap()
                .matches("$(MAKE)")
                .count(),
            2
        );
    }
}
