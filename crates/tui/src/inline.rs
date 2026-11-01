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
//! do a whole screen's. While the panes of agents at work side by side
//! hold the top rows, the region starts below them, and what it scrolls
//! away is not kept: the window is written again whole once they go.

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
    /// The rows at the top held by the panes, none when there are none.
    head: u16,
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
            head: 0,
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

    /// Holds the `head` rows at the top for the panes, on a screen just
    /// wiped: the conversation scrolls below them.
    pub(crate) fn hold(&mut self, head: u16) {
        self.head = head.min(self.rows.saturating_sub(3));
    }

    /// The rows held at the top for the panes.
    pub(crate) fn head(&self) -> u16 {
        self.head
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
        let height = height.clamp(1, self.rows.saturating_sub(self.head + 1).max(1));
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
        let first = self.head + 1;
        if to < from {
            write!(out, "\x1b[{first};{from}r")?;
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
            write!(out, "\x1b[{first};{to}r")?;
            queue!(out, MoveTo(0, self.head))?;
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
        if lines.is_empty() || self.top <= self.head {
            return Ok(());
        }
        write!(out, "\x1b[{};{}r", self.head + 1, self.top)?;
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
        self.head = self.head.min(rows.saturating_sub(3));
        self.height = self
            .height
            .clamp(1, rows.saturating_sub(self.head + 1).max(1));
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

/// Writes `line` where the cursor stands, each span in its style, a span
/// that carries a link's address made a hyperlink to it.
fn write_line(out: &mut impl Write, line: &Line<'_>) -> io::Result<()> {
    for span in &line.spans {
        let style = line.style.patch(span.style);
        let text = printable(&span.content);
        match crate::link::address_of(style) {
            Some((number, address, plain)) => {
                let (open, close) = crate::link::escapes(number, &address);
                let plain: ContentStyle = plain.into_crossterm();
                out.write_all(open.as_bytes())?;
                queue!(out, PrintStyledContent(plain.apply(text)))?;
                out.write_all(close.as_bytes())?;
            }
            None => {
                let style: ContentStyle = style.into_crossterm();
                queue!(out, PrintStyledContent(style.apply(text)))?;
            }
        }
    }
    Ok(())
}

/// `text` without its control characters. What the conversation shows
/// comes from models, and through them from files, pages and servers: an
/// escape sequence among it would be the terminal's to obey (writing to the
/// clipboard, moving the cursor, renaming the window), so none is written
/// but those iq makes itself. ratatui does the same for the live part.
fn printable(text: &str) -> std::borrow::Cow<'_, str> {
    if text.chars().any(char::is_control) {
        text.chars().filter(|c| !c.is_control()).collect()
    } else {
        std::borrow::Cow::Borrowed(text)
    }
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

    #[test]
    fn no_escape_written_in_the_conversation_reaches_the_terminal() {
        let mut out = Vec::new();
        let mut inline = Inline::new(40, 6);
        inline.place(&mut out, 1).unwrap();
        let mut written = Vec::new();
        // A clipboard write, a title and a bell, as a model could echo them.
        inline
            .insert(
                &mut written,
                &[Line::from(
                    "ok \u{1b}]52;c;aGk=\u{7}\u{1b}]0;title\u{7}done\u{9b}2J",
                )],
            )
            .unwrap();
        let text = String::from_utf8(written).unwrap();
        assert!(text.contains("ok ") && text.contains("]52;c;aGk=") && text.contains("done"));
        assert!(!text.contains("\u{1b}]52") && !text.contains("\u{7}") && !text.contains('\u{9b}'));
    }

    #[test]
    fn a_link_is_written_as_a_hyperlink_around_its_text_alone() {
        let style =
            crate::link::mark(ratatui::style::Style::new(), "https://example.com/x").unwrap();
        let (number, _, _) = crate::link::address_of(style).unwrap();
        let mut inline = Inline::new(40, 6);
        inline.place(&mut Vec::new(), 1).unwrap();
        let mut written = Vec::new();
        inline
            .insert(
                &mut written,
                &[Line::from(vec![
                    ratatui::text::Span::raw("see "),
                    ratatui::text::Span::styled("x", style),
                ])],
            )
            .unwrap();
        let text = String::from_utf8(written).unwrap();
        let link = format!("\u{1b}]8;id=iq{number};https://example.com/x\u{1b}\\");
        let at = text.find(&link).expect("the hyperlink opens");
        let closed = text[at..].find("\u{1b}]8;;\u{1b}\\").expect("and closes");
        assert!(text[at..at + closed].contains('x'));
        // Its address rides on no background the terminal would paint.
        assert!(!text.contains("48;2;"), "{text:?}");
    }

    #[test]
    fn below_held_rows_the_conversation_scrolls_and_leaves_them_be() {
        let size = |n: u16| std::num::NonZeroU16::new(n).unwrap();
        let mut parser = vt100::Parser::new(size(8), size(20), 50);
        let mut out = Vec::new();
        let mut inline = Inline::new(20, 8);
        inline.hold(2);
        assert!(inline.place(&mut out, 2).unwrap());
        // What the panes draw at the top.
        out.extend_from_slice(b"\x1b[1;1Hpane one\x1b[2;1Hpane two");
        let lines: Vec<Line> = ["a", "b", "c", "d", "e"]
            .into_iter()
            .map(Line::from)
            .collect();
        inline.insert(&mut out, &lines).unwrap();
        parser.process(&out);
        assert_eq!(
            screen(&parser)[..8],
            ["pane one", "pane two", "b", "c", "d", "e", "", ""]
        );
    }
}
