//! The conversation iq draws itself, as the settings ask (`conversation =
//! "drawn"`): on the terminal's other screen, the mouse taken, from the
//! blocks each chat's window keeps, laid out again at every draw. The end
//! follows what is written while the person is at it; scrolled up, the
//! view stays put as more comes, its top held by the block it is in.

use iq_ui::Reading;
use ratatui::text::Line;

use crate::diagram::Image;
use crate::history::{Block, History};

/// What iq opens with, before the first block, as a place of the view.
const OPENING: usize = usize::MAX;

/// The conversation as iq draws it, and where the person reads it.
#[derive(Debug, Default)]
pub(crate) struct View {
    /// The chat whose window was drawn last: another starts at its end.
    window: Option<Option<String>>,
    /// The view's top: the block it is in, by its number ([`OPENING`] for
    /// what iq opens with), and the row in it; none while it follows the
    /// end.
    top: Option<(usize, usize)>,
}

/// What the conversation shows, laid out.
#[derive(Debug, Default)]
pub(crate) struct Shown {
    /// Its rows, as many as its height at most.
    pub(crate) lines: Vec<Line<'static>>,
    /// The images they show, for the terminal to be sent first.
    pub(crate) images: Vec<Image>,
}

/// The blocks of a window, each with its number, after what iq opens with.
struct Laid<'a> {
    opening: Vec<Line<'static>>,
    first: usize,
    blocks: Vec<&'a Block>,
    width: usize,
    cell: (u16, u16),
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

    /// The rows of place `at`, the images they show put in `images`.
    fn lines(&self, at: usize, images: &mut Vec<Image>) -> Vec<Line<'static>> {
        if at == 0 {
            self.opening.clone()
        } else {
            self.blocks[at - 1].lines(self.width, self.cell, images)
        }
    }

    /// How many rows place `at` takes.
    fn rows(&self, at: usize) -> usize {
        self.lines(at, &mut Vec::new()).len()
    }
}

impl View {
    /// What the conversation of `window` shows within `width` columns and
    /// `height` rows, once `readings`, what the person asked of it, are
    /// carried out.
    pub(crate) fn shown(
        &mut self,
        history: &History,
        window: Option<&str>,
        width: usize,
        height: usize,
        readings: &[Reading],
    ) -> Shown {
        let key = window.map(str::to_owned);
        if self.window.as_ref() != Some(&key) {
            self.window = Some(key);
            self.top = None;
        }
        let (first, blocks) = history.blocks(window);
        let laid = Laid {
            opening: history.opening_lines(width),
            first,
            blocks: blocks.into_iter().flatten().collect(),
            width,
            cell: history.cell(),
        };
        let height = height.max(1);
        for reading in readings {
            match reading {
                Reading::Scroll(rows) => self.scroll(&laid, height, *rows),
                Reading::Page(pages) => {
                    let rows = height.saturating_sub(1).max(1) as isize;
                    self.scroll(&laid, height, pages * rows);
                }
            }
        }
        self.lay_out(&laid, height)
    }

    /// The rows from the view's top, or the last `height` while it
    /// follows the end: it follows it again once that is reached.
    fn lay_out(&mut self, laid: &Laid<'_>, height: usize) -> Shown {
        let mut shown = Shown::default();
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
                shown.lines.extend(taken);
                skip = 0;
                at += 1;
            }
            if shown.lines.len() >= height {
                shown.lines.truncate(height);
                return shown;
            }
            self.top = None;
            shown = Shown::default();
        }
        // From the end up, as many rows as there is room for.
        let mut parts = Vec::new();
        let mut rows = 0;
        for at in (0..laid.len()).rev() {
            if rows >= height {
                break;
            }
            let mut images = Vec::new();
            let lines = laid.lines(at, &mut images);
            rows += lines.len();
            parts.push((lines, images));
        }
        for (lines, images) in parts.into_iter().rev() {
            shown.lines.extend(lines);
            shown.images.extend(images);
        }
        let over = shown.lines.len().saturating_sub(height);
        shown.lines.drain(..over);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Depth, Palette};
    use iq_agent::{Event, PERSON};
    use iq_ui::look::Theme;
    use iq_ui::{Board, Update};

    fn said(board: &mut Board, history: &mut History, p: &Palette, text: &str) {
        let event = Event::Text {
            agent: "lead".into(),
            text: text.into(),
        };
        board.update(Update::Crew(event.clone()));
        history.event(&event, board, p);
    }

    fn rows(shown: &Shown) -> Vec<String> {
        shown
            .lines
            .iter()
            .map(|l| l.to_string().trim().to_owned())
            .collect()
    }

    #[test]
    fn the_view_follows_the_end_and_stays_put_once_scrolled_up() {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        let joined = Event::Joined {
            agent: "lead".into(),
            model: "lab/lead".into(),
            by: PERSON.into(),
        };
        board.update(Update::Crew(joined.clone()));
        history.event(&joined, &board, &p);
        let lines: String = (1..=20).map(|n| format!("line {n}\n\n")).collect();
        said(&mut board, &mut history, &p, &lines);
        let _ = history.take(40, Some("lead"));
        let mut drawn = View::default();
        let shown = drawn.shown(&history, Some("lead"), 40, 5, &[]);
        assert_eq!(rows(&shown).last().map(String::as_str), Some("line 20"));
        // Up by 3 pages less a row each: what shows stays as more comes.
        let up = drawn.shown(&history, Some("lead"), 40, 5, &[Reading::Page(-3)]);
        let before = rows(&up);
        assert!(before.contains(&"line 14".to_owned()), "{before:#?}");
        said(&mut board, &mut history, &p, "line 21\n");
        let _ = history.take(40, Some("lead"));
        assert_eq!(
            rows(&drawn.shown(&history, Some("lead"), 40, 5, &[])),
            before
        );
        // Never above the start.
        let top = drawn.shown(&history, Some("lead"), 40, 5, &[Reading::Scroll(-500)]);
        assert_eq!(rows(&top)[1], "● lead");
        // Back down to the end, it follows it again.
        drawn.shown(&history, Some("lead"), 40, 5, &[Reading::Scroll(500)]);
        said(&mut board, &mut history, &p, "line 22\n");
        let _ = history.take(40, Some("lead"));
        let end = drawn.shown(&history, Some("lead"), 40, 5, &[]);
        assert_eq!(rows(&end).last().map(String::as_str), Some("line 22"));
    }
}
