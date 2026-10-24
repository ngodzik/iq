//! How many tests passed, as the common test runners print it: read from
//! their output, never asked of a model.

/// The tests that passed and those run, when `output` says, as pytest,
/// unittest, cargo, Jest, Vitest or `go test` print it.
///
/// # Examples
///
/// ```
/// use iq_assay::tally;
///
/// assert_eq!(tally("===== 1 failed, 7 passed in 0.31s ====="), Some((7, 8)));
/// assert_eq!(tally("nothing to see"), None);
/// ```
#[must_use]
pub fn tally(output: &str) -> Option<(u32, u32)> {
    cargo(output)
        .or_else(|| jest(output))
        .or_else(|| vitest(output))
        .or_else(|| pytest(output))
        .or_else(|| unittest(output))
        .or_else(|| go(output))
}

/// The number written just before `word` in `line`, as `7 passed`.
fn before(line: &str, word: &str) -> Option<u32> {
    let at = line.find(word)?;
    line[..at]
        .trim_end()
        .rsplit(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|n| n.parse().ok())
}

/// `test result: ok. 5 passed; 1 failed; ...`, every crate's summed.
fn cargo(output: &str) -> Option<(u32, u32)> {
    let mut seen = false;
    let (mut passed, mut failed) = (0, 0);
    for line in output.lines().filter(|l| l.contains("test result:")) {
        seen = true;
        passed += before(line, " passed").unwrap_or(0);
        failed += before(line, " failed").unwrap_or(0);
    }
    seen.then_some((passed, passed + failed))
}

/// `Tests:       1 failed, 4 passed, 5 total`.
fn jest(output: &str) -> Option<(u32, u32)> {
    let line = output
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with("Tests:") && l.contains(" total"))?;
    Some((
        before(line, " passed").unwrap_or(0),
        before(line, " total")?,
    ))
}

/// ` Tests  1 failed | 4 passed (5)`.
fn vitest(output: &str) -> Option<(u32, u32)> {
    let line = output.lines().rev().find(|l| {
        let l = l.trim_start();
        l.starts_with("Tests ") && l.contains(" passed") && l.ends_with(')')
    })?;
    let total = line[line.rfind('(')? + 1..line.len() - 1]
        .trim()
        .parse()
        .ok()?;
    Some((before(line, " passed").unwrap_or(0), total))
}

/// `== 1 failed, 7 passed, 2 errors in 0.31s ==`.
fn pytest(output: &str) -> Option<(u32, u32)> {
    // Framed by `=` in its default output, bare with `-q`.
    let line = output.lines().rev().find(|l| {
        let bare = l.trim().trim_matches('=').trim();
        bare.starts_with(|c: char| c.is_ascii_digit())
            && (bare.contains(" passed") || bare.contains(" failed") || bare.contains(" error"))
            && bare.contains(" in ")
            && bare.ends_with('s')
    })?;
    let passed = before(line, " passed").unwrap_or(0);
    let failed = before(line, " failed").unwrap_or(0);
    let errors = before(line, " error").unwrap_or(0);
    Some((passed, passed + failed + errors))
}

/// `Ran 5 tests in 0.002s`, then `OK` or `FAILED (failures=1, errors=1)`.
fn unittest(output: &str) -> Option<(u32, u32)> {
    let ran = output.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("Ran ")?;
        rest.split_whitespace().next()?.parse::<u32>().ok()
    })?;
    let failed = output
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with("FAILED ("))
        .map_or(0, |line| {
            ["failures=", "errors="]
                .iter()
                .filter_map(|key| {
                    let at = line.find(key)? + key.len();
                    line[at..]
                        .split(|c: char| !c.is_ascii_digit())
                        .next()?
                        .parse::<u32>()
                        .ok()
                })
                .sum()
        });
    Some((ran.saturating_sub(failed), ran))
}

/// `--- PASS: TestX` and `--- FAIL: TestY`, as `go test -v` prints them.
fn go(output: &str) -> Option<(u32, u32)> {
    let passed = output.matches("--- PASS:").count() as u32;
    let failed = output.matches("--- FAIL:").count() as u32;
    (passed + failed > 0).then_some((passed, passed + failed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_runner_is_read() {
        let cases = [
            ("....\n4 passed in 0.02s\n", Some((4, 4))),
            ("==== 4 passed in 0.02s ====", Some((4, 4))),
            (
                "FAILED t.py::x\n==== 1 failed, 3 passed, 1 error in 0.40s ====",
                Some((3, 5)),
            ),
            (
                "test result: ok. 5 passed; 0 failed; 0 ignored\ntest result: FAILED. 2 passed; 1 failed;",
                Some((7, 8)),
            ),
            (
                "Tests:       1 failed, 4 passed, 5 total\nTime: 1s",
                Some((4, 5)),
            ),
            ("  Tests  1 failed | 4 passed (5)\n", Some((4, 5))),
            (
                "Ran 6 tests in 0.002s\n\nFAILED (failures=1, errors=1)",
                Some((4, 6)),
            ),
            ("Ran 6 tests in 0.002s\n\nOK", Some((6, 6))),
            (
                "--- PASS: TestA (0.00s)\n--- FAIL: TestB (0.00s)\nFAIL",
                Some((1, 2)),
            ),
            ("Compiling...\nerror[E0425]", None),
        ];
        for (output, expected) in cases {
            assert_eq!(tally(output), expected, "{output}");
        }
    }
}
