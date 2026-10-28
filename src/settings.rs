//! The person's choices, kept in `~/.iq/settings.toml`. Keys never go
//! there: they come from the environment only.
//!
//! ```toml
//! base_url = "https://router.requesty.ai/v1"  # the provider: another region's router, any OpenAI compatible endpoint
//! model = "anthropic/claude-opus-5-5"    # the lead, which decides who does what
//! reviewer = "anthropic/claude-haiku-5-5"  # answers for the person when the autopilot is on
//! effort = "high"                        # how hard the models think; /effort changes an agent's
//! context_every = 25000                  # tokens an agent's conversation grows between two words to it of its size; 0 never
//! autopilot = false
//! autopilot_internet = true              # the reviewer also answers what only reaches the internet
//! autopilot_host = true                  # the reviewer also answers what runs on the machine itself (containers, clusters, the forge), as your skill says; true once a skill is set
//! autopilot_refusals = 3                 # refusals of the reviewer in a row before an agent's next action is asked of you; 0 never
//! internet = false                       # true: every command reaches the internet
//! worktree = false                       # true: each chat you start works in a worktree of its own, on a branch iq/<chat>; your checkout is left as it is
//! theme = "graphite"                     # or "ink"; "porcelain" for a light terminal; /theme
//! rules = ["~/.iq/AGENTS.md"]           # your rules for every project, given to the agents you start; unless set, this one, else ~/.claude/CLAUDE.md
//! autopilot_skill = "local-review"     # a skill of yours the reviewer follows, kept outside the workspace, read again at each decision, never shown to the agents
//! skills = ["~/.iq/skills"]              # folders of your skills (a folder with its SKILL.md, or a .md file); this one unless set
//! claude_memory = true                   # the notes kept of the project under ~/.claude read beside iq's own
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
//! keep = ["~/notes", "~/.zshrc"]         # files and folders of the home kept too, put back on a new machine; your rules and skills in ~/.iq are
//! auto = false                           # true: what changes is sent without asking, and said; else iq asks first
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

/// The person's rules for iq, unless the settings list others: read, and
/// synced with iq's own files.
const IQ_RULES: &str = "~/.iq/AGENTS.md";

/// The rules many already keep for their agents: read when iq's are not
/// there. They are not synced: they may name what the person keeps out of
/// their repository, and hold another tool's habits.
const CLAUDE_RULES: &str = "~/.claude/CLAUDE.md";

