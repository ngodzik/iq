//! The person's choices, kept in `~/.iq/settings.toml`. Keys never go
//! there: they come from the environment only.
//!
//! ```toml
//! base_url = "https://router.requesty.ai/v1"  # the provider: another region's router, any OpenAI compatible endpoint
//! model = "anthropic/claude-opus-5-5"    # the lead, which decides who does what
//! autopilot_model = "anthropic/claude-haiku-5-5"  # answers the gate for you when the autopilot is on (once `reviewer`, still read until the next release)
//! allowed_models = ["anthropic/claude-opus-5-5", "anthropic/claude-haiku-5-5"]  # the only models iq starts, for you and every agent, by their whole names; any other is refused, the agent told which there are; unset, any the provider lists, by its whole name
//! autopilot_effort = "low"               # how hard it thinks (once `reviewer_effort`)
//! summarizer = "anthropic/claude-haiku-5-5"  # sums up in a few lines what agents write to each other, in the conversation; Ctrl-X gives it whole; unset, their first lines
//! cache_marks = ["claude"]              # the models iq marks for the provider's cache itself, by a word of their names; the others are left to Requesty's auto_cache; add "gemini" to try it on Gemini (Vertex), and compare on the costs page what each model read from the cache
//! turn_cost_ask = 1.0                    # dollars a chat's turn may spend (its agents, those they started, the autopilot, the summaries) before you are asked whether it goes on, then as much again each time; unset, never asked
//! effort = "high"                        # how hard the models think; /effort changes an agent's
//! context_every = 25000                  # tokens an agent's conversation grows between two words to it of its size; 0 never
//! light_from = 40000                     # tokens an agent's calls read past which its old tool results (all but the latest six, the long ones) are set apart, each replaced by a line saying what it was, `reread` reading it again; again each time they grow as much, so that the provider's cache breaks seldom; 0 never
//! autopilot = false
//! autopilot_internet = true              # the reviewer also answers what only reaches the internet
//! autopilot_host = true                  # the reviewer also answers what runs on the machine itself (containers, clusters, the forge), as your skill says; true once a skill is set
//! autopilot_refusals = 3                 # refusals of the reviewer in a row before an agent's next action is asked of you; 0 never
//! internet = false                       # true: every command reaches the internet
//! command_timeout = 600                  # seconds a command may run before it is stopped
//! command_quiet = 180                    # seconds a command may write nothing before it is stopped: one waiting for a login, or an answer at a terminal
//! provider_retries = 6                   # times a call is sent again while its provider is out of capacity (a quota, an overload), waiting as it asks, longer each time; 0 never
//! worktree = false                       # true: each chat you start works in a worktree of its own, on a branch iq/<chat>, its path under where you write; your checkout is left as it is
//! worktree_from = "HEAD"                 # what a worktree starts from: your checkout's commit, or a branch such as "origin/main"
//! worktree_copy = [".env"]               # files of your checkout git does not keep, copied into a worktree as it is made, where it has their folder, never over one; a name alone is looked for in every folder
//! worktree_setup = ["uv sync"]           # commands run in a worktree as it is made, on your machine, to give it the project's environment
//! chat_keep = 30                         # days a chat may go untouched before iq forgets it as it starts, its worktree with it when nothing there is uncommitted; 0 never
//! scratch_mb = 500                       # most a chat's folder of scratch holds (~/.cache/iq/scratch), where its agents keep logs and snapshots outside the repository; past it, its oldest files go
//! theme = "graphite"                     # or "ink"; "porcelain" for a light terminal; /theme
//! steps = 5                              # an agent's latest steps (tools, what was allowed) shown as it works; once it writes again only their count, Ctrl-X unfolds them; 0 none
//! job_lines = 3                          # a background job's latest lines shown under its line while it runs, cut to the screen's width; 0 none
//! turn_view = "panes"                    # a turn of several agents shown as panes side by side at the top, or "strip": a few lines of its map below the conversation; Ctrl-G opens the whole map either way
//! rules = ["~/.iq/AGENTS.md"]           # your rules for every project, given to the agents you start; unless set, this one, else ~/.claude/CLAUDE.md
//! autopilot_skill = "local-review"     # a skill of yours the reviewer follows, kept outside the workspace, read again at each decision, never shown to the agents
//! skills = ["~/.iq/skills"]              # folders of your skills (a folder with its SKILL.md, or a .md file); this one unless set; the project's .claude/skills are read after them, yours winning a name both hold
//! claude_memory = true                   # the notes kept of the project under ~/.claude read beside iq's own
//! plugins = ["name"]                     # programs of their own iq starts and speaks with: iq-name for name, found on the PATH, run on your machine as you
//! mermaid = "auto"                       # Mermaid diagrams drawn as images in Kitty, by mmdr (cargo install mermaid-rs-renderer) run in the sandbox; or a command with {input} and {output}; off unless set
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
//! [mcp.tracker]                          # an MCP server, by a name of yours: its tools offered to the agents as tracker__<tool>
//! url = "https://mcp.example.com/mcp"    # over HTTP; when it asks for OAuth, you consent in the browser, its grant kept in ~/.iq/mcp-auth
//! headers = { Authorization = "TRACKER_AUTHORIZATION" }  # or headers sent, each value read from the variable it names; then OAuth is never asked
//! tools = ["get_issue", "search"]        # the only tools offered, as the server names them; all unless set
//!
//! [mcp.notes]                            # one over stdio: a program run on your machine as you, with your environment but iq's variables
//! command = "notes-mcp"
//! args = ["--folder", "~/notes"]
//!
//! [web]                                  # the agents read pages with fetch, judged as a command with the internet
//! search = "duckduckgo"                  # a search tool, off unless set: a fallback with no key, fine for moderate use; used often, its engine slows robots down and another would be needed
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

