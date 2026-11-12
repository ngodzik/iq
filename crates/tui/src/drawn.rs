//! The conversation iq draws itself, as the settings ask (`conversation =
//! "drawn"`): on the terminal's other screen, the mouse taken, from the
//! blocks each chat's window keeps, laid out again at every draw, so that
//! what is folded (a fold of steps, a job that ended, a message between
//! agents, a diagram folded) unfolds in place, alone, with a click or
//! Enter on the mark, and folds again the same, nothing moving but what
//! follows it. The end follows what is written while the person is at it;
//! scrolled up, the view stays put as more comes, its top held by the
//! block it is in.

use std::collections::HashSet;
use std::sync::Arc;

use iq_ui::steps::Entry;
use iq_ui::{Board, Reading};
use ratatui::text::{Line, Span};

use crate::diagram::Image;
use crate::history::{Block, History};
use crate::pages::steps::body;
use crate::theme::Palette;

/// What iq opens with, before the first block, as a place of the view.
const OPENING: usize = usize::MAX;

/// The conversation as iq draws it, and where the person reads it.
#[derive(Debug, Default)]
pub(crate) struct View {
    /// The chat whose window was drawn last: another starts at its end,
    /// all folded.
    window: Option<Option<String>>,
    /// The view's top: the block it is in, by its number ([`OPENING`] for
    /// what iq opens with), and the row in it; none while it follows the
    /// end.
    top: Option<(usize, usize)>,
    /// The blocks unfolded in place, by their numbers.
    unfolded: HashSet<usize>,
    /// The block marked, by its number: what Enter unfolds or folds.
    mark: Option<usize>,
    /// The block each row of the conversation showed, as last drawn.
    rows: Vec<Option<usize>>,
    /// The screen's row the conversation began at, as last drawn.
    from: u16,
}

/// What the conversation shows, laid out.
#[derive(Debug, Default)]
pub(crate) struct Shown {
    /// Its rows, as many as its height at most.
    pub(crate) lines: Vec<Line<'static>>,
    /// The images they show, for the terminal to be sent first.
    pub(crate) images: Vec<Image>,
    /// Whether a click fell outside it: the person reads it no more.
    pub(crate) missed: bool,
}

/// Where the conversation is drawn: the screen's rows from `from`, as many
/// as `height`, within `width` columns.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Area {
    pub(crate) from: u16,
    pub(crate) width: usize,
    pub(crate) height: usize,
}

/// The blocks of a window, each with its number, after what iq opens with.
struct Laid<'a> {
    opening: Vec<Line<'static>>,
    first: usize,
    blocks: Vec<&'a Block>,
    width: usize,
    cell: (u16, u16),
    board: &'a Board,
    p: &'a Palette,
    unfolded: &'a HashSet<usize>,
}

