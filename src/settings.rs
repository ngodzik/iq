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
//!
//! [assay]                                # agents tried on the same exercises; Ctrl-L
//! judges = ["claude-code/opus", "codex"] # the first talks with you, all grade; subscriptions first
//! candidates = ["models"]                # "models", "harnesses", "teams", or names; all when unset
//! folder = "~/.local/share/iq/assay"     # its exercises and results, plain files
//! parallel = 2                           # runs at once
//!
//! [sync]                                 # iq's data kept in a private repository of yours, any name
//! remote = "https://github.com/someone/any-name.git"
//! branch = "main"                        # another when main must go through pull requests
//! keep = ["~/notes", "~/.zshrc"]         # files and folders of the home kept too, put back on a new machine
//!
//! [mcp.notion]                           # an MCP server, by its name: any server, not only this one
//! url = "https://mcp.notion.com/mcp"     # over HTTP; or command = "npx", args = [...]
//! headers = { Authorization = "NOTION_AUTHORIZATION" }  # each value read from that variable
//! env = ["NOTION_TOKEN"]                 # for a command: variables passed from iq's environment
//! mirror = "a page named Assay"          # the assay's judge keeps a copy of its results there
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use iq_core::{Agent, Effort, ModelId, Usd};
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
    /// The assay.
    #[serde(default)]
    assay: AssayFile,
    /// MCP servers, by name.
    #[serde(default)]
    mcp: BTreeMap<String, McpFile>,
    /// Where iq's data is synced.
    #[serde(default)]
    sync: SyncFile,
}

/// The sync as the settings write it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SyncFile {
    /// The repository, a private one of the person's.
    remote: Option<String>,
    /// Its branch, when not the default.
    branch: Option<String>,
    /// The files and folders of the home kept too, each `~/…`.
    #[serde(default)]
    keep: Vec<String>,
}

/// An MCP server as written: a command or an address, and what of iq's
/// environment it gets. Values never go in the settings: only the names
/// of the variables that hold them.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpFile {
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    url: Option<String>,
    /// For a command: the variables passed from iq's environment.
    #[serde(default)]
    env: Vec<String>,
    /// For an address: each header, and the variable its value is read
    /// from.
    #[serde(default)]
    headers: BTreeMap<String, String>,
    /// Where in it the assay's judge keeps a copy of the assay.
    mirror: Option<String>,
}

/// The assay's settings as written.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssayFile {
    judges: Option<Vec<String>>,
    candidates: Option<Vec<String>>,
    folder: Option<String>,
    parallel: Option<usize>,
}

/// iq's folder of data: the assay, the projects' memory, a copy of the
/// settings and what the person keeps, synced when the machine names a
/// repository. Out of `~/.iq`, which agents' tools cannot reach.
pub(crate) fn data_folder(home: &Path) -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|data| data.is_absolute())
        .unwrap_or_else(|| home.join(".local/share"))
        .join("iq")
}