/// The project's own skills, read beside the person's whatever the settings
/// list: never the reviewer's policy, which no workspace may hold.
const PROJECT_SKILLS: &str = ".claude/skills";

/// How many days a chat may go untouched before iq forgets it, unless the
/// settings say.
const CHAT_KEEP_DAYS: u64 = 30;

/// What the settings once held and iq reads no more, since it works with
/// the provider's models alone and lets the lead decide.
const RETIRED: [&str; 5] = ["budget", "steward", "[limits]", "[assay]", "[route]"];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    /// The provider's address, when not Requesty's router.
    base_url: Option<String>,
    model: Option<String>,
    /// The model that answers the gate for the person when the autopilot
    /// is on.
    autopilot_model: Option<String>,
    /// `autopilot_model`'s old name, read until the next release.
    reviewer: Option<String>,
    /// The only models iq starts, for the person and every agent; any the
    /// provider lists when empty.
    #[serde(default)]
    allowed_models: Vec<String>,
    /// The most a chat's folder of scratch holds, in megabytes.
    scratch_mb: Option<u64>,
    /// The model that sums up what agents write to each other, for the
    /// person; none unless set.
    summarizer: Option<String>,
    /// What a chat's turn may spend before the person is asked whether it
    /// goes on, in dollars; never asked unless set.
    turn_cost_ask: Option<f64>,
    /// The models iq marks for the provider's cache itself, by words of
    /// their names; Claude's unless set.
    cache_marks: Option<Vec<String>>,
    /// How hard the autopilot's model thinks; low unless set.
    autopilot_effort: Option<Effort>,
    /// `autopilot_effort`'s old name, read until the next release.
    reviewer_effort: Option<Effort>,
    effort: Option<Effort>,
    /// Seconds a command may run before it is stopped.
    command_timeout: Option<u64>,
    /// Seconds a command may write nothing before it is stopped.
    command_quiet: Option<u64>,
    /// Times a call is sent again while its provider is out of capacity.
    provider_retries: Option<u32>,
    /// How many tokens an agent's conversation grows between two words to
    /// it of its size.
    context_every: Option<u64>,
    /// The tokens an agent's calls read past which its old tool results
    /// are set apart; 0 never.
    light_from: Option<u64>,
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
    /// What a worktree starts from; `HEAD` unless set.
    worktree_from: Option<String>,
    /// The files of the checkout git does not keep, copied into a
    /// worktree as it is made.
    #[serde(default)]
    worktree_copy: Vec<String>,
    /// The commands run in a worktree as it is made.
    #[serde(default)]
    worktree_setup: Vec<String>,
    /// Days a chat may go untouched before it is forgotten; 0 never.
    chat_keep: Option<u64>,
    /// The theme.
    theme: Option<String>,
    /// How many of an agent's latest steps show as it works.
    steps: Option<usize>,
    /// How many of a job's latest lines show while it runs.
    job_lines: Option<usize>,
    /// What shows a turn of several agents: panes, or a strip of its map.
    turn_view: Option<String>,
    /// The person's rules for every project, files of their home.
    rules: Option<Vec<String>>,
    /// The folders of the person's skills.
    skills: Option<Vec<String>>,
    /// Whether the notes kept of the project under `~/.claude` are read
    /// beside iq's.
    claude_memory: Option<bool>,
    /// The plugins iq starts, by name: `iq-<name>` on the `PATH`.
    #[serde(default)]
    pub(crate) plugins: Vec<String>,
    /// What draws the Mermaid diagrams agents write, in Kitty; none unless
    /// set.
    pub(crate) mermaid: Option<Mermaid>,
    /// The shortcuts moved: an action's name and its key.
    #[serde(default)]
    keys: BTreeMap<String, String>,
    /// Where iq's data is synced.
    #[serde(default)]
    sync: SyncFile,
    /// How the agents search the web.
    #[serde(default)]
    web: WebFile,
    /// What the work goes to.
    #[serde(default)]
    observe: ObserveFile,
    /// The MCP servers, by the names the person gives them.
    #[serde(default)]
    mcp: BTreeMap<String, McpFile>,
    /// What iq reads no more, in the order of [`RETIRED`]: read, so that
    /// older settings still open, and said, for the person to remove it.
    #[serde(default)]
    budget: Option<toml::Value>,
    #[serde(default)]
    steward: Option<toml::Value>,
    #[serde(default)]
    limits: Option<toml::Value>,
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

