//! iq in the terminal, as every terminal agent is: the conversation
//! written into the terminal's own scrollback as it happens, scrolled,
//! selected and copied as any output; below it a live part, drawn again as
//! things move: the agents at work, the question waiting, where the person
//! writes, the status line; and the pages (the crew, the costs, the
//! observations, the keys) on the terminal's other screen, which gives
//! the conversation back as it was when they close.
//!
//! It draws `iq-ui`'s state and hands it the keys, which it turns into what
//! the person asks of the crew: nothing here decides what a key does.
//! Colours are 24-bit where the terminal says it takes them, and the
//! terminal's background is never painted. Nothing is drawn while nothing
//! moves but the clock: the loop then wakes a few times a second.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod chart;
mod history;
mod inline;
mod keys;
mod live;
mod markdown;
mod motion;
mod pages;
mod theme;
mod tty;
mod wrap;

use std::io::{self, Write};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use iq_core::Usd;
use iq_ui::crew::Request;
use iq_ui::{Board, Update};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::MoveTo;
use ratatui::crossterm::event::{self, Event, KeyEventKind, MouseEventKind};
use ratatui::crossterm::terminal::{Clear, ClearType};
use ratatui::crossterm::{cursor, queue, terminal};
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};

use crate::history::History;
use crate::inline::Inline;
use crate::live::Live;
use crate::motion::Motion;
use crate::theme::{Depth, Palette};
use crate::tty::{Tty, Wait};

/// How long the loop waits for a key while something moves.
const MOVING: Duration = Duration::from_millis(33);

/// The same while nothing does.
const STILL: Duration = Duration::from_millis(250);

/// How often the live part is drawn again anyway, for its clocks.
const CLOCK: Duration = Duration::from_secs(1);

/// Runs iq in the terminal until the person quits or iq is asked to stop:
/// `board` as the settings made it, what `feed` brings, what the person
/// asks of the crew sent on `requests`. The terminal is given back as it
/// was, the conversation left in its scrollback.
///
/// # Errors
///
/// The terminal could not be taken, read or written.
pub fn run(feed: Receiver<Update>, requests: Sender<Request>, board: Board) -> io::Result<()> {
    let tty = Tty::take()?;
    let (cols, rows) = terminal::size()?;
    // What the shell showed goes into the scrollback: iq opens on a clean
    // screen, the live part at its bottom.
    let (_, row) = cursor::position()?;
    let mut out = io::stdout();
    Inline::clear_above(&mut out, row, rows)?;
    out.flush()?;
    let screen = Screen {
        tty,
        out,
        inline: Inline::new(cols, rows),
        terminal: Terminal::with_options(
            CrosstermBackend::new(io::stdout()),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, rows.saturating_sub(1), cols, 1)),
            },
        )?,
        paged: false,
        away: None,
        caret: None,
    };
    let depth = Depth::detect();
    let p = Palette::new(board.look().theme(), depth);
    let mut iq = Iq {
        screen,
        board,
        history: History::default(),
        motion: Motion::default(),
        depth,
        p,
        requests,
    };
    let ran = iq.run(&feed);
    let ended = iq.end();
    ran.and(ended)
}

/// The terminal and where iq stands on it.
struct Screen {
    tty: Tty,
    out: io::Stdout,
    inline: Inline,
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    /// Whether a page shows on the other screen.
    paged: bool,
    /// Whether the terminal was resized while a page showed, and made
    /// narrower.
    away: Option<bool>,
    /// Where the cursor stood in the live part when last drawn, from its
    /// top: after a resize, where the terminal moved it says where the
    /// live part went.
    caret: Option<u16>,
}

impl Screen {
    /// Draws from now on in `area`, cleared, all of it drawn again. The
    /// terminal is made anew rather than resized: resized narrower,
    /// ratatui clears the whole screen and moves what it draws to its top.
    fn refit(&mut self, area: Rect) -> io::Result<()> {
        for y in area.top()..area.bottom() {
            queue!(self.out, MoveTo(0, y), Clear(ClearType::CurrentLine))?;
        }
        self.out.flush()?;
        self.terminal = Terminal::with_options(
            CrosstermBackend::new(io::stdout()),
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )?;
        Ok(())
    }
}

/// iq at work in the terminal.
struct Iq {
    screen: Screen,
    board: Board,
    history: History,
    motion: Motion,
    depth: Depth,
    p: Palette,
    requests: Sender<Request>,
}

