//! Tools a part of iq offers some of the crew's agents beside the crew's
//! own, such as the memory's: which agents get them, what they
//! are told of them, and what a call does. A call acts through the crew it
//! is given, so through the gate, and asks the person through it.

use std::future::Future;
use std::pin::Pin;

use iq_core::ToolSpec;
use serde_json::Value;

use crate::crew::Crew;

/// What a call of an extension's tool comes to: its text for the model,
/// or why it failed.
pub type Called = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;

/// Tools offered to some agents of a crew. Most agents get none of them:
/// an extension knows the agents it serves by their names.
pub trait Extension<M>: Send + Sync {
    /// The tools `agent` gets, none for most.
    fn tools(&self, agent: &str) -> Vec<ToolSpec>;

    /// Whether only the agents the person started get its tools and are
    /// told of them, as a lead is: those agents give the ones they start
    /// what their part needs.
    fn leads_only(&self) -> bool {
        false
    }

    /// What `agent` is told of them, added to its standing instructions.
    fn instructions(&self, agent: &str) -> Option<String> {
        let _ = agent;
        None
    }

    /// Local policy for the permission reviewer, read again for each decision.
    /// An error leaves the decision to the person. This cannot widen the gate.
    fn review_instructions(&self) -> Result<Option<String>, String> {
        Ok(None)
    }

    /// Runs the tool `name` for `agent` with `args`, a JSON object. The
    /// crew is the one the agent works in: what the call does on the
    /// machine, it asks of the crew's gate, and what it must ask the
    /// person, it asks through the crew.
    fn call(&self, crew: Crew<M>, agent: String, name: String, args: Value) -> Called;
}
