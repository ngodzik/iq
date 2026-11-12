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

/// What shows a turn of several agents below the conversation, as the
/// settings' `turn_view` says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TurnShown {
    /// The agents at work side by side, a pane each, at the top.
    #[default]
    Panes,
    /// A strip of the turn's map: a line for the turn, one for each agent
    /// at work, the rest folded.
    Strip,
}

impl std::str::FromStr for TurnShown {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.trim().to_ascii_lowercase().as_str() {
            "panes" => Ok(TurnShown::Panes),
            "strip" => Ok(TurnShown::Strip),
            other => Err(format!(
                "turn_view = {other:?} is none iq knows: \"panes\" or \"strip\""
            )),
        }
    }
}

/// Where the conversation is, as the settings' `conversation` says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Conversation {
    /// Written once into the terminal's own scrollback, scrolled,
    /// selected and searched as the terminal's own output.
    #[default]
    Scrollback,
    /// Drawn by iq on the terminal's other screen, the mouse taken: what
    /// is folded unfolds in place, a diagram is enlarged with a click, and
    /// iq scrolls, searches and copies it itself. Leaving iq leaves that
    /// screen: the conversation stays out of the terminal's scrollback.
    Drawn,
}

impl std::str::FromStr for Conversation {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.trim().to_ascii_lowercase().as_str() {
            "scrollback" => Ok(Conversation::Scrollback),
            "drawn" => Ok(Conversation::Drawn),
            other => Err(format!(
                "conversation = {other:?} is none iq knows: \"scrollback\" or \"drawn\""
            )),
        }
    }
}

/// How iq looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    theme: Theme,
    /// How many of an agent's latest steps show while it works.
    steps: usize,
    /// How many of a job's latest lines show while it runs.
    job_lines: usize,
    /// What shows a turn of several agents.
    turn: TurnShown,
    /// Whether a diagram shows folded, as a line, until unfolded.
    diagrams_folded: bool,
    /// Where the conversation is.
    conversation: Conversation,
}

/// How many of a job's latest lines show while it runs, unless the
/// settings say otherwise.
pub const JOB_LINES: usize = 3;

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
            job_lines: JOB_LINES,
            turn: TurnShown::Panes,
            diagrams_folded: false,
            conversation: Conversation::Scrollback,
        }
    }

    /// With the conversation where `conversation` says.
    #[must_use]
    pub fn with_conversation(mut self, conversation: Conversation) -> Self {
        self.conversation = conversation;
        self
    }

    /// Where the conversation is.
    #[must_use]
    pub fn conversation(&self) -> Conversation {
        self.conversation
    }

    /// With diagrams shown folded, as a line the person unfolds with the
    /// down arrow while it is the last thing written, when `folded`.
    #[must_use]
    pub fn with_diagrams_folded(mut self, folded: bool) -> Self {
        self.diagrams_folded = folded;
        self
    }

    /// Whether diagrams show folded.
    #[must_use]
    pub fn diagrams_folded(&self) -> bool {
        self.diagrams_folded
    }

    /// With a turn of several agents shown as `turn` says.
    #[must_use]
    pub fn with_turn(mut self, turn: TurnShown) -> Self {
        self.turn = turn;
        self
    }

    /// What shows a turn of several agents.
    #[must_use]
    pub fn turn(&self) -> TurnShown {
        self.turn
    }

    /// With the latest `lines` of a job shown while it runs; none for 0.
    #[must_use]
    pub fn with_job_lines(mut self, lines: usize) -> Self {
        self.job_lines = lines;
        self
    }

    /// How many of a job's latest lines show while it runs.
    #[must_use]
    pub fn job_lines(&self) -> usize {
        self.job_lines
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
