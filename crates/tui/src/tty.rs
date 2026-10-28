//! The terminal as iq takes it, and gives it back as it was, whatever
//! happens, a panic too: raw, so that every key comes to iq; its pastes
//! bracketed, so that a pasted line is never sent; its keys told apart
//! where the terminal can (Shift-Enter from Enter); the mouse taken only on
//! a page, so that the conversation is selected and scrolled as the
//! terminal's own output. iq watches the terminal itself for its keys: once
//! its window is closed, crossterm would read it forever and find nothing.

use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nix::errno::Errno;
use nix::sys::select::{FD_SETSIZE, FdSet, select};
use nix::sys::time::{TimeVal, TimeValLike};

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
        if !watchable(tty.as_fd()) {
            return Err(io::Error::other(
                "iq holds too many files to watch its terminal",
            ));
        }
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

    /// Waits up to `wait` for something to read from the terminal.
    pub(crate) fn wait(&self, wait: Duration) -> Wait {
        wait_for(self.tty.as_fd(), wait)
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

/// Waits up to `wait` for something to read from `tty`, with select(2):
/// macOS answers poll(2) on a terminal with `POLLNVAL` at once, every
/// time, where every system answers select(2). A terminal ready with
/// nothing to read has hung up, its window closed: a key read leaves it
/// ready no more. Why iq takes it for gone goes to iq's log.
fn wait_for(tty: BorrowedFd<'_>, wait: Duration) -> Wait {
    if !watchable(tty) {
        tracing::warn!("the terminal cannot be watched: iq ends");
        return Wait::Gone;
    }
    let mut ready = FdSet::new();
    ready.insert(tty);
    // iq waits a fraction of a second at a time: a minute is only a bound.
    let micros = wait.min(Duration::from_secs(60)).as_micros();
    let mut timeout = TimeVal::microseconds(i64::try_from(micros).unwrap_or(0));
    let said = match select(None, &mut ready, None, None, &mut timeout) {
        Ok(0) | Err(Errno::EINTR) => return Wait::Nothing,
        Ok(_) => match rustix::io::ioctl_fionread(tty) {
            Ok(0) => "it is ready with nothing to read".to_owned(),
            Ok(_) => return Wait::Ready,
            Err(error) => format!("it is ready, yet cannot tell what it holds ({error})"),
        },
        Err(error) => format!("waiting for it failed ({error})"),
    };
    tracing::warn!("the terminal is taken for closed, as {said}: iq ends");
    Wait::Gone
}

/// Whether select(2) can watch `fd`: below `FD_SETSIZE`, as the terminal
/// iq opens as it starts is.
fn watchable(fd: BorrowedFd<'_>) -> bool {
    usize::try_from(fd.as_raw_fd()).is_ok_and(|fd| fd < FD_SETSIZE)
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

#[cfg(test)]
mod tests {
    use std::io::Read;

    use nix::pty::openpty;
    use nix::sys::termios::{SetArg, cfmakeraw, tcgetattr, tcsetattr};

    use super::*;

    #[test]
    fn a_key_is_seen_at_once_and_a_closed_window_for_what_it_is() {
        let pty = openpty(None, None).unwrap();
        // Raw, as iq takes its terminal: a key is there to read at once.
        let mut raw = tcgetattr(&pty.slave).unwrap();
        cfmakeraw(&mut raw);
        tcsetattr(&pty.slave, SetArg::TCSANOW, &raw).unwrap();
        let (mut window, tty) = (File::from(pty.master), File::from(pty.slave));
        let (short, long) = (Duration::from_millis(20), Duration::from_secs(5));
        assert_eq!(wait_for(tty.as_fd(), short), Wait::Nothing);
        window.write_all(b"y").unwrap();
        assert_eq!(wait_for(tty.as_fd(), long), Wait::Ready);
        // Until it is read, the key is still there.
        assert_eq!(wait_for(tty.as_fd(), long), Wait::Ready);
        let mut key = [0; 1];
        (&tty).read_exact(&mut key).unwrap();
        assert_eq!(&key, b"y");
        assert_eq!(wait_for(tty.as_fd(), short), Wait::Nothing);
        drop(window);
        assert_eq!(wait_for(tty.as_fd(), long), Wait::Gone);
    }
}
