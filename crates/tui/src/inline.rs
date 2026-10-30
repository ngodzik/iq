//! Where the live part stands on the terminal's screen: at its bottom,
//! the conversation above it, its last line just above. A line written
//! scrolls the rows above the live part up, and their top line goes into
//! the terminal's own scrollback, as any output does; the live part itself
//! never scrolls, it is drawn again where it stands. As it grows, the
//! conversation above goes up to make room; as it shrinks, the
//! conversation comes down with it, so that nothing gapes between them.
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
    /// Its first row: the screen's height less its own.
    top: u16,
    /// How many rows it takes.
    height: u16,
    cols: u16,
    rows: u16,
}

impl Inline {
    /// The live part on a screen of `cols` by `rows`, empty, at its bottom.
    pub(crate) fn new(cols: u16, rows: u16) -> Inline {
        Inline {
            top: rows,
            height: 0,
            cols,
            rows,
        }
    }

    /// Sends the `rows` at the top of the screen into the terminal's
    /// scrollback, as iq opens: what the shell showed is scrolled away,
    /// not lost.
    pub(crate) fn clear_above(out: &mut impl Write, rows: u16, screen_rows: u16) -> io::Result<()> {
        queue!(out, MoveTo(0, screen_rows.saturating_sub(1)))?;
        for _ in 0..rows {
            out.write_all(b"\n")?;
        }
        queue!(out, MoveTo(0, 0))
    }

    /// Where it is.
    pub(crate) fn area(&self) -> Rect {
        Rect::new(0, self.top, self.cols, self.height)
    }

    /// How wide the screen is.
    pub(crate) fn cols(&self) -> u16 {
        self.cols
    }

    /// Gives the live part `height` rows at the bottom, at least one row
    /// of the screen left above it, the conversation moved to stay just
    /// above it. Returns whether it changed, to be drawn again whole.
    pub(crate) fn place(&mut self, out: &mut impl Write, height: u16) -> io::Result<bool> {
        let height = height.clamp(1, self.rows.saturating_sub(1).max(1));
        let before = (self.top, self.height);
        let top = self.rows - height;
        self.follow(out, self.top.min(self.rows), top)?;
        self.top = top;
        self.height = height;
        Ok((self.top, self.height) != before)
    }

    /// Moves the conversation, whose last line is above row `from`, to end
    /// above row `to`: up, its top lines into the scrollback, or down, the
    /// rows between cleared first.
    fn follow(&self, out: &mut impl Write, from: u16, to: u16) -> io::Result<()> {
        if to < from {
            write!(out, "\x1b[1;{from}r")?;
            queue!(out, MoveTo(0, from - 1))?;
            for _ in to..from {
                out.write_all(b"\n")?;
            }
            out.write_all(b"\x1b[r")?;
        } else if to > from {
            for y in from..to {
                queue!(out, MoveTo(0, y), Clear(ClearType::CurrentLine))?;
            }
            // A reverse index at the region's top scrolls it down.
            write!(out, "\x1b[1;{to}r")?;
            queue!(out, MoveTo(0, 0))?;
            for _ in from..to {
                out.write_all(b"\x1bM")?;
            }
            out.write_all(b"\x1b[r")?;
        }
        Ok(())
    }

    /// Writes `lines` just above the live part, the rows above it scrolled
    /// up to make room.
    pub(crate) fn insert(&mut self, out: &mut impl Write, lines: &[Line<'_>]) -> io::Result<()> {
        if lines.is_empty() || self.top == 0 {
            return Ok(());
        }
        write!(out, "\x1b[1;{}r", self.top)?;
        queue!(out, MoveTo(0, self.top - 1))?;
        for line in lines {
            out.write_all(b"\n\r")?;
            queue!(out, Clear(ClearType::CurrentLine))?;
            write_line(out, line)?;
        }
        out.write_all(b"\x1b[r")?;
        Ok(())
    }

    /// The screen is now `cols` by `rows`, the live part's first row at
    /// `top` when the terminal said where its cursor went, which tells
    /// where it moved what it showed. What the live part showed is
    /// cleared, and the conversation moved to end above where it now
    /// stands, at the bottom.
    pub(crate) fn resized(
        &mut self,
        out: &mut impl Write,
        cols: u16,
        rows: u16,
        top: Option<u16>,
    ) -> io::Result<()> {
        let was = top.unwrap_or(self.top).min(rows);
        queue!(out, MoveTo(0, was), Clear(ClearType::FromCursorDown))?;
        self.cols = cols;
        self.rows = rows;
        self.height = self.height.clamp(1, rows.saturating_sub(1).max(1));
        let top = rows - self.height;
        self.follow(out, was, top)?;
        self.top = top;
        Ok(())
    }

    /// Clears the live part as iq ends, the cursor where it began: what
    /// the shell writes next goes just below the conversation.
    pub(crate) fn leave(&mut self, out: &mut impl Write) -> io::Result<()> {
        queue!(
            out,
            MoveTo(0, self.top.min(self.rows)),
            Clear(ClearType::FromCursorDown)
        )?;
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
    fn the_live_part_stays_at_the_bottom_the_conversation_just_above() {
        let size = |n: u16| std::num::NonZeroU16::new(n).unwrap();
        let mut parser = vt100::Parser::new(size(6), size(20), 50);
        let mut out = Vec::new();
        // The shell's prompt, scrolled away as iq opens.
        parser.process(b"$ ls\r\nfile\r\n$ iq\r\n");
        Inline::clear_above(&mut out, 3, 6).unwrap();
        let mut inline = Inline::new(20, 6);
        assert!(inline.place(&mut out, 2).unwrap());
        assert_eq!(inline.area(), Rect::new(0, 4, 20, 2));
        let lines: Vec<Line> = ["one", "two", "three"]
            .into_iter()
            .map(Line::from)
            .collect();
        inline.insert(&mut out, &lines).unwrap();
        parser.process(&out);
        assert_eq!(screen(&parser), ["", "one", "two", "three", "", ""]);
        let mut out = Vec::new();
        inline
            .insert(&mut out, &[Line::from("four"), Line::from("five")])
            .unwrap();
        parser.process(&out);
        assert_eq!(screen(&parser), ["two", "three", "four", "five", "", ""]);
        // Growing, it pushes the conversation up; shrinking, it brings it
        // down with it.
        let mut out = Vec::new();
        assert!(inline.place(&mut out, 3).unwrap());
        assert_eq!(inline.area(), Rect::new(0, 3, 20, 3));
        parser.process(&out);
        assert_eq!(screen(&parser)[..3], ["three", "four", "five"]);
        let mut out = Vec::new();
        assert!(inline.place(&mut out, 1).unwrap());
        parser.process(&out);
        assert_eq!(screen(&parser), ["", "", "three", "four", "five", ""]);
        assert!(!inline.place(&mut Vec::new(), 1).unwrap());
    }
}