impl Iq {
    fn run(&mut self, feed: &Receiver<Update>) -> io::Result<()> {
        let mut drawn = Instant::now() - CLOCK;
        let mut changed = true;
        loop {
            let now = iq_ui::now();
            loop {
                match feed.try_recv() {
                    Ok(update) => {
                        self.take(update);
                        changed = true;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Ok(()),
                }
            }
            for post in self.board.take_posts() {
                self.motion.posted(&post);
            }
            if let Some(recall) = self.board.take_recall() {
                self.history.recall(&recall, &self.board, &self.p);
                changed = true;
            }
            if self.board.quitting() {
                return Ok(());
            }
            let look = Palette::new(self.board.look().theme(), self.depth);
            if look.dark != self.p.dark || look.accent != self.p.accent {
                self.p = look;
                changed = true;
            }
            changed |= self.turn_page()?;
            let moving =
                self.motion.moving() || self.board.crew().agents().iter().any(|a| a.working);
            if changed || moving || drawn.elapsed() >= CLOCK {
                self.draw(now)?;
                drawn = Instant::now();
            }
            changed = self.keys(if moving { MOVING } else { STILL })?;
        }
    }

    /// Takes what the program sent: the conversation written from what the
    /// crew did, once the board took it.
    fn take(&mut self, update: Update) {
        match update {
            Update::Crew(event) => {
                let copy = event.clone();
                self.board.update(Update::Crew(event));
                self.history.event(&copy, &self.board, &self.p);
            }
            Update::Started {
                lead,
                reviewer,
                autopilot,
                internet,
            } => {
                self.history.opening(&lead, &reviewer, &self.p);
                self.board.update(Update::Started {
                    lead,
                    reviewer,
                    autopilot,
                    internet,
                });
            }
            Update::Said(text) => self.history.say(&text, &self.p),
            Update::Back(back) => {
                self.history.back(&back.agent);
                self.board.update(Update::Back(back));
            }
            Update::Synced(synced) => {
                // What moved is said in the conversation; a sync that moved
                // nothing only in the status line.
                if !synced.ok {
                    self.history
                        .say(&format!("the sync failed: {}", synced.said), &self.p);
                } else if synced.said != "nothing changed" {
                    self.history
                        .say(&format!("synced: {}", synced.said), &self.p);
                }
                self.board.update(Update::Synced(synced));
            }
            other => self.board.update(other),
        }
    }

    fn ask(&mut self, request: Request) {
        // What the person named is said under what they wrote, as it
        // decides who works.
        match &request {
            Request::Start {
                name, model, named, ..
            } if !named.is_empty() => {
                let mut said = format!("→ on {model}, as you named it");
                if named.len() > 1 {
                    said.push_str(&format!(
                        "; it may start agents on {}",
                        named[1..].join(", ")
                    ));
                }
                self.history.after_message(name, said);
            }
            Request::Tell { to, named, .. } if !named.is_empty() => {
                let said = format!(
                    "→ {to} may start agents on {}, as you named them",
                    named.join(", ")
                );
                self.history.after_message(to, said);
            }
            _ => {}
        }
        // The program is gone only as iq ends.
        let _ = self.requests.send(request);
    }

    /// Opens the page the board shows on the other screen, or goes back to
    /// the conversation. Returns whether it did either.
    fn turn_page(&mut self) -> io::Result<bool> {
        let open = self.board.screen().page().is_some();
        let screen = &mut self.screen;
        if open == screen.paged {
            return Ok(false);
        }
        screen.tty.page(open)?;
        screen.paged = open;
        if open {
            let (cols, rows) = terminal::size()?;
            screen.refit(Rect::new(0, 0, cols, rows))?;
        } else if let Some(narrower) = screen.away.take() {
            let (cols, rows) = terminal::size()?;
            self.fit(cols, rows, narrower)?;
        } else {
            let area = screen.inline.area();
            screen.refit(area)?;
        }
        Ok(true)
    }

    /// Where the live part's top is now, from where the terminal says its
    /// cursor is.
    fn caret_top(&self) -> Option<u16> {
        let offset = self.screen.caret?;
        cursor::position()
            .ok()
            .map(|(_, y)| y.saturating_sub(offset))
    }

    /// Writes what the conversation gained, then draws the live part or
    /// the page.
    fn draw(&mut self, now: u64) -> io::Result<()> {
        let screen = &mut self.screen;
        if screen.paged {
            let (board, p) = (&mut self.board, &self.p);
            screen
                .terminal
                .draw(|frame| pages::draw(frame, board, p, now))?;
            return Ok(());
        }
        if self.history.waiting() {
            let width = usize::from(screen.inline.cols().saturating_sub(1)).max(10);
            let lines = self.history.take(width);
            screen.inline.insert(&mut screen.out, &lines)?;
            screen.out.flush()?;
        }
        let live = Live {
            board: &self.board,
            history: &self.history,
            p: &self.p,
            now,
        };
        let height = live.height(&self.motion, screen.inline.cols());
        if screen.inline.place(&mut screen.out, height)? {
            let area = screen.inline.area();
            screen.refit(area)?;
        }
        let top = screen.inline.area().top();
        let motion = &mut self.motion;
        let mut caret = None;
        screen
            .terminal
            .draw(|frame| caret = live.draw(frame, motion))?;
        screen.caret = caret.map(|(_, y)| y.saturating_sub(top));
        Ok(())
    }

    /// Waits up to `wait` for keys, pastes, the wheel and resizes, and
    /// takes them all. Returns whether any came.
    fn keys(&mut self, wait: Duration) -> io::Result<bool> {
        match self.screen.tty.wait(wait) {
            Wait::Nothing => return Ok(false),
            Wait::Gone => {
                // Its window closed: nobody is there to answer, and the
                // agents stop with iq.
                self.board.act(iq_ui::keys::Action::Quit);
                return Ok(true);
            }
            Wait::Ready => {}
        }
        loop {
            if !event::poll(Duration::ZERO)? {
                return Ok(true);
            }
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    let (chord, typed) = keys::chord(&key);
                    for request in self.board.press(&chord, typed, iq_ui::now()) {
                        self.ask(request);
                    }
                }
                Event::Paste(text) => self.board.paste(&text),
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => self.board.scroll(-3),
                    MouseEventKind::ScrollDown => self.board.scroll(3),
                    _ => {}
                },
                Event::Resize(cols, rows) => {
                    let narrower = cols < self.screen.inline.cols();
                    if self.screen.paged {
                        self.screen.away = Some(self.screen.away.unwrap_or(false) || narrower);
                        self.screen.refit(Rect::new(0, 0, cols, rows))?;
                    } else {
                        self.fit(cols, rows, narrower)?;
                    }
                }
                _ => {}
            }
        }
    }

    /// Fits the live part to a screen now `cols` by `rows`. Where the
    /// terminal moved the cursor says where it moved the live part. Made
    /// narrower, a terminal may fold what it showed again, the live part
    /// with it: the screen then goes into its scrollback whole, and the end
    /// of the conversation is written again at the new width.
    fn fit(&mut self, cols: u16, rows: u16, narrower: bool) -> io::Result<()> {
        if narrower {
            let screen = &mut self.screen;
            Inline::clear_above(&mut screen.out, rows, rows)?;
            screen.inline = Inline::new(cols, rows);
            let live = Live {
                board: &self.board,
                history: &self.history,
                p: &self.p,
                now: iq_ui::now(),
            };
            let height = live.height(&self.motion, cols);
            let width = usize::from(cols.saturating_sub(1)).max(10);
            let room = usize::from(rows.saturating_sub(height + 1));
            let recent = self.history.recent(width, room);
            screen.inline.place(&mut screen.out, height)?;
            screen.inline.insert(&mut screen.out, &recent)?;
        } else {
            let top = self.caret_top();
            self.screen
                .inline
                .resized(&mut self.screen.out, cols, rows, top)?;
        }
        let area = self.screen.inline.area();
        self.screen.refit(area)
    }

    /// Writes what is left of the conversation and what the session cost,
    /// and leaves the cursor below it, for the shell.
    fn end(&mut self) -> io::Result<()> {
        if self.screen.paged {
            self.screen.tty.page(false)?;
            self.screen.paged = false;
        }
        self.history.finish_all(&self.board, &self.p);
        let mut billed = Usd(0.0);
        for agent in self.board.crew().agents() {
            billed += agent.billed;
        }
        let said = if billed.0 > 0.0 {
            format!("iq ends: {billed} billed to the provider.")
        } else {
            "iq ends.".to_owned()
        };
        self.history.say(&said, &self.p);
        let screen = &mut self.screen;
        let width = usize::from(screen.inline.cols().saturating_sub(1)).max(10);
        let lines = self.history.take(width);
        screen.inline.insert(&mut screen.out, &lines)?;
        screen.inline.leave(&mut screen.out)?;
        screen.out.flush()
    }
}