impl Laid<'_> {
    /// How many places there are: what iq opens with, then each block.
    fn len(&self) -> usize {
        1 + self.blocks.len()
    }

    /// The place `at` holds, by its number.
    fn number(&self, at: usize) -> usize {
        if at == 0 {
            OPENING
        } else {
            self.first + at - 1
        }
    }

    /// Where the block numbered `number` is; the first block kept for one
    /// let go since.
    fn at(&self, number: usize) -> usize {
        if number == OPENING {
            0
        } else {
            (number.saturating_sub(self.first) + 1).min(self.len() - 1)
        }
    }

    /// Whether what place `at` holds folds and unfolds.
    fn foldable(&self, at: usize) -> bool {
        at > 0
            && matches!(
                self.blocks[at - 1],
                Block::Entry { .. } | Block::Folded { .. }
            )
    }

    /// The rows of place `at`, unfolded when the person unfolded it, the
    /// images they show put in `images`.
    fn lines(&self, at: usize, images: &mut Vec<Image>) -> Vec<Line<'static>> {
        if at == 0 {
            return self.opening.clone();
        }
        let block = self.blocks[at - 1];
        if self.unfolded.contains(&self.number(at))
            && let Some(lines) = self.unfolded(block, images)
        {
            return lines;
        }
        block.lines(self.width, self.cell, images)
    }

    /// `block` unfolded: a fold of several steps under its count, each
    /// whole with what its tools returned; one step whole; a job under its
    /// line, its command whole and the end of its output; a message as it
    /// was written; a diagram folded, its image.
    fn unfolded(&self, block: &Block, images: &mut Vec<Image>) -> Option<Vec<Line<'static>>> {
        match block {
            Block::Entry { id, blocks } => {
                let entry = self.board.steps().entry(*id)?;
                let whole = body(&entry, true, self.p, self.width, &|_| None);
                let above = |count: usize, images: &mut Vec<Image>| -> Vec<Line<'static>> {
                    blocks
                        .iter()
                        .take(count)
                        .flat_map(|block| block.lines(self.width, self.cell, images))
                        .collect()
                };
                let mut lines = match entry {
                    Entry::Fold(fold) if fold.steps.len() > 1 => above(1, images),
                    Entry::Ran(_) => above(blocks.len(), images),
                    Entry::Fold(_) | Entry::Said(_) => Vec::new(),
                };
                lines.extend(whole);
                Some(lines)
            }
            Block::Folded {
                diagram,
                code,
                failed,
                ..
            } => Some(
                Block::Diagram {
                    diagram: Arc::clone(diagram),
                    code: code.clone(),
                    failed: *failed,
                }
                .lines(self.width, self.cell, images),
            ),
            _ => None,
        }
    }

    /// How many rows place `at` takes.
    fn rows(&self, at: usize) -> usize {
        self.lines(at, &mut Vec::new()).len()
    }
}

impl View {
    /// What the conversation of `window` shows in `area`, once `readings`,
    /// what the person asked of it, are carried out.
    pub(crate) fn shown(
        &mut self,
        history: &History,
        board: &Board,
        p: &Palette,
        window: Option<&str>,
        area: Area,
        readings: &[Reading],
    ) -> Shown {
        let key = window.map(str::to_owned);
        if self.window.as_ref() != Some(&key) {
            self.window = Some(key);
            self.top = None;
            self.mark = None;
            self.unfolded.clear();
        }
        let height = area.height.max(1);
        let mut missed = false;
        let (first, blocks) = history.blocks(window);
        let blocks: Vec<&Block> = blocks.into_iter().flatten().collect();
        for reading in readings {
            let unfolded = self.unfolded.clone();
            let laid = Laid {
                opening: history.opening_lines(area.width),
                first,
                blocks: blocks.clone(),
                width: area.width,
                cell: history.cell(),
                board,
                p,
                unfolded: &unfolded,
            };
            match reading {
                Reading::Scroll(rows) => self.scroll(&laid, height, *rows),
                Reading::Page(pages) => {
                    let rows = height.saturating_sub(1).max(1) as isize;
                    self.scroll(&laid, height, pages * rows);
                }
                Reading::Mark(by) => self.move_mark(&laid, height, *by),
                Reading::Toggle => {
                    if let Some(number) = self.mark
                        && laid.foldable(laid.at(number))
                    {
                        self.toggle(&laid, height, number);
                    }
                }
                Reading::Click(_, row) => {
                    let shown = usize::from(row.saturating_sub(self.from));
                    let clicked = (*row >= self.from)
                        .then(|| self.rows.get(shown).copied().flatten())
                        .flatten();
                    match clicked {
                        Some(number) => {
                            self.mark = Some(number);
                            if laid.foldable(laid.at(number)) {
                                self.toggle(&laid, height, number);
                            }
                        }
                        None => {
                            self.mark = None;
                            missed = true;
                        }
                    }
                }
                Reading::Leave => self.mark = None,
            }
        }
        let unfolded = self.unfolded.clone();
        let laid = Laid {
            opening: history.opening_lines(area.width),
            first,
            blocks,
            width: area.width,
            cell: history.cell(),
            board,
            p,
            unfolded: &unfolded,
        };
        let mark = self.mark;
        let mut shown = self.lay_out(&laid, height);
        shown.missed = missed;
        self.from = area.from;
        if let Some(mark) = mark {
            for (line, number) in shown.lines.iter_mut().zip(&self.rows) {
                if *number == Some(mark) {
                    marked(line, p);
                }
            }
        }
        shown
    }

