//! The person's choices, kept in `~/.iq/settings.toml`. Keys never go
//! there: they come from the environment only.
//!
//! ```toml
//! model = "anthropic/claude-sonnet-4-5"  # the lead: a model of the provider, or claude-code, codex
//! reviewer = "openai/gpt-5-mini"         # answers for the person when the autopilot is on
//! budget = 2.0                           # the most the provider is billed per session, in dollars
//! effort = "high"
//! autopilot = false
//! internet = false                       # true: every command reaches the internet
//! ```

use std::path::Path;

use anyhow::{Context, Result};
use iq_core::{Effort, ModelId, Usd};
use serde::Deserialize;

/// The lead's model when a key for the provider is there and the settings
/// name none.
const LEAD: &str = "anthropic/claude-sonnet-4-5";

/// The reviewer's, likewise: cheap, as it answers often and briefly.
const REVIEWER: &str = "openai/gpt-5-mini";

/// The lead's and the reviewer's without a key: Claude Code, paid by its
/// subscription.
const LEAD_WITHOUT_KEY: &str = "claude-code";
const REVIEWER_WITHOUT_KEY: &str = "claude-code/haiku";

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    model: Option<String>,
    reviewer: Option<String>,
    budget: Option<f64>,
    effort: Option<Effort>,
    #[serde(default)]
    pub(crate) autopilot: bool,
    /// Every command reaches the internet, rather than what a rule gives
    /// it to and what the person allows.
    #[serde(default)]
    pub(crate) internet: bool,
}

impl Settings {
    /// The settings in `folder`, the defaults when there are none.
    pub(crate) fn load(folder: &Path) -> Result<Self> {
        let file = folder.join("settings.toml");
        match std::fs::read_to_string(&file) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("in {}", file.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("reading {}", file.display())),
        }
    }

    /// The lead's model: as set, else by whether there is a key.
    pub(crate) fn lead(&self, keyed: bool) -> Result<ModelId> {
        let default = if keyed { LEAD } else { LEAD_WITHOUT_KEY };
        Ok(ModelId::new(self.model.as_deref().unwrap_or(default))?)
    }

    /// What the crew starts with.
    pub(crate) fn crew(&self, keyed: bool) -> Result<iq_agent::Settings> {
        let default = if keyed {
            REVIEWER
        } else {
            REVIEWER_WITHOUT_KEY
        };
        Ok(iq_agent::Settings {
            reviewer: ModelId::new(self.reviewer.as_deref().unwrap_or(default))?,
            effort: self.effort,
            budget: self.budget.map(Usd),
            ..iq_agent::Settings::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_settings_the_key_decides_the_models() {
        let settings = Settings::default();
        assert_eq!(settings.lead(true).unwrap().as_str(), LEAD);
        assert_eq!(settings.lead(false).unwrap().as_str(), "claude-code");
        assert_eq!(
            settings.crew(false).unwrap().reviewer.as_str(),
            "claude-code/haiku"
        );
        let set: Settings = toml::from_str(
            "model = \"codex\"\nbudget = 1.5\neffort = \"low\"\nautopilot = true\ninternet = true",
        )
        .unwrap();
        assert_eq!(set.lead(true).unwrap().as_str(), "codex");
        assert_eq!(set.crew(true).unwrap().budget, Some(Usd(1.5)));
        assert!(set.autopilot && set.internet);
        assert!(toml::from_str::<Settings>("modle = \"x\"").is_err());
    }
}
