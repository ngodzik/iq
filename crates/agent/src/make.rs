//! What a `make` target does, read from the project's Makefile for the
//! reviewer: a target is named for what it means, and judged by its name
//! alone, `make down` stops the services and `make clean` tidies up; its
//! recipe says what it does, which may be to delete the database's data.

use std::collections::BTreeMap;

/// The most targets whose recipes are given, those asked and what they
/// depend on.
const MOST: usize = 8;

/// The most lines of one recipe given.
const LINES: usize = 20;

/// The targets `line` makes, when it is a `make` command: its words that
/// are not options nor variables, up to the next command of the line.
pub(crate) fn targets(line: &str) -> Vec<String> {
    let mut words = line.split_whitespace();
    if !words.next().is_some_and(|first| {
        std::path::Path::new(first)
            .file_name()
            .is_some_and(|name| name == "make" || name == "gmake")
    }) {
        return Vec::new();
    }
    let mut targets = Vec::new();
    for word in words {
        if ["&&", "||", "|", ";"].contains(&word) {
            break;
        }
        let last = word.ends_with(';');
        let word = word.trim_end_matches(';');
        if !word.is_empty() && !word.starts_with('-') && !word.contains('=') {
            targets.push(word.to_owned());
        }
        if last {
            break;
        }
    }
    targets
}

/// The recipes of `targets` in `makefile`, each with what it depends on,
/// and those of what it depends on, as far as [`MOST`] targets; `None`
/// when none is found there.
pub(crate) fn recipes(makefile: &str, targets: &[String]) -> Option<String> {
    let rules = parse(makefile);
    let mut queue: Vec<String> = targets.to_vec();
    let mut given: Vec<String> = Vec::new();
    let mut out = String::new();
    while let Some(target) = queue.first().cloned() {
        queue.remove(0);
        if given.contains(&target) || given.len() >= MOST {
            continue;
        }
        let Some((needs, recipe)) = rules.get(&target) else {
            continue;
        };
        given.push(target.clone());
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
        queue.extend(needs.iter().cloned());
    }
    (!out.is_empty()).then_some(out)
}

/// Each target of `makefile`, with what it depends on and its recipe.
/// Variables are left as written: what a recipe does is read from it as it
/// is.
fn parse(makefile: &str) -> BTreeMap<String, (Vec<String>, Vec<String>)> {
    let mut rules: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    let mut current: Vec<String> = Vec::new();
    for line in makefile.lines() {
        if let Some(command) = line.strip_prefix('\t') {
            let command = command.trim_end();
            if !command.is_empty() {
                for target in &current {
                    if let Some((_, recipe)) = rules.get_mut(target) {
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
        // A rule: targets, a colon, what they depend on. Not a variable
        // (`:=`), nor a target with a pattern, which names no one target.
        let Some((heads, rest)) = trimmed.split_once(':') else {
            current.clear();
            continue;
        };
        if rest.starts_with('=') || heads.contains('=') || heads.contains('%') {
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
        for target in &current {
            rules
                .entry(target.clone())
                .or_insert_with(|| (needs.clone(), Vec::new()));
        }
    }
    rules
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

    #[test]
    fn the_targets_of_a_make_line_are_read_up_to_the_next_command() {
        assert_eq!(targets("make down && make up-db"), ["down"]);
        assert_eq!(
            targets("make -j4 up-db VERBOSE=1 test; echo ok"),
            ["up-db", "test"]
        );
        assert_eq!(targets("/usr/bin/gmake up"), ["up"]);
        assert!(targets("cargo make down").is_empty());
        assert!(targets("make").is_empty());
    }

    #[test]
    fn a_target_is_given_with_its_recipe_and_what_it_depends_on() {
        let given = recipes(MAKEFILE, &["up-db".to_owned()]).unwrap();
        assert_eq!(
            given,
            "up-db: down\n    $(COMPOSE) up -d db\n    ./scripts/wait-db.sh\ndown:\n    \
             $(COMPOSE) down --volumes --remove-orphans\n"
        );
        assert!(recipes(MAKEFILE, &["nothing".to_owned()]).is_none());
        // A pattern rule names no target.
        assert!(recipes(MAKEFILE, &["x.o".to_owned()]).is_none());
    }
}