/// What draws Mermaid diagrams, as the settings write it: `"auto"`, or a
/// program and its arguments, `{input}` and `{output}` standing for the
/// diagram's source and its PNG image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mermaid {
    /// mmdr, when installed.
    Auto,
    /// The program and its arguments.
    Command(Vec<String>),
}

impl<'de> Deserialize<'de> for Mermaid {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Written {
            Word(String),
            Words(Vec<String>),
        }
        match Written::deserialize(deserializer)? {
            Written::Word(word) if word.trim() == "auto" => Ok(Mermaid::Auto),
            Written::Words(words)
                if words.first().is_some_and(|w| !w.trim().is_empty())
                    && words.iter().any(|w| w.contains("{output}")) =>
            {
                Ok(Mermaid::Command(words))
            }
            _ => Err(serde::de::Error::custom(
                "mermaid is \"auto\", or a program and its arguments naming {output}, the image it writes",
            )),
        }
    }
}

/// An MCP server as the settings write it: an address, with the headers
/// sent there, or a program and its arguments. A header's value is never
/// written there: only the name of the variable that holds it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpFile {
    url: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    /// The only tools offered, as the server names them.
    tools: Option<Vec<String>>,
}

/// An MCP server ready to start: its name, what reaches it, and the line
/// its program runs, for the audit log.
pub(crate) struct McpServer {
    pub(crate) name: String,
    pub(crate) declared: iq_mcp::Declared,
    pub(crate) line: Option<String>,
}

