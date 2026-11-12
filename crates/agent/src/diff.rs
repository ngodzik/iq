//! How a file an agent wrote changed, for the person to follow: a unified
//! diff of its lines, a few around each change. What both ends share at
//! their start and their end is set aside first, so that an edit in a long
//! file costs only its own lines.

/// How many unchanged lines are kept around each change.
const AROUND: usize = 3;

/// The most lines a diff holds; past them, it says how many it left out.
const MOST: usize = 400;

/// The most characters of a line a diff holds: a minified file's one line
/// is cut there.
const LINE_MOST: usize = 400;

/// The most cells the comparison of what differs may take: past them, all
/// of the old is said removed and all of the new added.
const CELLS: usize = 1_000_000;

/// One line of the comparison: kept, at its index in the old and the new;
/// removed from the old; added in the new.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Kept(usize, usize),
    Removed(usize),
    Added(usize),
}

/// `before` changed into `after`, as a unified diff of their lines, each
/// change with [`AROUND`] lines around it; none when no line changed.
pub(crate) fn unified(before: &str, after: &str) -> Option<String> {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let ops = compared(&old, &new);
    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| !matches!(op, Op::Kept(..)))
        .map(|(at, _)| at)
        .collect();
    if changed.is_empty() {
        return None;
    }
    // Each change with the lines around it, those that touch made one.
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    for &at in &changed {
        let start = at.saturating_sub(AROUND);
        let end = (at + AROUND + 1).min(ops.len());
        match hunks.last_mut() {
            Some((_, until)) if start <= *until => *until = end,
            _ => hunks.push((start, end)),
        }
    }
    let mut out: Vec<String> = Vec::new();
    let mut written = 0;
    let total: usize = hunks.iter().map(|(start, end)| end - start).sum();
    for (start, end) in hunks {
        if written >= MOST {
            break;
        }
        let (old_at, new_at) = cursor(&ops, start);
        let span = &ops[start..end];
        let olds = span.iter().filter(|op| !matches!(op, Op::Added(_))).count();
        let news = span
            .iter()
            .filter(|op| !matches!(op, Op::Removed(_)))
            .count();
        // A side with no line names the line before it, as diff does.
        let from = |at: usize, count: usize| if count == 0 { at } else { at + 1 };
        out.push(format!(
            "@@ -{},{olds} +{},{news} @@",
            from(old_at, olds),
            from(new_at, news)
        ));
        for op in span.iter().take(MOST - written) {
            out.push(match *op {
                Op::Kept(at, _) => format!(" {}", cut(old[at])),
                Op::Removed(at) => format!("-{}", cut(old[at])),
                Op::Added(at) => format!("+{}", cut(new[at])),
            });
            written += 1;
        }
    }
    if total > written {
        out.push(format!("\\ {} more lines left out", total - written));
    }
    Some(out.join("\n"))
}

/// `line`, or its first [`LINE_MOST`] characters and a mark that it goes
/// on.
fn cut(line: &str) -> String {
    match line.char_indices().nth(LINE_MOST) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_owned(),
    }
}

/// Where the old and the new stand at `ops[at]`, as indices of their
/// lines.
fn cursor(ops: &[Op], at: usize) -> (usize, usize) {
    ops[..at].iter().fold((0, 0), |(old, new), op| match op {
        Op::Kept(..) => (old + 1, new + 1),
        Op::Removed(_) => (old + 1, new),
        Op::Added(_) => (old, new + 1),
    })
}

/// The lines of `old` and `new` compared: what they share at their start
/// and their end kept, the rest by their longest common run, or all of it
/// removed then added when that would take too much.
fn compared(old: &[&str], new: &[&str]) -> Vec<Op> {
    let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (a, b) = (&old[head..old.len() - tail], &new[head..new.len() - tail]);
    let mut ops: Vec<Op> = (0..head).map(|at| Op::Kept(at, at)).collect();
    if a.len().saturating_mul(b.len()) > CELLS {
        ops.extend((0..a.len()).map(|at| Op::Removed(head + at)));
        ops.extend((0..b.len()).map(|at| Op::Added(head + at)));
    } else {
        // How long the longest common run of `a[i..]` and `b[j..]` is.
        let width = b.len() + 1;
        let mut longest = vec![0u32; (a.len() + 1) * width];
        for i in (0..a.len()).rev() {
            for j in (0..b.len()).rev() {
                longest[i * width + j] = if a[i] == b[j] {
                    longest[(i + 1) * width + j + 1] + 1
                } else {
                    longest[(i + 1) * width + j].max(longest[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < a.len() || j < b.len() {
            if i < a.len() && j < b.len() && a[i] == b[j] {
                ops.push(Op::Kept(head + i, head + j));
                i += 1;
                j += 1;
            } else if j == b.len()
                || i < a.len() && longest[(i + 1) * width + j] >= longest[i * width + j + 1]
            {
                ops.push(Op::Removed(head + i));
                i += 1;
            } else {
                ops.push(Op::Added(head + j));
                j += 1;
            }
        }
    }
    let (old_end, new_end) = (old.len() - tail, new.len() - tail);
    ops.extend((0..tail).map(|at| Op::Kept(old_end + at, new_end + at)));
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_is_said_with_the_lines_around_it_numbered_from_one() {
        let before: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        let after = before.replace("line 10\n", "line ten\nline ten and a half\n");
        assert_eq!(
            unified(&before, &after).unwrap(),
            "@@ -7,7 +7,8 @@\n line 7\n line 8\n line 9\n-line 10\n+line ten\n+line ten and a half\n line 11\n line 12\n line 13"
        );
        // Nothing changed, nothing said.
        assert_eq!(unified(&before, &before), None);
    }

    #[test]
    fn a_new_file_is_all_added_and_changes_apart_are_hunks_apart() {
        assert_eq!(unified("", "a\nb\n").unwrap(), "@@ -0,0 +1,2 @@\n+a\n+b");
        let before: Vec<String> = (1..=30).map(|n| n.to_string()).collect();
        let mut after = before.clone();
        after[1] = "two".into();
        after.remove(27);
        let said = unified(&before.join("\n"), &after.join("\n")).unwrap();
        let heads: Vec<&str> = said.lines().filter(|l| l.starts_with("@@")).collect();
        assert_eq!(heads, ["@@ -1,5 +1,5 @@", "@@ -25,6 +25,5 @@"]);
        assert!(said.contains("-28\n"), "{said}");
    }

    #[test]
    fn a_long_line_is_cut() {
        let line = "x".repeat(LINE_MOST + 10);
        let said = unified("", &line).unwrap();
        assert_eq!(
            said.lines().nth(1).unwrap(),
            format!("+{}…", "x".repeat(LINE_MOST))
        );
    }

    #[test]
    fn a_long_diff_says_how_much_it_left_out() {
        let after: String = (0..MOST + 50).map(|n| format!("{n}\n")).collect();
        let said = unified("", &after).unwrap();
        assert_eq!(said.lines().count(), 1 + MOST + 1);
        assert!(said.ends_with("\\ 50 more lines left out"), "{said}");
    }
}
