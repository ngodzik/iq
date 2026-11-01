//! iq's state in the terminal: the crew, where the person writes and what
//! page they look at, the costs, the look and the keys, as what the program reads arrives and as the person acts. Every
//! key the person presses is taken here, so that what it does is tested
//! without a terminal.

use iq_agent::{Event, PERSON};

use crate::command::{self, Command, Completion};
use crate::costs::{Costs, Period, Spent};
use crate::crew::{Back, CrewView, Question, Request};
use crate::keys::{Action, Chord, Keymap};
use crate::look::Look;
use crate::observations::{By, Span};
use crate::screen::{Page, Screen, Talk};

/// A message on the board, for the terminal to show it travel.
#[derive(Debug, Clone, PartialEq)]
pub struct Post {
    /// Its writer.
    pub from: String,
    /// Its reader: an agent, `all`, or the person.
    pub to: String,
    /// The message.
    pub text: String,
}

impl Post {
    /// Whether it was written to every agent.
    #[must_use]
    pub fn to_everyone(&self) -> bool {
        self.to == iq_agent::ALL
    }

    /// Whether it was written to the person.
    #[must_use]
    pub fn to_person(&self) -> bool {
        self.to == PERSON
    }
}

/// A chat of an earlier session whose last exchanges the conversation
/// writes again, as the person comes back to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recall {
    /// The chat.
    pub agent: String,
    /// Whether iq opened on it, as the chat the person wrote to last.
    pub opening: bool,
}

/// What the program sends the terminal.
#[derive(Debug, Clone)]
pub enum Update {
    /// What the crew just did.
    Crew(Event),
    /// A chat of an earlier session came back, as it was kept.
    Back(Back),
    /// The calls iq kept of earlier sessions, oldest first.
    Kept(Vec<Spent>),
    /// iq is asked to stop: the terminal is given back, the agents
    /// stopped.
    Quit,
    /// iq's tasks, by the kinds of task the person's rules place them in.
    Tasks(Vec<crate::costs::TaskSpent>),
    /// A sync of iq's data ended.
    Synced(Synced),
    /// What changed waits to be sent to the sync's repository, for the
    /// person to say whether it goes.
    SyncAsk(SyncAsk),
    /// What a sync would send, line by line, as the page that shows it
    /// asked; empty where nothing would be sent.
    Changes(String),
    /// The provider's models, as it lists them: what the person may name.
    Models(Vec<String>),
    /// Something iq says to the person in passing, in the conversation.
    Said(String),
    /// What iq answers the person where they write: what a command they
    /// gave did, or why it did not.
    Notice(String),
    /// What a plugin told iq to show.
    Plugin {
        /// The plugin.
        plugin: String,
        /// What it told.
        told: crate::plugins::Told,
    },
    /// How the crew starts: the lead's model, the reviewer's, whether
    /// the reviewer answers for the person, and whether every command has
    /// the internet.
    Started {
        /// The lead's model.
        lead: String,
        /// The reviewer's.
        reviewer: String,
        /// Whether the autopilot is on.
        autopilot: bool,
        /// Whether the internet is open to every command.
        internet: bool,
    },
}

/// How the last sync of iq's data with the machine's repository went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synced {
    /// When, in seconds since 1970.
    pub at: u64,
    /// Whether it succeeded.
    pub ok: bool,
    /// What it did, or why it failed.
    pub said: String,
}

/// What a sync would send, asked of the person before it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncAsk {
    /// The repository.
    pub remote: String,
    /// The files that changed, as the repository names them.
    pub files: Vec<String>,
    /// Those files, said for the person to decide on: the person's own by
    /// name, the rest counted by folder.
    pub said: String,
}

/// The most of the provider's models a command being written is offered.
const OFFERED: usize = 8;

/// How many of the person's chats the switcher shows at once.
pub const SWITCH_ROWS: usize = 5;

/// The person's chats as the switcher shows them, the one used last
/// first.
#[derive(Debug, Clone)]
pub struct Switcher<'a> {
    /// Every chat, the one used last first, each with when the person
    /// last went to it or wrote to it, if they did.
    pub chats: Vec<(&'a crate::crew::AgentView, Option<u64>)>,
    /// The one chosen.
    pub chosen: usize,
    /// The first shown: [`SWITCH_ROWS`] are, the chosen among them.
    pub first: usize,
}

/// How far back the costs are summed, as the costs' page offers it.
pub const PERIODS: [Period; 5] = [
    Period::Last(3600),
    Period::Last(6 * 3600),
    Period::Today,
    Period::Last(24 * 3600),
    Period::Last(7 * 24 * 3600),
];

/// iq's state in the terminal.
#[derive(Debug, Clone)]
pub struct Board {
    crew: CrewView,
    screen: Screen,
    keys: Keymap,
    look: Look,
    /// How far back the costs are summed, one of [`PERIODS`].
    period: Period,
    costs: Costs,
    /// The messages posted since the terminal last took them.
    posts: Vec<Post>,
    /// Whether iq was asked to stop.
    quitting: bool,
    /// How the last sync went, when the machine syncs.
    synced: Option<Synced>,
    /// What waits to be sent, asked of the person.
    sync_ask: Option<SyncAsk>,
    /// What a sync would send, line by line, once read for its page.
    changes: Option<String>,
    /// What the agents did between their lines of text.
    steps: crate::steps::Steps,
    /// How the work is looked at.
    lens: crate::observations::Lens,
    /// The provider's models, as it lists them.
    models: Vec<String>,
    /// What the person's plugins show.
    plugins: crate::plugins::Plugins,
    /// The chat whose last exchanges are to be written again.
    recall: Option<Recall>,
    /// The person's chats of earlier sessions not come back to yet.
    unrecalled: std::collections::HashSet<String>,
    /// Whether the person chose where they write, or wrote: the chats
    /// coming back then leave it as it is.
    chose: bool,
    /// When the person wrote last to the chat iq opens on.
    opening: Option<u64>,
    /// The person's chats they went to or wrote to, the latest first, each
    /// with when.
    recent: Vec<(String, u64)>,
    /// The chat chosen in the switcher, while it is open.
    switching: Option<usize>,
}

impl Default for Board {
    fn default() -> Self {
        Self::new(Keymap::default(), Look::default())
    }
}

impl Board {
    /// iq as it opens: the person writing to a new chat, no page open,
    /// with `keys` and `look` as the settings make them.
    #[must_use]
    pub fn new(keys: Keymap, look: Look) -> Self {
        Self {
            crew: CrewView::default(),
            screen: Screen::default(),
            keys,
            look,
            period: Period::Last(24 * 3600),
            costs: Costs::default(),
            posts: Vec::new(),
            quitting: false,
            synced: None,
            sync_ask: None,
            changes: None,
            steps: crate::steps::Steps::default(),
            lens: crate::observations::Lens::default(),
            models: Vec::new(),
            plugins: crate::plugins::Plugins::default(),
            recall: None,
            unrecalled: std::collections::HashSet::new(),
            chose: false,
            opening: None,
            recent: Vec::new(),
            switching: None,
        }
    }

    /// The chat whose last exchanges the conversation writes again, once:
    /// one of an earlier session the person comes back to.
    pub fn take_recall(&mut self) -> Option<Recall> {
        self.recall.take()
    }

    /// Writes to `agent` from now on; a chat of an earlier session is
    /// recalled as the person comes back to it.
    fn write_to(&mut self, agent: &str) {
        self.screen.talk_to(agent);
        if self.unrecalled.remove(agent) {
            let recall = Recall {
                agent: agent.to_owned(),
                opening: false,
            };
            if let Some(before) = self.recall.replace(recall)
                && before.agent != agent
            {
                self.unrecalled.insert(before.agent);
            }
        }
    }

    /// The person chose where they write: `agent`.
    fn chose_to(&mut self, agent: &str) {
        self.chose = true;
        self.write_to(agent);
        self.visit(agent, crate::now());
    }

    /// The person went to `agent`, or wrote to it, `at`: it comes first
    /// among the chats they used.
    fn visit(&mut self, agent: &str, at: u64) {
        self.recent.retain(|(name, _)| name != agent);
        self.recent.insert(0, (agent.to_owned(), at));
    }

    /// The person's chats, the one used last first: those they went to or
    /// wrote to, then the others, the latest to work first.
    fn chats_by_use(&self) -> Vec<(&crate::crew::AgentView, Option<u64>)> {
        let mut chats: Vec<(&crate::crew::AgentView, Option<u64>)> = self
            .recent
            .iter()
            .filter_map(|(name, at)| {
                let view = self.crew.agent(name).filter(|view| view.by == PERSON)?;
                Some((view, Some(*at)))
            })
            .collect();
        let mut others: Vec<&crate::crew::AgentView> = self
            .crew
            .chats()
            .filter(|view| !chats.iter().any(|(seen, _)| seen.name == view.name))
            .collect();
        others.sort_by_key(|view| std::cmp::Reverse(view.worked));
        chats.extend(others.into_iter().map(|view| (view, None)));
        chats
    }

