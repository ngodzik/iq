//! What the terminal shows besides the conversation, and where what the
//! person writes goes: to an agent, or to a new chat waiting for its first
//! message; the page over the conversation, when one is open, and how far
//! down it is read; a word to the person for a few seconds.
//!
//! The conversation itself is the terminal's: what the agents do is
//! written in it as it happens, and scrolls there as any output does.

use crate::input::Editor;

/// A chat not started yet: the model it will run on, how its team works,
/// and its part in the crew.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NewChat {
    /// The model: one of the provider's, or `claude-code`, `codex`.
    pub model: String,
    /// Its part in the crew, as the person wrote it: empty for none.
    pub role: String,
    /// How its team works.
    pub pattern: iq_agent::Pattern,
    /// The second model, for a pattern that takes one: empty for the
    /// first's.
    pub partner: String,
}

/// Where what the person writes goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Talk {
    /// To an agent, by its name.
    To(String),
    /// To a new chat, which its first message starts.
    New(NewChat),
}

/// A page over the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Page {
    /// The crew: each agent, what it does and spends.
    Team,
    /// One agent's whole thread.
    Thread(String),
    /// The costs, the limits and the subscriptions.
    Costs,
    /// The assay.
    Assay,
    /// What the work went to, over time.
    Observations,
    /// The shortcuts and the commands.
    Keys,
}

impl Page {
    /// Its name, as its title says it.
    #[must_use]
    pub fn title(&self) -> String {
        match self {
            Page::Team => "Team".to_owned(),
            Page::Thread(agent) => format!("Thread of {agent}"),
            Page::Costs => "Costs".to_owned(),
            Page::Assay => "Assay".to_owned(),
            Page::Observations => "Observations".to_owned(),
            Page::Keys => "Keys and commands".to_owned(),
        }
    }
}

/// How long a word to the person shows, in seconds.
const NOTICE_SECS: u64 = 5;

/// How long a first Ctrl-C waits for the second that quits, in seconds.
const QUIT_SECS: u64 = 3;

/// The terminal's state besides the conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    talk: Talk,
    editor: Editor,
    page: Option<Page>,
    /// The first line of the page shown.
    scroll: usize,
    /// How many lines of the page show at once, as the terminal last drew
    /// it.
    rows: usize,
    /// The agent chosen on the team's page, by its place.
    chosen: usize,
    /// The completion picked in the list, by its place.
    picked: usize,
    /// Whether the person sent the list away for what is written now.
    unlisted: bool,
    /// The names asked for agents that have not joined yet.
    starting: Vec<String>,
    /// A word to the person, and when it was said.
    notice: Option<(String, u64)>,
    /// When a first Ctrl-C asked to quit.
    quit_asked: Option<u64>,
}

impl Default for Screen {
    fn default() -> Self {
        Self {
            talk: Talk::New(NewChat::default()),
            editor: Editor::default(),
            page: None,
            scroll: 0,
            rows: 0,
            chosen: 0,
            picked: 0,
            unlisted: false,
            starting: Vec::new(),
            notice: None,
            quit_asked: None,
        }
    }
}

impl Screen {
    /// Where what the person writes goes.
    #[must_use]
    pub fn talk(&self) -> &Talk {
        &self.talk
    }

    /// The agent the person writes to, when it is one.
    #[must_use]
    pub fn talking_to(&self) -> Option<&str> {
        match &self.talk {
            Talk::To(agent) => Some(agent),
            Talk::New(_) => None,
        }
    }

    /// Writes to `agent` from now on.
    pub fn talk_to(&mut self, agent: &str) {
        self.talk = Talk::To(agent.to_owned());
    }

    /// Writes to a new chat on `model` from now on.
    pub fn new_chat(&mut self, model: &str) {
        self.talk = Talk::New(NewChat {
            model: model.to_owned(),
            ..NewChat::default()
        });
    }

    /// The new chat, to change how it starts, when the person writes to
    /// one.
    pub fn new_chat_mut(&mut self) -> Option<&mut NewChat> {
        match &mut self.talk {
            Talk::New(chat) => Some(chat),
            Talk::To(_) => None,
        }
    }

    /// Gives `model` to the new chat when it has none yet: the lead's, once
    /// the program says which it is.
    pub fn new_chats_on(&mut self, model: &str) {
        if let Talk::New(chat) = &mut self.talk
            && chat.model.is_empty()
        {
            chat.model = model.to_owned();
        }
    }

    /// What the person writes.
    #[must_use]
    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    /// What the person writes, to edit it.
    pub fn editor_mut(&mut self) -> &mut Editor {
        self.unlisted = false;
        self.picked = 0;
        &mut self.editor
    }

    /// The page open, if one is.
    #[must_use]
    pub fn page(&self) -> Option<&Page> {
        self.page.as_ref()
    }

