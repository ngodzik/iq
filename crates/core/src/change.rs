//! What a tool changed in a file: the lines it removed and added, with a
//! few of those around them, as a diff shows them.

/// The lines kept on each side of a change, to tell where it is.
const CONTEXT: usize = 3;

/// The most pairs of lines compared to find those kept between the first
/// and the last that differ: beyond, they are all taken as replaced, which
/// still reads right and costs nothing.
const COMPARED: usize = 400_000;

/// One line of a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    /// The same before and after.
    Kept(String),
    /// There before only.
    Removed(String),
    /// There after only.
    Added(String),
}

/// Lines changed together, with those around them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The number of its first line before and after, from 1, when the
    /// change was read with the whole file.
    pub at: Option<(usize, usize)>,
    /// Its lines, in order.
    pub lines: Vec<DiffLine>,
}

/// What a tool changed in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The file, as the tool named it.
    pub path: String,
    /// What changed, in order.
    pub hunks: Vec<Hunk>,
}

/// A line of the comparison, by its index in the text it is read from:
/// after, for a line kept.
#[derive(Debug, Clone, Copy)]
enum Step {
    Kept(usize),
    Removed(usize),
    Added(usize),
}

impl Change {
    /// What changed in `path` from `before` to `after`. `whole` when they
    /// are the whole file, so that its lines are numbered; a part of it,
    /// such as the text an edit replaced, leaves them unnumbered.
    ///
    /// # Examples
    ///
    /// ```
    /// use iq_core::{Change, DiffLine};
    ///
    /// let change = Change::between("calc.py", "a\nb - c\n", "a\nb + c\n", true);
    /// assert_eq!((change.added(), change.removed()), (1, 1));
    /// assert_eq!(change.hunks[0].at, Some((1, 1)));
    /// assert_eq!(change.hunks[0].lines[1], DiffLine::Removed("b - c".into()));
    /// ```
    #[must_use]
    pub fn between(path: impl Into<String>, before: &str, after: &str, whole: bool) -> Self {
        let old: Vec<&str> = before.lines().collect();
        let new: Vec<&str> = after.lines().collect();
        let steps = compare(&old, &new);
        // Where each step stands in the file before and after.
        let mut at = Vec::with_capacity(steps.len());
        let (mut was, mut is) = (1, 1);
        for step in &steps {
            at.push((was, is));
            match step {
                Step::Kept(..) => {
                    was += 1;
                    is += 1;
                }
                Step::Removed(_) => was += 1,
                Step::Added(_) => is += 1,
            }
        }
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for (index, _) in steps
            .iter()
            .enumerate()
            .filter(|(_, step)| !matches!(step, Step::Kept(..)))
        {
            let start = index.saturating_sub(CONTEXT);
            let end = (index + 1 + CONTEXT).min(steps.len());
            match spans.last_mut() {
                Some(last) if start <= last.1 => last.1 = end,
                _ => spans.push((start, end)),
            }
        }
        let hunks = spans
            .into_iter()
            .map(|(start, end)| Hunk {
                at: whole.then(|| at[start]),
                lines: steps[start..end]
                    .iter()
                    .map(|step| match *step {
                        Step::Kept(after) => DiffLine::Kept(new[after].to_owned()),
                        Step::Removed(before) => DiffLine::Removed(old[before].to_owned()),
                        Step::Added(after) => DiffLine::Added(new[after].to_owned()),
                    })
                    .collect(),
            })
            .collect();
        Self {
            path: path.into(),
            hunks,
        }
    }

    /// The lines it added.
    #[must_use]
    pub fn added(&self) -> usize {
        self.count(|line| matches!(line, DiffLine::Added(_)))
    }

    /// The lines it removed.
    #[must_use]
    pub fn removed(&self) -> usize {
        self.count(|line| matches!(line, DiffLine::Removed(_)))
    }

