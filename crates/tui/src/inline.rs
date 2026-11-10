//! Where the live part stands on the terminal's screen: at its bottom,
//! the conversation above it, its last line just above. A line written
//! scrolls the rows above the live part up, and their top line goes into
//! the terminal's own scrollback, as any output does; the live part itself
//! never scrolls, it is drawn again where it stands. As it grows, the
//! conversation above goes up to make room. As it shrinks, the
//! conversation comes down with it while all of it is on the screen; once
//! its start went into the scrollback, it stays where it is and the rows
//! given back wait just above the live part for the next lines: bringing
//! it down would put empty rows above it, which the terminal would keep in
//! its scrollback amid the conversation, rows no model wrote.
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
use unicode_width::UnicodeWidthChar;

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
    /// The row below the conversation's last: the live part's first, but
    /// for the rows it gave back that no line filled yet.
    end: u16,
    /// The empty rows above the conversation while all of it is on the
    /// screen; none once its start went into the scrollback, after which
    /// no row is ever added above it.
    room: Option<u16>,
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
            end: rows,
            room: Some(rows),
        }
    }

    /// Says that the conversation began above the screen, its start in the
    /// scrollback already: its end was written again at another width.
    pub(crate) fn began_above(&mut self) {
        self.room = None;
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
        self.room = self
            .room
            .map(|room| room.min(self.end.saturating_sub(self.head)));
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
    /// above it as far as it may. Returns whether it changed, to be drawn
    /// again whole.
    pub(crate) fn place(&mut self, out: &mut impl Write, height: u16) -> io::Result<bool> {
        let height = height.clamp(1, self.rows.saturating_sub(self.head + 1).max(1));
        let before = (self.top, self.height);
        let top = self.rows - height;
        let was = self.top.min(self.rows);
        if top > was {
            // What the live part showed in the rows it gives back.
            for y in was..top {
                queue!(out, MoveTo(0, y), Clear(ClearType::CurrentLine))?;
            }
        }
        self.follow(out, top)?;
        self.top = top;
        self.height = height;
        Ok((self.top, self.height) != before)
    }

    /// Moves the conversation to end above row `to`: up, its top lines into
    /// the scrollback, once the rows given back below it are taken; down,
    /// while all of it is on the screen, empty rows put above it. Once its
    /// start is in the scrollback, it does not come down: the rows between
    /// it and `to` stay empty for the lines to come.
    fn follow(&mut self, out: &mut impl Write, to: u16) -> io::Result<()> {
        let first = self.head + 1;
        let from = self.end.min(self.rows);
        if to < from {
            let up = from - to;
            write!(out, "\x1b[{first};{from}r")?;
            queue!(out, MoveTo(0, from - 1))?;
            for _ in 0..up {
                out.write_all(b"\n")?;
            }
            out.write_all(b"\x1b[r")?;
            self.scrolled(up);
            self.end = to;
        } else if to > from
            && let Some(room) = self.room
        {
            let down = to - from;
            // A reverse index at the region's top scrolls it down.
            write!(out, "\x1b[{first};{to}r")?;
            queue!(out, MoveTo(0, self.head))?;
            for _ in 0..down {
                out.write_all(b"\x1bM")?;
            }
            out.write_all(b"\x1b[r")?;
            self.room = Some(room + down);
            self.end = to;
        }
        Ok(())
    }

    /// Counts `up` rows of the conversation scrolled up: from the empty rows
    /// above it, then from it, its start into the scrollback.
    fn scrolled(&mut self, up: u16) {
        self.room = self.room.and_then(|room| room.checked_sub(up));
    }

    /// Writes `lines` just above the live part, the rows above it scrolled
    /// up to make room.
    pub(crate) fn insert(&mut self, out: &mut impl Write, lines: &[Line<'_>]) -> io::Result<()> {
        if lines.is_empty() || self.top <= self.head {
            return Ok(());
        }
        let end = self.end.clamp(self.head, self.top);
        let written = lines
            .iter()
            .map(|line| rows_of(line, self.cols))
            .fold(0u16, u16::saturating_add);
        write!(out, "\x1b[{};{}r", self.head + 1, self.top)?;
        // Below the conversation's last line: in the rows the live part
        // gave back while there are some, at the region's bottom then,
        // where each new line scrolls it up.
        let mut lines = lines.iter();
        if end == self.head {
            queue!(out, MoveTo(0, end), Clear(ClearType::CurrentLine))?;
            if let Some(line) = lines.next() {
                write_line(out, line)?;
            }
        } else {
            queue!(out, MoveTo(0, end - 1))?;
        }
        for line in lines {
            out.write_all(b"\n\r")?;
            queue!(out, Clear(ClearType::CurrentLine))?;
            write_line(out, line)?;
        }
        out.write_all(b"\x1b[r")?;
        let below = self.top - end;
        if written <= below {
            self.end = end + written;
        } else {
            self.scrolled(written - below);
            self.end = self.top;
        }
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
        // The conversation moved with the live part, the rows between
        // them, if any, kept.
        let gap = self.top.min(self.rows).saturating_sub(self.end);
        self.cols = cols;
        self.rows = rows;
        self.head = self.head.min(rows.saturating_sub(3));
        self.end = was.saturating_sub(gap).max(self.head);
        self.height = self
            .height
            .clamp(1, rows.saturating_sub(self.head + 1).max(1));
        let top = rows - self.height;
        self.follow(out, top)?;
        self.top = top;
        Ok(())
    }

    /// Clears the live part as iq ends, the cursor where it began: what
    /// the shell writes next goes just below the conversation.
    pub(crate) fn leave(&mut self, out: &mut impl Write) -> io::Result<()> {
        queue!(
            out,
            MoveTo(0, self.end.min(self.top).min(self.rows)),
            Clear(ClearType::FromCursorDown)
        )?;
        Ok(())
    }
}

/// How many rows of a screen `cols` wide `line` takes, folded by the
/// terminal as it folds it: a wide character that does not fit at a row's
/// end goes whole to the next.
fn rows_of(line: &Line<'_>, cols: u16) -> u16 {
    let cols = usize::from(cols.max(1));
    let (mut rows, mut col) = (1u16, 0usize);
    for c in line
        .spans
        .iter()
        .flat_map(|span| span.content.chars())
        .filter(|c| !c.is_control())
    {
        let width = c.width().unwrap_or(0);
        if col + width > cols {
            rows = rows.saturating_add(1);
            col = 0;
        }
        col += width;
    }
    rows
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

    /// Every row the terminal holds, its scrollback's then its screen's.
    fn history(parser: &mut vt100::Parser) -> Vec<String> {
        parser.screen_mut().set_scrollback(usize::MAX);
        let back = parser.screen().scrollback();
        let mut all = Vec::new();
        for offset in (1..=back).rev() {
            parser.screen_mut().set_scrollback(offset);
            all.extend(
                parser
                    .screen()
                    .rows(0, 20)
                    .take(1)
                    .map(|r| r.trim_end().to_owned()),
            );
        }
        parser.screen_mut().set_scrollback(0);
        all.extend(screen(parser));
        all
    }

    #[test]
    fn the_live_part_shrinking_leaves_no_empty_row_in_the_conversation() {
        let size = |n: u16| std::num::NonZeroU16::new(n).unwrap();
        let mut parser = vt100::Parser::new(size(6), size(20), 50);
        let mut out = Vec::new();
        let mut inline = Inline::new(20, 6);
        inline.place(&mut out, 2).unwrap();
        let lines: Vec<Line> = ["one", "two", "three", "four", "five"]
            .into_iter()
            .map(Line::from)
            .collect();
        inline.insert(&mut out, &lines).unwrap();
        // A list of choices opens below, and closes.
        inline.place(&mut out, 4).unwrap();
        inline.place(&mut out, 2).unwrap();
        let more: Vec<Line> = ["six", "seven", "eight", "nine"]
            .into_iter()
            .map(Line::from)
            .collect();
        inline.insert(&mut out, &more).unwrap();
        parser.process(&out);
        let all = history(&mut parser);
        let written: Vec<&str> = all
            .iter()
            .map(String::as_str)
            .skip_while(|row| row.is_empty())
            .collect();
        assert_eq!(
            written[..9],
            [
                "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"
            ],
            "{all:?}"
        );
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
        // Growing, it pushes the conversation up. Shrinking, it leaves the
        // rows it gives back empty, the conversation's start being in the
        // scrollback: the next lines take them first.
        let mut out = Vec::new();
        assert!(inline.place(&mut out, 3).unwrap());
        assert_eq!(inline.area(), Rect::new(0, 3, 20, 3));
        parser.process(&out);
        assert_eq!(screen(&parser)[..3], ["three", "four", "five"]);
        let mut out = Vec::new();
        assert!(inline.place(&mut out, 1).unwrap());
        parser.process(&out);
        assert_eq!(screen(&parser), ["three", "four", "five", "", "", ""]);
        assert!(!inline.place(&mut Vec::new(), 1).unwrap());
        let mut out = Vec::new();
        inline
            .insert(&mut out, &[Line::from("six"), Line::from("seven")])
            .unwrap();
        parser.process(&out);
        assert_eq!(
            screen(&parser),
            ["three", "four", "five", "six", "seven", ""]
        );
        let mut out = Vec::new();
        inline
            .insert(&mut out, &[Line::from("eight"), Line::from("nine")])
            .unwrap();
        parser.process(&out);
        assert_eq!(
            screen(&parser),
            ["five", "six", "seven", "eight", "nine", ""]
        );
    }

    #[test]
    fn a_conversation_wholly_on_the_screen_comes_down_with_the_live_part() {
        let size = |n: u16| std::num::NonZeroU16::new(n).unwrap();
        let mut parser = vt100::Parser::new(size(8), size(20), 50);
        let mut out = Vec::new();
        let mut inline = Inline::new(20, 8);
        inline.place(&mut out, 2).unwrap();
        inline
            .insert(&mut out, &[Line::from("one"), Line::from("two")])
            .unwrap();
        inline.place(&mut out, 4).unwrap();
        parser.process(&out);
        assert_eq!(screen(&parser)[..4], ["", "", "one", "two"]);
        let mut out = Vec::new();
        inline.place(&mut out, 2).unwrap();
        parser.process(&out);
        assert_eq!(screen(&parser)[..6], ["", "", "", "", "one", "two"]);
    }

    #[test]
    fn a_line_the_terminal_folds_is_counted_by_its_rows() {
        assert_eq!(rows_of(&Line::from(""), 10), 1);
        assert_eq!(rows_of(&Line::from("a".repeat(10)), 10), 1);
        assert_eq!(rows_of(&Line::from("a".repeat(11)), 10), 2);
        // A wide character that would straddle the edge starts the next row.
        assert_eq!(rows_of(&Line::from(format!("{}界", "a".repeat(9))), 10), 2);
        let size = |n: u16| std::num::NonZeroU16::new(n).unwrap();
        let mut parser = vt100::Parser::new(size(6), size(10), 50);
        let mut out = Vec::new();
        let mut inline = Inline::new(10, 6);
        inline.place(&mut out, 1).unwrap();
        let lines: Vec<Line> = ["a", "b", "c", "d", "e"]
            .into_iter()
            .map(Line::from)
            .collect();
        inline.insert(&mut out, &lines).unwrap();
        inline.place(&mut out, 4).unwrap();
        inline.place(&mut out, 1).unwrap();
        let long = "x".repeat(15);
        inline
            .insert(&mut out, &[Line::from(long.as_str()), Line::from("after")])
            .unwrap();
        parser.process(&out);
        assert_eq!(
            screen(&parser)[..5],
            ["d", "e", &long[..10], &long[10..], "after"]
        );
    }

    #[test]
    fn a_line_wider_than_the_screen_is_folded_by_the_terminal_itself() {
        let size = |n: u16| std::num::NonZeroU16::new(n).unwrap();
        let mut parser = vt100::Parser::new(size(6), size(20), 50);
        let mut out = Vec::new();
        let mut inline = Inline::new(20, 6);
        inline.place(&mut out, 2).unwrap();
        // What the live part draws on its first row.
        out.extend_from_slice(b"\x1b[5;1Hlive");
        let long = format!("cp {} b", "a".repeat(25));
        inline
            .insert(&mut out, &[Line::from(long.as_str()), Line::from("after")])
            .unwrap();
        parser.process(&out);
        assert_eq!(
            screen(&parser)[..6],
            ["", &long[..20], &long[20..], "after", "live", ""]
        );
        // Folded by the terminal, not cut by iq: a selection of it copies
        // one line.
        assert!(parser.screen().row_wrapped(1));
        assert_eq!(
            parser.screen().contents_between(1, 0, 2, 20).trim_end(),
            long
        );
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
