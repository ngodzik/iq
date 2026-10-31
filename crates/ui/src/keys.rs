//! The shortcuts: which key does what, as the person may set it in the
//! `[keys]` of their settings. The terminal reads a key in its own terms
//! and asks here what it does, so that the bindings are tested without one.
//!
//! The defaults are letters held with Ctrl, which sit where they are named
//! on QWERTY and AZERTY alike: digits and signs move between the two. None
//! takes a key a terminal keeps for itself (Ctrl-I is Tab there, Ctrl-M
//! Enter) or one its line editors use (Ctrl-A, Ctrl-E, Ctrl-K, Ctrl-U,
//! Ctrl-W and their like edit what the person writes).

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use crate::error::UiError;

/// What a shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    /// Writes to the next agent, or to a new chat after the last.
    NextAgent,
    /// Writes to the agent before the one written to.
    PrevAgent,
    /// The crew's page: each agent, what it does and spends.
    Team,
    /// The costs' page.
    Costs,
    /// What the work went to, over time.
    Observations,
    /// The shortcuts and the commands.
    Keys,
    /// The next theme.
    Theme,
    /// The reviewer answering for the person, or not.
    Autopilot,
    /// The internet open to every command, or only to what fetches packages.
    Internet,
    /// Allows what an agent asks, the oldest question first.
    Allow,
    /// Refuses it.
    Refuse,
    /// What a sync would send, line by line, before the person says
    /// whether it goes.
    Changes,
    /// Every step the agents took, unfolded.
    Steps,
    /// Stops the agent the person writes to.
    Stop,
    /// Quits.
    Quit,
}

impl Action {
    /// Every action, in the order the list of shortcuts gives them.
    pub const ALL: [Action; 15] = [
        Action::NextAgent,
        Action::PrevAgent,
        Action::Team,
        Action::Costs,
        Action::Observations,
        Action::Keys,
        Action::Theme,
        Action::Autopilot,
        Action::Internet,
        Action::Allow,
        Action::Refuse,
        Action::Changes,
        Action::Steps,
        Action::Stop,
        Action::Quit,
    ];

    /// Its name in the settings' `[keys]`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Action::NextAgent => "next_agent",
            Action::PrevAgent => "prev_agent",
            Action::Team => "team",
            Action::Costs => "costs",
            Action::Observations => "observations",
            Action::Keys => "keys",
            Action::Theme => "theme",
            Action::Autopilot => "autopilot",
            Action::Internet => "internet",
            Action::Allow => "allow",
            Action::Refuse => "refuse",
            Action::Changes => "changes",
            Action::Steps => "steps",
            Action::Stop => "stop",
            Action::Quit => "quit",
        }
    }

    /// What it does, as the list of shortcuts says it.
    #[must_use]
    pub fn what(self) -> &'static str {
        match self {
            Action::NextAgent => "write to the next agent, or start a new chat",
            Action::PrevAgent => "write to the agent before",
            Action::Team => "the crew: each agent, what it does and spends",
            Action::Costs => "the costs: by model, by kind of task, over time",
            Action::Observations => "what the work went to, over time",
            Action::Keys => "this list",
            Action::Theme => "the next theme",
            Action::Autopilot => "the autopilot",
            Action::Internet => "the internet open to every command",
            Action::Allow => "allow what an agent asks",
            Action::Refuse => "refuse it",
            Action::Changes => "what a sync would send, line by line",
            Action::Steps => "every step the agents took, unfolded",
            Action::Stop => "stop the agent you write to",
            Action::Quit => "quit",
        }
    }

    /// Its key unless the settings say otherwise; empty for none.
    fn default_chord(self) -> &'static str {
        match self {
            Action::NextAgent => "ctrl+n",
            Action::PrevAgent => "ctrl+p",
            Action::Team => "ctrl+t",
            Action::Costs => "ctrl+o",
            Action::Observations => "ctrl+j",
            Action::Keys => "ctrl+s",
            // Seldom wanted: the commands do them, `/theme`, `/internet`.
            Action::Theme | Action::Internet => "",
            Action::Autopilot => "shift+tab",
            Action::Allow => "ctrl+y",
            Action::Refuse => "ctrl+r",
            Action::Changes => "ctrl+g",
            Action::Steps => "ctrl+x",
            Action::Stop => "escape",
            // A terminal never hands Cmd-Q to the program in it.
            Action::Quit => "ctrl+q",
        }
    }

    fn named(name: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|action| action.name() == name)
    }
}