    /// Unfolds the block numbered `number`, or folds it again: what is
    /// above it stays where it is, what follows it moves.
    fn toggle(&mut self, laid: &Laid<'_>, height: usize, number: usize) {
        let (at, row) = self.top_now(laid, height);
        self.top = Some((laid.number(at), row));
        if !self.unfolded.remove(&number) {
            self.unfolded.insert(number);
        }
    }

    /// Moves the mark to what is folded before it, or after it, by `by`,
    /// from the end when none is marked, and scrolls to show it.
    fn move_mark(&mut self, laid: &Laid<'_>, height: usize, by: isize) {
        let foldable: Vec<usize> = (0..laid.len()).filter(|at| laid.foldable(*at)).collect();
        let Some(&last) = foldable.last() else {
            return;
        };
        let at = match self.mark.map(|number| laid.at(number)) {
            None => last,
            Some(now) if by < 0 => foldable
                .iter()
                .rev()
                .find(|at| **at < now)
                .copied()
                .unwrap_or(now),
            Some(now) => foldable
                .iter()
                .find(|at| **at > now)
                .copied()
                .unwrap_or(now),
        };
        self.mark = Some(laid.number(at));
        // Shown whole when it fits, its start at least.
        let (top, row) = self.top_now(laid, height);
        let start: isize = if at >= top {
            (top..at)
                .map(|place| laid.rows(place) as isize)
                .sum::<isize>()
                - row as isize
        } else {
            -((at..top)
                .map(|place| laid.rows(place) as isize)
                .sum::<isize>()
                + row as isize)
        };
        let end = start + laid.rows(at) as isize;
        if start < 0 {
            self.scroll(laid, height, start);
        } else if end > height as isize {
            self.scroll(laid, height, (end - height as isize).min(start));
        }
    }

    /// The rows from the view's top, or the last `height` while it
    /// follows the end: it follows it again once that is reached. Notes
    /// which block each row shows.
    fn lay_out(&mut self, laid: &Laid<'_>, height: usize) -> Shown {
        let mut shown = Shown::default();
        let mut rows = Vec::new();
        if let Some((number, row)) = self.top {
            let mut at = laid.at(number);
            let mut skip = row;
            while at < laid.len() && shown.lines.len() < height {
                let mut images = Vec::new();
                let lines = laid.lines(at, &mut images);
                let taken: Vec<Line<'static>> = lines.into_iter().skip(skip).collect();
                if !taken.is_empty() {
                    shown.images.extend(images);
                }
                rows.extend(std::iter::repeat_n(
                    (at > 0).then(|| laid.number(at)),
                    taken.len(),
                ));
                shown.lines.extend(taken);
                skip = 0;
                at += 1;
            }
            if shown.lines.len() >= height {
                shown.lines.truncate(height);
                rows.truncate(height);
                self.rows = rows;
                return shown;
            }
            self.top = None;
            shown = Shown::default();
            rows.clear();
        }
        // From the end up, as many rows as there is room for.
        let mut parts = Vec::new();
        let mut count = 0;
        for at in (0..laid.len()).rev() {
            if count >= height {
                break;
            }
            let mut images = Vec::new();
            let lines = laid.lines(at, &mut images);
            count += lines.len();
            parts.push((at, lines, images));
        }
        for (at, lines, images) in parts.into_iter().rev() {
            rows.extend(std::iter::repeat_n(
                (at > 0).then(|| laid.number(at)),
                lines.len(),
            ));
            shown.lines.extend(lines);
            shown.images.extend(images);
        }
        let over = shown.lines.len().saturating_sub(height);
        shown.lines.drain(..over);
        rows.drain(..over);
        self.rows = rows;
        shown
    }