/// The folders of the person's skills unless the settings list others.
const SKILLS: [&str; 1] = ["~/.iq/skills"];

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
    /// How many tokens an agent's conversation grows between two words to
    /// it of its size.
    context_every: Option<u64>,
    #[serde(default)]
    pub(crate) autopilot: bool,
    /// Whether the autopilot's reviewer also answers what only reaches the
    /// internet, rather than the person; true unless set.
    autopilot_internet: Option<bool>,
    /// A skill from the configured folders, read by the permission reviewer.
    pub(crate) autopilot_skill: Option<String>,
    /// Whether the autopilot's reviewer also answers what runs on the
    /// machine itself, outside the sandbox; true once a skill is set.
    autopilot_host: Option<bool>,
    /// How many refusals of the reviewer in a row before an agent's next
    /// action is asked of the person.
    autopilot_refusals: Option<u32>,
    /// Every command reaches the internet, rather than what a rule gives
    /// it to and what the person allows.
    #[serde(default)]
    pub(crate) internet: bool,
    /// Each chat the person starts works in a worktree of its own, on a
    /// branch of its own, rather than in their checkout.
    #[serde(default)]
    pub(crate) worktree: bool,
    /// The theme.
    theme: Option<String>,
    /// The person's rules for every project, files of their home.
    rules: Option<Vec<String>>,
    /// The folders of the person's skills.
    skills: Option<Vec<String>>,
    /// Whether the notes kept of the project under `~/.claude` are read
    /// beside iq's.
    claude_memory: Option<bool>,
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
    /// Whether what changes is sent without asking the person.
    #[serde(default)]
    auto: bool,
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

    /// The files of the person's rules, as the settings write them and
    /// where they are: `~/` in `home`. Unless set, iq's own, else
    /// `~/.claude/CLAUDE.md`.
    pub(crate) fn rules_files(&self, home: &Path) -> Vec<(String, PathBuf)> {
        let listed = self.rules.clone().unwrap_or_else(|| {
            let own = placed(IQ_RULES, home, home);
            vec![if own.exists() { IQ_RULES } else { CLAUDE_RULES }.to_owned()]
        });
        listed
            .into_iter()
            .filter(|file| !file.trim().is_empty())
            .map(|file| {
                let path = placed(file.trim(), home, home);
                (file, path)
            })
            .collect()
    }

    /// The folders of the person's skills: `~/` in `home`, others in
    /// `workspace`.
    pub(crate) fn skill_folders(&self, home: &Path, workspace: &Path) -> Vec<PathBuf> {
        self.skills
            .clone()
            .unwrap_or_else(|| SKILLS.map(str::to_owned).to_vec())
            .iter()
            .filter(|folder| !folder.trim().is_empty())
            .map(|folder| placed(folder.trim(), home, workspace))
            .collect()
    }

    /// Whether the rules read are `~/.claude/CLAUDE.md`, by default: there are no
    /// rules of iq's own yet.
    pub(crate) fn claude_rules_by_default(&self, home: &Path) -> bool {
        self.rules.is_none()
            && !placed(IQ_RULES, home, home).exists()
            && placed(CLAUDE_RULES, home, home).exists()
    }

    /// Whether the notes kept of the project under `~/.claude` are read
    /// beside iq's.
    pub(crate) fn claude_memory(&self) -> bool {
        self.claude_memory.unwrap_or(true)
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

    /// Whether the autopilot's reviewer answers what runs on the machine
    /// itself: when the person says, else once they wrote a skill for it.
    pub(crate) fn autopilot_host(&self) -> bool {
        self.autopilot_host
            .unwrap_or_else(|| self.autopilot_skill.is_some())
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

    /// Whether what changes is sent without asking the person.
    pub(crate) fn sync_auto(&self) -> bool {
        self.sync.auto
    }

    /// The machine's sync of iq's folder of data `data`, with the settings
    /// and rules of iq's `folder` mirrored in it, the person's rules and
    /// skills kept there too, and the files of `home` they list kept; none
    /// without a repository. Rules or skills elsewhere are synced only as
    /// listed: `~/.claude/CLAUDE.md` may hold what is not for the repository.
    pub(crate) fn sync(&self, folder: &Path, data: &Path, home: &Path) -> Option<iq_sync::Sync> {
        let remote = self.sync.remote.as_deref()?.trim();
        if remote.is_empty() {
            return None;
        }
        let mut sync = iq_sync::Sync::new(data, remote, crate::sync::machine())
            .mirror(folder.join("settings.toml"), "config/settings.toml")
            .mirror(folder.join("rules.toml"), "config/rules.toml");
        let rules = self
            .rules
            .clone()
            .unwrap_or_else(|| vec![IQ_RULES.to_owned()]);
        let skills = self
            .skills
            .clone()
            .unwrap_or_else(|| SKILLS.map(str::to_owned).to_vec());
        for written in rules.iter().chain(&skills).map(|w| w.trim()) {
            let Some(relative) = written.strip_prefix("~/") else {
                // A project's own, which its repository keeps.
                continue;
            };
            let place = home.join(relative);
            if let Ok(inside) = place.strip_prefix(folder) {
                sync = sync.mirror(&place, format!("config/{}", inside.display()));
            }
        }
        let mut sync = sync.keeping(home, self.sync.keep.clone());
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
            context_every: self.context_every.unwrap_or(iq_agent::CONTEXT_EVERY),
            refusals: self.autopilot_refusals.unwrap_or(iq_agent::REFUSALS),
            ..iq_agent::Settings::default()
        })
    }
}