/// A key and the modifiers held with it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Chord {
    /// Control.
    pub ctrl: bool,
    /// Alt, Option on macOS.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
    /// Command, on macOS.
    pub cmd: bool,
    /// The key: a letter in capitals, `Tab`, `Escape`, `F2`.
    pub key: String,
}

impl Chord {
    /// `key` with no modifier.
    #[must_use]
    pub fn plain(key: &str) -> Self {
        Self {
            ctrl: false,
            alt: false,
            shift: false,
            cmd: false,
            key: key_name(key),
        }
    }

    /// Reads a chord as the settings write it: modifiers and a key joined
    /// by `+`, in any case, such as `ctrl+o`, `shift+tab` or `cmd+q`.
    ///
    /// # Examples
    ///
    /// ```
    /// use iq_ui::keys::Chord;
    ///
    /// let chord = Chord::parse("Ctrl+o").unwrap();
    /// assert!(chord.ctrl && !chord.shift);
    /// assert_eq!(chord.key, "O");
    /// assert_eq!(chord.to_string(), "Ctrl-O");
    /// ```
    pub fn parse(text: &str) -> Result<Self, String> {
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let Some((key, modifiers)) = parts.split_last() else {
            return Err("it is empty".to_owned());
        };
        if key.is_empty() {
            return Err("it names no key after its modifiers".to_owned());
        }
        let mut chord = Chord::plain(key);
        for modifier in modifiers {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => chord.ctrl = true,
                "alt" | "option" | "opt" => chord.alt = true,
                "shift" => chord.shift = true,
                "cmd" | "command" | "super" | "meta" => chord.cmd = true,
                other => return Err(format!("`{other}` is not a modifier")),
            }
        }
        Ok(chord)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (held, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.cmd, "Cmd"),
        ] {
            if held {
                write!(f, "{name}-")?;
            }
        }
        f.write_str(&self.key)
    }
}