    /// The switcher, while it is open.
    #[must_use]
    pub fn switcher(&self) -> Option<Switcher<'_>> {
        let chosen = self.switching?;
        let chats = self.chats_by_use();
        let chosen = chosen.min(chats.len().saturating_sub(1));
        let first = (chosen + 1).saturating_sub(SWITCH_ROWS);
        Some(Switcher {
            chats,
            chosen,
            first,
        })
    }

    /// Opens the switcher on the chat used before the one written to, or
    /// moves it to the next; around from the last to the first.
    fn switch(&mut self, now: u64) {
        let count = self.chats_by_use().len();
        if count == 0 {
            self.screen
                .notice("No chat yet: what you write starts one", now);
            return;
        }
        self.screen.close();
        self.switching = Some(match self.switching {
            Some(chosen) => (chosen + 1) % count,
            None => {
                let current = self.screen.talking_to();
                let first = self
                    .chats_by_use()
                    .first()
                    .map(|(view, _)| view.name.as_str())
                    == current;
                usize::from(first && count > 1)
            }
        });
    }

    /// What a key does while the switcher is open: it moves in it, goes to
    /// the chat chosen, or closes it; another key closes it and does what
    /// it does.
    fn switch_key(&mut self, chord: &Chord, now: u64) -> Option<Vec<Request>> {
        let chosen = self.switching?;
        let count = self.chats_by_use().len().max(1);
        let plain = !chord.ctrl && !chord.alt && !chord.cmd;
        match (plain, chord.shift, chord.key.as_str()) {
            (true, false, "Down" | "Tab") => self.switching = Some((chosen + 1) % count),
            (true, false, "Up") | (true, true, "Tab") => {
                self.switching = Some((chosen + count - 1) % count);
            }
            (true, false, "Enter") => {
                self.switching = None;
                let chosen = self
                    .chats_by_use()
                    .get(chosen)
                    .map(|(view, _)| view.name.clone());
                if let Some(agent) = chosen {
                    self.chose_to(&agent);
                }
            }
            (true, false, "Escape") => self.switching = None,
            _ if self.keys.action(chord) == Some(Action::Switch) => self.switch(now),
            _ => {
                self.switching = None;
                return None;
            }
        }
        Some(Vec::new())
    }

    /// What the person's plugins show.
    #[must_use]
    pub fn plugins(&self) -> &crate::plugins::Plugins {
        &self.plugins
    }

    /// Whether iq was asked to stop, for the terminal to be given back.
    #[must_use]
    pub fn quitting(&self) -> bool {
        self.quitting
    }

    /// The messages posted since the last call, oldest first.
    pub fn take_posts(&mut self) -> Vec<Post> {
        std::mem::take(&mut self.posts)
    }

    /// How far back the costs are summed.
    #[must_use]
    pub fn period(&self) -> Period {
        self.period
    }

    /// When the period the costs are summed over starts, `now` being in
    /// seconds since 1970, in the person's time zone.
    #[must_use]
    pub fn period_since(&self, now: u64) -> u64 {
        self.period.since(now, &jiff::tz::TimeZone::system())
    }

    /// What the provider billed since midnight, in the person's time
    /// zone: this session's calls and those iq kept of earlier ones.
    #[must_use]
    pub fn billed_today(&self, now: u64) -> iq_core::Usd {
        self.costs
            .billed(Period::Today.since(now, &jiff::tz::TimeZone::system()))
    }

    /// Sums the costs over `period`, one of [`PERIODS`].
    pub fn sum_over(&mut self, period: Period) {
        if PERIODS.contains(&period) {
            self.period = period;
        }
    }

    /// The next period the costs are summed over, or the one before.
    pub fn next_period(&mut self, forward: bool) {
        let at = PERIODS.iter().position(|p| *p == self.period).unwrap_or(0);
        let next = if forward {
            (at + 1) % PERIODS.len()
        } else {
            (at + PERIODS.len() - 1) % PERIODS.len()
        };
        self.period = PERIODS[next];
    }

    /// Where the person writes, and the page they look at.
    #[must_use]
    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    /// The same, for the terminal to keep a page's scroll within its
    /// lines.
    pub fn screen_mut(&mut self) -> &mut Screen {
        &mut self.screen
    }

    /// The keys.
    #[must_use]
    pub fn keys(&self) -> &Keymap {
        &self.keys
    }

    /// The look.
    #[must_use]
    pub fn look(&self) -> Look {
        self.look
    }

    /// The questions waiting for the person, the oldest first: the keys
    /// answer the first.
    #[must_use]
    pub fn questions(&self) -> &[Question] {
        self.crew.questions()
    }

    /// The models a new chat is offered: the lead's.
    #[must_use]
    pub fn models_offered(&self) -> Vec<String> {
        Some(self.crew.lead().to_owned())
            .filter(|lead| !lead.is_empty())
            .into_iter()
            .collect()
    }

    /// What completes what the person writes, while it is a command or an
    /// agent's name after `@`: none on a page, or once sent away.
    #[must_use]
    pub fn completions(&self) -> Vec<Completion> {
        if self.screen.unlisted() || self.screen.page().is_some() {
            return Vec::new();
        }
        let text = self.screen.editor().text();
        let agents: Vec<&str> = self.crew.agents().iter().map(|a| a.name.as_str()).collect();
        if let Some(name) = text.strip_prefix('@')
            && !name.contains(char::is_whitespace)
        {
            return agents
                .iter()
                .filter(|agent| agent.starts_with(name))
                .map(|agent| Completion {
                    line: format!("@{agent} "),
                    label: format!("@{agent}"),
                    what: "a message to it alone".to_owned(),
                })
                .collect();
        }
        let mut models = self.models_offered();
        // Once a model is begun, the provider's that it begins.
        if let Some(begun) = ["/new ", "/model "]
            .iter()
            .find_map(|command| text.strip_prefix(command))
            .filter(|begun| !begun.is_empty())
        {
            let begun = begun.to_lowercase();
            models.extend(
                self.models
                    .iter()
                    .filter(|model| {
                        model.starts_with(&begun)
                            || model
                                .rsplit('/')
                                .next()
                                .is_some_and(|m| m.starts_with(&begun))
                    })
                    .filter(|model| !models.contains(model))
                    .take(OFFERED)
                    .cloned()
                    .collect::<Vec<_>>(),
            );
        }
        let models: Vec<&str> = models.iter().map(String::as_str).collect();
        let mut completions = command::complete(text, &agents, &models);
        completions.extend(self.plugin_completions(text, &agents));
        completions
    }

    /// What completes `text` among the plugins' commands: their names, then
    /// the chat one that takes something is for.
    fn plugin_completions(&self, text: &str, agents: &[&str]) -> Vec<Completion> {
        let Some(written) = text.strip_prefix('/') else {
            return Vec::new();
        };
        if written.contains('\n') {
            return Vec::new();
        }
        match written.split_once(' ') {
            None => self
                .plugins
                .commands()
                .filter(|(_, spec)| spec.name.starts_with(written))
                .map(|(plugin, spec)| Completion {
                    line: if spec.args.is_empty() {
                        format!("/{}", spec.name)
                    } else {
                        format!("/{} ", spec.name)
                    },
                    label: if spec.args.is_empty() {
                        format!("/{}", spec.name)
                    } else {
                        format!("/{} {}", spec.name, spec.args)
                    },
                    what: format!("{} ({plugin})", spec.what),
                })
                .collect(),
            Some((name, arg)) => match self.plugins.command(name) {
                Some((_, spec)) if !spec.args.is_empty() && !arg.contains(' ') => agents
                    .iter()
                    .filter(|agent| agent.starts_with(arg) && **agent != arg)
                    .map(|agent| Completion {
                        line: format!("/{name} {agent}"),
                        label: (*agent).to_owned(),
                        what: "a chat".to_owned(),
                    })
                    .collect(),
                _ => Vec::new(),
            },
        }
    }

    /// The request a plugin's command written as `text` makes, when it is
    /// one: the chat written to and its place go with it.
    fn plugin_request(&self, text: &str) -> Option<Request> {
        let line = text.strip_prefix('/')?;
        let (name, args) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        if command::known(name) {
            return None;
        }
        let (plugin, spec) = self.plugins.command(name)?;
        let chat = self.screen.talking_to().map(str::to_owned);
        let folder = chat
            .as_deref()
            .and_then(|chat| self.crew.agent(chat))
            .and_then(|view| view.place.clone());
        Some(Request::Plugin {
            plugin: plugin.to_owned(),
            command: spec.name.clone(),
            args: args.trim().to_owned(),
            chat,
            folder,
        })
    }

    /// Takes a key the person pressed at `now`: `chord`, and the
    /// character it types, if it types one. Returns what it asks of the
    /// crew.
    ///
    /// A page open takes its own keys first, Escape among them; then the
    /// list of completions its own; then the shortcuts; then what edits
    /// and sends what the person writes.
    pub fn press(&mut self, chord: &Chord, typed: Option<char>, now: u64) -> Vec<Request> {
        let plain = !chord.ctrl && !chord.alt && !chord.cmd;
        if let Some(done) = self.switch_key(chord, now) {
            return done;
        }
        if self.screen.page().is_some() {
            if plain && !chord.shift && chord.key == "Escape" {
                self.screen.close();
                return Vec::new();
            }
            if let Some(action) = self.keys.action(chord) {
                return self.act(action, now).into_iter().collect();
            }
            return self.page_key(chord, typed);
        }
        let completions = self.completions();
        if !completions.is_empty() && plain {
            let count = completions.len();
            let picked = completions[self.screen.picked().min(count - 1)].clone();
            match chord.key.as_str() {
                "Up" => {
                    self.screen.pick(self.screen.picked() as isize - 1, count);
                    return Vec::new();
                }
                "Down" => {
                    self.screen.pick(self.screen.picked() as isize + 1, count);
                    return Vec::new();
                }
                "Tab" => {
                    self.screen.editor_mut().set(&picked.line);
                    return Vec::new();
                }
                "Escape" => {
                    self.screen.unlist();
                    return Vec::new();
                }
                "Enter" if !chord.shift => {
                    // What takes more stays to be written; the rest runs.
                    let line = picked.line.clone();
                    self.screen.editor_mut().set(&line);
                    if line.ends_with(' ') {
                        return Vec::new();
                    }
                    return self.send(now);
                }
                _ => {}
            }
        }
        if let Some(action) = self.keys.action(chord) {
            return self.act(action, now).into_iter().collect();
        }
        let only_ctrl = chord.ctrl && !chord.alt && !chord.cmd;
        if only_ctrl && (chord.key == "C" || chord.key == "D" && self.screen.editor().is_empty()) {
            if !self.screen.editor().is_empty() {
                self.screen.editor_mut().clear();
                return Vec::new();
            }
            if chord.key == "C"
                && let Some(stop) = self
                    .screen
                    .talking_to()
                    .and_then(|agent| self.crew.stop(agent))
            {
                return vec![stop];
            }
            if self.screen.ask_quit(now) {
                self.quitting = true;
            }
            return Vec::new();
        }
        if chord.key == "Enter" && !chord.ctrl && !chord.cmd {
            let editor = self.screen.editor_mut();
            if chord.shift || chord.alt {
                editor.insert("\n");
            } else if editor.text()[..editor.cursor()].ends_with('\\') {
                // A line ending in a backslash goes on, as in a shell.
                editor.key(&Chord::plain("Backspace"));
                editor.insert("\n");
            } else {
                return self.send(now);
            }
            return Vec::new();
        }
        if self.screen.editor_mut().key(chord) {
            return Vec::new();
        }
        if let Some(c) = typed
            && plain
        {
            self.screen.editor_mut().insert(c.encode_utf8(&mut [0; 4]));
        }
        Vec::new()
    }

    /// Takes a pasted text into what the person writes, its lines kept.
    pub fn paste(&mut self, text: &str) {
        if self.screen.page().is_none() {
            self.screen.editor_mut().insert(text);
        }
    }

    /// Scrolls the page open by `lines`, as the wheel does; or the
    /// switcher, while it is open.
    pub fn scroll(&mut self, lines: isize) {
        if let Some(chosen) = self.switching {
            let last = self.chats_by_use().len().saturating_sub(1) as isize;
            self.switching = Some((chosen as isize + lines.signum()).clamp(0, last) as usize);
        } else if self.screen.page() == Some(&Page::Team) {
            let chosen = self.screen.chosen() as isize + lines.signum();
            self.screen
                .choose(chosen.max(0) as usize, self.crew.agents().len());
        } else {
            self.screen.scroll_by(lines);
        }
    }

    /// What a key does on the page open: it moves in it, and a few letters
    /// choose what the page shows, as its foot names them.
    fn page_key(&mut self, chord: &Chord, typed: Option<char>) -> Vec<Request> {
        if chord.ctrl || chord.alt || chord.cmd {
            return Vec::new();
        }
        let Some(page) = self.screen.page().cloned() else {
            return Vec::new();
        };
        let letter = typed.map(|c| c.to_ascii_lowercase());
        let step = self.screen.rows().saturating_sub(2).max(1) as isize;
        let agents = self.crew.agents().len();
        let chosen = self.screen.chosen();
        match (chord.key.as_str(), letter) {
            (_, Some('q')) => self.screen.close(),
            ("Up", _) | (_, Some('k')) if page == Page::Team => {
                self.screen.choose(chosen.saturating_sub(1), agents);
            }
            ("Down", _) | (_, Some('j')) if page == Page::Team => {
                self.screen.choose(chosen + 1, agents);
            }
            ("Up", _) | (_, Some('k')) => self.screen.scroll_by(-1),
            ("Down", _) | (_, Some('j')) => self.screen.scroll_by(1),
            ("PageUp", _) => self.screen.scroll_by(-step),
            ("PageDown", _) | ("Space", _) => self.screen.scroll_by(step),
            ("Home", _) | (_, Some('g')) => self.screen.scroll_to(0),
            ("End", _) => self.screen.scroll_to(usize::MAX),
            _ => return self.page_choice(&page, chord, letter),
        }
        Vec::new()
    }

    /// What a page's own keys choose.
    fn page_choice(&mut self, page: &Page, chord: &Chord, letter: Option<char>) -> Vec<Request> {
        let key = chord.key.as_str();
        match page {
            Page::Costs => match (key, letter) {
                ("Left", _) => self.next_period(false),
                ("Right", _) | (_, Some('p')) => self.next_period(true),
                _ => {}
            },
            Page::Observations => {
                let mut lens = self.lens;
                match (key, letter) {
                    ("Left", _) => lens.back += 1,
                    ("Right", _) => lens.back = lens.back.saturating_sub(1),
                    (_, Some('n')) => lens.back = 0,
                    (_, Some('s')) => {
                        let at = Span::ALL.iter().position(|s| *s == lens.span).unwrap_or(0);
                        lens.span = Span::ALL[(at + 1) % Span::ALL.len()];
                        lens.back = 0;
                    }
                    (_, Some('b')) => {
                        let at = By::ALL.iter().position(|b| *b == lens.by).unwrap_or(0);
                        lens.by = By::ALL[(at + 1) % By::ALL.len()];
                    }
                    _ => {}
                }
                if lens.span == Span::All {
                    lens.back = 0;
                }
                self.lens = lens;
            }
            Page::Team => {
                let agent = self
                    .crew
                    .listed()
                    .get(self.screen.chosen())
                    .map(|(_, a)| a.name.clone());
                match (key, letter, agent) {
                    ("Enter", _, Some(agent)) | (_, Some('w'), Some(agent)) => {
                        self.chose_to(&agent);
                        self.screen.close();
                    }
                    ("Right", _, Some(agent)) | (_, Some('v'), Some(agent)) => {
                        self.screen.open(Page::Thread(agent));
                    }
                    (_, Some('n'), _) => {
                        self.chose = true;
                        self.screen.new_chat(self.crew.lead());
                        self.screen.close();
                    }
                    _ => {}
                }
            }
            Page::Thread(_) if key == "Left" => self.screen.close(),
            Page::Thread(_) | Page::Keys | Page::Changes | Page::Steps => {}
        }
        Vec::new()
    }

    /// The page of what a sync would send closes once the person said
    /// whether it goes.
    fn sync_answered(&mut self) {
        if self.screen.page() == Some(&Page::Changes) {
            self.screen.close();
        }
    }

    /// Does what `action` says at `now`. Returns what it asks of the crew,
    /// if it asks something.
    pub fn act(&mut self, action: Action, now: u64) -> Option<Request> {
        match action {
            Action::Switch => self.switch(now),
            Action::NewChat => {
                self.chose = true;
                self.screen.new_chat(self.crew.lead());
                self.screen.close();
            }
            Action::Team => self.screen.toggle(Page::Team),
            Action::Costs => self.screen.toggle(Page::Costs),
            Action::Observations => self.screen.toggle(Page::Observations),
            Action::Keys => self.screen.toggle(Page::Keys),
            Action::Steps => {
                // The latest last, where the page opens.
                self.screen.toggle(Page::Steps);
                if self.screen.page() == Some(&Page::Steps) {
                    self.screen.scroll_to(usize::MAX);
                }
            }
            Action::Theme => self.look.next_theme(),
            Action::Autopilot => return Some(self.crew.toggle_autopilot()),
            Action::Internet => return Some(self.crew.toggle_internet()),
            Action::Allow | Action::Refuse => {
                // The agents' questions first: they hold work.
                if let Some(id) = self.crew.questions().first().map(|q| q.id) {
                    return Some(self.crew.answer(id, action == Action::Allow));
                }
                self.sync_ask.take()?;
                self.sync_answered();
                return Some(Request::Sync {
                    send: action == Action::Allow,
                    always: false,
                });
            }
            Action::Changes => {
                // Shown as it stands when the page opens: the same key
                // closes it.
                if self.screen.page() == Some(&Page::Changes) {
                    self.screen.close();
                    return None;
                }
                self.screen.open(Page::Changes);
                self.changes = None;
                return Some(Request::Changes);
            }
            Action::Stop => {
                let agent = self.screen.talking_to()?.to_owned();
                return self.crew.stop(&agent);
            }
            Action::Quit => self.quitting = true,
        }
        None
    }

    /// Sends what the person wrote at `now`: a command, a message to an
    /// agent named after `@`, a message to the agent written to, or the
    /// task that starts a new chat.
    pub fn send(&mut self, now: u64) -> Vec<Request> {
        let text = self.screen.editor().text().trim().to_owned();
        if text.is_empty() {
            return Vec::new();
        }
        if let Some(request) = self.plugin_request(&text) {
            self.screen.editor_mut().take();
            return vec![request];
        }
        if text.starts_with('/') {
            return match command::parse(&text) {
                Ok(command) => {
                    self.screen.editor_mut().take();
                    self.run(command, now)
                }
                Err(why) => {
                    self.screen.notice(why, now);
                    Vec::new()
                }
            };
        }
        if let Some(addressed) = text.strip_prefix('@') {
            let (name, message) = addressed
                .split_once(char::is_whitespace)
                .unwrap_or((addressed, ""));
            if self.crew.agent(name).is_none() {
                self.screen.notice(self.no_agent(name), now);
                return Vec::new();
            }
            if message.trim().is_empty() {
                self.chose_to(name);
                self.screen.editor_mut().take();
                return Vec::new();
            }
            self.chose = true;
            self.screen.editor_mut().take();
            self.visit(name, now);
            let text = message.trim().to_owned();
            return vec![Request::Tell {
                to: name.to_owned(),
                named: crate::named::named(&text, &self.models),
                text,
            }];
        }
        self.chose = true;
        let request = match self.screen.talk().clone() {
            Talk::To(agent) => {
                if self.crew.agent(&agent).is_none() && !self.screen.starting().contains(&agent) {
                    self.screen
                        .notice(format!("{agent} is gone: /new starts a chat"), now);
                    return Vec::new();
                }
                self.visit(&agent, now);
                Request::Tell {
                    to: agent,
                    named: crate::named::named(&text, &self.models),
                    text,
                }
            }
            Talk::New(chat) => {
                if chat.model.trim().is_empty() {
                    self.screen
                        .notice("The new chat has no model: /new names one", now);
                    return Vec::new();
                }
                let role = Some(chat.role.trim().to_owned()).filter(|r| !r.is_empty());
                // A model named in the task is the one it starts on, else
                // the chat's: the lead's unless the person chose another.
                let named = crate::named::named(&text, &self.models);
                let model = named
                    .first()
                    .cloned()
                    .unwrap_or_else(|| chat.model.trim().to_owned());
                // The name the person gave it, else a one-word role, else
                // the first words of its task until a model names it after
                // what its work is about, else its model.
                let worded = match &role {
                    _ if !chat.name.trim().is_empty() => None,
                    Some(role) if !role.contains(char::is_whitespace) => None,
                    _ => from_words(&text, &named),
                };
                let from_words = worded.is_some();
                let base = match (&role, worded) {
                    _ if !chat.name.trim().is_empty() => chat.name.trim().to_owned(),
                    (_, Some(words)) => words,
                    (Some(role), None) if !role.contains(char::is_whitespace) => role.clone(),
                    _ => model.clone(),
                };
                let name = self.name_for(&base);
                let partner = Some(chat.partner.trim().to_owned())
                    .filter(|p| !p.is_empty() && chat.pattern.takes_partner());
                self.screen.starts(&name);
                self.screen.talk_to(&name);
                self.visit(&name, now);
                Request::Start {
                    name,
                    model,
                    task: text,
                    role,
                    pattern: chat.pattern,
                    partner,
                    named,
                    effort: chat.effort,
                    from_words,
                }
            }
        };
        self.screen.editor_mut().take();
        vec![request]
    }

    /// Carries out `command` at `now`.
    pub fn run(&mut self, command: Command, now: u64) -> Vec<Request> {
        let open = |board: &mut Self, page: Page| board.screen.open(page);
        match command {
            Command::New(model) => {
                let model = match model {
                    None => self.crew.lead().to_owned(),
                    Some(word) => match self.model_named(&word) {
                        Ok(model) => model,
                        Err(why) => {
                            self.screen.notice(why, now);
                            return Vec::new();
                        }
                    },
                };
                self.chose = true;
                self.screen.new_chat(&model);
            }
            Command::Model(word) => {
                let model = match self.model_named(&word) {
                    Ok(model) => model,
                    Err(why) => {
                        self.screen.notice(why, now);
                        return Vec::new();
                    }
                };
                match self.screen.talk().clone() {
                    Talk::To(agent) => {
                        if self.crew.agent(&agent).is_none() {
                            self.screen.notice(self.no_agent(&agent), now);
                            return Vec::new();
                        }
                        return vec![Request::Model { agent, model }];
                    }
                    Talk::New(_) => {
                        if let Some(chat) = self.screen.new_chat_mut() {
                            chat.model = model;
                        }
                    }
                }
            }
            Command::Name(name) => match self.screen.talk().clone() {
                Talk::To(agent) => {
                    if self.crew.agent(&agent).is_none() {
                        self.screen.notice(self.no_agent(&agent), now);
                        return Vec::new();
                    }
                    return vec![Request::Rename { agent, name }];
                }
                Talk::New(_) => {
                    if let Some(chat) = self.screen.new_chat_mut() {
                        chat.name = name;
                    }
                }
            },
            Command::Pattern(pattern, partner) => match self.screen.new_chat_mut() {
                Some(chat) => {
                    chat.pattern = pattern;
                    chat.partner = partner.unwrap_or_default();
                }
                None => self
                    .screen
                    .notice("/pattern sets a new chat's: /new first", now),
            },
            Command::Role(role) => match self.screen.new_chat_mut() {
                Some(chat) => chat.role = role,
                None => self
                    .screen
                    .notice("/role sets a new chat's: /new first", now),
            },
            Command::To(agent) => {
                if self.crew.agent(&agent).is_some() {
                    self.chose_to(&agent);
                } else {
                    self.screen.notice(self.no_agent(&agent), now);
                }
            }
            Command::Team => open(self, Page::Team),
            Command::Costs => open(self, Page::Costs),
            Command::Observations => open(self, Page::Observations),
            Command::Keys => open(self, Page::Keys),
            Command::Autopilot => return self.act(Action::Autopilot, now).into_iter().collect(),
            Command::Internet => return self.act(Action::Internet, now).into_iter().collect(),
            Command::Theme(None) => self.look.next_theme(),
            Command::Theme(Some(theme)) => self.look.set_theme(theme),
            Command::Effort(effort) => return self.effort(effort, now),
            Command::Sync(always) => {
                self.sync_ask = None;
                self.sync_answered();
                return vec![Request::Sync { send: true, always }];
            }
            Command::Stop(None) => return self.act(Action::Stop, now).into_iter().collect(),
            Command::Stop(Some(all)) if all == "all" => {
                return self
                    .crew
                    .agents()
                    .iter()
                    .filter_map(|a| self.crew.stop(&a.name))
                    .collect();
            }
            Command::Stop(Some(agent)) => {
                if self.crew.agent(&agent).is_none() {
                    self.screen.notice(self.no_agent(&agent), now);
                }
                return self.crew.stop(&agent).into_iter().collect();
            }
            Command::Quit => self.quitting = true,
        }
        Vec::new()
    }

    /// Sets how hard the agent written to thinks, or the new chat's; says
    /// it when `effort` is none.
    fn effort(&mut self, effort: Option<iq_core::Effort>, now: u64) -> Vec<Request> {
        let shown =
            |effort: Option<iq_core::Effort>| effort.map_or("as its model chooses", |e| e.as_str());
        match (self.screen.talk().clone(), effort) {
            (Talk::To(agent), Some(effort)) => {
                if self.crew.agent(&agent).is_none() {
                    self.screen.notice(self.no_agent(&agent), now);
                    return Vec::new();
                }
                self.screen.notice(
                    format!("{agent} thinks: {}, from its next call", effort.as_str()),
                    now,
                );
                return vec![Request::Effort { agent, effort }];
            }
            (Talk::To(agent), None) => {
                let current = self.crew.agent(&agent).and_then(|a| a.effort);
                self.screen
                    .notice(format!("{agent} thinks: {}", shown(current)), now);
            }
            (Talk::New(_), Some(effort)) => {
                if let Some(chat) = self.screen.new_chat_mut() {
                    chat.effort = Some(effort);
                }
            }
            (Talk::New(chat), None) => {
                self.screen
                    .notice(format!("The new chat thinks: {}", shown(chat.effort)), now);
            }
        }
        Vec::new()
    }

    /// The model of the provider `word` names, as a model named in a
    /// message is read: by its whole name, the part after the provider's,
    /// or a word of it.
    ///
    /// # Errors
    ///
    /// It names none, said with what `/name` is for, as a word after
    /// `/new` is often meant as the chat's name.
    fn model_named(&self, word: &str) -> Result<String, String> {
        if self.models.is_empty() {
            // The provider's list not read yet: a whole name is taken as
            // it is.
            return if word.contains('/') {
                Ok(word.to_owned())
            } else {
                Err(format!(
                    "iq has not read the provider's models yet, so {word} cannot be matched: \
                     name it whole (provider/model); /name names a chat"
                ))
            };
        }
        crate::named::model(word, &self.models).ok_or_else(|| {
            format!(
                "The provider has no model {word}: /new and /model take one; /name names a chat"
            )
        })
    }

    /// What the person is told when no agent is named `name`.
    fn no_agent(&self, name: &str) -> String {
        let names: Vec<&str> = self.crew.agents().iter().map(|a| a.name.as_str()).collect();
        if names.is_empty() {
            format!("No agent is named {name}: none has joined yet")
        } else {
            format!("No agent is named {name}: there are {}", names.join(", "))
        }
    }

    /// A name for an agent on `model` that no agent has: the model's own
    /// name, without its provider, made unique as the crew makes them.
    fn name_for(&self, model: &str) -> String {
        let model = model.trim();
        let short = model.rsplit_once('/').map_or(model, |(_, rest)| rest);
        let base: String = short
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let base = if base.is_empty() || base == PERSON {
            "agent".to_owned()
        } else {
            base
        };
        let taken = |name: &str| {
            self.crew.agent(name).is_some() || self.screen.starting().iter().any(|n| n == name)
        };
        let mut name = base.clone();
        let mut n = 2;
        while taken(&name) {
            name = format!("{base}-{n}");
            n += 1;
        }
        name
    }

    /// How the work is looked at.
    #[must_use]
    pub fn lens(&self) -> crate::observations::Lens {
        self.lens
    }

    /// Looks at the work as `lens` says.
    pub fn look_at(&mut self, lens: crate::observations::Lens) {
        self.lens = lens;
    }

    /// The work as the lens shows it, now, in the person's time zone.
    #[must_use]
    pub fn seen(&self) -> crate::observations::Seen {
        crate::observations::see(
            self.costs.tasks(),
            self.lens,
            crate::now(),
            &jiff::tz::TimeZone::system(),
        )
    }

    /// How the last sync went, when the machine syncs.
    #[must_use]
    pub fn synced(&self) -> Option<&Synced> {
        self.synced.as_ref()
    }

    /// What waits to be sent to the sync's repository, asked of the
    /// person.
    #[must_use]
    pub fn sync_ask(&self) -> Option<&SyncAsk> {
        self.sync_ask.as_ref()
    }

    /// What a sync would send, line by line, once read for its page;
    /// empty where nothing would be sent.
    #[must_use]
    pub fn changes(&self) -> Option<&str> {
        self.changes.as_deref()
    }

    /// What the agents did between their lines of text, fold by fold.
    #[must_use]
    pub fn steps(&self) -> &crate::steps::Steps {
        &self.steps
    }

    /// The costs.
    #[must_use]
    pub fn costs(&self) -> &Costs {
        &self.costs
    }

    /// The crew.
    #[must_use]
    pub fn crew(&self) -> &CrewView {
        &self.crew
    }

    /// The crew, for what the person does with it.
    pub fn crew_mut(&mut self) -> &mut CrewView {
        &mut self.crew
    }

    /// Takes what the program sends.
    pub fn update(&mut self, update: Update) {
        match update {
            Update::Crew(event) => {
                // Before the crew takes it: an answer's question is still
                // there.
                self.steps.event(&event);
                match &event {
                    Event::Spent {
                        model, cost, usage, ..
                    } => {
                        self.costs.add(Spent {
                            at: crate::now(),
                            model: model.clone(),
                            cost: *cost,
                            tokens: usage.input.0 + usage.output.0,
                        });
                    }
                    Event::Joined { agent, .. } => self.screen.joined(agent),
                    Event::Renamed { from, to } => {
                        self.screen.renamed(from, to);
                        for (name, _) in &mut self.recent {
                            if name == from {
                                name.clone_from(to);
                            }
                        }
                        self.plugins.rename(from, to);
                        if self.unrecalled.remove(from) {
                            self.unrecalled.insert(to.clone());
                        }
                        if let Some(recall) = &mut self.recall
                            && recall.agent == *from
                        {
                            recall.agent.clone_from(to);
                        }
                    }
                    Event::Posted { from, to, text } => {
                        self.posts.push(Post {
                            from: from.clone(),
                            to: to.clone(),
                            text: text.clone(),
                        });
                    }
                    _ => {}
                }
                self.crew.event(event);
            }
            Update::Back(back) => {
                let agent = back.agent.clone();
                let at = back.written.unwrap_or(back.worked);
                let written = back.written;
                let chat = self.crew.agent(&agent).is_some_and(|a| a.by == PERSON);
                self.crew.back(back);
                if !chat {
                    return;
                }
                self.unrecalled.insert(agent.clone());
                if let Some(written) = written {
                    let at = self
                        .recent
                        .iter()
                        .position(|(_, seen)| *seen < written)
                        .unwrap_or(self.recent.len());
                    self.recent.insert(at, (agent.clone(), written));
                }
                // iq opens on the chat the person wrote to last, unless
                // they chose already.
                if !self.chose && self.opening.is_none_or(|best| at >= best) {
                    self.opening = Some(at);
                    self.write_to(&agent);
                    if let Some(recall) = &mut self.recall {
                        recall.opening = true;
                    }
                }
            }
            Update::Kept(kept) => {
                for spent in kept {
                    self.costs.add(spent);
                }
            }
            Update::Quit => self.quitting = true,
            Update::Tasks(tasks) => self.costs.tasks_read(tasks),
            Update::Synced(synced) => {
                self.synced = Some(synced);
                self.sync_ask = None;
                self.changes = None;
            }
            Update::SyncAsk(ask) => self.sync_ask = Some(ask),
            Update::Changes(changes) => self.changes = Some(changes),
            Update::Models(models) => self.models = models,
            // The conversation is the terminal's: the board keeps nothing.
            Update::Said(_) => {}
            Update::Notice(text) => self.screen.notice(text, crate::now()),
            Update::Plugin { plugin, told } => self.plugins.told(&plugin, told),
            Update::Started {
                lead,
                reviewer,
                autopilot,
                internet,
            } => {
                self.screen.new_chats_on(&lead);
                self.crew.started(lead, reviewer, autopilot, internet);
            }
        }
    }
}