/// How many runs of the assay go at once unless the settings say.
const PARALLEL: usize = 2;

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

    /// The assay's folder: as set, a leading `~` the home, else iq's data
    /// folder, out of `~/.iq`, where the agents' tools cannot reach.
    pub(crate) fn assay_folder(&self, home: &Path) -> PathBuf {
        match &self.assay.folder {
            Some(folder) => match folder.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(folder),
            },
            None => data_folder(home).join("assay"),
        }
    }

    /// The assay's judges: as set, else those a subscription pays for,
    /// as they cost less, the strongest of each, else the lead.
    pub(crate) fn assay_judges(&self, installed: &[Agent], lead: &ModelId) -> Result<Vec<ModelId>> {
        if let Some(judges) = self.assay.judges.as_ref().filter(|j| !j.is_empty()) {
            return judges
                .iter()
                .map(|j| Ok(ModelId::new(j.as_str())?))
                .collect();
        }
        let mut judges: Vec<ModelId> = installed
            .iter()
            .map(|agent| match agent {
                Agent::ClaudeCode => ModelId::new("claude-code/opus"),
                Agent::Codex => Ok(ModelId::agent(Agent::Codex)),
            })
            .collect::<Result<_, _>>()?;
        if judges.is_empty() {
            judges.push(lead.clone());
        }
        Ok(judges)
    }

    /// Which candidates the assay may test.
    pub(crate) fn assay_allowed(&self) -> iq_assay::Allowed {
        iq_assay::Allowed::from_words(self.assay.candidates.as_deref().unwrap_or_default())
    }

    /// The machine's sync of iq's folder of data `data`, with the settings
    /// and rules of iq's `folder` mirrored in it and the files of `home`
    /// they list kept; none without a repository.
    pub(crate) fn sync(&self, folder: &Path, data: &Path, home: &Path) -> Option<iq_sync::Sync> {
        let remote = self.sync.remote.as_deref()?.trim();
        if remote.is_empty() {
            return None;
        }
        let mut sync = iq_sync::Sync::new(data, remote, crate::sync::machine())
            .mirror(folder.join("settings.toml"), "config/settings.toml")
            .mirror(folder.join("rules.toml"), "config/rules.toml")
            .keeping(home, self.sync.keep.clone());
        if let Some(branch) = self.sync.branch.as_deref().filter(|b| !b.trim().is_empty()) {
            sync = sync.on_branch(branch.trim());
        }
        Some(sync)
    }

    /// How many of the assay's runs go at once.
    pub(crate) fn assay_parallel(&self) -> usize {
        self.assay.parallel.unwrap_or(PARALLEL).clamp(1, 8)
    }

    /// The servers the assay's judge keeps a copy in, with where in each,
    /// their variables read through `var`; and what is wrong with those
    /// that cannot be handed, for the person to see.
    pub(crate) fn mirrors(
        &self,
        var: impl Fn(&str) -> Option<String>,
    ) -> (Vec<(iq_core::McpServer, String)>, Vec<String>) {
        let mut mirrors = Vec::new();
        let mut wrong = Vec::new();
        for (name, server) in &self.mcp {
            let Some(place) = server.mirror.as_ref().filter(|m| !m.trim().is_empty()) else {
                continue;
            };
            let mut missing = Vec::new();
            let mut read = |variable: &str| -> String {
                var(variable).unwrap_or_else(|| {
                    missing.push(variable.to_owned());
                    String::new()
                })
            };
            let reach = match (&server.command, &server.url) {
                (Some(program), None) => iq_core::McpReach::Command {
                    program: program.clone(),
                    args: server.args.clone(),
                    env: server
                        .env
                        .iter()
                        .map(|variable| (variable.clone(), read(variable)))
                        .collect(),
                },
                (None, Some(url)) => iq_core::McpReach::Http {
                    url: url.clone(),
                    headers: server
                        .headers
                        .iter()
                        .map(|(header, variable)| (header.clone(), read(variable)))
                        .collect(),
                },
                _ => {
                    wrong.push(format!(
                        "[mcp.{name}] needs a command or a url, one of them"
                    ));
                    continue;
                }
            };
            if !missing.is_empty() {
                wrong.push(format!(
                    "[mcp.{name}]: iq's environment does not set {}",
                    missing.join(", ")
                ));
                continue;
            }
            mirrors.push((
                iq_core::McpServer {
                    name: name.clone(),
                    reach,
                },
                place.clone(),
            ));
        }
        (mirrors, wrong)
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

    #[test]
    fn the_assay_judges_with_subscriptions_first() {
        let lead = ModelId::new("anthropic/claude-sonnet-4-5").unwrap();
        let unset = Settings::default();
        let judges = |s: &Settings, installed: &[Agent]| -> Vec<String> {
            s.assay_judges(installed, &lead)
                .unwrap()
                .iter()
                .map(ToString::to_string)
                .collect()
        };
        assert_eq!(
            judges(&unset, &[Agent::ClaudeCode, Agent::Codex]),
            ["claude-code/opus", "codex"]
        );
        assert_eq!(judges(&unset, &[]), ["anthropic/claude-sonnet-4-5"]);
        let set: Settings = toml::from_str(
            "[assay]\njudges = [\"codex\"]\ncandidates = [\"models\"]\nfolder = \"~/lab\"\nparallel = 40",
        )
        .unwrap();
        assert_eq!(judges(&set, &[Agent::ClaudeCode]), ["codex"]);
        assert_eq!(set.assay_folder(Path::new("/h")), PathBuf::from("/h/lab"));
        assert_eq!(set.assay_parallel(), 8);
        assert!(set.assay_allowed().allows(iq_assay::Kind::Model, "a/b"));
        assert!(!set.assay_allowed().allows(iq_assay::Kind::Harness, "codex"));
        assert!(toml::from_str::<Settings>("[assay]\njudge = \"x\"").is_err());
    }

    #[test]
    fn the_sync_is_set_in_the_settings_only() {
        let dir = tempfile::tempdir().unwrap();
        let none = Settings::default();
        assert!(none.sync(dir.path(), dir.path(), dir.path()).is_none());
        let set: Settings = toml::from_str(
            "[sync]\nremote = \"https://github.com/someone/any-name.git\"\nbranch = \"iq\"\nkeep = [\"~/notes\", \"~/.ssh\"]",
        )
        .unwrap();
        let sync = set
            .sync(dir.path(), &dir.path().join("data"), dir.path())
            .unwrap();
        assert_eq!(sync.remote(), "https://github.com/someone/any-name.git");
        assert_eq!(sync.kept(), ["~/notes"]);
        assert_eq!(sync.refused().len(), 1);
        assert!(toml::from_str::<Settings>("[sync]\nremot = \"x\"").is_err());
    }

    #[test]
    fn a_server_is_handed_with_values_read_from_the_environment_only() {
        let set: Settings = toml::from_str(
            r#"
[mcp.notes]
url = "https://notes.example/mcp"
headers = { Authorization = "NOTES_AUTH" }
mirror = "a page named Assay"

[mcp.local]
command = "notes-server"
env = ["NOTES_TOKEN"]
mirror = "the assay notebook"

[mcp.other]
command = "x"
"#,
        )
        .unwrap();
        let var = |name: &str| (name == "NOTES_AUTH").then(|| "Bearer secret".to_owned());
        let (mirrors, wrong) = set.mirrors(var);
        assert_eq!(mirrors.len(), 1);
        assert_eq!(mirrors[0].0.name, "notes");
        assert_eq!(mirrors[0].1, "a page named Assay");
        assert!(
            matches!(&mirrors[0].0.reach, iq_core::McpReach::Http { headers, .. } if headers[0].1 == "Bearer secret")
        );
        assert_eq!(
            wrong,
            ["[mcp.local]: iq's environment does not set NOTES_TOKEN"]
        );
        assert!(toml::from_str::<Settings>("[mcp.x]\ntoken = \"t\"").is_err());
    }
}