/// Where `path` is: `~/` in `home`, a relative path in `base`.
fn placed(path: &str, home: &Path, base: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if path == "~" => home.to_path_buf(),
        None => base.join(path),
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
        assert_eq!(set.crew().unwrap().context_every, iq_agent::CONTEXT_EVERY);
        let quiet: Settings = toml::from_str("context_every = 0").unwrap();
        assert_eq!(quiet.crew().unwrap().context_every, 0);
        assert!(set.autopilot && set.internet);
        assert!(!set.worktree);
        let apart: Settings = toml::from_str("worktree = true").unwrap();
        assert!(apart.worktree);
        assert!(set.autopilot_skill.is_none());
        let local: Settings = toml::from_str("autopilot_skill = \"local-review\"").unwrap();
        assert_eq!(local.autopilot_skill.as_deref(), Some("local-review"));
        // The machine itself is the reviewer's once a skill says what runs
        // there, unless the person says.
        assert!(!set.autopilot_host());
        assert!(local.autopilot_host());
        let kept: Settings = toml::from_str(
            "autopilot_skill = \"x\"\nautopilot_host = false\nautopilot_refusals = 5",
        )
        .unwrap();
        assert!(!kept.autopilot_host());
        assert_eq!(kept.crew().unwrap().refusals, 5);
        assert_eq!(set.crew().unwrap().refusals, iq_agent::REFUSALS);
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
    }

    #[test]
    fn the_rules_and_the_skills_are_iqs_else_those_of_dot_claude_unless_set() {
        let (home, work) = (Path::new("/h"), Path::new("/w"));
        let none = Settings::default();
        assert_eq!(
            none.rules_files(home),
            [(
                "~/.claude/CLAUDE.md".to_owned(),
                PathBuf::from("/h/.claude/CLAUDE.md")
            )]
        );
        // Rules of iq's own, once written, are read instead.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(dir.path().join(".claude/CLAUDE.md"), "theirs\n").unwrap();
        assert!(none.claude_rules_by_default(dir.path()));
        std::fs::create_dir_all(dir.path().join(".iq")).unwrap();
        std::fs::write(dir.path().join(".iq/AGENTS.md"), "iq's\n").unwrap();
        assert_eq!(none.rules_files(dir.path())[0].0, "~/.iq/AGENTS.md");
        assert!(!none.claude_rules_by_default(dir.path()));
        assert_eq!(
            none.skill_folders(home, work),
            [PathBuf::from("/h/.iq/skills")]
        );
        assert!(none.claude_memory());
        let set: Settings = toml::from_str(
            "rules = []\nskills = [\"~/.claude/skills\", \".claude/skills\"]\nclaude_memory = false",
        )
        .unwrap();
        assert!(set.rules_files(home).is_empty());
        assert_eq!(
            set.skill_folders(home, work),
            [
                PathBuf::from("/h/.claude/skills"),
                PathBuf::from("/w/.claude/skills")
            ]
        );
        assert!(!set.claude_memory());
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
            .sync(
                &dir.path().join(".iq"),
                &dir.path().join("data"),
                dir.path(),
            )
            .unwrap();
        assert_eq!(sync.remote(), "https://github.com/someone/any-name.git");
        // ~/.claude/CLAUDE.md is not kept unless listed.
        assert_eq!(sync.kept(), ["~/notes"]);
        assert_eq!(sync.refused().len(), 1);
        assert!(!set.sync_auto());
        let auto: Settings =
            toml::from_str("[sync]\nremote = \"https://example.com/n.git\"\nauto = true").unwrap();
        assert!(auto.sync_auto());
        assert!(toml::from_str::<Settings>("[sync]\nremot = \"x\"").is_err());
    }
}