    /// The view's top, the place and the row in it, as it now stands.
    fn top_now(&self, laid: &Laid<'_>, height: usize) -> (usize, usize) {
        if let Some((number, row)) = self.top {
            return (laid.at(number), row);
        }
        let mut above = height;
        for at in (0..laid.len()).rev() {
            let rows = laid.rows(at);
            if rows >= above {
                return (at, rows - above);
            }
            above -= rows;
        }
        (0, 0)
    }

    /// Moves the view by `rows`, up when fewer than none, never above the
    /// start; back down to the end, it follows it again.
    fn scroll(&mut self, laid: &Laid<'_>, height: usize, rows: isize) {
        let (mut at, mut row) = self.top_now(laid, height);
        if rows < 0 {
            let mut up = rows.unsigned_abs();
            while up > row {
                if at == 0 {
                    up = row;
                    break;
                }
                up -= row + 1;
                at -= 1;
                row = laid.rows(at).saturating_sub(1);
            }
            row -= up;
        } else {
            let mut down = rows.unsigned_abs();
            loop {
                let left = laid.rows(at).saturating_sub(row);
                if down < left || at + 1 >= laid.len() {
                    row = (row + down).min(laid.rows(at).saturating_sub(1));
                    break;
                }
                down -= left;
                at += 1;
                row = 0;
            }
        }
        // What lies below the top fills the view no more: the end.
        let mut below = laid.rows(at).saturating_sub(row);
        let mut next = at + 1;
        while below <= height && next < laid.len() {
            below += laid.rows(next);
            next += 1;
        }
        self.top = (below > height).then(|| (laid.number(at), row));
    }
}

