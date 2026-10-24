//! How the window looks: its theme, and how much of the desktop shows
//! through it. The window turns a theme into colours; here it is only a
//! name, chosen in the settings or switched with a key.

use std::fmt;
use std::str::FromStr;

use crate::error::UiError;

/// A theme of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// Graphite and brass.
    #[default]
    Graphite,
    /// Night blue, lit in a paler blue.
    Ink,
    /// Light: off-white and copper.
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

/// The steps of opacity the key goes through: see-through, more, more
/// still, then opaque.
pub const OPACITIES: [f32; 4] = [0.8, 0.65, 0.5, 1.0];

/// The least opacity the settings may ask for: below it, text over a busy
/// desktop cannot be read.
const LEAST_OPACITY: f32 = 0.3;

/// The window's look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    theme: Theme,
    opacity: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self::new(Theme::default(), OPACITIES[0])
    }
}

impl Look {
    /// `theme`, as opaque as `opacity`, from 0.3 to 1.
    #[must_use]
    pub fn new(theme: Theme, opacity: f32) -> Self {
        let opacity = if opacity.is_finite() {
            opacity.clamp(LEAST_OPACITY, 1.0)
        } else {
            1.0
        };
        Self { theme, opacity }
    }

    /// The theme.
    #[must_use]
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// How opaque the window is, from 0.3 to 1.
    #[must_use]
    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    /// The next theme.
    pub fn next_theme(&mut self) {
        self.theme = self.theme.next();
    }

    /// The next step of opacity after the one closest to the current.
    pub fn next_opacity(&mut self) {
        let closest = OPACITIES
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (*a - self.opacity)
                    .abs()
                    .total_cmp(&(*b - self.opacity).abs())
            })
            .map_or(0, |(at, _)| at);
        self.opacity = OPACITIES[(closest + 1) % OPACITIES.len()];
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

    #[test]
    fn opacity_steps_from_the_closest_and_stays_readable() {
        let mut look = Look::new(Theme::Ink, 0.62);
        look.next_opacity();
        assert_eq!(look.opacity(), 0.5);
        look.next_opacity();
        look.next_opacity();
        assert_eq!(look.opacity(), 0.8);
        assert_eq!(Look::new(Theme::Ink, 0.0).opacity(), 0.3);
        assert_eq!(Look::new(Theme::Ink, f32::NAN).opacity(), 1.0);
    }
}
