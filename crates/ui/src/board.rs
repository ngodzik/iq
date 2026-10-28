//! iq's state in the terminal: the crew, where the person writes and what
//! page they look at, the costs, the look and the keys, as what the program reads arrives and as the person acts. Every
//! key the person presses is taken here, so that what it does is tested
//! without a terminal.

use iq_agent::{Event, PERSON};

use crate::command::{self, Command, Completion};
use crate::costs::{Costs, Spent};
use crate::crew::{CrewView, Question, Request};
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

/// What the program sends the terminal.
#[derive(Debug, Clone)]
pub enum Update {
    /// What the crew just did.
    Crew(Event),
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
    /// The provider's models, as it lists them: what the person may name.
    Models(Vec<String>),
    /// Something iq says to the person in passing, in the conversation.
    Said(String),
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
}

/// How far back the costs are summed, in seconds, as the costs' page
/// offers it.
pub const PERIODS: [u64; 4] = [3600, 6 * 3600, 24 * 3600, 7 * 24 * 3600];

/// iq's state in the terminal.
#[derive(Debug, Clone)]
pub struct Board {
    crew: CrewView,
    screen: Screen,
    keys: Keymap,
    look: Look,
    /// How far back the costs are summed, one of [`PERIODS`].
    period: u64,
    costs: Costs,
    /// The messages posted since the terminal last took them.
    posts: Vec<Post>,
    /// Whether iq was asked to stop.
    quitting: bool,
    /// How the last sync went, when the machine syncs.
    synced: Option<Synced>,
    /// What waits to be sent, asked of the person.
    sync_ask: Option<SyncAsk>,
    /// How the work is looked at.
    lens: crate::observations::Lens,
    /// The provider's models, as it lists them.
    models: Vec<String>,
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
            period: PERIODS[2],
            costs: Costs::default(),
            posts: Vec::new(),
            quitting: false,
            synced: None,
            sync_ask: None,
            lens: crate::observations::Lens::default(),
            models: Vec::new(),
        }
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

    /// How far back the costs are summed, in seconds.
    #[must_use]
    pub fn period(&self) -> u64 {
        self.period
    }

    /// Sums the costs over `period`, one of [`PERIODS`].
    pub fn sum_over(&mut self, period: u64) {
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
        let models = self.models_offered();
        let models: Vec<&str> = models.iter().map(String::as_str).collect();
        command::complete(text, &agents, &models)
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
        if self.screen.page().is_some() {
            if plain && !chord.shift && chord.key == "Escape" {
                self.screen.close();
                return Vec::new();
            }
            if let Some(action) = self.keys.action(chord) {
                return self.act(action).into_iter().collect();
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
            return self.act(action).into_iter().collect();
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

    /// Scrolls the page open by `lines`, as the wheel does.
    pub fn scroll(&mut self, lines: isize) {
        if self.screen.page() == Some(&Page::Team) {
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
                    .agents()
                    .get(self.screen.chosen())
                    .map(|a| a.name.clone());
                match (key, letter, agent) {
                    ("Enter", _, Some(agent)) | (_, Some('w'), Some(agent)) => {
                        self.screen.talk_to(&agent);
                        self.screen.close();
                    }
                    ("Right", _, Some(agent)) | (_, Some('v'), Some(agent)) => {
                        self.screen.open(Page::Thread(agent));
                    }
                    (_, Some('n'), _) => {
                        self.screen.new_chat(self.crew.lead());
                        self.screen.close();
                    }
                    _ => {}
                }
            }
            Page::Thread(_) if key == "Left" => self.screen.close(),
            Page::Thread(_) | Page::Keys => {}
        }
        Vec::new()
    }

    /// Does what `action` says. Returns what it asks of the crew, if it
    /// asks something.
    pub fn act(&mut self, action: Action) -> Option<Request> {
        match action {
            Action::NextAgent => self.next_agent(),
            Action::Team => self.screen.toggle(Page::Team),
            Action::Costs => self.screen.toggle(Page::Costs),
            Action::Observations => self.screen.toggle(Page::Observations),
            Action::Keys => self.screen.toggle(Page::Keys),
            Action::Theme => self.look.next_theme(),
            Action::Autopilot => return Some(self.crew.toggle_autopilot()),
            Action::Internet => return Some(self.crew.toggle_internet()),
            Action::Allow | Action::Refuse => {
                // The agents' questions first: they hold work.
                if let Some(id) = self.crew.questions().first().map(|q| q.id) {
                    return Some(self.crew.answer(id, action == Action::Allow));
                }
                self.sync_ask.take()?;
                return Some(Request::Sync {
                    send: action == Action::Allow,
                    always: false,
                });
            }
            Action::Stop => {
                let agent = self.screen.talking_to()?.to_owned();
                return self.crew.stop(&agent);
            }
            Action::Quit => self.quitting = true,
        }
        None
    }

    /// Writes to the agent after the one written to, in the order they
    /// joined; after the last, to a new chat on the lead's model.
    fn next_agent(&mut self) {
        let names: Vec<String> = self.crew.agents().iter().map(|a| a.name.clone()).collect();
        let next = match self.screen.talking_to() {
            None => names.first(),
            Some(agent) => names
                .iter()
                .position(|name| name == agent)
                .and_then(|at| names.get(at + 1)),
        };
        match next {
            Some(agent) => self.screen.talk_to(agent),
            None => self.screen.new_chat(self.crew.lead()),
        }
    }

    /// Sends what the person wrote at `now`: a command, a message to an
    /// agent named after `@`, a message to the agent written to, or the
    /// task that starts a new chat.
    pub fn send(&mut self, now: u64) -> Vec<Request> {
        let text = self.screen.editor().text().trim().to_owned();
        if text.is_empty() {
            return Vec::new();
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
                self.screen.talk_to(name);
                self.screen.editor_mut().take();
                return Vec::new();
            }
            self.screen.editor_mut().take();
            let text = message.trim().to_owned();
            return vec![Request::Tell {
                to: name.to_owned(),
                named: crate::named::named(&text, &self.models),
                text,
            }];
        }
        let request = match self.screen.talk().clone() {
            Talk::To(agent) => {
                if self.crew.agent(&agent).is_none() && !self.screen.starting().contains(&agent) {
                    self.screen
                        .notice(format!("{agent} is gone: /new starts a chat"), now);
                    return Vec::new();
                }
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
                // A one-word role names the agent, else its model does.
                let base = match &role {
                    Some(role) if !role.contains(char::is_whitespace) => role.clone(),
                    _ => model.clone(),
                };
                let name = self.name_for(&base);
                let partner = Some(chat.partner.trim().to_owned())
                    .filter(|p| !p.is_empty() && chat.pattern.takes_partner());
                self.screen.starts(&name);
                self.screen.talk_to(&name);
                Request::Start {
                    name,
                    model,
                    task: text,
                    role,
                    pattern: chat.pattern,
                    partner,
                    named,
                    effort: chat.effort,
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
                let model = model.unwrap_or_else(|| self.crew.lead().to_owned());
                self.screen.new_chat(&model);
            }
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
                    self.screen.talk_to(&agent);
                } else {
                    self.screen.notice(self.no_agent(&agent), now);
                }
            }
            Command::Team => open(self, Page::Team),
            Command::Costs => open(self, Page::Costs),
            Command::Observations => open(self, Page::Observations),
            Command::Keys => open(self, Page::Keys),
            Command::Autopilot => return self.act(Action::Autopilot).into_iter().collect(),
            Command::Internet => return self.act(Action::Internet).into_iter().collect(),
            Command::Theme(None) => self.look.next_theme(),
            Command::Theme(Some(theme)) => self.look.set_theme(theme),
            Command::Effort(effort) => return self.effort(effort, now),
            Command::Sync(always) => {
                self.sync_ask = None;
                return vec![Request::Sync { send: true, always }];
            }
            Command::Stop(None) => return self.act(Action::Stop).into_iter().collect(),
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
            }
            Update::SyncAsk(ask) => self.sync_ask = Some(ask),
            Update::Models(models) => self.models = models,
            // The conversation is the terminal's: the board keeps nothing.
            Update::Said(_) => {}
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
        assert_eq!(board.period(), 24 * 3600);
        board.next_period(true);
        assert_eq!(board.period(), 7 * 24 * 3600);
        board.next_period(true);
        assert_eq!(board.period(), 3600);
        board.next_period(false);
        assert_eq!(board.period(), 7 * 24 * 3600);
        board.sum_over(5);
        assert_eq!(board.period(), 7 * 24 * 3600);
    }

    #[test]
    fn a_new_chat_starts_its_agent_then_is_written_to() {
        let mut board = Board::default();
        started(&mut board, "anthropic/claude-opus-5-5");
        write(&mut board, "  fix the build ");
        assert_eq!(
            key(&mut board, "enter"),
            vec![Request::Start {
                name: "claude-opus-5-5".into(),
                model: "anthropic/claude-opus-5-5".into(),
                task: "fix the build".into(),
                role: None,
                pattern: iq_agent::Pattern::Solo,
                partner: None,
                named: Vec::new(),
                effort: None,
            }]
        );
        assert_eq!(board.screen().talking_to(), Some("claude-opus-5-5"));
        // Written to before it joined, it is not gone: it starts.
        write(&mut board, "and the tests");
        assert!(matches!(
            &key(&mut board, "enter")[..],
            [Request::Tell { to, .. }] if to == "claude-opus-5-5"
        ));
        // A second chat on the same model is named apart, a one-word role
        // names one.
        board.run(Command::New(None), 100);
        write(&mut board, "review it");
        assert!(matches!(
            &key(&mut board, "enter")[..],
            [Request::Start { name, .. }] if name == "claude-opus-5-5-2"
        ));
        board.run(Command::New(Some("lab/second".into())), 100);
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
        joined(&mut board, "claude-opus-5-5", PERSON);
        assert!(
            board
                .screen()
                .starting()
                .iter()
                .all(|n| n != "claude-opus-5-5")
        );
        assert_eq!(key(&mut board, "enter"), vec![]);
    }

    #[test]
    fn messages_go_where_the_person_says_and_lines_go_on() {
        let mut board = Board::default();
        started(&mut board, "lab/second");
        joined(&mut board, "lead", PERSON);
        joined(&mut board, "helper", "lead");
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
        // Ctrl-N goes round the agents, then to a new chat.
        key(&mut board, "ctrl+c");
        key(&mut board, "ctrl+n");
        assert_eq!(board.screen().talking_to(), Some("helper"));
        key(&mut board, "ctrl+n");
        assert!(matches!(board.screen().talk(), Talk::New(chat) if chat.model == "lab/second"));
        key(&mut board, "ctrl+n");
        assert_eq!(board.screen().talking_to(), Some("lead"));
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
        assert_eq!(board.screen().talking_to(), Some("claude-haiku-5"));
        joined(&mut board, "claude-haiku-5", PERSON);
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
        assert_eq!(board.period(), 7 * 24 * 3600);
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
        assert_eq!(board.act(Action::Allow), None);
        let ask = || {
            Update::SyncAsk(SyncAsk {
                remote: "https://example.com/notes.git".into(),
                files: vec!["config/skills/teams/SKILL.md".into()],
            })
        };
        board.update(ask());
        assert!(board.sync_ask().is_some());
        assert_eq!(
            board.act(Action::Refuse),
            Some(Request::Sync {
                send: false,
                always: false
            })
        );
        assert!(board.sync_ask().is_none());
        board.update(ask());
        assert_eq!(
            board.act(Action::Allow),
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