/// The web as the settings write it: the engine the agents search with,
/// none unless set.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WebFile {
    search: Option<String>,
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

    /// The models iq marks for the provider's cache itself, as set.
    pub(crate) fn cache_marks(&self) -> Option<Vec<String>> {
        self.cache_marks.clone()
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

    /// The settings named by an old name, each with its new one: read
    /// until the next release, the new name winning when both are set.
    pub(crate) fn renamed(&self) -> Vec<(&'static str, &'static str)> {
        [
            (self.reviewer.is_some(), ("reviewer", "autopilot_model")),
            (
                self.reviewer_effort.is_some(),
                ("reviewer_effort", "autopilot_effort"),
            ),
        ]
        .into_iter()
        .filter_map(|(set, names)| set.then_some(names))
        .collect()
    }

    /// What the settings hold that iq reads no more.
    pub(crate) fn retired(&self) -> Vec<&'static str> {
        [
            self.budget.is_some(),
            self.steward.is_some(),
            self.limits.is_some(),
            self.assay.is_some(),
            self.route.is_some(),
        ]
        .into_iter()
        .zip(RETIRED)
        .filter_map(|(set, name)| set.then_some(name))
        .collect()
    }

    /// The engine the agents search the web with, none unless set: only
    /// DuckDuckGo's page of plain HTML for now, a fallback for moderate use.
    pub(crate) fn search(&self) -> Result<Option<iq_web::Engine>> {
        match self.web.search.as_deref().map(str::trim) {
            None | Some("") => Ok(None),
            Some("duckduckgo") => Ok(Some(iq_web::Engine::DuckDuckGo)),
            Some(other) => anyhow::bail!(
                "[web] search = {other:?} is no engine iq knows: \"duckduckgo\", or none"
            ),
        }
    }

    /// The MCP servers the settings declare, their headers read from the
    /// variables `var` gives and a program's `~/` in `home`, each program
    /// given `env`; and why each of the others is not started.
    pub(crate) fn mcp_servers(
        &self,
        var: impl Fn(&str) -> Option<String>,
        home: &Path,
        env: impl Fn() -> Vec<(std::ffi::OsString, std::ffi::OsString)>,
    ) -> (Vec<McpServer>, Vec<String>) {
        let mut servers = Vec::new();
        let mut wrong = Vec::new();
        for (name, server) in &self.mcp {
            let not = |why: String| format!("the MCP server {name} is not started: {why}");
            let plain = name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if name.is_empty() || !plain {
                wrong.push(not(
                    "its name is letters, digits, hyphens and underscores".to_owned()
                ));
                continue;
            }
            let url = server
                .url
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty());
            let command = server
                .command
                .as_deref()
                .map(str::trim)
                .filter(|c| !c.is_empty());
            let declared = match (url, command) {
                (Some(url), None) => {
                    let mut declared = iq_mcp::Declared::at(url);
                    let mut missing = Vec::new();
                    for (header, variable) in &server.headers {
                        match var(variable) {
                            Some(value) => declared = declared.header(header, value),
                            None => missing.push(variable.as_str()),
                        }
                    }
                    if !missing.is_empty() {
                        wrong.push(not(format!("{} is not set", missing.join(", "))));
                        continue;
                    }
                    McpServer {
                        name: name.clone(),
                        declared,
                        line: None,
                    }
                }
                (None, Some(command)) if server.headers.is_empty() => {
                    let words: Vec<String> = std::iter::once(command)
                        .chain(server.args.iter().map(String::as_str))
                        .map(|word| match word.strip_prefix("~/") {
                            Some(rest) => home.join(rest).to_string_lossy().into_owned(),
                            None => word.to_owned(),
                        })
                        .collect();
                    McpServer {
                        name: name.clone(),
                        line: Some(words.join(" ")),
                        declared: iq_mcp::Declared::run(words).env(env()),
                    }
                }
                (None, Some(_)) => {
                    wrong.push(not("headers are for a server reached by its url".to_owned()));
                    continue;
                }
                (Some(_), Some(_)) => {
                    wrong.push(not("give it a url or a command, not both".to_owned()));
                    continue;
                }
                (None, None) => {
                    wrong.push(not("give it a url or a command".to_owned()));
                    continue;
                }
            };
            let declared = match &server.tools {
                Some(tools) => McpServer {
                    declared: declared.declared.only(tools.iter().cloned()),
                    ..declared
                },
                None => declared,
            };
            servers.push(declared);
        }
        (servers, wrong)
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
        let mut folders: Vec<PathBuf> = self
            .skills
            .clone()
            .unwrap_or_else(|| SKILLS.map(str::to_owned).to_vec())
            .iter()
            .filter(|folder| !folder.trim().is_empty())
            .map(|folder| placed(folder.trim(), home, workspace))
            .collect();
        // The project's own after the person's, whose skills win where both
        // hold one of the same name.
        let project = workspace.join(PROJECT_SKILLS);
        if !folders.contains(&project) {
            folders.push(project);
        }
        folders
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
        Ok(iq_ui::look::Look::new(theme)
            .showing(self.steps.unwrap_or(iq_ui::steps::SHOWN))
            .with_job_lines(self.job_lines.unwrap_or(iq_ui::look::JOB_LINES))
            .with_turn(match self.turn_view.as_deref() {
                Some(turn) => turn.parse().map_err(anyhow::Error::msg)?,
                None => iq_ui::look::TurnShown::Panes,
            }))
    }

    /// The model that sums up what agents write to each other, when set.
    pub(crate) fn summarizer(&self) -> Result<Option<ModelId>> {
        match self.summarizer.as_deref().map(str::trim) {
            None | Some("") => Ok(None),
            Some(model) => Ok(Some(ModelId::new(model)?)),
        }
    }

    /// The only models iq starts, as set: any the provider lists when
    /// none is.
    pub(crate) fn allowed_models(&self) -> Result<Vec<ModelId>> {
        self.allowed_models
            .iter()
            .map(|model| model.trim())
            .filter(|model| !model.is_empty())
            .map(|model| Ok(ModelId::new(model)?))
            .collect()
    }

    /// What iq says of each model in use, `in_use` as the setting that
    /// names it and the model, that the allowed models leave out: the
    /// person set both, and only they can make them agree.
    pub(crate) fn disallowed(&self, in_use: &[(&str, String)]) -> Result<Vec<String>> {
        let allowed = self.allowed_models()?;
        if allowed.is_empty() {
            return Ok(Vec::new());
        }
        Ok(in_use
            .iter()
            .filter(|(_, model)| !allowed.iter().any(|a| a.as_str() == model))
            .map(|(key, model)| {
                format!(
                    "allowed_models leaves out {model}, which iq uses for `{key}`: allow it, or \
                     set `{key}` to one of them, in ~/.iq/settings.toml"
                )
            })
            .collect())
    }

    /// What a worktree starts from: as set, else `HEAD`.
    pub(crate) fn worktree_from(&self) -> &str {
        self.worktree_from.as_deref().map_or("HEAD", str::trim)
    }

    /// The commands run in a worktree as it is made, those that say
    /// something.
    pub(crate) fn worktree_setup(&self) -> Vec<String> {
        said(&self.worktree_setup)
    }

    /// The patterns of the files copied from the checkout into a worktree
    /// as it is made, those that say something.
    pub(crate) fn worktree_copy(&self) -> Vec<String> {
        said(&self.worktree_copy)
    }

    /// How long a chat may go untouched before it is forgotten, its
    /// worktree with it; `None` never.
    pub(crate) fn chat_keep(&self) -> Option<std::time::Duration> {
        let days = self.chat_keep.unwrap_or(CHAT_KEEP_DAYS);
        (days > 0).then(|| std::time::Duration::from_secs(days * 24 * 3600))
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
            reviewer: Some(ModelId::new(
                self.autopilot_model
                    .as_deref()
                    .or(self.reviewer.as_deref())
                    .unwrap_or(REVIEWER),
            )?),
            effort: self.effort,
            context_every: self.context_every.unwrap_or(iq_agent::CONTEXT_EVERY),
            refusals: self.autopilot_refusals.unwrap_or(iq_agent::REFUSALS),
            reviewer_effort: self
                .autopilot_effort
                .or(self.reviewer_effort)
                .unwrap_or(Effort::Low),
            command_timeout: self
                .command_timeout
                .map_or(iq_agent::COMMAND_TIMEOUT, std::time::Duration::from_secs),
            command_quiet: self
                .command_quiet
                .map_or(iq_agent::COMMAND_QUIET, std::time::Duration::from_secs),
            busy_tries: self.provider_retries.unwrap_or(iq_agent::BUSY_TRIES),
            // The cheap model that places the tasks names the chats.
            namer: Some(self.observing()?.model),
            light_from: self.light_from.unwrap_or(iq_agent::LIGHT_FROM),
            turn_ask: self
                .turn_cost_ask
                .filter(|dollars| *dollars > 0.0)
                .map(iq_core::Usd),
            allowed: self.allowed_models()?,
            scratch_most: self
                .scratch_mb
                .map_or(iq_agent::SCRATCH_MOST, |mb| mb.saturating_mul(1024 * 1024)),
            ..iq_agent::Settings::default()
        })
    }
}

