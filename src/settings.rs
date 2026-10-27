//! The person's choices, kept in `~/.iq/settings.toml`. Keys never go
//! there: they come from the environment only.
//!
//! ```toml
//! base_url = "https://router.requesty.ai/v1"  # the provider: another region's router, any OpenAI compatible endpoint
//! model = "anthropic/claude-opus-5-5"    # the lead, which decides who does what
//! reviewer = "anthropic/claude-haiku-5-5"  # answers for the person when the autopilot is on
//! effort = "high"                        # how hard the models think; /effort changes an agent's
//! autopilot = false
//! autopilot_internet = true              # the reviewer also answers what only reaches the internet
//! internet = false                       # true: every command reaches the internet
//! theme = "graphite"                     # or "ink"; "porcelain" for a light terminal; /theme
//!
//! [keys]                                 # any shortcut moved, "" for none; Ctrl-S lists them
//! costs = "ctrl+o"
//!
//! [observe]                              # what your work goes to; iq's own, never synced
//! model = "anthropic/claude-haiku-5-5"   # who places what no rule places: a cheap one
//! every = 1                              # tasks placed at once
//!
//! [[observe.category]]                   # your categories of work, any number
//! name = "Python"
//! description = "code in Python: the back end, the scripts"
//! files = ["*.py"]                       # rules iq applies itself before any model, all optional
//! commands = ["pytest"]
//! words = ["django"]
//!
//! [sync]                                 # iq's data kept in a private repository of yours, any name
//! remote = "https://github.com/someone/any-name.git"
//! branch = "main"                        # another when main must go through pull requests
//! keep = ["~/notes", "~/.zshrc"]         # files and folders of the home kept too, put back on a new machine
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use iq_core::{Effort, ModelId};
use serde::Deserialize;

/// The lead's model when the settings name none: the one that decides
/// who does what.
const LEAD: &str = "anthropic/claude-opus-5-5";

/// The reviewer's, likewise: cheap, as it answers often and briefly; the
/// observer's too.
const REVIEWER: &str = "anthropic/claude-haiku-5-5";

/// What the settings once held and iq reads no more, since it works with
/// the provider's models alone and lets the lead decide.
const RETIRED: [&str; 6] = [
    "budget", "steward", "[limits]", "[mcp]", "[assay]", "[route]",
];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    /// The provider's address, when not Requesty's router.
    base_url: Option<String>,
    model: Option<String>,
    reviewer: Option<String>,
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
    /// Where iq's data is synced.
    #[serde(default)]
    sync: SyncFile,
    /// What the work goes to.
    #[serde(default)]
    observe: ObserveFile,
    /// What iq reads no more, in the order of [`RETIRED`]: read, so that
    /// older settings still open, and said, for the person to remove it.
    #[serde(default)]
    budget: Option<toml::Value>,
    #[serde(default)]
    steward: Option<toml::Value>,
    #[serde(default)]
    limits: Option<toml::Value>,
    #[serde(default)]
    mcp: Option<toml::Value>,
    #[serde(default)]
    assay: Option<toml::Value>,
    #[serde(default)]
    route: Option<toml::Value>,
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

/// How many tasks wait before a model places them: one, as each call is
/// small.
const OBSERVE_EVERY: usize = 1;

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

