//! The person's choices, kept in `~/.iq/settings.toml`. Keys never go
//! there: they come from the environment only.
//!
//! ```toml
//! model = "anthropic/claude-sonnet-4-5"  # the lead: a model of the provider, or claude-code, codex
//! reviewer = "openai/gpt-5-mini"         # answers for the person when the autopilot is on
//! budget = 2.0                           # the most the provider is billed per session, in dollars
//! effort = "high"
//! autopilot = false
//! autopilot_internet = true              # the reviewer also answers what only reaches the internet
//! internet = false                       # true: every command reaches the internet
//! theme = "graphite"                     # or "ink", "porcelain"; Ctrl-T goes through them
//! opacity = 0.86                         # how much of the desktop shows through, 0.3 to 1
//!
//! steward = true                         # the steward writes to who spends too fast; Ctrl-G
//!
//! [keys]                                 # any shortcut moved, "" for none
//! costs = "ctrl+o"
//!
//! [limits]                               # what iq may use; what you spend outside iq counts too
//! requesty_month = 20.0                  # dollars the provider may bill in a month
//! claude_5h = 80                         # percent of Claude Code's five hours
//! claude_week = 70
//! codex_5h = 80
//! codex_week = 70
//! ```

use std::collections::BTreeMap;
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
    /// Whether the autopilot's reviewer also answers what only reaches the
    /// internet, rather than the person; true unless set.
    autopilot_internet: Option<bool>,
    /// Every command reaches the internet, rather than what a rule gives
    /// it to and what the person allows.
    #[serde(default)]
    pub(crate) internet: bool,
    /// The window's theme.
    theme: Option<String>,
    /// How opaque the window is.
    opacity: Option<f32>,
    /// The shortcuts moved: an action's name and its key.
    #[serde(default)]
    keys: BTreeMap<String, String>,
    /// Whether the steward writes to the agents that spend too fast.
    #[serde(default)]
    pub(crate) steward: bool,
    /// What iq may use of each payer.
    #[serde(default)]
    limits: LimitsFile,
}

/// The limits as the settings write them, each left to its default when
/// unset.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitsFile {
    requesty_month: Option<f64>,
    claude_5h: Option<f64>,
    claude_week: Option<f64>,
    codex_5h: Option<f64>,
    codex_week: Option<f64>,
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

    /// The window's look: its theme and opacity, as set.
    pub(crate) fn look(&self) -> Result<iq_ui::look::Look> {
        let theme = match &self.theme {
            Some(name) => name.parse()?,
            None => iq_ui::look::Theme::default(),
        };
        let opacity = self.opacity.unwrap_or(iq_ui::look::OPACITIES[0]);
        Ok(iq_ui::look::Look::new(theme, opacity))
    }

    /// Whether the autopilot's reviewer answers what only reaches the
    /// internet.
    pub(crate) fn autopilot_internet(&self) -> bool {
        self.autopilot_internet.unwrap_or(true)
    }

    /// The limits, the defaults changed as set.
    pub(crate) fn limits(&self) -> iq_ui::steward::Limits {
        use iq_core::Agent;
        let mut limits = iq_ui::steward::Limits {
            provider_month: self.limits.requesty_month.map(Usd),
            ..iq_ui::steward::Limits::default()
        };
        for (agent, short, week) in [
            (
                Agent::ClaudeCode,
                self.limits.claude_5h,
                self.limits.claude_week,
            ),
            (Agent::Codex, self.limits.codex_5h, self.limits.codex_week),
        ] {
            if let Some(share) = short {
                limits.short.insert(agent, share);
            }
            if let Some(share) = week {
                limits.week.insert(agent, share);
            }
        }
        limits
    }

    /// The window's shortcuts, the defaults moved as set.
    pub(crate) fn keymap(&self) -> Result<iq_ui::keys::Keymap> {
        Ok(iq_ui::keys::Keymap::with(&self.keys)?)
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
        let looks: Settings =
            toml::from_str("theme = \"Ink\"\nopacity = 0.6\n[keys]\ncosts = \"ctrl+u\"").unwrap();
        let look = looks.look().unwrap();
        assert_eq!((look.theme().name(), look.opacity()), ("ink", 0.6));
        assert!(
            looks
                .keymap()
                .unwrap()
                .chord(iq_ui::keys::Action::Costs)
                .is_some()
        );
        let wrong: Settings = toml::from_str("theme = \"neon\"").unwrap();
        assert!(wrong.look().is_err());
        let limited: Settings =
            toml::from_str("steward = true\n[limits]\nrequesty_month = 20.0\nclaude_5h = 50")
                .unwrap();
        let limits = limited.limits();
        assert!(limited.steward);
        assert_eq!(limits.provider_month, Some(Usd(20.0)));
        assert_eq!(limits.short[&iq_core::Agent::ClaudeCode], 50.0);
        assert_eq!(limits.week[&iq_core::Agent::Codex], 70.0);
        assert!(toml::from_str::<Settings>("[limits]\nclaude_6h = 1").is_err());
    }
}
