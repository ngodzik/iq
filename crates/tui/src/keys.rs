//! A key as the terminal reports it, in `iq-ui`'s terms: the chord the
//! shortcuts are matched against, and the character it types, if it types
//! one.

use iq_ui::keys::Chord;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// `key` as a chord, and the character it types.
pub(crate) fn chord(key: &KeyEvent) -> (Chord, Option<char>) {
    let held = key.modifiers;
    let mut chord = Chord {
        ctrl: held.contains(KeyModifiers::CONTROL),
        alt: held.contains(KeyModifiers::ALT),
        shift: held.contains(KeyModifiers::SHIFT),
        cmd: held.contains(KeyModifiers::SUPER),
        key: String::new(),
    };
    let types = !chord.ctrl && !chord.alt && !chord.cmd;
    let mut typed = None;
    chord.key = match key.code {
        KeyCode::Char(' ') => {
            typed = types.then_some(' ');
            "Space".to_owned()
        }
        KeyCode::Char(c) => {
            typed = types.then_some(c);
            c.to_uppercase().collect()
        }
        KeyCode::Enter => "Enter".to_owned(),
        KeyCode::Tab => "Tab".to_owned(),
        KeyCode::BackTab => {
            chord.shift = true;
            "Tab".to_owned()
        }
        KeyCode::Backspace => "Backspace".to_owned(),
        KeyCode::Delete => "Delete".to_owned(),
        KeyCode::Esc => "Escape".to_owned(),
        KeyCode::Left => "Left".to_owned(),
        KeyCode::Right => "Right".to_owned(),
        KeyCode::Up => "Up".to_owned(),
        KeyCode::Down => "Down".to_owned(),
        KeyCode::Home => "Home".to_owned(),
        KeyCode::End => "End".to_owned(),
        KeyCode::PageUp => "PageUp".to_owned(),
        KeyCode::PageDown => "PageDown".to_owned(),
        KeyCode::Insert => "Insert".to_owned(),
        KeyCode::F(n) => format!("F{n}"),
        _ => String::new(),
    };
    (chord, typed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_become_the_chords_the_settings_name() {
        let press = |code, held| chord(&KeyEvent::new(code, held));
        let (ctrl_o, typed) = press(KeyCode::Char('o'), KeyModifiers::CONTROL);
        assert_eq!((ctrl_o, typed), (Chord::parse("ctrl+o").unwrap(), None));
        let (shift_tab, _) = press(KeyCode::BackTab, KeyModifiers::NONE);
        assert_eq!(shift_tab, Chord::parse("shift+tab").unwrap());
        let (a, typed) = press(KeyCode::Char('é'), KeyModifiers::NONE);
        assert_eq!((a.key.as_str(), typed), ("É", Some('é')));
        assert_eq!(
            press(KeyCode::Esc, KeyModifiers::NONE).0,
            Chord::plain("esc")
        );
    }
}