/// `line` marked: a bar where it begins, in the accent's colour, over the
/// space its indent leaves there.
fn marked(line: &mut Line<'static>, p: &Palette) {
    let Some(first) = line.spans.iter().position(|span| !span.content.is_empty()) else {
        line.spans.push(Span::styled("▌", p.bold(p.accent)));
        return;
    };
    let span = &line.spans[first];
    if let Some(rest) = span.content.strip_prefix(' ') {
        let rest = Span::styled(rest.to_owned(), span.style);
        line.spans
            .splice(first..=first, [Span::styled("▌", p.bold(p.accent)), rest]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Depth;
    use iq_agent::{Event, PERSON};
    use iq_ui::Update;
    use iq_ui::look::Theme;

    fn crew(board: &mut Board, history: &mut History, p: &Palette, event: Event) {
        board.update(Update::Crew(event.clone()));
        history.event(&event, board, p);
    }

    fn said(board: &mut Board, history: &mut History, p: &Palette, text: &str) {
        let event = Event::Text {
            agent: "lead".into(),
            text: text.into(),
        };
        crew(board, history, p, event);
    }

    fn rows(shown: &Shown) -> Vec<String> {
        shown
            .lines
            .iter()
            .map(|l| l.to_string().trim_end().to_owned())
            .collect()
    }

    /// A board and a history with the chat `lead` joined, and its window
    /// taken.
    fn joined() -> (Palette, Board, History) {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        crew(
            &mut board,
            &mut history,
            &p,
            Event::Joined {
                agent: "lead".into(),
                model: "lab/lead".into(),
                by: PERSON.into(),
            },
        );
        (p, board, history)
    }

    fn area(height: usize) -> Area {
        Area {
            from: 2,
            width: 60,
            height,
        }
    }

    #[test]
    fn the_view_follows_the_end_and_stays_put_once_scrolled_up() {
        let (p, mut board, mut history) = joined();
        let lines: String = (1..=20).map(|n| format!("line {n}\n\n")).collect();
        said(&mut board, &mut history, &p, &lines);
        let _ = history.take(60, Some("lead"));
        let mut view = View::default();
        let mut shown = |history: &History, board: &Board, readings: &[Reading]| {
            rows(&view.shown(history, board, &p, Some("lead"), area(5), readings))
        };
        assert_eq!(
            shown(&history, &board, &[]).last().map(String::as_str),
            Some("  line 20")
        );
        // Up by 3 pages less a row each: what shows stays as more comes.
        let before = shown(&history, &board, &[Reading::Page(-3)]);
        assert!(before.contains(&"  line 14".to_owned()), "{before:#?}");
        said(&mut board, &mut history, &p, "line 21\n");
        let _ = history.take(60, Some("lead"));
        assert_eq!(shown(&history, &board, &[]), before);
        // Never above the start.
        let top = shown(&history, &board, &[Reading::Scroll(-500)]);
        assert_eq!(top[1], "● lead");
        // Back down to the end, it follows it again.
        let end = shown(&history, &board, &[Reading::Scroll(500)]);
        assert_eq!(end.last().map(String::as_str), Some("  line 21"));
    }

    #[test]
    fn a_click_unfolds_a_fold_in_place_alone_and_folds_it_again() {
        let (p, mut board, mut history) = joined();
        let read = |path: &str| Event::Tool {
            agent: "lead".into(),
            name: "read_file".into(),
            input: path.into(),
            output: format!("1\tin {path}"),
            ok: true,
        };
        for event in [
            Event::Working {
                agent: "lead".into(),
                prompt: "look".into(),
            },
            read("src/a.rs"),
            read("src/b.rs"),
            Event::TextStart {
                agent: "lead".into(),
            },
            Event::Text {
                agent: "lead".into(),
                text: "Read.\n".into(),
            },
            read("src/c.rs"),
            read("src/d.rs"),
            Event::Done {
                agent: "lead".into(),
                outcome: Ok("done".into()),
            },
        ] {
            crew(&mut board, &mut history, &p, event);
        }
        let _ = history.take(60, Some("lead"));
        let mut view = View::default();
        let folded = rows(&view.shown(&history, &board, &p, Some("lead"), area(20), &[]));
        let first = folded
            .iter()
            .position(|row| row.contains("2 steps: read_file ×2") && row.contains("#1"))
            .unwrap();
        // Clicked, the first fold unfolds under its count, alone: what is
        // above it stays.
        let click = Reading::Click(5, 2 + first as u16);
        let unfolded = rows(&view.shown(&history, &board, &p, Some("lead"), area(20), &[click]));
        assert_eq!(unfolded[..first], folded[..first]);
        assert!(unfolded[first].starts_with("▌ ▸ 2 steps"), "{unfolded:#?}");
        // Marked, a bar where its rows begin.
        assert_eq!(unfolded[first + 1], "▌ ▸ read_file src/a.rs");
        assert_eq!(unfolded[first + 2], "▌   1\tin src/a.rs");
        assert!(
            !unfolded.iter().any(|row| row.contains("src/c.rs")),
            "{unfolded:#?}"
        );
        // Clicked again, it folds.
        let click = Reading::Click(5, 2 + first as u16);
        let again = rows(&view.shown(&history, &board, &p, Some("lead"), area(20), &[click]));
        assert_eq!(again[first + 1], folded[first + 1]);
        // With the keys: the last fold marked, unfolded, then the one before.
        let keys = [Reading::Leave, Reading::Mark(-1), Reading::Toggle];
        let last = rows(&view.shown(&history, &board, &p, Some("lead"), area(20), &keys));
        assert!(
            last.iter().any(|row| row == "▌ ▸ read_file src/c.rs"),
            "{last:#?}"
        );
        let keys = [Reading::Mark(-1), Reading::Toggle];
        let both = rows(&view.shown(&history, &board, &p, Some("lead"), area(20), &keys));
        assert!(
            both.iter().any(|row| row == "▌ ▸ read_file src/a.rs"),
            "{both:#?}"
        );
        assert!(
            both.iter().any(|row| row == "  ▸ read_file src/c.rs"),
            "{both:#?}"
        );
        // A click below the conversation misses it.
        let missed = view.shown(
            &history,
            &board,
            &p,
            Some("lead"),
            area(20),
            &[Reading::Click(0, 40)],
        );
        assert!(missed.missed);
    }
}