/// iq's folder of data: the projects' memory, a copy of the settings and
/// what the person keeps, synced when the machine names a
/// repository. Out of `~/.iq`, which agents' tools cannot reach.
pub(crate) fn data_folder(home: &Path) -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|data| data.is_absolute())
        .unwrap_or_else(|| home.join(".local/share"))
        .join("iq")
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

    /// The lead's model: as set, else Opus.
    pub(crate) fn lead(&self) -> Result<ModelId> {
        Ok(ModelId::new(self.model.as_deref().unwrap_or(LEAD))?)
    }

    /// What the settings hold that iq reads no more.
    pub(crate) fn retired(&self) -> Vec<&'static str> {
        [
            self.budget.is_some(),
            self.steward.is_some(),
            self.limits.is_some(),
            self.mcp.is_some(),
            self.assay.is_some(),
            self.route.is_some(),
        ]
        .into_iter()
        .zip(RETIRED)
        .filter_map(|(set, name)| set.then_some(name))
        .collect()
    }

    /// The models the settings name that were Claude Code's or Codex's,
    /// each with where: they reach the provider no more.
    pub(crate) fn agents_named(&self) -> Vec<(&'static str, String)> {
        let agent = |model: &str| {
            let first = model.trim().split('/').next().unwrap_or_default();
            first == "claude-code" || first == "codex"
        };
        [
            ("model", self.model.as_deref()),
            ("reviewer", self.reviewer.as_deref()),
            ("[observe] model", self.observe.model.as_deref()),
        ]
        .into_iter()
        .filter_map(|(key, model)| Some((key, model?.to_owned())))
        .filter(|(_, model)| agent(model))
        .collect()
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

    /// The shortcuts, the defaults moved as set.
    pub(crate) fn keymap(&self) -> Result<iq_ui::keys::Keymap> {
        Ok(iq_ui::keys::Keymap::with(&self.keys)?)
    }

    /// The person's categories of work.
    pub(crate) fn categories(&self) -> iq_observe::Categories {
        iq_observe::Categories::new(self.observe.category.clone())
    }

    /// What the observer is set with: the model that places what no rule
    /// places (as set, else the reviewer's cheap one), and how many tasks
    /// wait for it.
    pub(crate) fn observing(&self) -> Result<iq_observe::Settings> {
        let model = ModelId::new(self.observe.model.as_deref().unwrap_or(REVIEWER))?;
        Ok(iq_observe::Settings {
            model,
            every: self.observe.every.unwrap_or(OBSERVE_EVERY).clamp(1, 50),
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

    /// What the crew starts with.
    pub(crate) fn crew(&self) -> Result<iq_agent::Settings> {
        Ok(iq_agent::Settings {
            reviewer: Some(ModelId::new(self.reviewer.as_deref().unwrap_or(REVIEWER))?),
            effort: self.effort,
            ..iq_agent::Settings::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_settings_opus_leads_and_haiku_reviews() {
        let settings = Settings::default();
        assert_eq!(settings.lead().unwrap().as_str(), LEAD);
        assert_eq!(
            settings.crew().unwrap().reviewer.unwrap().as_str(),
            REVIEWER
        );
        assert!(settings.retired().is_empty());
        let set: Settings = toml::from_str(
            "model = \"lab/lead\"\neffort = \"low\"\nautopilot = true\ninternet = true\nbase_url = \" https://router.example/v1 \"",
        )
        .unwrap();
        assert_eq!(set.base_url().as_deref(), Some("https://router.example/v1"));
        assert_eq!(set.lead().unwrap().as_str(), "lab/lead");
        assert_eq!(set.crew().unwrap().effort, Some(Effort::Low));
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
        // Older settings still open, what iq reads no more said.
        let older: Settings = toml::from_str(
            "budget = 2.0\nsteward = true\n[limits]\nrequesty_month = 20.0\n[mcp.notes]\nurl = \"https://notes.example/mcp\"\n[assay]\ngrader = \"x/y\"\n[route]\noff = true",
        )
        .unwrap();
        assert_eq!(older.retired(), RETIRED);
        let agents: Settings = toml::from_str(
            "model = \"claude-code/opus\"\nreviewer = \"z-ai/glm-5\"\n[observe]\nmodel = \"codex\"",
        )
        .unwrap();
        assert_eq!(
            agents.agents_named(),
            [
                ("model", "claude-code/opus".to_owned()),
                ("[observe] model", "codex".to_owned())
            ]
        );
    }

    #[test]
    fn the_categories_and_who_places_tasks_are_set_in_the_settings() {
        let none = Settings::default();
        assert!(none.categories().is_empty());
        assert_eq!(none.observing().unwrap().model.as_str(), REVIEWER);
        assert_eq!(none.observing().unwrap().every, 1);
        let set: Settings = toml::from_str(
            "[observe]\nmodel = \"cheap/one\"\n\n[[observe.category]]\nname = \"Python\"\nfiles = [\"*.py\"]\n\n[[observe.category]]\nname = \"Reviews\"\ndescription = \"reading others' changes\"",
        )
        .unwrap();
        let observing = set.observing().unwrap();
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
}
