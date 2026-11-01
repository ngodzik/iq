//! Links the terminal opens on a click: OSC 8 hyperlinks, in the terminals
//! known to take them. A link's text is written alone; its address rides on
//! its spans' background, which iq never paints in the conversation, so
//! that it survives the folding of lines, and the line's writer turns it
//! into the hyperlink. Only addresses of the web and of mail are taken,
//! and none holding what could end the escape it is written in.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use ratatui::style::{Color, Style};

/// Every address marked, by its number, and the number of each.
#[derive(Default)]
struct Marked {
    addresses: Vec<String>,
    numbers: HashMap<String, usize>,
}

fn marked() -> &'static Mutex<Marked> {
    static MARKED: OnceLock<Mutex<Marked>> = OnceLock::new();
    MARKED.get_or_init(Mutex::default)
}

/// Whether the terminal takes OSC 8 hyperlinks, as its variables (`var`)
/// say: kitty, Ghostty, iTerm2, WezTerm, VS Code's, Windows Terminal and
/// VTE's (GNOME's terminals) from 0.50. Inside tmux, which may not pass
/// them on, none.
pub(crate) fn supported(var: impl Fn(&str) -> Option<String>) -> bool {
    let set = |name: &str| var(name).is_some_and(|v| !v.is_empty());
    if set("TMUX") {
        return false;
    }
    let term = var("TERM").unwrap_or_default();
    let program = var("TERM_PROGRAM").unwrap_or_default();
    set("KITTY_WINDOW_ID")
        || set("WT_SESSION")
        || term == "xterm-kitty"
        || term == "xterm-ghostty"
        || ["iTerm.app", "WezTerm", "ghostty", "vscode"].contains(&program.as_str())
        || var("VTE_VERSION")
            .and_then(|v| v.parse::<u32>().ok())
            .is_some_and(|v| v >= 5000)
}

/// `address` as a link may take it: one of the web or of mail, with no
/// space nor control character; none otherwise.
fn safe(address: &str) -> Option<&str> {
    let address = address.trim();
    let scheme = ["https://", "http://", "mailto:"]
        .iter()
        .any(|scheme| address.starts_with(scheme));
    let clean = !address.chars().any(|c| c.is_control() || c.is_whitespace());
    (scheme && clean && address.len() <= 2048).then_some(address)
}

/// `style` carrying `address`, for the line's writer to make a hyperlink of;
/// none when the address is not one a link takes.
pub(crate) fn mark(style: Style, address: &str) -> Option<Style> {
    let address = safe(address)?;
    let mut marked = marked().lock().unwrap_or_else(PoisonError::into_inner);
    let next = marked.addresses.len();
    let number = *marked.numbers.entry(address.to_owned()).or_insert(next);
    if number == next {
        marked.addresses.push(address.to_owned());
    }
    // Numbered from one, so that no colour of a theme is mistaken for one.
    let [_, r, g, b] = u32::try_from(number + 1).ok()?.to_be_bytes();
    Some(style.bg(Color::Rgb(r, g, b)))
}

/// The address `style` carries, its number, and the style without it.
pub(crate) fn address_of(style: Style) -> Option<(usize, String, Style)> {
    let Some(Color::Rgb(r, g, b)) = style.bg else {
        return None;
    };
    let number = usize::try_from(u32::from_be_bytes([0, r, g, b])).ok()?;
    let marked = marked().lock().unwrap_or_else(PoisonError::into_inner);
    let address = marked.addresses.get(number.checked_sub(1)?)?.clone();
    let mut plain = style;
    plain.bg = None;
    Some((number, address, plain))
}

/// The escape that opens the hyperlink to `address`, numbered `number` so
/// that a link folded over lines is one, and the one that closes it.
pub(crate) fn escapes(number: usize, address: &str) -> (String, &'static str) {
    (
        format!("\x1b]8;id=iq{number};{address}\x1b\\"),
        "\x1b]8;;\x1b\\",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn terminals_that_take_hyperlinks_are_known_by_their_variables() {
        assert!(supported(env(&[("KITTY_WINDOW_ID", "1")])));
        assert!(supported(env(&[("TERM_PROGRAM", "iTerm.app")])));
        assert!(supported(env(&[("VTE_VERSION", "7600")])));
        assert!(!supported(env(&[("VTE_VERSION", "4600")])));
        assert!(!supported(env(&[("TERM", "xterm-256color")])));
        assert!(!supported(env(&[
            ("KITTY_WINDOW_ID", "1"),
            ("TMUX", "/tmp/t,1,0")
        ])));
    }

    #[test]
    fn an_address_rides_on_its_style_and_only_a_safe_one_is_taken() {
        let style = mark(Style::new(), "https://example.com/a").unwrap();
        let (number, address, plain) = address_of(style).unwrap();
        assert_eq!(address, "https://example.com/a");
        assert_eq!(plain, Style::new());
        // The same address, the same link.
        let again = mark(Style::new(), "https://example.com/a").unwrap();
        assert_eq!(address_of(again).unwrap().0, number);
        for refused in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://x.example/\x1b]52;c;aGk=\x07",
            "https://x.example/a b",
        ] {
            assert_eq!(mark(Style::new(), refused), None, "{refused:?}");
        }
        assert_eq!(address_of(Style::new()), None);
        let (open, close) = escapes(3, "https://example.com");
        assert_eq!(open, "\x1b]8;id=iq3;https://example.com\x1b\\");
        assert_eq!(close, "\x1b]8;;\x1b\\");
    }
}