/// A key's name as chords hold it, whatever case or alias the settings
/// use: letters in capitals, `Escape` for `esc`.
fn key_name(key: &str) -> String {
    let lower = key.to_ascii_lowercase();
    let known = match lower.as_str() {
        "esc" | "escape" => Some("Escape"),
        "enter" | "return" => Some("Enter"),
        "tab" => Some("Tab"),
        "space" => Some("Space"),
        "backspace" => Some("Backspace"),
        "delete" | "del" => Some("Delete"),
        "up" => Some("Up"),
        "down" => Some("Down"),
        "left" => Some("Left"),
        "right" => Some("Right"),
        "home" => Some("Home"),
        "end" => Some("End"),
        "pageup" => Some("PageUp"),
        "pagedown" => Some("PageDown"),
        _ => None,
    };
    if let Some(name) = known {
        return name.to_owned();
    }
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Which key does what.
#[derive(Debug, Clone, PartialEq)]
pub struct Keymap {
    actions: HashMap<Chord, Action>,
    chords: BTreeMap<Action, Chord>,
}

impl Default for Keymap {
    fn default() -> Self {
        let chords: BTreeMap<Action, Chord> = Action::ALL
            .into_iter()
            .filter_map(|action| {
                Chord::parse(action.default_chord())
                    .ok()
                    .map(|c| (action, c))
            })
            .collect();
        let actions = chords.iter().map(|(a, c)| (c.clone(), *a)).collect();
        Self { actions, chords }
    }
}

/// The actions iq had and has no more: a setting that moves one is left,
/// so that older settings still open.
const RETIRED: [&str; 2] = ["assay", "steward"];

impl Keymap {
    /// The defaults, changed as the settings' `[keys]` say: an action's
    /// name and its key, `""` for none.
    ///
    /// # Errors
    ///
    /// An action iq does not have, a key it cannot read, or two
    /// actions on one key.
    pub fn with(changes: &BTreeMap<String, String>) -> Result<Self, UiError> {
        let mut chords = Self::default().chords;
        for (name, text) in changes {
            if RETIRED.contains(&name.as_str()) {
                continue;
            }
            let action = Action::named(name).ok_or_else(|| {
                let names: Vec<&str> = Action::ALL.iter().map(|a| a.name()).collect();
                UiError::UnknownAction(name.clone(), names.join(", "))
            })?;
            if text.trim().is_empty() {
                chords.remove(&action);
                continue;
            }
            let chord = Chord::parse(text).map_err(|why| UiError::BadChord {
                action: name.clone(),
                chord: text.clone(),
                why,
            })?;
            chords.insert(action, chord);
        }
        let mut actions = HashMap::new();
        for (action, chord) in &chords {
            if let Some(first) = actions.insert(chord.clone(), *action) {
                return Err(UiError::Taken {
                    chord: chord.to_string(),
                    first: first.name().to_owned(),
                    second: action.name().to_owned(),
                });
            }
        }
        Ok(Self { actions, chords })
    }

    /// What `chord` does, if anything.
    #[must_use]
    pub fn action(&self, chord: &Chord) -> Option<Action> {
        self.actions.get(chord).copied()
    }

    /// The key that does `action`, if one does.
    #[must_use]
    pub fn chord(&self, action: Action) -> Option<&Chord> {
        self.chords.get(&action)
    }

    /// Every bound action with its key, in the list's order.
    #[must_use]
    pub fn list(&self) -> Vec<(String, Action)> {
        Action::ALL
            .into_iter()
            .filter_map(|action| self.chord(action).map(|c| (c.to_string(), action)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_is_read_and_none_shares_a_key() {
        let keys = Keymap::default();
        // All but the two the commands do.
        assert_eq!(keys.list().len(), Action::ALL.len() - 2);
        let costs = Chord::parse("ctrl+o").unwrap();
        assert_eq!(keys.action(&costs), Some(Action::Costs));
        assert_eq!(
            keys.action(&Chord::parse("shift+Tab").unwrap()),
            Some(Action::Autopilot)
        );
        assert_eq!(keys.action(&Chord::plain("esc")), Some(Action::Stop));
        assert_eq!(keys.action(&Chord::plain("o")), None);
        // The defaults are built through the same check as the person's.
        assert_eq!(Keymap::with(&BTreeMap::new()).unwrap(), keys);
    }

    #[test]
    fn the_settings_move_unbind_and_are_refused_when_wrong() {
        let changes = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect()
        };
        let keys = Keymap::with(&changes(&[("costs", "ctrl+u"), ("theme", "")])).unwrap();
        assert_eq!(
            keys.action(&Chord::parse("ctrl+u").unwrap()),
            Some(Action::Costs)
        );
        assert_eq!(keys.action(&Chord::parse("ctrl+o").unwrap()), None);
        assert_eq!(keys.chord(Action::Theme), None);
        assert!(matches!(
            Keymap::with(&changes(&[("coast", "ctrl+u")])),
            Err(UiError::UnknownAction(..))
        ));
        assert!(matches!(
            Keymap::with(&changes(&[("costs", "hyper+u")])),
            Err(UiError::BadChord { .. })
        ));
        assert!(matches!(
            Keymap::with(&changes(&[("costs", "ctrl+t")])),
            Err(UiError::Taken { .. })
        ));
        // A shortcut of an action iq had once is left, not refused.
        assert!(Keymap::with(&changes(&[("assay", "ctrl+l")])).is_ok());
    }
}
