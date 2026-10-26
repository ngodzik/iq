//! Where the live part stands on the terminal's screen: just below the
//! conversation, which iq writes above it. While there is room under it,
//! a line written moves it down; once it reaches the bottom, the rows
//! above it scroll, and their top line goes into the terminal's own
//! scrollback, as any output does. The live part itself never scrolls: it
//! is drawn again where it stands.
//!
//! The rows above are scrolled within a scrolling region that starts at
//! the top of the screen, which terminals keep in their scrollback as they
//! do a whole screen's.

use std::io::{self, Write};

use ratatui::backend::IntoCrossterm;
use ratatui::crossterm::cursor::MoveTo;
use ratatui::crossterm::queue;
use ratatui::crossterm::style::{ContentStyle, PrintStyledContent};
use ratatui::crossterm::terminal::{Clear, ClearType};
use ratatui::layout::Rect;
use ratatui::text::Line;

/// The live part's place on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Inline {
    /// Its first row.
    top: u16,
    /// How many rows it takes.
    height: u16,
    cols: u16,
    rows: u16,
}

impl Inline {
    /// The live part starting at `top`, empty, on a screen of `cols` by
    /// `rows`.
    pub(crate) fn new(top: u16, cols: u16, rows: u16) -> Inline {
        Inline {
            top: top.min(rows.saturating_sub(1)),
            height: 0,
            cols,
            rows,
        }
    }

    /// Where it is.
    pub(crate) fn area(&self) -> Rect {
        Rect::new(0, self.top, self.cols, self.height)
    }

    /// How wide the screen is.
    pub(crate) fn cols(&self) -> u16 {
        self.cols
    }

    /// Gives the live part `height` rows, at least one row of the screen
    /// left above it: when there is no room below it, the screen scrolls
    /// up to make it. Returns whether it moved or changed its size, to be
    /// drawn again whole.
    pub(crate) fn place(&mut self, out: &mut impl Write, height: u16) -> io::Result<bool> {
        let height = height.clamp(1, self.rows.saturating_sub(1).max(1));
        let before = (self.top, self.height);
        if self.top + height > self.rows {
            let over = self.top + height - self.rows;
            queue!(out, MoveTo(0, self.rows.saturating_sub(1)))?;
            for _ in 0..over {
                out.write_all(b"\n")?;
            }
            self.top = self.top.saturating_sub(over);
        }
        if height < self.height {
            // What it left below itself goes.
            for y in self.top + height..(self.top + self.height).min(self.rows) {
                queue!(out, MoveTo(0, y), Clear(ClearType::CurrentLine))?;
            }
        }
        self.height = height;
        Ok((self.top, self.height) != before)
    }

    /// Writes `lines` above the live part. Returns whether it moved down,
    /// to be drawn again where it now stands.
    pub(crate) fn insert(&mut self, out: &mut impl Write, lines: &[Line<'_>]) -> io::Result<bool> {
        if lines.is_empty() {
            return Ok(false);
        }
        let below = self.rows.saturating_sub(self.top + self.height);
        let direct = lines.len().min(usize::from(below));
        for (at, line) in lines[..direct].iter().enumerate() {
            queue!(
                out,
                MoveTo(0, self.top + at as u16),
                Clear(ClearType::CurrentLine)
            )?;
            write_line(out, line)?;
        }
        self.top += direct as u16;
        let rest = &lines[direct..];
        if !rest.is_empty() && self.top > 0 {
            // Only the rows above the live part scroll.
            write!(out, "\x1b[1;{}r", self.top)?;
            queue!(out, MoveTo(0, self.top - 1))?;
            for line in rest {
                out.write_all(b"\n\r")?;
                queue!(out, Clear(ClearType::CurrentLine))?;
                write_line(out, line)?;
            }
            out.write_all(b"\x1b[r")?;
        }
        Ok(direct > 0)
    }

    /// The screen is now `cols` by `rows`, the live part's first row at
    /// `top` when the terminal said where its cursor went, which tells
    /// where it moved what it showed. What the live part showed is cleared,
    /// to be drawn again.
    pub(crate) fn resized(
        &mut self,
        out: &mut impl Write,
        cols: u16,
        rows: u16,
        top: Option<u16>,
    ) -> io::Result<()> {
        self.cols = cols;
        self.rows = rows;
        if let Some(top) = top {
            self.top = top;
        }
        self.top = self.top.min(rows.saturating_sub(self.height.max(1)));
        queue!(out, MoveTo(0, self.top), Clear(ClearType::FromCursorDown))?;
        Ok(())
    }

    /// Leaves the cursor below the live part, cleared, as iq ends: what
    /// the shell writes next goes there.
    pub(crate) fn leave(&mut self, out: &mut impl Write) -> io::Result<()> {
        queue!(out, MoveTo(0, self.top), Clear(ClearType::FromCursorDown))?;
        self.height = 0;
        Ok(())
    }
}

/// Writes `line` where the cursor stands, each span in its style.
fn write_line(out: &mut impl Write, line: &Line<'_>) -> io::Result<()> {
    for span in &line.spans {
        let style: ContentStyle = line.style.patch(span.style).into_crossterm();
        queue!(out, PrintStyledContent(style.apply(span.content.as_ref())))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(parser: &vt100::Parser) -> Vec<String> {
        parser
            .screen()
            .rows(0, 20)
            .map(|r| r.trim_end().to_owned())
            .collect()
    }

    #[test]
    fn lines_go_above_the_live_part_which_goes_down_then_stays() {
        let size = |n: u16| std::num::NonZeroU16::new(n).unwrap();
        let mut parser = vt100::Parser::new(size(6), size(20), 50);
        let mut out = Vec::new();
        // The shell's prompt, then iq at the next row.
        parser.process(b"$ iq\r\n");
        let mut inline = Inline::new(1, 20, 6);
        assert!(inline.place(&mut out, 2).unwrap());
        let lines: Vec<Line> = ["one", "two", "three"]
            .into_iter()
            .map(Line::from)
            .collect();
        assert!(inline.insert(&mut out, &lines).unwrap());
        assert_eq!(inline.area(), Rect::new(0, 4, 20, 2));
        // At the bottom: the rows above scroll, the live part stays.
        assert!(
            !inline
                .insert(&mut out, &[Line::from("four"), Line::from("five")])
                .unwrap()
        );
        assert_eq!(inline.area(), Rect::new(0, 4, 20, 2));
        parser.process(&out);
        assert_eq!(screen(&parser), ["two", "three", "four", "five", "", ""]);
        // Growing at the bottom scrolls the screen up.
        let mut out = Vec::new();
        assert!(inline.place(&mut out, 3).unwrap());
        assert_eq!(inline.area(), Rect::new(0, 3, 20, 3));
        parser.process(&out);
        assert_eq!(screen(&parser)[..3], ["three", "four", "five"]);
    }
}
