//! Paths matched against patterns, as a shell and the rules agents read
//! write them: `*` any letters but `/`, `**` any folders, `?` one letter,
//! `{a,b}` either.

/// Whether `path`, or its name when `pattern` names no folder, matches
/// `pattern`.
///
/// # Examples
///
/// ```
/// use iq_core::glob_matches;
///
/// assert!(glob_matches("*.py", "app/models.py"));
/// assert!(glob_matches("src/**/*.{ts,tsx}", "src/ui/button.tsx"));
/// assert!(!glob_matches("src/*.ts", "src/ui/button.ts"));
/// ```
#[must_use]
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    let path = path.strip_prefix("./").unwrap_or(path);
    let name = path.rsplit('/').next().unwrap_or(path);
    alternatives(pattern.trim()).iter().any(|pattern| {
        let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
        glob(pattern.as_bytes(), path.as_bytes())
            || (!pattern.contains('/') && glob(pattern.as_bytes(), name.as_bytes()))
    })
}

/// `pattern` with its first `{a,b}` written out, each alternative's own
/// written out in turn.
fn alternatives(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_owned()];
    };
    let Some(close) = pattern[open..].find('}').map(|at| open + at) else {
        return vec![pattern.to_owned()];
    };
    let (before, after) = (&pattern[..open], &pattern[close + 1..]);
    pattern[open + 1..close]
        .split(',')
        .flat_map(|choice| alternatives(&format!("{before}{choice}{after}")))
        .collect()
}

fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match pattern {
        [] => text.is_empty(),
        [b'*', b'*', rest @ ..] => {
            let rest = rest.strip_prefix(b"/").unwrap_or(rest);
            (0..=text.len()).any(|at| glob(rest, &text[at..]))
        }
        [b'*', rest @ ..] => (0..=text.len())
            .take_while(|at| *at == 0 || text[at - 1] != b'/')
            .any(|at| glob(rest, &text[at..])),
        [b'?', rest @ ..] => !text.is_empty() && text[0] != b'/' && glob(rest, &text[1..]),
        [c, rest @ ..] => text.first() == Some(c) && glob(rest, &text[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_read_as_a_shell_reads_them() {
        assert!(glob_matches("*.py", "a/b/c.py"));
        assert!(!glob_matches("src/*.py", "src/a/b.py"));
        assert!(glob_matches("src/**/*.py", "src/a/b.py"));
        assert!(glob_matches("src/**/*.py", "src/b.py"));
        assert!(glob_matches("test_?.py", "test_a.py"));
        assert!(glob_matches("src/api/**", "src/api/v1/routes.ts"));
        assert!(glob_matches("./src/*.rs", "src/main.rs"));
        assert!(glob_matches("**/*.{ts,tsx}", "./web/app.tsx"));
        assert!(glob_matches("{lib,src}/**/*.{rs,toml}", "lib/a/Cargo.toml"));
        assert!(!glob_matches("{lib,src}/*.rs", "test/a.rs"));
        assert!(glob_matches("a{b", "a{b"));
    }
}
