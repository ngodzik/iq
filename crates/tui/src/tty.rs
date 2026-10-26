//! The terminal as iq takes it, and gives it back as it was, whatever
//! happens, a panic too: raw, so that every key comes to iq; its pastes
//! bracketed, so that a pasted line is never sent; its keys told apart
//! where the terminal can (Shift-Enter from Enter); the mouse taken only on
//! a page, so that the conversation is selected and scrolled as the
//! terminal's own output.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

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
pub(crate) struct Tty(());

impl Tty {
    /// Takes the terminal.
    pub(crate) fn take() -> io::Result<Tty> {
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
        Ok(Tty(()))
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
