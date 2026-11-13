//! The map of a chat's turn: its own agent at the root, the agents it
//! started under it as a tree, theirs under them, each followed by the
//! jobs it runs as leaves. Those retired show a moment as they finish,
//! then fold into a count at its foot, unless the person unfolds them.

use crate::crew::{AgentView, CrewView, JobView};

/// How long an agent that finished stays on the map before it folds into
/// the count of those finished, in seconds.
pub const LINGER: u64 = 20;

/// What a row of the map is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind<'a> {
    /// An agent, at work, resting or finished.
    Agent(&'a AgentView),
    /// A command one runs in the background.
    Job(&'a JobView),
}

/// One row of the map.
#[derive(Debug, Clone, PartialEq)]
pub struct Row<'a> {
    /// What it is.
    pub kind: Kind<'a>,
    /// For each of its ancestors below the root, then for itself, whether
    /// it is the last under its parent: what the tree's lines are drawn
    /// from. Empty for the root.
    pub lasts: Vec<bool>,
}

impl Row<'_> {
    /// How deep it stands: 0 for the chat's own agent.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.lasts.len()
    }
}

/// The map of `chat`'s turn at `now`: its agents, those finished within
/// [`LINGER`] seconds, or all of them when `finished` is unfolded, and
/// their jobs.
#[must_use]
pub fn rows<'a>(crew: &'a CrewView, chat: &str, finished: bool, now: u64) -> Vec<Row<'a>> {
    let shown = |agent: &&AgentView| {
        !agent.retired
            || finished
            || agent
                .worked
                .is_some_and(|at| now.saturating_sub(at) < LINGER)
    };
    let all: Vec<&AgentView> = crew
        .agents()
        .iter()
        .chain(crew.finished())
        .filter(shown)
        .collect();
    let mut rows = Vec::new();
    if let Some(root) = crew.agent(chat) {
        rows.push(Row {
            kind: Kind::Agent(root),
            lasts: Vec::new(),
        });
        under(crew, &all, root, &[], &mut rows);
    }
    rows
}

/// The rows under `parent`, whose own row's `lasts` are `above`.
fn under<'a>(
    crew: &'a CrewView,
    all: &[&'a AgentView],
    parent: &'a AgentView,
    above: &[bool],
    rows: &mut Vec<Row<'a>>,
) {
    // A retired agent's name may be another's now: none is under it.
    let children: Vec<Kind<'a>> = all
        .iter()
        .filter(|agent| !parent.retired && agent.by == parent.name)
        .map(|agent| Kind::Agent(agent))
        .chain(
            crew.jobs()
                .iter()
                .filter(|job| !parent.retired && job.agent == parent.name)
                .map(Kind::Job),
        )
        .collect();
    let count = children.len();
    for (at, kind) in children.into_iter().enumerate() {
        let mut lasts = above.to_vec();
        lasts.push(at + 1 == count);
        rows.push(Row {
            kind,
            lasts: lasts.clone(),
        });
        if let Kind::Agent(agent) = kind
            && rows.len() < 500
        {
            under(crew, all, agent, &lasts, rows);
        }
    }
}

/// The agents `chat` started that its turn under way concerns, and how
/// many of them finished in it: those at work, those that worked since
/// the turn began, those just started that wait for their first message.
/// None once nothing of the chat works: what earlier turns started is the
/// whole map's to show.
#[must_use]
pub fn of_turn<'a>(crew: &'a CrewView, chat: &str) -> (Vec<&'a AgentView>, usize) {
    let ours = |agent: &AgentView| crew.chat_of(&agent.name).as_deref() == Some(chat);
    let under_way = crew
        .agents()
        .iter()
        .any(|agent| agent.working && (agent.name == chat || ours(agent)));
    if !under_way {
        return (Vec::new(), 0);
    }
    let since = crew.turn(chat).map_or(0, |turn| turn.since);
    let agents = crew
        .agents()
        .iter()
        .filter(|agent| agent.name != chat && ours(agent))
        .filter(|agent| agent.working || agent.worked.is_none_or(|at| at >= since))
        .collect();
    let finished = crew
        .finished()
        .iter()
        .filter(|agent| agent.name.split('/').next() == Some(chat))
        .filter(|agent| agent.worked.is_some_and(|at| at >= since))
        .count();
    (agents, finished)
}

/// How many of `chat`'s agents finished, retired: the count at the map's
/// foot.
#[must_use]
pub fn finished(crew: &CrewView, chat: &str) -> usize {
    crew.finished()
        .iter()
        .filter(|agent| agent.name.split('/').next() == Some(chat))
        .count()
}
