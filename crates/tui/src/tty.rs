//! The terminal as iq takes it, and gives it back as it was, whatever
//! happens, a panic too: raw, so that every key comes to iq; its pastes
//! bracketed, so that a pasted line is never sent; its keys told apart
//! where the terminal can (Shift-Enter from Enter); the mouse taken only on
//! a page, so that the conversation is selected and scrolled as the
//! terminal's own output. iq watches the terminal itself for its keys: once
//! its window is closed, crossterm would read nothing from it forever.

use std::fs::File;
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::crossterm::{cursor, execute, queue};

/// Whether the terminal tells keys apart, as iq asked it.
static ENHANCED: AtomicBool = AtomicBool::new(false);

/// Whether a page shows on the terminal's other screen.
static PAGED: AtomicBool = AtomicBool::new(false);

/// The terminal, taken.
pub(crate) struct Tty {
    /// The terminal itself, as the system names it for this process,
    /// watched for keys.
    tty: File,
}

/// What a wait for the terminal found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wait {
    /// Something to read.
    Ready,
    /// Nothing, in the time given.
    Nothing,
    /// No terminal any more: its window was closed.
    Gone,
}

impl Tty {
    /// Takes the terminal.
    pub(crate) fn take() -> io::Result<Tty> {
        let tty = File::open("/dev/tty")?;
        terminal::enable_raw_mode()?;
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            give_back();
            hook(info);
        }));
        let enhanced = matches!(terminal::supports_keyboard_enhancement(), Ok(true));
        let mut out = io::stdout();
        queue!(out, EnableBracketedPaste)?;
        if enhanced {
            queue!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
        }
        out.flush()?;
        ENHANCED.store(enhanced, Ordering::SeqCst);
        Ok(Tty { tty })
    }

    /// Waits up to `wait` for something to read from the terminal. A
    /// hang up or an error it reports is taken for its window closed only
    /// once the terminal no longer answers: some come from what another
    /// program did to it, such as one iq ran, and pass. What it said goes
    /// to iq's log either way.
    pub(crate) fn wait(&self, wait: Duration) -> Wait {
        let millis = u16::try_from(wait.as_millis()).unwrap_or(u16::MAX);
        let mut watched = [PollFd::new(self.tty.as_fd(), PollFlags::POLLIN)];
        let said = match poll(&mut watched, PollTimeout::from(millis)) {
            Ok(0) | Err(nix::errno::Errno::EINTR) => return Wait::Nothing,
            Ok(_) => {
                let seen = watched[0].revents().unwrap_or(PollFlags::empty());
                let trouble = PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL;
                if !seen.intersects(trouble) {
                    return Wait::Ready;
                }
                format!("{seen:?}")
            }
            Err(error) => format!("poll failed: {error}"),
        };
        match nix::sys::termios::tcgetattr(self.tty.as_fd()) {
            Ok(_) => {
                tracing::warn!("the terminal said {said}, yet still answers: iq goes on");
                // So that a word that stays is not read again at once,
                // over and over.
                std::thread::sleep(wait);
                if watched[0]
                    .revents()
                    .is_some_and(|seen| seen.contains(PollFlags::POLLIN))
                {
                    Wait::Ready
                } else {
                    Wait::Nothing
                }
            }
            Err(error) => {
                tracing::warn!("the terminal said {said} and answers no more ({error}): iq ends");
                Wait::Gone
            }
        }
    }

    /// Shows a page on the terminal's other screen, the mouse taken for
    /// its wheel, or goes back to the conversation, as it was.
    pub(crate) fn page(&mut self, on: bool) -> io::Result<()> {
        let mut out = io::stdout();
        if on {
            execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
        } else {
            execute!(out, DisableMouseCapture, LeaveAlternateScreen)?;
        }
        PAGED.store(on, Ordering::SeqCst);
        Ok(())
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        give_back();
    }
}

/// Gives the terminal back as iq found it.
fn give_back() {
    let mut out = io::stdout();
    if PAGED.swap(false, Ordering::SeqCst) {
        let _ = queue!(out, DisableMouseCapture, LeaveAlternateScreen);
    }
    if ENHANCED.swap(false, Ordering::SeqCst) {
        let _ = queue!(out, PopKeyboardEnhancementFlags);
    }
    // No scrolling region left behind, the cursor where it was: clearing
    // the region alone would send it to the top.
    let _ = out.write_all(b"\x1b7\x1b[r\x1b8");
    let _ = queue!(out, DisableBracketedPaste, cursor::Show);
    let _ = out.flush();
    let _ = terminal::disable_raw_mode();
}
