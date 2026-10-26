//! The person's choices, kept in `~/.iq/settings.toml`. Keys never go
//! there: they come from the environment only.
//!
//! ```toml
//! base_url = "https://router.requesty.ai/v1"  # the provider: another region's router, any OpenAI compatible endpoint
//! model = "anthropic/claude-sonnet-4-5"  # the lead: a model of the provider, or claude-code, codex
//! reviewer = "openai/gpt-5-mini"         # answers for the person when the autopilot is on
//! budget = 2.0                           # the most the provider is billed per session, in dollars
//! effort = "high"
//! autopilot = false
//! autopilot_internet = true              # the reviewer also answers what only reaches the internet
//! internet = false                       # true: every command reaches the internet
//! theme = "graphite"                     # or "ink"; "porcelain" for a light terminal; /theme
//!
//! steward = true                         # the steward writes to who spends too fast; Ctrl-G
//!
//! [keys]                                 # any shortcut moved, "" for none; Ctrl-S lists them
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
//! grader = "claude-code/opus"            # grades every run, always the same so that grades compare
//! candidates = ["models"]                # "models", "harnesses", "teams", or names; all when unset
//! folder = "~/.local/share/iq/assay"     # its exercises and results, plain files
//! parallel = 2                           # runs at once
//!
//! [observe]                              # what your work goes to; iq's own, never synced
//! model = "openai/gpt-5-mini"            # who places what no rule places: a cheap one; claude-code/haiku without a key
//! every = 1                              # tasks placed at once: more pays less per task with Claude Code
//!
//! [[observe.category]]                   # your categories of work, any number
//! name = "Python"
//! description = "code in Python: the back end, the scripts"
//! files = ["*.py"]                       # rules iq applies itself before any model, all optional
//! commands = ["pytest"]
//! words = ["django"]
//!
//! [route]                                # the assay's results choose agents by kind of task; your choices
//! vetoed = { "acme/slow" = "holds requests ten minutes" }  # never routed, why
//! pinned = { design = "claude-code/opus" }                 # this kind, this agent, whatever the results
//! off = false                            # true: every chat on the model it starts on
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
    /// The provider's address, when not Requesty's router.
    base_url: Option<String>,
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
    /// The theme.
    theme: Option<String>,
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
    /// What the work goes to.
    #[serde(default)]
    observe: ObserveFile,
    /// The person's own choices of routing.
    #[serde(default)]
    route: RouteFile,
}

/// The routing as the settings write it: the person's own choices, beside
/// what the assay's records route.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteFile {
    /// No routing: every chat on the model it was started on.
    #[serde(default)]
    off: bool,
    /// Models never routed, each with why.
    #[serde(default)]
    vetoed: BTreeMap<String, String>,
    /// A model pinned to a kind of task.
    #[serde(default)]
    pinned: BTreeMap<String, String>,
}

/// The observations as the settings write them.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObserveFile {
    /// The model that places what no rule places.
    model: Option<String>,
    /// How many tasks wait before it is asked.
    every: Option<usize>,
    /// The person's categories of work.
    #[serde(default)]
    category: Vec<iq_observe::Category>,
}

/// How many tasks wait before a model of the provider places them: one,
/// as each call is small.
const OBSERVE_EVERY: usize = 1;

