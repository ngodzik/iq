//! The window's state: its panes and what they show, the crew, the costs
//! and the subscriptions, its look and its keys, as what the program reads
//! arrives and as the person acts.

use iq_agent::{Event, PERSON, PROVIDER, STEWARD};
use iq_core::{ModelId, SubscriptionCall};

use crate::assay::{AssayView, JUDGE, Overview, Progress};
use crate::costs::{Costs, Payer, Spent};
use crate::crew::{AssayAsk, CrewView, Question, Request};
use crate::keys::{Action, Keymap};
#[cfg(test)]
use crate::layout::Pane;
use crate::layout::{Layout, NewChat, Shows};
use crate::look::Look;
use crate::steward::{self, Gauge, Limits, Pace, Steward};
use crate::subscriptions::{Seen, Subscriptions};

/// How often the steward looks again, in seconds.
const STEWARD_EVERY: u64 = 5;

/// How many of the steward's exchanges the window keeps.
const STEWARD_KEPT: usize = 40;

/// A message on the board, for the window to show it travel.
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

    /// Whether the steward wrote it.
    #[must_use]
    pub fn from_steward(&self) -> bool {
        self.from == STEWARD
    }

    /// Whether it was written to the steward.
    #[must_use]
    pub fn to_steward(&self) -> bool {
        self.to == STEWARD
    }
}

/// What the program sends the window.
#[derive(Debug, Clone)]
pub enum Update {
    /// The agents' files, as just read.
    Read(Read),
    /// What the crew just did.
    Crew(Event),
    /// What iq kept of earlier sessions: the calls billed to the
    /// provider, and the quotas heard.
    Kept {
        /// The calls billed, oldest first.
        provider: Vec<Spent>,
        /// The quotas.
        quotas: Seen,
    },
    /// iq is asked to stop: the window closes, its agents with it.
    Quit,
    /// The assay's store, as just read.
    Assay(Box<Overview>),
    /// The assay's runs at work, as they go.
    AssayRunning(Vec<Progress>),
    /// iq's tasks, by the kinds of task the assay's rules place them in.
    Tasks(Vec<crate::costs::TaskSpent>),
    /// A sync of iq's data ended.
    Synced(Synced),
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

/// How far back the costs are summed, in seconds, as the window offers it.
pub const PERIODS: [u64; 4] = [3600, 6 * 3600, 24 * 3600, 7 * 24 * 3600];

/// What the agents' files hold, as the loop just read them: each call, and
/// what it would cost at list prices.
#[derive(Debug, Clone, Default)]
pub struct Read {
    /// The calls, oldest first.
    pub calls: Vec<SubscriptionCall>,
    /// What each call would cost, in the same order.
    pub spent: Vec<Spent>,
}

/// The window's state.
#[derive(Debug, Clone)]
pub struct Board {
    crew: CrewView,
    layout: Layout,
    keys: Keymap,
    look: Look,
    /// How far back the costs are summed, one of [`PERIODS`].
    period: u64,
    costs: Costs,
    subscriptions: Subscriptions,
    /// Whether the agents' files were read once.
    read: bool,
    /// Whether the menu shows at the top.
    menu: bool,
    /// Whether the costs show on the right.
    costs_shown: bool,
    /// Whether the list of shortcuts shows.
    keys_shown: bool,
    /// Whether the costs count the person's own sessions of Claude Code
    /// and Codex, beside iq's calls.
    everyone: bool,
    /// The messages posted since the window last took them.
    posts: Vec<Post>,
    /// Whether iq was asked to stop.
    quitting: bool,
    /// The limits the person set.
    limits: Limits,
    /// The steward.
    steward: Steward,
    /// How the limits stood when the steward last looked, and when.
    gauges: Vec<Gauge>,
    looked: u64,
    /// The payers last closed.
    closed: Vec<Payer>,
    /// What each agent spent lately: when, who, on whom, how much.
    spending: Vec<(u64, String, Payer, f64)>,
    /// The steward's exchanges with the agents, oldest first.
    letters: Vec<Post>,
    /// The assay.
    assay: AssayView,
    /// How the last sync went, when the machine syncs.
    synced: Option<Synced>,
}

impl Default for Board {
    fn default() -> Self {
        Self::new(Keymap::default(), Look::default())
    }
}

impl Board {
    /// The window as it opens: one pane, the menu and the costs hidden,
    /// with `keys` and `look` as the settings make them.
    #[must_use]
    pub fn new(keys: Keymap, look: Look) -> Self {
        Self {
            crew: CrewView::default(),
            layout: Layout::default(),
            keys,
            look,
            period: PERIODS[2],
            costs: Costs::default(),
            subscriptions: Subscriptions::default(),
            read: false,
            menu: false,
            costs_shown: false,
            keys_shown: false,
            everyone: false,
            posts: Vec::new(),
            quitting: false,
            limits: Limits::default(),
            steward: Steward::default(),
            gauges: Vec::new(),
            looked: 0,
            closed: Vec::new(),
            spending: Vec::new(),
            letters: Vec::new(),
            assay: AssayView::default(),
            synced: None,
        }
    }

