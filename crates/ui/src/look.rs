//! How iq looks: its theme. The terminal turns a theme into colours over
//! its own background; here it is only a name, chosen in the settings or
//! switched with a command.

use std::fmt;
use std::str::FromStr;

use crate::error::UiError;

/// A theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// Brass and warm greys, for a dark terminal.
    #[default]
    Graphite,
    /// A pale blue, for a dark terminal.
    Ink,
    /// Copper and ink, for a light terminal.
    Porcelain,
}

impl Theme {
    /// Every theme, in the order the key goes through them.
    pub const ALL: [Theme; 3] = [Theme::Graphite, Theme::Ink, Theme::Porcelain];

    /// Its name, as the settings write it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Theme::Graphite => "graphite",
            Theme::Ink => "ink",
            Theme::Porcelain => "porcelain",
        }
    }

    /// The theme after it.
    #[must_use]
    pub fn next(self) -> Theme {
        let at = Theme::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Theme::ALL[(at + 1) % Theme::ALL.len()]
    }
}

impl fmt::Display for Theme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Theme {
    type Err = UiError;

    fn from_str(text: &str) -> Result<Self, UiError> {
        let wanted = text.trim().to_ascii_lowercase();
        Theme::ALL
            .into_iter()
            .find(|theme| theme.name() == wanted)
            .ok_or_else(|| {
                let names: Vec<&str> = Theme::ALL.iter().map(|t| t.name()).collect();
                UiError::UnknownTheme(text.to_owned(), names.join(", "))
            })
    }
}

/// How iq looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    theme: Theme,
    /// How many of an agent's latest steps show while it works.
    steps: usize,
}

impl Default for Look {
    fn default() -> Self {
        Self::new(Theme::default())
    }
}

impl Look {
    /// In `theme`, the latest [`crate::steps::SHOWN`] steps of an agent
    /// shown while it works.
    #[must_use]
    pub fn new(theme: Theme) -> Self {
        Self {
            theme,
            steps: crate::steps::SHOWN,
        }
    }

    /// With the latest `steps` of an agent shown while it works; none
    /// for 0.
    #[must_use]
    pub fn showing(mut self, steps: usize) -> Self {
        self.steps = steps;
        self
    }

    /// How many of an agent's latest steps show while it works.
    #[must_use]
    pub fn steps(&self) -> usize {
        self.steps
    }

    /// The theme.
    #[must_use]
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// The next theme.
    pub fn next_theme(&mut self) {
        self.theme = self.theme.next();
    }

    /// In `theme` from now on.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_are_named_in_the_settings_and_go_round() {
        assert_eq!(" Ink".parse::<Theme>().unwrap(), Theme::Ink);
        assert!("paper".parse::<Theme>().is_err());
        let mut look = Look::default();
        look.next_theme();
        look.next_theme();
        assert_eq!(look.theme(), Theme::Porcelain);
        look.next_theme();
        assert_eq!(look.theme(), Theme::Graphite);
    }
}