/// The words that say little of a request, in English and French, left
/// out of the name made from it.
const SMALL_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "could", "do", "does", "for", "from",
    "how", "i", "in", "is", "it", "its", "me", "my", "of", "on", "or", "our", "please", "should",
    "so", "that", "the", "this", "to", "us", "we", "what", "when", "why", "will", "with", "would",
    "you", "your", "au", "aux", "avec", "ce", "ces", "cette", "comment", "dans", "de", "des", "du",
    "elle", "en", "est", "et", "il", "je", "la", "le", "les", "ma", "mes", "mon", "ne", "nous",
    "on", "ou", "par", "pas", "plus", "pour", "pourquoi", "que", "quel", "quelle", "qui", "sa",
    "se", "ses", "son", "stp", "sur", "svp", "ta", "tes", "ton", "tu", "un", "une", "vous",
];

/// How many words of a request name its chat.
const NAME_WORDS: usize = 3;

/// The longest name made from a request, in characters.
const NAME_CHARS: usize = 24;

/// A chat's name made from the first words of its first request that say
/// something, lowercase, joined by hyphens: `fix-login-redirect` for "Fix
/// the login redirect"; the words of the models it names left out, which
/// say whom it goes to, not what for. None when no word is left.
fn from_words(request: &str, named: &[String]) -> Option<String> {
    let words_of = |text: &str| -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let models: Vec<String> = named.iter().flat_map(|model| words_of(model)).collect();
    let mut name = String::new();
    let words = words_of(request)
        .into_iter()
        .filter(|word| !SMALL_WORDS.contains(&word.as_str()) && !models.contains(word))
        .take(NAME_WORDS);
    for word in words {
        let more = usize::from(!name.is_empty()) + word.chars().count();
        if name.chars().count() + more > NAME_CHARS {
            break;
        }
        if !name.is_empty() {
            name.push('-');
        }
        name.push_str(&word);
    }
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(board: &mut Board, agent: &str, by: &str) {
        board.update(Update::Crew(Event::Joined {
            agent: agent.into(),
            model: "m".into(),
            by: by.into(),
        }));
    }

    fn working(board: &mut Board, agent: &str) {
        board.update(Update::Crew(Event::Working {
            agent: agent.into(),
            prompt: "go".into(),
        }));
    }

    fn key(board: &mut Board, chord: &str) -> Vec<Request> {
        board.press(&Chord::parse(chord).unwrap(), None, 100)
    }

    fn write(board: &mut Board, text: &str) {
        for c in text.chars() {
            let chord = Chord::plain(&c.to_string());
            board.press(&chord, Some(c), 100);
        }
    }

    fn started(board: &mut Board, lead: &str) {
        board.update(Update::Started {
            lead: lead.into(),
            reviewer: "r".into(),
            autopilot: false,
            internet: false,
        });
    }

    #[test]
    fn the_periods_go_round_and_are_those_offered() {
        let mut board = Board::default();
        assert_eq!(board.period(), Period::Last(24 * 3600));
        board.next_period(false);
        assert_eq!(board.period(), Period::Today);
        board.next_period(true);
        board.next_period(true);
        assert_eq!(board.period(), Period::Last(7 * 24 * 3600));
        board.next_period(true);
        assert_eq!(board.period(), Period::Last(3600));
        board.next_period(false);
        assert_eq!(board.period(), Period::Last(7 * 24 * 3600));
        board.sum_over(Period::Last(5));
        assert_eq!(board.period(), Period::Last(7 * 24 * 3600));
        board.sum_over(Period::Today);
        assert_eq!(board.period(), Period::Today);
    }

    #[test]
    fn a_new_chat_starts_its_agent_then_is_written_to() {
        let mut board = Board::default();
        started(&mut board, "lab/lead-5");
        write(&mut board, "  fix the build ");
        assert_eq!(
            key(&mut board, "enter"),
            vec![Request::Start {
                name: "fix-build".into(),
                model: "lab/lead-5".into(),
                task: "fix the build".into(),
                role: None,
                pattern: iq_agent::Pattern::Solo,
                partner: None,
                named: Vec::new(),
                effort: None,
                from_words: true,
            }]
        );
        assert_eq!(board.screen().talking_to(), Some("fix-build"));
        // Written to before it joined, it is not gone: it starts.
        write(&mut board, "and the tests");
        assert!(matches!(
            &key(&mut board, "enter")[..],
            [Request::Tell { to, .. }] if to == "fix-build"
        ));
        // Another chat asked the same is named apart; one the person
        // names, or with no word that says something, is named so.
        board.run(Command::New(None), 100);
        write(&mut board, "Fix the build!");
        assert!(matches!(
            &key(&mut board, "enter")[..],
            [Request::Start { name, .. }] if name == "fix-build-2"
        ));
        board.run(Command::New(None), 100);
        board.run(Command::Name("Login redirect".into()), 100);
        write(&mut board, "it loops");
        assert!(matches!(
            &key(&mut board, "enter")[..],
            [Request::Start { name, from_words: false, .. }] if name == "login-redirect"
        ));
        board.run(Command::New(None), 100);
        write(&mut board, "is it?");
        assert!(matches!(
            &key(&mut board, "enter")[..],
            [Request::Start { name, .. }] if name == "lead-5"
        ));
        // A chat written to is renamed by the crew, once it joined.
        joined(&mut board, "lead-5", PERSON);
        assert_eq!(
            board.run(Command::Name("build".into()), 100),
            vec![Request::Rename {
                agent: "lead-5".into(),
                name: "build".into()
            }]
        );
        board.run(Command::New(None), 100);
        board.run(Command::Model("lab/second".into()), 100);
        board.run(Command::Role("review".into()), 100);
        board.run(
            Command::Pattern(iq_agent::Pattern::Pair, Some("z-ai/glm-5".into())),
            100,
        );
        write(&mut board, "look at it");
        assert!(matches!(
            &key(&mut board, "enter")[..],
            [Request::Start { name, model, role: Some(role), partner: Some(partner), .. }]
                if name == "review" && model == "lab/second" && role == "review"
                    && partner == "z-ai/glm-5"
        ));
        joined(&mut board, "fix-build", PERSON);
        assert!(board.screen().starting().iter().all(|n| n != "fix-build"));
        assert_eq!(key(&mut board, "enter"), vec![]);
    }

    #[test]
    fn messages_go_where_the_person_says_and_lines_go_on() {
        let mut board = Board::default();
        started(&mut board, "lab/second");
        joined(&mut board, "lead", PERSON);
        joined(&mut board, "helper", "lead");
        joined(&mut board, "fix", PERSON);
        board.screen_mut().talk_to("lead");
        write(&mut board, "@helper read the tests");
        assert_eq!(
            key(&mut board, "enter"),
            vec![Request::Tell {
                to: "helper".into(),
                text: "read the tests".into(),
                named: Vec::new(),
            }]
        );
        // Shift-Enter and a trailing backslash go to a new line.
        write(&mut board, "one");
        key(&mut board, "shift+enter");
        write(&mut board, "two\\");
        key(&mut board, "enter");
        write(&mut board, "three");
        assert_eq!(board.screen().editor().text(), "one\ntwo\nthree");
        assert_eq!(
            key(&mut board, "enter"),
            vec![Request::Tell {
                to: "lead".into(),
                text: "one\ntwo\nthree".into(),
                named: Vec::new(),
            }]
        );
        // An agent no one has is said so, and the message stays.
        write(&mut board, "@nobody hi");
        assert_eq!(key(&mut board, "enter"), vec![]);
        assert!(
            board
                .screen()
                .noticed(100)
                .unwrap()
                .contains("lead, helper")
        );
        assert_eq!(board.screen().editor().text(), "@nobody hi");
        // Ctrl-N writes to a new chat, on the lead's model.
        key(&mut board, "ctrl+c");
        key(&mut board, "ctrl+n");
        assert!(matches!(board.screen().talk(), Talk::New(chat) if chat.model == "lab/second"));
    }

    #[test]
    fn the_switcher_goes_to_the_chat_used_before_and_on_to_the_others() {
        let mut board = Board::default();
        started(&mut board, "lab/lead");
        // No chat yet: nothing to go to.
        key(&mut board, "ctrl+space");
        assert!(board.switcher().is_none());
        assert!(board.screen().noticed(100).unwrap().contains("No chat yet"));
        for chat in ["one", "two", "three", "four", "five", "six", "seven"] {
            joined(&mut board, chat, PERSON);
        }
        joined(&mut board, "one/review", "one");
        // Used in this order: the latest first, then those never used.
        for chat in ["two", "one", "three"] {
            board.run(Command::To(chat.into()), 100);
        }
        let names = |board: &Board| -> Vec<String> {
            board
                .switcher()
                .unwrap()
                .chats
                .iter()
                .map(|(view, _)| view.name.clone())
                .collect()
        };
        key(&mut board, "ctrl+space");
        assert_eq!(names(&board)[..3], ["three", "one", "two"]);
        // An agent a chat started is no chat of the person's.
        assert_eq!(names(&board).len(), 7);
        // Open, on the chat used before the one written to.
        assert_eq!(board.switcher().unwrap().chosen, 1);
        key(&mut board, "enter");
        assert!(board.switcher().is_none());
        assert_eq!(board.screen().talking_to(), Some("one"));
        // Once more, back to the other: the two used last, one key apart.
        key(&mut board, "ctrl+space");
        key(&mut board, "enter");
        assert_eq!(board.screen().talking_to(), Some("three"));
        // Again, and again, the next; the rows shown follow the one chosen.
        key(&mut board, "ctrl+space");
        for _ in 0..5 {
            key(&mut board, "ctrl+space");
        }
        let switcher = board.switcher().unwrap();
        assert_eq!((switcher.chosen, switcher.first), (6, 2));
        key(&mut board, "down");
        assert_eq!(board.switcher().unwrap().chosen, 0);
        key(&mut board, "up");
        board.scroll(-1);
        assert_eq!(board.switcher().unwrap().chosen, 5);
        // Escape closes it where the person writes; a letter closes it and
        // is written.
        key(&mut board, "escape");
        assert!(board.switcher().is_none());
        assert_eq!(board.screen().talking_to(), Some("three"));
        key(&mut board, "ctrl+space");
        write(&mut board, "h");
        assert!(board.switcher().is_none());
        assert_eq!(board.screen().editor().text(), "h");
        // Renamed, a chat keeps its place.
        board.update(Update::Crew(Event::Renamed {
            from: "three".into(),
            to: "login".into(),
        }));
        key(&mut board, "ctrl+space");
        assert_eq!(names(&board)[..2], ["login", "one"]);
    }

    #[test]
    fn a_chat_starts_and_goes_on_only_on_a_model_of_the_providers() {
        let mut board = Board::default();
        started(&mut board, "lab/lead");
        // The provider's models not read yet: only a whole name is taken.
        board.run(Command::New(Some("chat2".into())), 100);
        assert!(
            board
                .screen()
                .noticed(100)
                .unwrap()
                .contains("/name names a chat")
        );
        board.update(Update::Models(vec![
            "lab/lead".into(),
            "lab/sage-5-5".into(),
        ]));
        board.run(Command::New(Some("chat2".into())), 100);
        assert!(
            board
                .screen()
                .noticed(100)
                .unwrap()
                .contains("The provider has no model chat2")
        );
        assert!(matches!(board.screen().talk(), Talk::To(_) | Talk::New(_)));
        board.run(Command::New(Some("sage".into())), 100);
        assert!(matches!(board.screen().talk(), Talk::New(chat) if chat.model == "lab/sage-5-5"));
        // The chat written to goes on on another.
        joined(&mut board, "fix", PERSON);
        board.run(Command::To("fix".into()), 100);
        assert_eq!(
            board.run(Command::Model("lead".into()), 100),
            vec![Request::Model {
                agent: "fix".into(),
                model: "lab/lead".into()
            }]
        );
        assert!(board.run(Command::Model("nothing".into()), 100).is_empty());
        // What iq answers is said where the person writes.
        board.update(Update::Notice("fix answers on lab/lead".into()));
        assert_eq!(
            board.screen().noticed(crate::now()),
            Some("fix answers on lab/lead")
        );
        // The provider's models complete what is begun.
        board.screen_mut().editor_mut().set("/model sa");
        assert!(
            board
                .completions()
                .iter()
                .any(|c| c.line == "/model lab/sage-5-5")
        );
    }

    fn back(board: &mut Board, agent: &str, worked: u64, written: Option<u64>) {
        board.update(Update::Back(Back {
            agent: agent.into(),
            task: format!("the task of {agent}"),
            worked,
            written,
            thread: vec![crate::crew::Entry::Prompt(format!("the task of {agent}"))],
        }));
    }

    #[test]
    fn iq_opens_on_the_chat_written_to_last_and_recalls_a_chat_come_back_to() {
        let mut board = Board::default();
        started(&mut board, "lab/lead");
        for (agent, by) in [
            ("today", PERSON),
            ("yesterday", PERSON),
            ("review", "today"),
        ] {
            joined(&mut board, agent, by);
        }
        // Written to last, though another worked since; an agent a chat
        // started is not one to open on.
        back(&mut board, "yesterday", 100, Some(90));
        back(&mut board, "today", 200, Some(150));
        back(&mut board, "review", 300, None);
        assert_eq!(board.screen().talking_to(), Some("today"));
        assert_eq!(
            board.take_recall(),
            Some(Recall {
                agent: "today".into(),
                opening: true
            })
        );
        assert_eq!(board.take_recall(), None);
        // Come back to, a chat is recalled once.
        key(&mut board, "ctrl+space");
        assert_eq!(
            board.switcher().unwrap().chats[1].0.name,
            "yesterday",
            "the one written to before"
        );
        key(&mut board, "enter");
        assert_eq!(board.screen().talking_to(), Some("yesterday"));
        assert_eq!(
            board.take_recall(),
            Some(Recall {
                agent: "yesterday".into(),
                opening: false
            })
        );
        key(&mut board, "ctrl+space");
        key(&mut board, "enter");
        key(&mut board, "ctrl+space");
        key(&mut board, "enter");
        assert_eq!(board.screen().talking_to(), Some("yesterday"));
        assert_eq!(board.take_recall(), None);
        // Renamed, it is written to under its new name.
        board.update(Update::Crew(Event::Renamed {
            from: "yesterday".into(),
            to: "login".into(),
        }));
        assert_eq!(board.screen().talking_to(), Some("login"));
        assert_eq!(
            board.crew().agent("login").unwrap().task,
            "the task of yesterday"
        );
    }

    #[test]
    fn a_chat_coming_back_leaves_where_the_person_chose_to_write() {
        let mut board = Board::default();
        started(&mut board, "lab/lead");
        joined(&mut board, "old", PERSON);
        joined(&mut board, "new", PERSON);
        board.run(Command::To("new".into()), 100);
        back(&mut board, "old", 100, Some(100));
        assert_eq!(board.screen().talking_to(), Some("new"));
        assert_eq!(board.take_recall(), None);
    }

    #[test]
    fn a_chat_is_named_after_the_words_of_its_request_that_say_something() {
        assert_eq!(
            from_words("Fix the login redirect, please", &[]).as_deref(),
            Some("fix-login-redirect")
        );
        assert_eq!(
            from_words("corrige le test de la page de connexion", &[]).as_deref(),
            Some("corrige-test-page")
        );
        assert_eq!(
            from_words("with swift, translate the docs", &["lab/swift-5".into()]).as_deref(),
            Some("translate-docs")
        );
        assert_eq!(
            from_words("internationalisation configuration everywhere", &[]).as_deref(),
            Some("internationalisation")
        );
        assert_eq!(from_words("is it?", &[]), None);
    }

    #[test]
    fn a_model_named_in_a_message_starts_the_chat_or_goes_with_it() {
        let mut board = Board::default();
        started(&mut board, "anthropic/claude-opus-5-5");
        board.update(Update::Models(vec![
            "anthropic/claude-opus-5-5".into(),
            "anthropic/claude-haiku-5".into(),
            "z-ai/glm-5.3-flash".into(),
        ]));
        write(&mut board, "avec haiku, corrige le test");
        assert!(matches!(
            &key(&mut board, "enter")[..],
            [Request::Start { model, named, .. }]
                if model == "anthropic/claude-haiku-5" && named == &["anthropic/claude-haiku-5"]
        ));
        assert_eq!(board.screen().talking_to(), Some("corrige-test"));
        joined(&mut board, "corrige-test", PERSON);
        write(
            &mut board,
            "fais la conception avec opus et le code avec glm-5.3-flash",
        );
        assert!(matches!(
            &key(&mut board, "enter")[..],
            [Request::Tell { named, .. }]
                if named == &["anthropic/claude-opus-5-5", "z-ai/glm-5.3-flash"]
        ));
    }

    #[test]
    fn a_new_task_starts_on_the_lead_which_decides_whom_to_give_it() {
        let mut board = Board::default();
        started(&mut board, "anthropic/claude-opus-5-5");
        write(&mut board, "fix add() in calc.py");
        assert!(matches!(
            &key(&mut board, "enter")[..],
            [Request::Start { model, .. }] if model == "anthropic/claude-opus-5-5"
        ));
        assert_eq!(board.models_offered(), ["anthropic/claude-opus-5-5"]);
    }

    #[test]
    fn commands_complete_open_pages_and_pages_take_their_keys() {
        let mut board = Board::default();
        write(&mut board, "/co");
        assert_eq!(board.completions().len(), 1);
        key(&mut board, "enter");
        assert_eq!(board.screen().page(), Some(&Page::Costs));
        assert!(board.screen().editor().is_empty());
        // On a page, letters choose and Escape closes; nothing is written.
        board.press(&Chord::plain("p"), Some('p'), 100);
        assert_eq!(board.period(), Period::Last(7 * 24 * 3600));
        key(&mut board, "escape");
        assert_eq!(board.screen().page(), None);
        assert!(board.screen().editor().is_empty());
        // A shortcut opens and closes its page.
        key(&mut board, "ctrl+j");
        assert_eq!(board.screen().page(), Some(&Page::Observations));
        key(&mut board, "left");
        assert_eq!(board.lens().back, 1);
        key(&mut board, "ctrl+j");
        assert_eq!(board.screen().page(), None);
        // What takes more is only completed.
        write(&mut board, "/the");
        key(&mut board, "tab");
        assert_eq!(board.screen().editor().text(), "/theme ");
        write(&mut board, "ink");
        key(&mut board, "enter");
        assert_eq!(board.look().theme(), crate::look::Theme::Ink);
        // An unknown command is said so.
        write(&mut board, "/fly");
        key(&mut board, "escape");
        key(&mut board, "enter");
        assert!(board.screen().noticed(100).unwrap().contains("no /fly"));
    }

    #[test]
    fn ctrl_c_clears_then_stops_then_quits() {
        let mut board = Board::default();
        joined(&mut board, "lead", PERSON);
        board.screen_mut().talk_to("lead");
        working(&mut board, "lead");
        write(&mut board, "draft");
        assert_eq!(key(&mut board, "ctrl+c"), vec![]);
        assert!(board.screen().editor().is_empty());
        assert_eq!(
            key(&mut board, "ctrl+c"),
            vec![Request::Stop {
                agent: "lead".into()
            }]
        );
        board.update(Update::Crew(Event::Done {
            agent: "lead".into(),
            outcome: Ok("done".into()),
        }));
        key(&mut board, "ctrl+c");
        assert!(!board.quitting());
        key(&mut board, "ctrl+c");
        assert!(board.quitting());
    }

    #[test]
    fn keys_answer_the_oldest_question_and_stop_the_agent_written_to() {
        let mut board = Board::default();
        joined(&mut board, "lead", PERSON);
        joined(&mut board, "helper", "lead");
        board.screen_mut().talk_to("lead");
        working(&mut board, "lead");
        for (id, agent) in [(3, "helper"), (4, "lead")] {
            board.update(Update::Crew(Event::Asked {
                id,
                agent: agent.into(),
                action: "run x".into(),
                reason: "it reaches the internet".into(),
                widens: true,
            }));
        }
        assert_eq!(
            key(&mut board, "ctrl+y"),
            vec![Request::Answer {
                id: 3,
                approved: true
            }]
        );
        assert_eq!(
            key(&mut board, "ctrl+r"),
            vec![Request::Answer {
                id: 4,
                approved: false
            }]
        );
        assert_eq!(key(&mut board, "ctrl+r"), vec![]);
        assert_eq!(
            key(&mut board, "escape"),
            vec![Request::Stop {
                agent: "lead".into()
            }]
        );
        board.screen_mut().talk_to("helper");
        assert_eq!(key(&mut board, "escape"), vec![]);
        assert_eq!(key(&mut board, "shift+tab"), vec![Request::Autopilot(true)]);
        // On the team's page, the agent chosen is written to.
        key(&mut board, "ctrl+t");
        key(&mut board, "down");
        key(&mut board, "enter");
        assert_eq!(board.screen().page(), None);
        assert_eq!(board.screen().talking_to(), Some("helper"));
    }

    #[test]
    fn the_effort_is_set_for_the_agent_written_to_or_the_new_chat() {
        let mut board = Board::default();
        started(&mut board, "anthropic/claude-opus-5-5");
        board.run(Command::Effort(Some(iq_core::Effort::Low)), 100);
        write(&mut board, "rename it");
        assert!(matches!(
            &key(&mut board, "enter")[..],
            [Request::Start {
                effort: Some(iq_core::Effort::Low),
                ..
            }]
        ));
        joined(&mut board, "lead", PERSON);
        board.screen_mut().talk_to("lead");
        assert_eq!(
            board.run(Command::Effort(Some(iq_core::Effort::Max)), 100),
            vec![Request::Effort {
                agent: "lead".into(),
                effort: iq_core::Effort::Max
            }]
        );
        board.update(Update::Crew(Event::Effort {
            agent: "lead".into(),
            effort: Some(iq_core::Effort::Max),
        }));
        board.run(Command::Effort(None), 100);
        assert_eq!(board.screen().noticed(100), Some("lead thinks: max"));
    }

    #[test]
    fn posts_are_taken_once() {
        let mut board = Board::default();
        joined(&mut board, "lead", PERSON);
        board.update(Update::Crew(Event::Posted {
            from: "lead".into(),
            to: PERSON.into(),
            text: "done".into(),
        }));
        assert_eq!(board.take_posts().len(), 1);
        assert!(board.take_posts().is_empty());
    }

    #[test]
    fn what_changed_is_sent_once_the_person_says_so() {
        let mut board = Board::default();
        assert_eq!(board.act(Action::Allow, 100), None);
        let ask = || {
            Update::SyncAsk(SyncAsk {
                remote: "https://example.com/notes.git".into(),
                files: vec!["config/skills/teams/SKILL.md".into()],
                said: "config/skills/teams/SKILL.md".into(),
            })
        };
        board.update(ask());
        assert!(board.sync_ask().is_some());
        assert_eq!(
            board.act(Action::Refuse, 100),
            Some(Request::Sync {
                send: false,
                always: false
            })
        );
        assert!(board.sync_ask().is_none());
        board.update(ask());
        assert_eq!(
            board.act(Action::Allow, 100),
            Some(Request::Sync {
                send: true,
                always: false
            })
        );
        board.update(ask());
        assert_eq!(
            board.run(Command::Sync(true), 100),
            [Request::Sync {
                send: true,
                always: true
            }]
        );
        assert!(board.sync_ask().is_none());
    }

    #[test]
    fn a_plugins_command_is_completed_and_sent_to_it_with_the_chat_written_to() {
        use crate::plugins::{PluginCommand, Told};
        let mut board = Board::default();
        joined(&mut board, "lead", PERSON);
        joined(&mut board, "fix", PERSON);
        board.update(Update::Crew(Event::Placed {
            agent: "lead".into(),
            folder: "/w/lead".into(),
            said: None,
        }));
        board.update(Update::Plugin {
            plugin: "ports".into(),
            told: Told::Commands(vec![PluginCommand {
                name: "serve".into(),
                args: "[chat]".into(),
                what: "localhost to this chat".into(),
            }]),
        });
        board.screen_mut().talk_to("lead");
        board.screen_mut().editor_mut().set("/se");
        let listed = board.completions();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].line, "/serve ");
        assert_eq!(listed[0].what, "localhost to this chat (ports)");
        board.screen_mut().editor_mut().set("/serve f");
        assert_eq!(board.completions()[0].line, "/serve fix");
        board.screen_mut().editor_mut().set("/serve");
        assert_eq!(
            board.send(100),
            [Request::Plugin {
                plugin: "ports".into(),
                command: "serve".into(),
                args: String::new(),
                chat: Some("lead".into()),
                folder: Some("/w/lead".into()),
            }]
        );
        assert!(board.screen().editor().is_empty());
        // Gone, its command is iq's no more.
        board.update(Update::Plugin {
            plugin: "ports".into(),
            told: Told::Gone,
        });
        board.screen_mut().editor_mut().set("/serve");
        assert!(board.send(100).is_empty());
        assert!(!board.screen().editor().is_empty());
    }

    #[test]
    fn the_steps_unfold_on_their_page_the_latest_last() {
        let mut board = Board::default();
        assert_eq!(key(&mut board, "ctrl+x"), vec![]);
        assert_eq!(board.screen().page(), Some(&Page::Steps));
        // Opened at its end, which drawing brings back within its lines.
        assert_eq!(board.screen().scroll(), usize::MAX);
        key(&mut board, "ctrl+x");
        assert_eq!(board.screen().page(), None);
    }

    #[test]
    fn what_a_sync_would_send_is_read_on_its_page_and_answered_from_there() {
        let mut board = Board::default();
        board.update(Update::SyncAsk(SyncAsk {
            remote: "https://example.com/notes.git".into(),
            files: vec!["config/settings.toml".into()],
            said: "config/settings.toml".into(),
        }));
        assert_eq!(board.act(Action::Changes, 100), Some(Request::Changes));
        assert_eq!(board.screen().page(), Some(&Page::Changes));
        assert_eq!(board.changes(), None);
        board.update(Update::Changes("diff --git a/x b/x\n+one\n".into()));
        assert_eq!(board.changes(), Some("diff --git a/x b/x\n+one\n"));
        // The same key closes it, and nothing is read again.
        assert_eq!(board.act(Action::Changes, 100), None);
        assert_eq!(board.screen().page(), None);
        assert_eq!(board.changes(), Some("diff --git a/x b/x\n+one\n"));
        // Opened again, it is read afresh; answered there, it closes.
        assert_eq!(key(&mut board, "ctrl+g"), [Request::Changes]);
        assert_eq!(board.changes(), None);
        assert_eq!(
            key(&mut board, "ctrl+y"),
            [Request::Sync {
                send: true,
                always: false
            }]
        );
        assert_eq!(board.screen().page(), None);
        assert!(board.sync_ask().is_none());
        // Once synced, what was read is stale, and goes.
        board.update(Update::Changes(String::new()));
        board.update(Update::Synced(Synced {
            at: 100,
            ok: true,
            said: "sent 1 file".into(),
        }));
        assert_eq!(board.changes(), None);
    }

    #[test]
    fn calls_billed_to_the_provider_are_counted_as_they_come() {
        let mut board = Board::default();
        board.update(Update::Crew(Event::Spent {
            agent: "lead".into(),
            model: "openai/gpt-5-mini".into(),
            usage: iq_core::Usage::default(),
            cost: Some(iq_core::Usd(0.25)),
        }));
        let totals = board.costs().totals(0);
        assert_eq!(totals.len(), 1);
        assert_eq!(totals[0].cost, iq_core::Usd(0.25));
    }
}