/// The same for an agent with tools of its own, each of whose calls
/// carries its own large context: a batch, to pay it once for several.
const OBSERVE_EVERY_AGENT: usize = 10;

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
    /// Who grades every run, always the same so that grades compare.
    grader: Option<String>,
    /// Kept from before there was one grader: the first is it.
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

    /// The provider's address, as set.
    pub(crate) fn base_url(&self) -> Option<String> {
        self.base_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_owned)
    }

    /// The lead's model: as set, else by whether there is a key.
    pub(crate) fn lead(&self, keyed: bool) -> Result<ModelId> {
        let default = if keyed { LEAD } else { LEAD_WITHOUT_KEY };
        Ok(ModelId::new(self.model.as_deref().unwrap_or(default))?)
    }

    /// The look: its theme, as set.
    pub(crate) fn look(&self) -> Result<iq_ui::look::Look> {
        let theme = match &self.theme {
            Some(name) => name.parse()?,
            None => iq_ui::look::Theme::default(),
        };
        Ok(iq_ui::look::Look::new(theme))
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

    /// The shortcuts, the defaults moved as set.
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

    /// Who grades the assay's runs: as set, else the strongest model a
    /// subscription pays for, as it costs less, else the lead. Always the
    /// same, so that grades compare.
    pub(crate) fn assay_grader(&self, installed: &[Agent], lead: &ModelId) -> Result<ModelId> {
        let named = self.assay.grader.clone().or_else(|| {
            self.assay
                .judges
                .as_ref()
                .and_then(|judges| judges.first().cloned())
        });
        if let Some(grader) = named.filter(|g| !g.trim().is_empty()) {
            return Ok(ModelId::new(grader.trim())?);
        }
        Ok(match installed.first() {
            Some(Agent::ClaudeCode) => ModelId::new("claude-code/opus")?,
            Some(Agent::Codex) => ModelId::agent(Agent::Codex),
            None => lead.clone(),
        })
    }

    /// Which candidates the assay may test.
    pub(crate) fn assay_allowed(&self) -> iq_assay::Allowed {
        iq_assay::Allowed::from_words(self.assay.candidates.as_deref().unwrap_or_default())
    }

    /// The person's own choices of routing.
    pub(crate) fn route_choices(&self) -> iq_assay::Choices {
        iq_assay::Choices {
            off: self.route.off,
            vetoed: self.route.vetoed.clone(),
            pinned: self.route.pinned.clone(),
        }
    }

    /// The person's categories of work.
    pub(crate) fn categories(&self) -> iq_observe::Categories {
        iq_observe::Categories::new(self.observe.category.clone())
    }

    /// What the observer is set with: the model that places what no rule
    /// places (as set, else the reviewer's cheap one), and how many tasks
    /// wait for it.
    pub(crate) fn observing(&self, keyed: bool) -> Result<iq_observe::Settings> {
        let model = match &self.observe.model {
            Some(model) => ModelId::new(model.as_str())?,
            None => ModelId::new(if keyed {
                REVIEWER
            } else {
                REVIEWER_WITHOUT_KEY
            })?,
        };
        let every = self.observe.every.unwrap_or(if model.delegate().is_some() {
            OBSERVE_EVERY_AGENT
        } else {
            OBSERVE_EVERY
        });
        Ok(iq_observe::Settings {
            model,
            every: every.clamp(1, 50),
            categories: self.categories(),
        })
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
            "model = \"codex\"\nbudget = 1.5\neffort = \"low\"\nautopilot = true\ninternet = true\nbase_url = \" https://router.example/v1 \"",
        )
        .unwrap();
        assert_eq!(set.base_url().as_deref(), Some("https://router.example/v1"));
        assert_eq!(set.lead(true).unwrap().as_str(), "codex");
        assert_eq!(set.crew(true).unwrap().budget, Some(Usd(1.5)));
        assert!(set.autopilot && set.internet);
        assert!(toml::from_str::<Settings>("modle = \"x\"").is_err());
        let looks: Settings =
            toml::from_str("theme = \"Ink\"\n[keys]\ncosts = \"ctrl+u\"").unwrap();
        assert_eq!(looks.look().unwrap().theme().name(), "ink");
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
    fn the_assay_is_graded_by_one_grader_a_subscriptions_first() {
        let lead = ModelId::new("anthropic/claude-sonnet-4-5").unwrap();
        let unset = Settings::default();
        let grader = |s: &Settings, installed: &[Agent]| -> String {
            s.assay_grader(installed, &lead).unwrap().to_string()
        };
        assert_eq!(
            grader(&unset, &[Agent::ClaudeCode, Agent::Codex]),
            "claude-code/opus"
        );
        assert_eq!(grader(&unset, &[]), "anthropic/claude-sonnet-4-5");
        let set: Settings = toml::from_str(
            "[assay]\ngrader = \"codex\"\ncandidates = [\"models\"]\nfolder = \"~/lab\"\nparallel = 40",
        )
        .unwrap();
        assert_eq!(grader(&set, &[Agent::ClaudeCode]), "codex");
        let before: Settings = toml::from_str("[assay]\njudges = [\"codex\", \"x/y\"]").unwrap();
        assert_eq!(grader(&before, &[Agent::ClaudeCode]), "codex");
        assert_eq!(set.assay_folder(Path::new("/h")), PathBuf::from("/h/lab"));
        assert_eq!(set.assay_parallel(), 8);
        assert!(set.assay_allowed().allows(iq_assay::Kind::Model, "a/b"));
        assert!(!set.assay_allowed().allows(iq_assay::Kind::Harness, "codex"));
        assert!(toml::from_str::<Settings>("[assay]\njudge = \"x\"").is_err());
    }

    #[test]
    fn the_categories_and_who_places_tasks_are_set_in_the_settings() {
        let none = Settings::default();
        assert!(none.categories().is_empty());
        assert_eq!(
            none.observing(true).unwrap().model.as_str(),
            "openai/gpt-5-mini"
        );
        let without_key = none.observing(false).unwrap();
        assert_eq!(without_key.model.as_str(), "claude-code/haiku");
        assert_eq!(without_key.every, 10);
        let set: Settings = toml::from_str(
            "[observe]\nmodel = \"cheap/one\"\n\n[[observe.category]]\nname = \"Python\"\nfiles = [\"*.py\"]\n\n[[observe.category]]\nname = \"Reviews\"\ndescription = \"reading others' changes\"",
        )
        .unwrap();
        let observing = set.observing(true).unwrap();
        assert_eq!(
            (observing.model.as_str(), observing.every),
            ("cheap/one", 1)
        );
        assert_eq!(observing.categories.categories.len(), 2);
        assert!(toml::from_str::<Settings>("[[observe.category]]\nnom = \"x\"").is_err());
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