    /// The window with `limits`, the steward writing to the agents when
    /// `steward` says so.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits, steward: bool) -> Self {
        self.limits = limits;
        self.steward = Steward::new(steward);
        self
    }

    /// The steward.
    #[must_use]
    pub fn steward(&self) -> &Steward {
        &self.steward
    }

    /// How the limits stand, as the steward last saw them.
    #[must_use]
    pub fn gauges(&self) -> &[Gauge] {
        &self.gauges
    }

    /// The steward's exchanges with the agents, oldest first.
    #[must_use]
    pub fn letters(&self) -> &[Post] {
        &self.letters
    }

    /// The steward looks at `now`, every few seconds: how the limits
    /// stand, which payers close, and to whom it writes. Returns what that
    /// asks of the crew.
    pub fn watch(&mut self, now: u64) -> Vec<Request> {
        if now < self.looked + STEWARD_EVERY {
            return Vec::new();
        }
        self.looked = now;
        self.gauges = steward::gauges(&self.costs, &self.subscriptions, &self.limits, now);
        let mut asked = Vec::new();
        let closed = steward::closed(&self.gauges);
        if closed != self.closed {
            asked.push(Request::Closed(
                closed
                    .iter()
                    .map(|payer| match payer {
                        Payer::Provider => PROVIDER.to_owned(),
                        Payer::Subscription(agent) => agent.prefix().to_owned(),
                    })
                    .collect(),
            ));
            self.closed = closed;
        }
        self.spending
            .retain(|(at, ..)| now.saturating_sub(*at) <= steward::KEPT);
        let mut paces: Vec<Pace> = Vec::new();
        for (_, agent, payer, amount) in &self.spending {
            let working = self.crew.agent(agent).is_some_and(|a| a.working);
            if !working {
                continue;
            }
            match paces
                .iter_mut()
                .find(|p| p.agent == *agent && p.payer == *payer)
            {
                Some(pace) => pace.recent += amount,
                None => paces.push(Pace {
                    agent: agent.clone(),
                    payer: *payer,
                    recent: *amount,
                }),
            }
        }
        for advice in self.steward.advise(&self.gauges, &paces, now) {
            asked.push(Request::Advise {
                to: advice.to,
                text: advice.text,
            });
        }
        asked
    }

    /// Who pays for `agent`'s calls: its subscription when it is an agent
    /// of its own, else the provider.
    fn payer_of(&self, agent: &str) -> Payer {
        self.crew
            .agent(agent)
            .and_then(|view| ModelId::new(&view.model).ok())
            .and_then(|model| model.delegate().map(|(agent, _)| agent))
            .map_or(Payer::Provider, Payer::Subscription)
    }

    /// Whether iq was asked to stop, for the window to close.
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

    /// Whether the costs count the person's own sessions of Claude Code
    /// and Codex, beside iq's calls.
    #[must_use]
    pub fn everyone(&self) -> bool {
        self.everyone
    }

    /// Counts the person's own sessions in the costs, or only iq's calls.
    pub fn count_everyone(&mut self, everyone: bool) {
        self.everyone = everyone;
    }

