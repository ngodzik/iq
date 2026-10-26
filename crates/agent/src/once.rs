//! One question to one model, outside any crew's turn: a model of the
//! provider answers without tools, an agent with tools of its own reads
//! only and keeps nothing of it. What it costs is reported as a crew's
//! calls are, so that it is counted with the rest.

use std::path::Path;

use iq_core::{
    ChatModel, ChatRequest, Delegate, DelegateEvent, DelegateRequest, Effort, Message, ModelId,
};

use crate::crew::{Report, chain};
use crate::event::Event;

/// What a question costs is reported under this agent's name, and in
/// `directory` an agent with tools of its own runs.
pub struct Asked<'a> {
    /// Who the cost is reported for, as the costs show it.
    pub agent: &'a str,
    /// Where an agent with tools of its own runs, reading only.
    pub directory: &'a Path,
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
pub async fn ask_once<M, D>(
    provider: &M,
    agents: &D,
    model: &ModelId,
    instructions: &str,
    prompt: String,
    asked: Asked<'_>,
) -> Result<String, String>
where
    M: ChatModel,
    D: Delegate,
{
    ask_once_by(provider, agents, model, instructions, prompt, asked)
        .await
        .map(|answer| answer.text)
}

/// An answer, and the model that gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// What it said.
    pub text: String,
    /// The model that answered, as precisely as its agent says:
    /// `claude-code/claude-opus-5-5` for `claude-code/opus`.
    pub model: String,
}

/// As [`ask_once`], with the model that answered.
///
/// # Errors
///
/// The model could not answer, and why.
pub async fn ask_once_by<M, D>(
    provider: &M,
    agents: &D,
    model: &ModelId,
    instructions: &str,
    prompt: String,
    asked: Asked<'_>,
) -> Result<Answer, String>
where
    M: ChatModel,
    D: Delegate,
{
    let shown = model.to_string();
    match model.delegate() {
        Some((agent, its_model)) => {
            let request = DelegateRequest {
                agent,
                read_only: true,
                effort: Some(asked.effort),
                model: its_model.to_owned(),
                prompt,
                instructions: instructions.to_owned(),
                resume: None,
                fork: false,
                ephemeral: true,
                directory: asked.directory.to_path_buf(),
                reach: None,
                network: false,
            };
            let costs = asked.costs.clone();
            let who = asked.agent.to_owned();
            let answered = std::sync::Arc::new(std::sync::Mutex::new(shown.clone()));
            let heard = std::sync::Arc::clone(&answered);
            let mut on_event = move |event: DelegateEvent| {
                if let DelegateEvent::Usage {
                    usage,
                    cost,
                    billed,
                    model,
                    ..
                } = event
                {
                    let model = if model.is_empty() {
                        shown.clone()
                    } else {
                        format!("{}/{model}", agent.prefix())
                    };
                    if let Ok(mut heard) = heard.lock() {
                        heard.clone_from(&model);
                    }
                    costs(Event::Spent {
                        agent: who.clone(),
                        model,
                        usage,
                        cost,
                        billed,
                    });
                }
            };
            let text = agents
                .run(&request, &mut on_event)
                .await
                .map(|reply| reply.text)
                .map_err(|e| chain(&e))?;
            let model = answered.lock().map(|m| m.clone()).unwrap_or_default();
            Ok(Answer { text, model })
        }
        None => {
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
                billed: true,
            });
            Ok(Answer {
                text: response.content.unwrap_or_default(),
                model: shown,
            })
        }
    }
}
