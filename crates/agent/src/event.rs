//! What the crew reports as it works, for the window or the terminal to
//! show.

use iq_core::{CacheUse, Effort, Usage, Usd};

/// Who the person is on the board: they write to agents and agents to
/// them under this name.
pub const PERSON: &str = "person";

/// Something the crew did, reported as it happens.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// An agent joined the crew.
    Joined {
        /// Its name on the board.
        agent: String,
        /// The model of the provider it runs on.
        model: String,
        /// Who started it: the person or another agent.
        by: String,
    },
    /// Where an agent works, when it is not the crew's workspace: a place
    /// of its own, as a worktree of the project is, or the place of the
    /// agent that started it.
    Placed {
        /// The agent.
        agent: String,
        /// Its folder.
        folder: std::path::PathBuf,
        /// What the person is told of it, when the place was made for it.
        said: Option<String>,
    },
    /// An agent was given another name, by the person, after what its
    /// work is about, or after its chat's: what it did and what was
    /// written to it goes on under that one. One at work shows it at once,
    /// and its later events name it so.
    Renamed {
        /// Its name until now.
        from: String,
        /// Its name from now on.
        to: String,
    },
    /// The person gave an agent another model, which answers it from its
    /// next turn.
    Model {
        /// The agent.
        agent: String,
        /// Its model from now on.
        model: String,
    },
    /// How hard an agent thinks, as it joined or as it was changed.
    Effort {
        /// The agent.
        agent: String,
        /// Its effort; `None` leaves it to its model.
        effort: Option<Effort>,
    },
    /// An agent was given work: its task, a message that woke it.
    Working {
        /// The agent.
        agent: String,
        /// What it was given.
        prompt: String,
    },
    /// A new block of an agent's text begins.
    TextStart {
        /// The agent.
        agent: String,
    },
    /// More of an agent's current block of text.
    Text {
        /// The agent.
        agent: String,
        /// The text.
        text: String,
    },
    /// One call to a model ended.
    Spent {
        /// The agent it was made for, or `reviewer`.
        agent: String,
        /// The model.
        model: String,
        /// Its tokens.
        usage: Usage,
        /// What it cost; `None` when the price is unknown.
        cost: Option<Usd>,
        /// What it read from the provider's cache and wrote to it, when
        /// told.
        cache: Option<CacheUse>,
        /// The chat whose turn it served, and is counted in: its agents',
        /// the autopilot's for one of them, the summaries of their
        /// messages; none for what serves no chat.
        chat: Option<String>,
    },
    /// A chat's turn begins: the person wrote to it while none of its
    /// agents worked. What it spends is counted from here.
    Turn {
        /// The chat.
        chat: String,
        /// Its number, from 1, in this session.
        number: u64,
    },
    /// A tool call ended.
    Tool {
        /// The agent.
        agent: String,
        /// The tool.
        name: String,
        /// What it was called with, in short.
        input: String,
        /// What it returned, or why it failed.
        output: String,
        /// Whether it succeeded.
        ok: bool,
    },
    /// An action waits for the person.
    Asked {
        /// The question's number, to answer it with.
        id: u64,
        /// The agent that asks.
        agent: String,
        /// The action, in words.
        action: String,
        /// Why the gate asks.
        reason: String,
        /// Whether it gives more than the sandbox holds: the network, a
        /// file outside the workspace, where code runs later.
        widens: bool,
    },
    /// The person answered, or nobody did in time.
    Answered {
        /// The question.
        id: u64,
        /// Whether it was approved.
        approved: bool,
    },
    /// The reviewer answered for the person, the autopilot being on.
    Reviewed {
        /// The agent that asked.
        agent: String,
        /// The action, in words.
        action: String,
        /// Why the gate asked.
        reason: String,
        /// Whether it was approved.
        approved: bool,
        /// The reviewer's reason.
        why: String,
    },
    /// A message was posted on the board.
    Posted {
        /// Its writer.
        from: String,
        /// Its reader: an agent, `all`, or the person.
        to: String,
        /// The message.
        text: String,
    },
    /// An agent finished its work, or failed at it.
    Done {
        /// The agent.
        agent: String,
        /// Its answer, or why it failed.
        outcome: Result<String, String>,
    },
    /// An agent asks the person to choose among `choices`, or to answer in
    /// their own words.
    Choose {
        /// The question's number, to answer it with.
        id: u64,
        /// The agent that asks.
        agent: String,
        /// The question.
        question: String,
        /// What it offers, two to four.
        choices: Vec<String>,
    },
    /// The person answered a question with choices, or nobody did.
    Chosen {
        /// The question.
        id: u64,
        /// Their answer: a choice, or their own words; none when they gave
        /// none.
        answer: Option<String>,
    },
    /// A command an agent left to run in the background began.
    JobStarted {
        /// The agent.
        agent: String,
        /// Its number, as the agent names it.
        id: u64,
        /// The command line.
        line: String,
    },
    /// The latest lines a command run in the background wrote, as it
    /// runs: said at most twice a second, when they changed.
    JobOutput {
        /// The agent.
        agent: String,
        /// Its number.
        id: u64,
        /// Its last lines, the latest last, each cut, its secrets taken
        /// out.
        tail: Vec<String>,
    },
    /// A command run in the background ended, or was stopped.
    JobEnded {
        /// The agent.
        agent: String,
        /// Its number.
        id: u64,
        /// How it ended, in words: its status, or that it was stopped.
        said: String,
        /// The end of what it wrote, its secrets taken out.
        output: String,
    },
    /// An agent waits for the agents it started to write: none of them
    /// has yet, and they work.
    Waiting {
        /// The agent that waits.
        agent: String,
        /// Those it waits for, at work.
        on: Vec<String>,
    },
    /// An agent another started is done with its task: its answer reached
    /// who started it, and nothing more is asked of it. It is off the
    /// board, its name free again for the next.
    Retired {
        /// The agent.
        agent: String,
    },
    /// A call to the provider failed, and an agent's turn with it: what
    /// the provider said, and where the agent stood.
    Failed {
        /// The agent.
        agent: String,
        /// The model called.
        model: String,
        /// The status the provider answered with, none when it was not
        /// reached.
        status: Option<u16>,
        /// What it said, or why it could not be reached.
        message: String,
        /// About how many tokens the call held.
        context: u64,
        /// The calls the agent made before it.
        calls: u64,
        /// What those cost, as far as their prices are known.
        spent: Usd,
    },
    /// Something the person should know about how an agent works.
    Notice {
        /// The agent.
        agent: String,
        /// What.
        text: String,
    },
}

