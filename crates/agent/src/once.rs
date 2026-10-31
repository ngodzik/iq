//! One question to one model of the provider, outside any crew's turn,
//! answered without tools. What it costs is reported as a crew's calls
//! are, so that it is counted with the rest.

use iq_core::{ChatModel, ChatRequest, Effort, Message, ModelId};

use crate::crew::{Report, chain};
use crate::event::Event;

/// What a question costs is reported under this agent's name.
pub struct Asked<'a> {
    /// Who the cost is reported for, as the costs show it.
    pub agent: &'a str,
    /// How hard the model thinks.
    pub effort: Effort,
    /// Where its cost goes.
    pub costs: &'a Report,
}

/// Asks `model` `prompt`, with `instructions` standing, and gives its
/// answer.
///
/// # Errors
///
/// The model could not answer, and why.
pub async fn ask_once<M>(
    provider: &M,
    model: &ModelId,
    instructions: &str,
    prompt: String,
    asked: Asked<'_>,
) -> Result<String, String>
where
    M: ChatModel,
{
    let shown = model.to_string();
    let request = ChatRequest {
        model: model.clone(),
        messages: vec![Message::system(instructions), Message::user(prompt)],
        tools: Vec::new(),
        effort: Some(asked.effort),
    };
    let response = provider.complete(&request).await.map_err(|e| chain(&e))?;
    let cost = match response.cost {
        Some(cost) => Some(cost),
        None => provider
            .pricing(model)
            .await
            .map(|pricing| pricing.cost(&response.usage)),
    };
    (asked.costs)(Event::Spent {
        agent: asked.agent.to_owned(),
        model: shown.clone(),
        usage: response.usage,
        cost,
    });
    Ok(response.content.unwrap_or_default())
}