    /// Whether it changed nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added() + self.removed() == 0
    }

    /// The change as a unified diff, as `git diff` writes it, for a reader
    /// that knows the format: a model judging it, a person reading it.
    ///
    /// # Examples
    ///
    /// ```
    /// use iq_core::Change;
    ///
    /// let change = Change::between("calc.py", "a\nb\n", "a\nc\n", true);
    /// assert_eq!(
    ///     change.patch(),
    ///     "--- a/calc.py\n+++ b/calc.py\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n"
    /// );
    /// ```
    #[must_use]
    pub fn patch(&self) -> String {
        let mut out = format!("--- a/{0}\n+++ b/{0}\n", self.path);
        for hunk in &self.hunks {
            let before = hunk
                .lines
                .iter()
                .filter(|line| !matches!(line, DiffLine::Added(_)))
                .count();
            let after = hunk
                .lines
                .iter()
                .filter(|line| !matches!(line, DiffLine::Removed(_)))
                .count();
            match hunk.at {
                Some((was, is)) => {
                    out.push_str(&format!("@@ -{was},{before} +{is},{after} @@\n"));
                }
                None => out.push_str("@@\n"),
            }
            for line in &hunk.lines {
                let (sign, text) = match line {
                    DiffLine::Kept(text) => (' ', text),
                    DiffLine::Removed(text) => ('-', text),
                    DiffLine::Added(text) => ('+', text),
                };
                out.push(sign);
                out.push_str(text);
                out.push('\n');
            }
        }
        out
    }

    fn count(&self, which: impl Fn(&DiffLine) -> bool) -> usize {
        self.hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter(|line| which(line))
            .count()
    }
}

/// `old` and `new` line by line: what they share at their start and end is
/// kept, and between, the longest run of lines both hold in order.
fn compare(old: &[&str], new: &[&str]) -> Vec<Step> {
    let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (a, b) = (&old[head..old.len() - tail], &new[head..new.len() - tail]);
    let mut steps: Vec<Step> = (0..head).map(Step::Kept).collect();
    if a.len().saturating_mul(b.len()) <= COMPARED {
        // `longest[i][j]`: how many lines `a[i..]` and `b[j..]` share in
        // order.
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
                steps.push(Step::Kept(head + j));
                i += 1;
                j += 1;
            } else if j == b.len()
                || (i < a.len() && longest[(i + 1) * width + j] >= longest[i * width + j + 1])
            {
                steps.push(Step::Removed(head + i));
                i += 1;
            } else {
                steps.push(Step::Added(head + j));
                j += 1;
            }
        }
    } else {
        steps.extend((0..a.len()).map(|i| Step::Removed(head + i)));
        steps.extend((0..b.len()).map(|j| Step::Added(head + j)));
    }
    let from = new.len() - tail;
    steps.extend((from..new.len()).map(Step::Kept));
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(lines: std::ops::Range<usize>) -> String {
        lines.map(|n| format!("line {n}\n")).collect()
    }

    #[test]
    fn far_apart_changes_are_two_hunks_numbered_as_in_the_file() {
        let before = file(1..41);
        let after = before
            .replace("line 5\n", "line five\n")
            .replace("line 30\n", "")
            + "line 41\n";
        let change = Change::between("f", &before, &after, true);
        assert_eq!((change.added(), change.removed()), (2, 2));
        let at: Vec<_> = change.hunks.iter().map(|h| h.at).collect();
        // After the removed line, the file has one line less.
        assert_eq!(at, [Some((2, 2)), Some((27, 27)), Some((38, 37))]);
        assert_eq!(
            change.hunks[0].lines,
            [
                DiffLine::Kept("line 2".into()),
                DiffLine::Kept("line 3".into()),
                DiffLine::Kept("line 4".into()),
                DiffLine::Removed("line 5".into()),
                DiffLine::Added("line five".into()),
                DiffLine::Kept("line 6".into()),
                DiffLine::Kept("line 7".into()),
                DiffLine::Kept("line 8".into()),
            ]
        );
        assert_eq!(change.hunks[1].lines.len(), 3 + 1 + 3);
        assert_eq!(
            change.hunks[2].lines.last(),
            Some(&DiffLine::Added("line 41".into()))
        );
    }

    #[test]
    fn a_part_of_a_file_is_not_numbered_and_a_new_file_is_all_added() {
        let edit = Change::between("f", "a\nb\n", "a\nc\nb\n", false);
        assert_eq!(edit.hunks[0].at, None);
        assert_eq!(edit.added(), 1);
        let created = Change::between("f", "", "one\ntwo\n", true);
        assert_eq!((created.added(), created.removed()), (2, 0));
        assert_eq!(created.hunks[0].at, Some((1, 1)));
        assert!(Change::between("f", "same\n", "same\n", true).is_empty());
    }

    #[test]
    fn too_much_to_compare_is_taken_as_replaced_between_what_is_shared() {
        let before = format!("head\n{}tail\n", file(0..1000));
        let after = format!("head\n{}tail\n", file(1000..2000));
        let change = Change::between("f", &before, &after, true);
        assert_eq!((change.added(), change.removed()), (1000, 1000));
        assert_eq!(change.hunks.len(), 1);
        assert_eq!(change.hunks[0].lines[0], DiffLine::Kept("head".into()));
    }
}
