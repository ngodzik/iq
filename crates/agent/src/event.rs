//! What the crew reports as it works, for the window or the terminal to
//! show.

use iq_core::{Effort, Usage, Usd};

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
    /// Something the person should know about how an agent works.
    Notice {
        /// The agent.
        agent: String,
        /// What.
        text: String,
    },
}
