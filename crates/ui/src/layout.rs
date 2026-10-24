//! The window's panes: one, two side by side, three or four, each showing
//! a chat, a new chat waiting for its first message, the costs or the
//! subscriptions. Four panes are always kept, so that going back to four
//! finds them as they were; the split says how many show.
//!
//! The person lays the panes out: none opens or closes by itself. An
//! agent another started takes a pane shown that waits untouched for a
//! chat, when there is one.

/// What a pane shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Shows {
    /// An agent's chat, by its name.
    Chat(String),
    /// A chat not started yet.
    New(NewChat),
    /// The costs.
    Costs,
    /// The subscriptions.
    Subscriptions,
    /// The assay: its exercises, its runs, its results.
    Assay,
    /// What iq's work went to, over time.
    Observations,
}

/// A chat not started yet: the model it will run on, and the name asked
/// for it once its first message is sent, until the agent joins.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NewChat {
    /// The model: one of the provider's, or `claude-code`, `codex`.
    pub model: String,
    /// The agent's name, once it was asked for.
    pub starting: Option<String>,
    /// Its part in the crew, as the person wrote it: empty for none.
    pub role: String,
    /// How its team works.
    pub pattern: iq_agent::Pattern,
    /// The second model, for a pattern that takes one: empty for the
    /// first's.
    pub partner: String,
}

/// A pane.
#[derive(Debug, Clone, PartialEq)]
pub struct Pane {
    /// What it shows.
    pub shows: Shows,
    /// What the person writes in it.
    pub input: String,
}

impl Pane {
    fn new(shows: Shows) -> Self {
        Self {
            shows,
            input: String::new(),
        }
    }

    /// Whether it waits for a chat to be started in it, untouched.
    fn free(&self) -> bool {
        matches!(&self.shows, Shows::New(chat) if chat.starting.is_none()) && self.input.is_empty()
    }
}

/// How many panes show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    /// One, over the whole window.
    One,
    /// Two side by side.
    Two,
    /// Three: the first on the left, the two others above each other on
    /// the right.
    Three,
    /// Four, two above two.
    Four,
}

impl Split {
    /// How many panes it shows.
    #[must_use]
    pub fn count(self) -> usize {
        match self {
            Split::One => 1,
            Split::Two => 2,
            Split::Three => 3,
            Split::Four => 4,
        }
    }
}

/// The least share of the window a pane may be given by dragging.
const LEAST_SHARE: f32 = 0.15;

/// The panes and how they share the window.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    panes: [Pane; 4],
    split: Split,
    focus: usize,
    zoomed: bool,
    /// Where the panes side by side meet, as a share of the width.
    column: f32,
    /// Where the panes above meet those below, as a share of the height.
    row: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Self::new("")
    }
}

impl Layout {
    /// One pane, ready for a chat on `model`; the others, when split,
    /// ready for chats too, but the last, which shows the costs.
    #[must_use]
    pub fn new(model: &str) -> Self {
        let new = || {
            Pane::new(Shows::New(NewChat {
                model: model.to_owned(),
                ..NewChat::default()
            }))
        };
        Self {
            panes: [new(), new(), new(), Pane::new(Shows::Costs)],
            split: Split::One,
            focus: 0,
            zoomed: false,
            column: 0.56,
            row: 0.54,
        }
    }

    /// How many panes show.
    #[must_use]
    pub fn split(&self) -> Split {
        self.split
    }

    /// The panes shown, by their index, in reading order.
    #[must_use]
    pub fn shown(&self) -> Vec<usize> {
        if self.zoomed {
            vec![self.focus]
        } else {
            (0..self.split.count()).collect()
        }
    }

    /// The pane at `index`.
    #[must_use]
    pub fn pane(&self, index: usize) -> Option<&Pane> {
        self.panes.get(index)
    }

    /// The pane at `index`, to write in it.
    pub fn pane_mut(&mut self, index: usize) -> Option<&mut Pane> {
        self.panes.get_mut(index)
    }

    /// The pane with the focus, by its index.
    #[must_use]
    pub fn focus(&self) -> usize {
        self.focus
    }

    /// Whether the pane with the focus is over the whole window.
    #[must_use]
    pub fn zoomed(&self) -> bool {
        self.zoomed
    }

    /// Where panes meet: the share of the width left of the line between
    /// columns, and of the height above the line between rows.
    #[must_use]
    pub fn lines(&self) -> (f32, f32) {
        (self.column, self.row)
    }

    /// Moves the line between columns to `share` of the width.
    pub fn set_column(&mut self, share: f32) {
        self.column = share.clamp(LEAST_SHARE, 1.0 - LEAST_SHARE);
    }

    /// Moves the line between rows to `share` of the height.
    pub fn set_row(&mut self, share: f32) {
        self.row = share.clamp(LEAST_SHARE, 1.0 - LEAST_SHARE);
    }