    /// The panes.
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The panes, for what the person does with them: their focus, their
    /// lines dragged, what they write.
    pub fn layout_mut(&mut self) -> &mut Layout {
        &mut self.layout
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

    /// Whether the menu shows at the top.
    #[must_use]
    pub fn menu(&self) -> bool {
        self.menu
    }

    /// Whether the costs show on the right.
    #[must_use]
    pub fn costs_shown(&self) -> bool {
        self.costs_shown
    }

    /// Whether the list of shortcuts shows.
    #[must_use]
    pub fn keys_shown(&self) -> bool {
        self.keys_shown
    }

    /// Does what `action` says. Returns what it asks of the crew, if it
    /// asks something; quitting is the window's own.
    pub fn act(&mut self, action: Action) -> Option<Request> {
        let focus = self.layout.focus();
        match action {
            Action::Split => self.layout.next_split(),
            Action::Steward => self.steward.set_on(!self.steward.on()),
            Action::NextPane => self.layout.next_focus(),
            Action::Zoom => self.layout.toggle_zoom(),
            Action::NewChat => self.layout.show(
                focus,
                Shows::New(NewChat {
                    model: self.crew.lead().to_owned(),
                    ..NewChat::default()
                }),
            ),
            Action::Costs => self.costs_shown = !self.costs_shown,
            Action::Period => {
                let at = PERIODS.iter().position(|p| *p == self.period).unwrap_or(0);
                self.period = PERIODS[(at + 1) % PERIODS.len()];
            }
            Action::Assay => self.layout.show(focus, Shows::Assay),
            Action::Theme => self.look.next_theme(),
            Action::Opacity => self.look.next_opacity(),
            Action::Menu => self.menu = !self.menu,
            Action::Keys => self.keys_shown = !self.keys_shown,
            Action::Autopilot => return Some(self.crew.toggle_autopilot()),
            Action::Internet => return Some(self.crew.toggle_internet()),
            Action::Allow | Action::Refuse => {
                let id = self.questions_in(focus).first().map(|q| q.id)?;
                return Some(self.crew.answer(id, action == Action::Allow));
            }
            Action::Stop => {
                let agent = self.agent_in(focus)?.to_owned();
                return self.crew.stop(&agent);
            }
            Action::Quit => {}
        }
        None
    }

    /// The agent the pane at `pane` shows, if it shows one.
    #[must_use]
    pub fn agent_in(&self, pane: usize) -> Option<&str> {
        match &self.layout.pane(pane)?.shows {
            Shows::Chat(agent) => Some(agent),
            _ => None,
        }
    }

    /// The questions the pane at `pane` puts to the person: its agent's,
    /// and, in the pane with the focus, those of agents no pane shows.
    #[must_use]
    pub fn questions_in(&self, pane: usize) -> Vec<&Question> {
        let mine = self.agent_in(pane);
        let shown: Vec<&str> = self
            .layout
            .chats_shown()
            .into_iter()
            .map(|(_, a)| a)
            .collect();
        let focused = pane == self.layout.focus();
        self.crew
            .questions()
            .iter()
            .filter(|q| {
                Some(q.agent.as_str()) == mine || (focused && !shown.contains(&q.agent.as_str()))
            })
            .collect()
    }

    /// Answers question `id`.
    pub fn answer(&mut self, id: u64, approved: bool) -> Request {
        self.crew.answer(id, approved)
    }

    /// Sends what the person wrote in the pane at `pane`: to its agent, or
    /// to start one on the pane's model.
    pub fn send(&mut self, pane: usize) -> Option<Request> {
        let text = self.layout.pane(pane)?.input.trim().to_owned();
        if text.is_empty() {
            return None;
        }
        let shows = self.layout.pane(pane)?.shows.clone();
        let request = match shows {
            Shows::Chat(agent) => Request::Tell { to: agent, text },
            Shows::New(chat) if chat.starting.is_none() && !chat.model.trim().is_empty() => {
                let role = Some(chat.role.trim().to_owned()).filter(|r| !r.is_empty());
                // A one-word role names the agent, else its model does.
                let named = match &role {
                    Some(role) if !role.contains(char::is_whitespace) => role.clone(),
                    _ => chat.model.clone(),
                };
                let name = self.name_for(&named);
                let model = chat.model.trim().to_owned();
                let partner = Some(chat.partner.trim().to_owned())
                    .filter(|p| !p.is_empty() && chat.pattern.takes_partner());
                self.layout.show(
                    pane,
                    Shows::New(NewChat {
                        model: model.clone(),
                        starting: Some(name.clone()),
                        ..chat.clone()
                    }),
                );
                Request::Start {
                    name,
                    model,
                    task: text,
                    role,
                    pattern: chat.pattern,
                    partner,
                }
            }
            _ => return None,
        };
        if let Some(pane) = self.layout.pane_mut(pane) {
            pane.input.clear();
        }
        Some(request)
    }

    /// A name for an agent on `model` that no agent has: the model's own
    /// name, without its provider, made unique as the crew makes them.
    fn name_for(&self, model: &str) -> String {
        let model = model.trim();
        let short = match model.split_once('/') {
            Some(("claude-code", rest)) => rest,
            Some(("codex", _)) => "codex",
            Some((_, rest)) => rest,
            None if model == "claude-code" => "claude",
            None => model,
        };
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
        let taken =
            |name: &str| self.crew.agent(name).is_some() || self.layout.starting().contains(&name);
        let mut name = base.clone();
        let mut n = 2;
        while taken(&name) {
            name = format!("{base}-{n}");
            n += 1;
        }
        name
    }

    /// How the last sync went, when the machine syncs.
    #[must_use]
    pub fn synced(&self) -> Option<&Synced> {
        self.synced.as_ref()
    }

    /// The assay.
    #[must_use]
    pub fn assay(&self) -> &AssayView {
        &self.assay
    }

    /// Opens the chat with the assay's judge, from the pane at `pane`: in
    /// another pane shown when there is one, so that the assay stays in
    /// sight, else in this one. Starts the judge when it is not there yet.
    pub fn talk_to_judge(&mut self, pane: usize) -> Option<Request> {
        let target = self
            .layout
            .shown()
            .into_iter()
            .find(|at| *at != pane)
            .unwrap_or(pane);
        self.layout.set_focus(target);
        if self.crew.agent(JUDGE).is_some() {
            self.layout.show(target, Shows::Chat(JUDGE.to_owned()));
            return None;
        }
        let model = self.assay.judge().unwrap_or_default().to_owned();
        self.layout.show(
            target,
            Shows::New(NewChat {
                model,
                starting: Some(JUDGE.to_owned()),
                ..NewChat::default()
            }),
        );
        Some(Request::Assay(AssayAsk::Talk))
    }

    /// Stops the assay's runs.
    #[must_use]
    pub fn stop_runs(&self) -> Option<Request> {
        (!self.assay.running().is_empty()).then_some(Request::Assay(AssayAsk::Stop))
    }

    /// The costs.
    #[must_use]
    pub fn costs(&self) -> &Costs {
        &self.costs
    }

    /// The subscriptions.
    #[must_use]
    pub fn subscriptions(&self) -> &Subscriptions {
        &self.subscriptions
    }

    /// Whether the agents' files were read once.
    #[must_use]
    pub fn was_read(&self) -> bool {
        self.read
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
            Update::Read(read) => {
                self.subscriptions.calls_read(read.calls);
                self.costs.subscriptions_read(read.spent);
                self.read = true;
            }
            Update::Crew(event) => {
                match &event {
                    // A subscription's call is read from its agent's files.
                    // A subscription's call counts in tokens toward its
                    // agent's pace; its cost is read from the agent's files.
                    Event::Spent {
                        agent,
                        usage,
                        billed: false,
                        ..
                    } => {
                        let payer = self.payer_of(agent);
                        let tokens = (usage.input.0 + usage.output.0) as f64;
                        self.spending
                            .push((crate::now(), agent.clone(), payer, tokens));
                    }
                    Event::Spent {
                        agent,
                        model,
                        cost,
                        usage,
                        billed: true,
                    } => {
                        if let Some(cost) = cost {
                            self.spending.push((
                                crate::now(),
                                agent.clone(),
                                Payer::Provider,
                                cost.0,
                            ));
                        }
                        self.costs.add(Spent {
                            at: crate::now(),
                            payer: Payer::Provider,
                            model: model.clone(),
                            cost: *cost,
                            tokens: usage.input.0 + usage.output.0,
                            by_iq: true,
                        });
                    }
                    Event::Quota(quota) => self.subscriptions.add(quota.clone()),
                    Event::Joined { agent, by, .. } => {
                        self.layout.joined(agent, by == PERSON);
                    }
                    Event::Posted { from, to, text } => {
                        if from == STEWARD || to == STEWARD {
                            self.letters.push(Post {
                                from: from.clone(),
                                to: to.clone(),
                                text: text.clone(),
                            });
                            if self.letters.len() > STEWARD_KEPT {
                                self.letters.remove(0);
                            }
                        }
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
            Update::Kept { provider, quotas } => {
                for spent in provider {
                    self.costs.add(spent);
                }
                self.subscriptions.loaded(quotas);
            }
            Update::Quit => self.quitting = true,
            Update::Assay(overview) => self.assay.set(*overview),
            Update::AssayRunning(running) => self.assay.set_running(running),
            Update::Tasks(tasks) => self.costs.tasks_read(tasks),
            Update::Synced(synced) => self.synced = Some(synced),
            Update::Started {
                lead,
                reviewer,
                autopilot,
                internet,
            } => {
                self.layout.new_chats_on(&lead);
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

    #[test]
    fn the_periods_go_round_and_are_those_offered() {
        let mut board = Board::default();
        assert_eq!(board.period(), 24 * 3600);
        board.act(Action::Period);
        assert_eq!(board.period(), 7 * 24 * 3600);
        board.act(Action::Period);
        assert_eq!(board.period(), 3600);
        board.sum_over(5);
        assert_eq!(board.period(), 3600);
    }

    #[test]
    fn a_pane_starts_its_agent_then_writes_to_it() {
        let mut board = Board::default();
        board.update(Update::Started {
            lead: "claude-code/opus".into(),
            reviewer: "r".into(),
            autopilot: false,
            internet: false,
        });
        let write = |board: &mut Board, pane: usize, text: &str| {
            board
                .layout_mut()
                .pane_mut(pane)
                .unwrap()
                .input
                .push_str(text);
        };
        write(&mut board, 0, "  fix the build ");
        assert_eq!(
            board.send(0),
            Some(Request::Start {
                name: "opus".into(),
                model: "claude-code/opus".into(),
                task: "fix the build".into(),
                role: None,
                pattern: iq_agent::Pattern::Solo,
                partner: None,
            })
        );
        // A second chat on the same model is named apart.
        board.act(Action::Split);
        write(&mut board, 1, "review it");
        assert!(matches!(board.send(1), Some(Request::Start { name, .. }) if name == "opus-2"));
        // A one-word role names the agent.
        board.act(Action::Split);
        if let Some(Pane {
            shows: Shows::New(chat),
            ..
        }) = board.layout_mut().pane_mut(2)
        {
            chat.role = "review".into();
        }
        write(&mut board, 2, "look at it");
        assert!(matches!(
            board.send(2),
            Some(Request::Start { name, role: Some(role), .. }) if name == "review" && role == "review"
        ));
        joined(&mut board, "opus", PERSON);
        assert_eq!(board.agent_in(0), Some("opus"));
        write(&mut board, 0, "and the tests");
        assert_eq!(
            board.send(0),
            Some(Request::Tell {
                to: "opus".into(),
                text: "and the tests".into()
            })
        );
        assert_eq!(board.send(0), None);
    }

    #[test]
    fn panes_stay_as_the_person_left_them_while_agents_talk() {
        let mut board = Board::default();
        board.layout_mut().show(0, Shows::Chat("lead".into()));
        joined(&mut board, "lead", PERSON);
        joined(&mut board, "helper", "lead");
        board.update(Update::Crew(Event::Posted {
            from: "lead".into(),
            to: "helper".into(),
            text: "read the tests".into(),
        }));
        board.update(Update::Crew(Event::Working {
            agent: "helper".into(),
            prompt: "go".into(),
        }));
        assert_eq!(board.layout().shown(), vec![0]);
        assert_eq!(board.layout().chats_shown(), vec![(0, "lead")]);
        // The message still flies, for the panes that show its ends.
        assert_eq!(board.take_posts().len(), 1);
        assert!(board.take_posts().is_empty());
    }

    #[test]
    fn keys_answer_and_stop_the_agent_of_the_pane_with_the_focus() {
        let mut board = Board::default();
        board.act(Action::Split);
        board.layout_mut().show(0, Shows::Chat("lead".into()));
        joined(&mut board, "lead", PERSON);
        joined(&mut board, "helper", "lead");
        // Started by the lead, the helper takes the free pane beside it.
        assert_eq!(board.agent_in(1), Some("helper"));
        board.update(Update::Crew(Event::Working {
            agent: "lead".into(),
            prompt: "go".into(),
        }));
        for (id, agent) in [(3, "helper"), (4, "elsewhere")] {
            board.update(Update::Crew(Event::Asked {
                id,
                agent: agent.into(),
                action: "run x".into(),
                reason: "it reaches the internet".into(),
                widens: true,
            }));
        }
        // An agent no pane shows asks in the pane with the focus.
        let ids = |board: &Board, pane| -> Vec<u64> {
            board.questions_in(pane).iter().map(|q| q.id).collect()
        };
        assert_eq!((ids(&board, 0), ids(&board, 1)), (vec![4], vec![3]));
        assert_eq!(
            board.act(Action::Allow),
            Some(Request::Answer {
                id: 4,
                approved: true
            })
        );
        board.act(Action::NextPane);
        assert_eq!(
            board.act(Action::Refuse),
            Some(Request::Answer {
                id: 3,
                approved: false
            })
        );
        assert_eq!(board.act(Action::Refuse), None);
        // The helper rests: nothing to stop. The lead works.
        assert_eq!(board.act(Action::Stop), None);
        board.act(Action::NextPane);
        assert_eq!(
            board.act(Action::Stop),
            Some(Request::Stop {
                agent: "lead".into()
            })
        );
        assert_eq!(board.act(Action::Autopilot), Some(Request::Autopilot(true)));
    }

    #[test]
    fn the_steward_closes_what_is_reached_and_keeps_its_letters() {
        let limits = Limits {
            provider_month: Some(iq_core::Usd(1.0)),
            ..Limits::default()
        };
        let mut board = Board::default().with_limits(limits, true);
        joined(&mut board, "cheap", PERSON);
        board.update(Update::Crew(Event::Working {
            agent: "cheap".into(),
            prompt: "go".into(),
        }));
        board.update(Update::Crew(Event::Spent {
            agent: "cheap".into(),
            model: "m".into(),
            usage: iq_core::Usage::default(),
            cost: Some(iq_core::Usd(1.5)),
            billed: true,
        }));
        let asked = board.watch(crate::now());
        assert!(asked.contains(&Request::Closed(vec![PROVIDER.to_owned()])));
        // The agent spending it is told.
        assert!(
            asked
                .iter()
                .any(|r| matches!(r, Request::Advise { to, .. } if to == "cheap"))
        );
        // Not again within a few seconds.
        assert!(board.watch(crate::now()).is_empty());
        board.update(Update::Crew(Event::Posted {
            from: "cheap".into(),
            to: STEWARD.into(),
            text: "a long file".into(),
        }));
        assert_eq!(board.letters().len(), 1);
        board.act(Action::Steward);
        assert!(!board.steward().on());
    }

    #[test]
    fn the_judge_is_talked_to_beside_the_assay() {
        let mut board = Board::default();
        board.update(Update::Assay(Box::new(crate::assay::Overview {
            judges: vec!["claude-code/opus".into()],
            ..crate::assay::Overview::default()
        })));
        assert!(board.assay().told());
        board.act(Action::Split);
        board.act(Action::Assay);
        assert_eq!(board.layout().pane(0).unwrap().shows, Shows::Assay);
        // Started in the other pane, so that the assay stays in sight.
        assert_eq!(board.talk_to_judge(0), Some(Request::Assay(AssayAsk::Talk)));
        assert_eq!(board.layout().focus(), 1);
        assert!(matches!(
            &board.layout().pane(1).unwrap().shows,
            Shows::New(chat) if chat.model == "claude-code/opus" && chat.starting.as_deref() == Some(JUDGE)
        ));
        joined(&mut board, JUDGE, PERSON);
        assert_eq!(board.agent_in(1), Some(JUDGE));
        // Once there, it is only shown.
        board.layout_mut().show(1, Shows::Costs);
        assert_eq!(board.talk_to_judge(0), None);
        assert_eq!(board.agent_in(1), Some(JUDGE));
        assert_eq!(board.stop_runs(), None);
    }

    #[test]
    fn calls_billed_to_the_provider_are_counted_as_they_come() {
        let mut board = Board::default();
        board.update(Update::Crew(Event::Spent {
            agent: "lead".into(),
            model: "openai/gpt-5-mini".into(),
            usage: iq_core::Usage::default(),
            cost: Some(iq_core::Usd(0.25)),
            billed: true,
        }));
        // A subscription's call is read from its agent's files instead.
        board.update(Update::Crew(Event::Spent {
            agent: "coder".into(),
            model: "gpt-5.5".into(),
            usage: iq_core::Usage::default(),
            cost: Some(iq_core::Usd(4.0)),
            billed: false,
        }));
        let totals = board.costs().totals(0, false);
        assert_eq!(totals.len(), 1);
        assert_eq!(totals[0].payer, Payer::Provider);
        assert_eq!(totals[0].cost, iq_core::Usd(0.25));
    }
}