/// The lines of `lines` that say something, trimmed.
fn said(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
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
    fn the_allowed_models_hold_the_crew_and_what_the_settings_use_beyond_them_is_said() {
        let set: Settings = toml::from_str(
            "model = \"lab/sage@eu\"\nallowed_models = [\"lab/sage@eu\", \" lab/quill \", \"\"]\nscratch_mb = 20",
        )
        .unwrap();
        let crew = set.crew().unwrap();
        let allowed: Vec<&str> = crew.allowed.iter().map(ModelId::as_str).collect();
        assert_eq!(allowed, ["lab/sage@eu", "lab/quill"]);
        assert_eq!(crew.scratch_most, 20 * 1024 * 1024);
        let said = set
            .disallowed(&[
                ("model", "lab/sage@eu".into()),
                ("autopilot_model", "anthropic/claude-haiku-5-5".into()),
            ])
            .unwrap();
        assert_eq!(said.len(), 1);
        assert!(
            said[0].starts_with(
                "allowed_models leaves out anthropic/claude-haiku-5-5, which iq uses for `autopilot_model`"
            ),
            "{}",
            said[0]
        );
        // Unset: any model, and the folder of scratch at its usual size.
        let any = Settings::default();
        assert!(any.summarizer().unwrap().is_none());
        let summed: Settings = toml::from_str("summarizer = \" lab/quick \"").unwrap();
        assert_eq!(summed.summarizer().unwrap().unwrap().as_str(), "lab/quick");
        assert!(any.crew().unwrap().allowed.is_empty());
        assert!(
            any.disallowed(&[("model", "x/y".into())])
                .unwrap()
                .is_empty()
        );
        assert_eq!(any.crew().unwrap().scratch_most, iq_agent::SCRATCH_MOST);
    }

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
        assert_eq!(set.worktree_from(), "HEAD");
        assert_eq!(set.chat_keep().unwrap().as_secs(), 30 * 24 * 3600);
        let apart: Settings = toml::from_str(
            "worktree = true\nworktree_from = \" origin/main \"\nworktree_setup = [\"uv sync\", \" \"]\nworktree_copy = [\" .env \", \"\"]\nchat_keep = 0",
        )
        .unwrap();
        assert!(apart.worktree);
        assert!(apart.plugins.is_empty());
        let plugged: Settings = toml::from_str("plugins = [\"ports\"]").unwrap();
        assert_eq!(plugged.plugins, ["ports"]);
        assert_eq!(apart.worktree_from(), "origin/main");
        assert_eq!(apart.worktree_setup(), ["uv sync"]);
        assert_eq!(apart.worktree_copy(), [".env"]);
        assert!(set.worktree_copy().is_empty());
        assert!(apart.chat_keep().is_none());
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
        let limits: Settings = toml::from_str(
            "reviewer_effort = \"high\"\ncommand_timeout = 1200\ncommand_quiet = 30\nprovider_retries = 2",
        )
        .unwrap();
        let crew = limits.crew().unwrap();
        assert_eq!(crew.reviewer_effort, Effort::High);
        assert_eq!(crew.command_timeout.as_secs(), 1200);
        assert_eq!(crew.command_quiet.as_secs(), 30);
        assert_eq!(crew.busy_tries, 2);
        assert_eq!(set.crew().unwrap().busy_tries, iq_agent::BUSY_TRIES);
        assert_eq!(set.crew().unwrap().reviewer_effort, Effort::Low);
        assert_eq!(set.crew().unwrap().command_quiet, iq_agent::COMMAND_QUIET);
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
            "budget = 2.0\nsteward = true\n[limits]\nrequesty_month = 20.0\n[assay]\ngrader = \"x/y\"\n[route]\noff = true",
        )
        .unwrap();
        assert_eq!(older.retired(), RETIRED);
    }

    #[test]
    fn the_autopilot_s_model_is_read_under_its_old_name_the_new_one_winning() {
        let old: Settings =
            toml::from_str("reviewer = \"lab/quill\"\nreviewer_effort = \"high\"").unwrap();
        let crew = old.crew().unwrap();
        assert_eq!(crew.reviewer.unwrap().as_str(), "lab/quill");
        assert_eq!(crew.reviewer_effort, Effort::High);
        assert_eq!(
            old.renamed(),
            [
                ("reviewer", "autopilot_model"),
                ("reviewer_effort", "autopilot_effort")
            ]
        );
        let both: Settings = toml::from_str(
            "reviewer = \"lab/quill\"\nautopilot_model = \"lab/sage\"\nautopilot_effort = \"medium\"",
        )
        .unwrap();
        let crew = both.crew().unwrap();
        assert_eq!(crew.reviewer.unwrap().as_str(), "lab/sage");
        assert_eq!(crew.reviewer_effort, Effort::Medium);
        assert_eq!(both.renamed(), [("reviewer", "autopilot_model")]);
        assert!(Settings::default().renamed().is_empty());
    }

    #[test]
    fn mermaid_is_off_unless_set_and_auto_or_a_command_naming_its_image() {
        assert_eq!(Settings::default().mermaid, None);
        let auto: Settings = toml::from_str("mermaid = \"auto\"").unwrap();
        assert_eq!(auto.mermaid, Some(Mermaid::Auto));
        let own: Settings = toml::from_str(r#"mermaid = ["draw", "{input}", "{output}"]"#).unwrap();
        assert_eq!(
            own.mermaid,
            Some(Mermaid::Command(vec![
                "draw".into(),
                "{input}".into(),
                "{output}".into()
            ]))
        );
        assert!(toml::from_str::<Settings>("mermaid = \"always\"").is_err());
        assert!(toml::from_str::<Settings>(r#"mermaid = ["draw", "{input}"]"#).is_err());
    }

    #[test]
    fn mcp_servers_are_read_their_secrets_from_the_environment_and_the_others_said() {
        let settings: Settings = toml::from_str(
            r#"
            [mcp.tracker]
            url = "https://mcp.example.com/mcp"
            headers = { Authorization = "TRACKER_AUTHORIZATION" }

            [mcp.docs]
            url = " https://docs.example/mcp "

            [mcp.notes]
            command = "notes-mcp"
            args = ["--folder", "~/notes"]

            [mcp.unset]
            url = "https://unset.example/mcp"
            headers = { Authorization = "NOT_SET" }

            [mcp.both]
            url = "https://both.example/mcp"
            command = "both"

            [mcp."bad.name"]
            url = "https://bad.example/mcp"
            "#,
        )
        .unwrap();
        let var =
            |name: &str| (name == "TRACKER_AUTHORIZATION").then(|| "Bearer s3cret".to_owned());
        let env = || vec![("PATH".into(), "/bin".into())];
        let (servers, wrong) = settings.mcp_servers(var, Path::new("/h"), env);
        let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["docs", "notes", "tracker"]);
        let lines: Vec<Option<&str>> = servers.iter().map(|s| s.line.as_deref()).collect();
        assert_eq!(lines, [None, Some("notes-mcp --folder /h/notes"), None]);
        // The header's value is read, and never shown.
        let tracker = format!("{:?}", servers[2].declared);
        assert!(
            tracker.contains("Authorization") && !tracker.contains("s3cret"),
            "{tracker}"
        );
        assert_eq!(
            wrong,
            [
                "the MCP server bad.name is not started: its name is letters, digits, hyphens and underscores",
                "the MCP server both is not started: give it a url or a command, not both",
                "the MCP server unset is not started: NOT_SET is not set",
            ]
        );
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
            [
                PathBuf::from("/h/.iq/skills"),
                PathBuf::from("/w/.claude/skills")
            ]
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