    /// Opens `page`, from its top; the same page again closes it.
    pub fn toggle(&mut self, page: Page) {
        if self.page.as_ref() == Some(&page) {
            self.page = None;
        } else {
            self.open(page);
        }
    }

    /// Opens `page`, from its top.
    pub fn open(&mut self, page: Page) {
        self.page = Some(page);
        self.scroll = 0;
    }

    /// Closes the page open; a thread goes back to the team's page.
    pub fn close(&mut self) {
        self.page = match self.page.take() {
            Some(Page::Thread(_)) => Some(Page::Team),
            _ => None,
        };
        self.scroll = 0;
    }

    /// Closes whatever page is open.
    pub fn close_all(&mut self) {
        self.page = None;
        self.scroll = 0;
    }

    /// How many lines of the page show at once.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Keeps the page's first line within its `most`, and says how many
    /// of its lines show at once, as the terminal drew it.
    pub fn drawn(&mut self, most: usize, rows: usize) {
        self.scroll = self.scroll.min(most);
        self.rows = rows;
    }

    /// The first line of the page shown.
    #[must_use]
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Scrolls the page by `lines`, down when positive.
    pub fn scroll_by(&mut self, lines: isize) {
        self.scroll = self.scroll.saturating_add_signed(lines);
    }

    /// Scrolls the page to its top, or as far as it goes.
    pub fn scroll_to(&mut self, line: usize) {
        self.scroll = line;
    }

    /// The agent chosen on the team's page, by its place.
    #[must_use]
    pub fn chosen(&self) -> usize {
        self.chosen
    }

    /// Chooses the agent at `at` on the team's page, within `count`.
    pub fn choose(&mut self, at: usize, count: usize) {
        self.chosen = at.min(count.saturating_sub(1));
    }

    /// The completion picked, by its place.
    #[must_use]
    pub fn picked(&self) -> usize {
        self.picked
    }

    /// Picks the completion at `at`, within `count`, going round.
    pub fn pick(&mut self, at: isize, count: usize) {
        if count > 0 {
            self.picked = at.rem_euclid(count as isize) as usize;
        }
    }

    /// Whether the person sent the list of completions away.
    #[must_use]
    pub fn unlisted(&self) -> bool {
        self.unlisted
    }

    /// Sends the list of completions away, until the text changes.
    pub fn unlist(&mut self) {
        self.unlisted = true;
    }

    /// The names asked for agents that have not joined yet.
    #[must_use]
    pub fn starting(&self) -> &[String] {
        &self.starting
    }

    /// An agent was asked for under `name`.
    pub fn starts(&mut self, name: &str) {
        self.starting.push(name.to_owned());
    }

    /// `agent` joined.
    pub fn joined(&mut self, agent: &str) {
        self.starting.retain(|name| name != agent);
    }

    /// Says `text` to the person, for a few seconds.
    pub fn notice(&mut self, text: impl Into<String>, now: u64) {
        self.notice = Some((text.into(), now));
    }

    /// What is said to the person at `now`, if anything still is.
    #[must_use]
    pub fn noticed(&self, now: u64) -> Option<&str> {
        self.notice
            .as_ref()
            .filter(|(_, at)| now.saturating_sub(*at) < NOTICE_SECS)
            .map(|(text, _)| text.as_str())
    }

    /// A Ctrl-C with nothing written at `now`: true when it is the second
    /// within a few seconds, which quits.
    pub fn ask_quit(&mut self, now: u64) -> bool {
        match self.quit_asked {
            Some(at) if now.saturating_sub(at) < QUIT_SECS => true,
            _ => {
                self.quit_asked = Some(now);
                self.notice("Ctrl-C again to quit", now);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_open_close_and_scroll_within_their_lines() {
        let mut screen = Screen::default();
        screen.toggle(Page::Costs);
        screen.scroll_by(30);
        screen.drawn(12, 20);
        assert_eq!(screen.scroll(), 12);
        screen.scroll_by(-40);
        assert_eq!(screen.scroll(), 0);
        screen.toggle(Page::Costs);
        assert_eq!(screen.page(), None);
        screen.open(Page::Thread("lead".into()));
        screen.close();
        assert_eq!(screen.page(), Some(&Page::Team));
        screen.close();
        assert_eq!(screen.page(), None);
    }

    #[test]
    fn a_second_ctrl_c_within_seconds_quits() {
        let mut screen = Screen::default();
        assert!(!screen.ask_quit(100));
        assert_eq!(screen.noticed(101), Some("Ctrl-C again to quit"));
        assert!(screen.ask_quit(102));
        assert!(!screen.ask_quit(200));
        assert_eq!(screen.noticed(300), None);
    }
}