    /// One, two, four, one: the next split. Back to one, the pane with the
    /// focus is the one kept, and comes first.
    pub fn next_split(&mut self) {
        if self.zoomed {
            self.zoomed = false;
            return;
        }
        self.split = match self.split {
            Split::One => Split::Two,
            Split::Two | Split::Three => Split::Four,
            Split::Four => {
                self.panes.swap(0, self.focus);
                self.focus = 0;
                Split::One
            }
        };
    }

    /// Gives the focus to the pane at `index`, if it shows.
    pub fn set_focus(&mut self, index: usize) {
        if self.shown().contains(&index) {
            self.focus = index;
        }
    }

    /// The focus to the next pane shown.
    pub fn next_focus(&mut self) {
        if !self.zoomed {
            self.focus = (self.focus + 1) % self.split.count();
        }
    }

    /// The pane with the focus over the whole window, or back in place.
    pub fn toggle_zoom(&mut self) {
        self.zoomed = !self.zoomed && self.split != Split::One;
    }

    /// Shows `shows` in the pane at `index`.
    pub fn show(&mut self, index: usize, shows: Shows) {
        if let Some(pane) = self.panes.get_mut(index) {
            pane.shows = shows;
        }
    }

    /// Gives `model` to the new chats that have none yet: the lead's, once
    /// the program says which it is.
    pub fn new_chats_on(&mut self, model: &str) {
        for pane in &mut self.panes {
            if let Shows::New(chat) = &mut pane.shows
                && chat.model.is_empty()
            {
                chat.model = model.to_owned();
            }
        }
    }

    /// Where an agent that just joined shows: in the pane that started
    /// it, else, when another agent started it, in a pane shown that waits
    /// untouched for a chat. Returns the pane, if any.
    pub fn joined(&mut self, agent: &str, by_person: bool) -> Option<usize> {
        let started = self.panes.iter().position(
            |pane| matches!(&pane.shows, Shows::New(chat) if chat.starting.as_deref() == Some(agent)),
        );
        let at = started.or_else(|| {
            if by_person {
                return None;
            }
            self.shown()
                .into_iter()
                .find(|at| *at != self.focus && self.panes[*at].free())
        })?;
        self.panes[at].shows = Shows::Chat(agent.to_owned());
        Some(at)
    }

    /// The agents shown, by the panes that show them.
    #[must_use]
    pub fn chats_shown(&self) -> Vec<(usize, &str)> {
        self.shown()
            .into_iter()
            .filter_map(|at| match &self.panes[at].shows {
                Shows::Chat(agent) => Some((at, agent.as_str())),
                _ => None,
            })
            .collect()
    }

    /// The names asked for chats that have not joined yet.
    #[must_use]
    pub fn starting(&self) -> Vec<&str> {
        self.panes
            .iter()
            .filter_map(|pane| match &pane.shows {
                Shows::New(chat) => chat.starting.as_deref(),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_split_goes_one_two_four_and_back_keeping_the_focus() {
        let mut layout = Layout::new("codex");
        assert_eq!(layout.shown(), vec![0]);
        layout.next_split();
        layout.next_split();
        assert_eq!(layout.shown(), vec![0, 1, 2, 3]);
        assert_eq!(layout.pane(3).unwrap().shows, Shows::Costs);
        layout.set_focus(3);
        layout.next_split();
        // The costs had the focus: they are what stays.
        assert_eq!(layout.shown(), vec![0]);
        assert_eq!(layout.pane(0).unwrap().shows, Shows::Costs);
        layout.set_focus(2);
        assert_eq!(layout.focus(), 0);
    }

    #[test]
    fn a_zoomed_pane_is_alone_until_zoomed_back_or_split() {
        let mut layout = Layout::new("m");
        layout.toggle_zoom();
        assert!(!layout.zoomed(), "one pane has nothing to zoom over");
        layout.next_split();
        layout.next_focus();
        layout.toggle_zoom();
        assert_eq!(layout.shown(), vec![1]);
        layout.next_focus();
        assert_eq!(layout.focus(), 1);
        layout.next_split();
        assert_eq!(layout.shown(), vec![0, 1]);
        layout.set_column(0.99);
        assert_eq!(layout.lines().0, 0.85);
    }

    #[test]
    fn agents_show_where_they_were_started_or_in_a_free_pane() {
        let mut layout = Layout::new("m");
        layout.next_split();
        if let Some(Pane {
            shows: Shows::New(chat),
            ..
        }) = layout.pane_mut(0)
        {
            chat.starting = Some("opus".into());
        }
        assert_eq!(layout.starting(), vec!["opus"]);
        assert_eq!(layout.joined("opus", true), Some(0));
        // Started by an agent: the free pane beside it takes it.
        assert_eq!(layout.joined("helper", false), Some(1));
        // No free pane left among those shown.
        assert_eq!(layout.joined("other", false), None);
        assert_eq!(layout.chats_shown(), vec![(0, "opus"), (1, "helper")]);
    }
}