impl Event {
    /// Writes `to` wherever the event names the agent `from`: an agent the
    /// person named again while it worked is shown under that name at
    /// once, though it takes it only as its turn ends.
    pub(crate) fn rename(&mut self, from: &str, to: &str) {
        let renamed = |name: &mut String| {
            if name == from {
                to.clone_into(name);
            }
        };
        match self {
            Event::Joined { agent, by, .. } => {
                renamed(agent);
                renamed(by);
            }
            Event::Renamed { .. }
            | Event::Answered { .. }
            | Event::Chosen { .. }
            | Event::Turn { .. } => {}
            Event::Choose { agent, .. }
            | Event::JobStarted { agent, .. }
            | Event::JobOutput { agent, .. }
            | Event::JobEnded { agent, .. } => renamed(agent),
            Event::Posted {
                from: writer,
                to: reader,
                ..
            } => {
                renamed(writer);
                renamed(reader);
            }
            Event::Spent { agent, chat, .. } => {
                renamed(agent);
                if let Some(chat) = chat {
                    renamed(chat);
                }
            }
            Event::Placed { agent, .. }
            | Event::Model { agent, .. }
            | Event::Effort { agent, .. }
            | Event::Working { agent, .. }
            | Event::TextStart { agent }
            | Event::Text { agent, .. }
            | Event::Tool { agent, .. }
            | Event::Asked { agent, .. }
            | Event::Reviewed { agent, .. }
            | Event::Done { agent, .. }
            | Event::Failed { agent, .. }
            | Event::Retired { agent }
            | Event::Notice { agent, .. } => renamed(agent),
            Event::Waiting { agent, on } => {
                renamed(agent);
                on.iter_mut().for_each(renamed);
            }
        }
    }
}
