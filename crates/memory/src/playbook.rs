//! The person's playbooks: the steps of a recurring task, written beside a
//! skill (`playbook.toml` in its folder), run by iq rather than read by a
//! lead. The lead only recognises the request and starts the playbook with
//! its arguments; which steps, on which models and efforts, how many agents
//! at once and in what order are the playbook's, so that no lead skips,
//! reorders or runs them on another model. A step runs a command, judged by
//! the gate as the lead's own and run in the sandbox, or starts agents
//! whose answers come back to the playbook, never to the lead, which gets
//! the last step's answer. A skill without a playbook works as before.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};

use iq_agent::{Called, Crew, Extension};
use iq_core::{ChatModel, Effort, ModelId, ToolSpec};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::skills::{Skill, Skills};
use crate::slug;

/// The file a skill's folder holds its playbook in.
pub(crate) const PLAYBOOK_FILE: &str = "playbook.toml";

/// The most characters of what an earlier step gave put in a brief.
const GIVEN_MOST: usize = 40_000;

/// The most characters of its last step's answer a playbook gives back to
/// the lead: the whole is in its file.
const ANSWER_MOST: usize = 20_000;

/// The most agents one step runs at once, unless it says.
const PARALLEL: usize = 6;

/// A playbook, as its file writes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Playbook {
    /// What it does, in a line, for the lead to know when it serves.
    #[serde(default)]
    pub description: String,
    /// The arguments it is started with, by name.
    #[serde(default)]
    pub args: Vec<String>,
    /// Its steps, in order.
    #[serde(rename = "step", default)]
    pub steps: Vec<Step>,
}

/// One step of a playbook: a command, or agents on a model.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Its name, which later steps get what it gave by.
    pub name: String,
    /// The command it runs, in the workspace: `{arg}` an argument, quoted;
    /// `{step}` the file of what an earlier step gave; `{playbook}` the
    /// folder of the playbook's files; a first word naming one of those
    /// files, that file.
    #[serde(default)]
    pub run: Option<String>,
    /// Whether its command reaches the internet.
    #[serde(default)]
    pub internet: bool,
    /// Whether its command runs on the machine itself, outside the sandbox.
    #[serde(default)]
    pub host: bool,
    /// The model its agents run on.
    #[serde(default)]
    pub model: Option<String>,
    /// How hard they think; as the settings say when none.
    #[serde(default)]
    pub effort: Option<String>,
    /// The file of the playbook's folder its agents are given as their
    /// task: `{arg}` an argument, `{part}` the part of one of several.
    #[serde(default)]
    pub brief: Option<String>,
    /// The earlier steps whose answers its agents are given, after their
    /// brief.
    #[serde(default)]
    pub gets: Vec<String>,
    /// An earlier step listing parts, a line each: one agent per part.
    #[serde(default)]
    pub each: Option<String>,
    /// The most of its agents at once.
    #[serde(default)]
    pub parallel: Option<usize>,
}

impl Playbook {
    /// The playbook of the skill whose folder is `folder`, if it has one.
    ///
    /// # Errors
    ///
    /// Its file cannot be read, or says what no playbook may.
    pub fn read(folder: &Path) -> Result<Option<Playbook>, String> {
        let file = folder.join(PLAYBOOK_FILE);
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("{}: {error}", file.display())),
        };
        let playbook: Playbook =
            toml::from_str(&text).map_err(|error| format!("{}: {error}", file.display()))?;
        playbook.checked().map(Some)
    }

    /// The playbook, once it is one that can run: steps named apart, each
    /// a command or agents on a model with a brief, getting only what
    /// steps before it gave.
    fn checked(self) -> Result<Playbook, String> {
        if self.steps.is_empty() {
            return Err("a playbook has steps: [[step]]".into());
        }
        let mut before: Vec<&str> = Vec::new();
        for step in &self.steps {
            let name = step.name.as_str();
            if name.is_empty() || before.contains(&name) || self.args.iter().any(|a| a == name) {
                return Err(format!(
                    "a step is named {name:?}, as none or another is: each its own"
                ));
            }
            match (&step.run, &step.model) {
                (Some(_), None) => {}
                (None, Some(model)) => {
                    ModelId::new(model.as_str()).map_err(|e| format!("step {name}: {e}"))?;
                    if step.brief.is_none() {
                        return Err(format!("step {name} runs agents: it needs a brief"));
                    }
                    if let Some(effort) = &step.effort {
                        effort
                            .parse::<Effort>()
                            .map_err(|e| format!("step {name}: {e}"))?;
                    }
                }
                _ => {
                    return Err(format!(
                        "step {name} either runs a command (run) or agents (model)"
                    ));
                }
            }
            for got in step.gets.iter().chain(&step.each) {
                if !before.contains(&got.as_str()) {
                    return Err(format!(
                        "step {name} gets {got:?}, which is no step before it"
                    ));
                }
            }
            before.push(name);
        }
        Ok(self)
    }
}

/// What runs a playbook's steps: the crew, or what stands for it in a test.
pub(crate) trait Hands: Send + Sync {
    /// Runs the command `line` for `lead`, as the lead's own: its output
    /// and whether it succeeded.
    fn command(
        &self,
        lead: &str,
        line: String,
        internet: bool,
        host: bool,
    ) -> impl Future<Output = Result<(String, bool), String>> + Send;

    /// Starts an agent `name` for `lead` on `model` with `task`, and gives
    /// its answer.
    fn agent(
        &self,
        lead: &str,
        name: &str,
        model: &ModelId,
        task: String,
        effort: Option<Effort>,
        how: &str,
    ) -> impl Future<Output = Result<String, String>> + Send;

    /// Says `text` in `lead`'s chat.
    fn say(&self, lead: &str, text: String);
}

impl<M> Hands for Crew<M>
where
    M: ChatModel + 'static,
{
    async fn command(
        &self,
        lead: &str,
        line: String,
        internet: bool,
        host: bool,
    ) -> Result<(String, bool), String> {
        self.run_command(lead, line, internet, host).await
    }

    async fn agent(
        &self,
        lead: &str,
        name: &str,
        model: &ModelId,
        task: String,
        effort: Option<Effort>,
        how: &str,
    ) -> Result<String, String> {
        let role = Some(format!("{name}, a step of a playbook: do this step alone"));
        self.ask_agent(lead, name, model, task, role, effort, how)
            .await
    }

    fn say(&self, lead: &str, text: String) {
        self.notice(lead, text);
    }
}

/// Runs `playbook`, the playbook of the skill `skill`, whose files are in
/// `home`, for `lead` with `args`, through `hands`: each step in turn, what
/// each gave written in `home` as `<step>.txt`. Gives the last step's
/// answer.
///
/// # Errors
///
/// An argument is missing or unknown, or a step failed: what the steps
/// before it gave stays in `home`.
pub(crate) async fn play(
    hands: &impl Hands,
    lead: &str,
    skill: &str,
    playbook: &Playbook,
    home: &Path,
    args: &HashMap<String, String>,
) -> Result<String, String> {
    for name in &playbook.args {
        if !args.contains_key(name) {
            return Err(format!("the playbook {skill} needs the argument {name}"));
        }
    }
    if let Some(other) = args.keys().find(|name| !playbook.args.contains(name)) {
        return Err(format!(
            "the playbook {skill} takes no argument {other}: it takes {}",
            playbook.args.join(", ")
        ));
    }
    let how = format!("the playbook {skill}");
    let mut gave: HashMap<String, String> = HashMap::new();
    let mut last = String::new();
    for step in &playbook.steps {
        hands.say(lead, format!("playbook {skill}: {}", step.name));
        let given = match (&step.run, &step.model) {
            (Some(run), _) => {
                let line = command_line(run, args, &gave, home)?;
                let (output, ok) = hands.command(lead, line, step.internet, step.host).await?;
                if !ok {
                    return Err(stopped(skill, &step.name, home, &output));
                }
                output
            }
            (None, Some(model)) => {
                let model = ModelId::new(model.as_str()).map_err(|e| e.to_string())?;
                let effort = step
                    .effort
                    .as_deref()
                    .map(str::parse::<Effort>)
                    .transpose()?;
                let brief_file = home.join(step.brief.as_deref().unwrap_or_default());
                let brief = std::fs::read_to_string(&brief_file)
                    .map_err(|e| format!("{}: {e}", brief_file.display()))?;
                let parts: Vec<Option<String>> = match &step.each {
                    Some(listed) => gave
                        .get(listed)
                        .map(|text| {
                            text.lines()
                                .map(str::trim)
                                .filter(|line| !line.is_empty())
                                .map(|line| Some(line.to_owned()))
                                .collect()
                        })
                        .unwrap_or_default(),
                    None => vec![None],
                };
                let at_once = step.parallel.unwrap_or(PARALLEL).max(1);
                let mut answers: Vec<String> = Vec::new();
                for (chunk, group) in parts.chunks(at_once).enumerate() {
                    let asked = group.iter().enumerate().map(|(at, part)| {
                        let task = briefed(&brief, args, part.as_deref(), &step.gets, &gave);
                        let name = if step.each.is_some() {
                            format!("{}-{}", step.name, chunk * at_once + at + 1)
                        } else {
                            step.name.clone()
                        };
                        let (model, how) = (&model, &how);
                        async move { hands.agent(lead, &name, model, task, effort, how).await }
                    });
                    for (answer, part) in join_all(asked).await.into_iter().zip(group) {
                        let answer =
                            answer.map_err(|why| stopped(skill, &step.name, home, &why))?;
                        answers.push(match part {
                            Some(part) => format!("## {part}\n\n{answer}"),
                            None => answer,
                        });
                    }
                }
                answers.join("\n\n")
            }
            (None, None) => return Err(format!("step {} has nothing to do", step.name)),
        };
        let file = home.join(format!("{}.txt", step.name));
        std::fs::write(&file, &given).map_err(|e| format!("{}: {e}", file.display()))?;
        gave.insert(step.name.clone(), given.clone());
        last = given;
    }
    let step = playbook.steps.last().map_or("", |step| step.name.as_str());
    Ok(format!(
        "The playbook {skill} ran its {} steps; its last, {step}, gave what follows. What each \
         gave is in {}.\n\n{}",
        playbook.steps.len(),
        home.display(),
        cut(&last, ANSWER_MOST)
    ))
}

/// Runs every future of `futures` at once, and gives what each gave, in
/// their order.
async fn join_all<T>(futures: impl IntoIterator<Item = impl Future<Output = T>>) -> Vec<T> {
    let futures: Vec<_> = futures.into_iter().map(Box::pin).collect();
    let mut done: Vec<Option<T>> = futures.iter().map(|_| None).collect();
    let mut pending: Vec<(usize, _)> = futures.into_iter().enumerate().collect();
    while !pending.is_empty() {
        let mut ready = None;
        std::future::poll_fn(|cx| {
            for (at, (index, future)) in pending.iter_mut().enumerate() {
                if let std::task::Poll::Ready(value) = future.as_mut().poll(cx) {
                    ready = Some((at, *index, value));
                    return std::task::Poll::Ready(());
                }
            }
            std::task::Poll::Pending
        })
        .await;
        if let Some((at, index, value)) = ready {
            pending.remove(at);
            done[index] = Some(value);
        }
    }
    done.into_iter().flatten().collect()
}

/// Why a playbook stopped at `step`, with `said`, what it gave or why it
/// failed.
fn stopped(skill: &str, step: &str, home: &Path, said: &str) -> String {
    format!(
        "the playbook {skill} stopped at its step {step}; what the steps before gave is in {}:\n{}",
        home.display(),
        cut(said, GIVEN_MOST)
    )
}

/// The command line `run` stands for: its placeholders filled, each value
/// quoted for the shell, so that what a lead gives as an argument is never
/// read as more of the line.
fn command_line(
    run: &str,
    args: &HashMap<String, String>,
    gave: &HashMap<String, String>,
    home: &Path,
) -> Result<String, String> {
    let mut line = String::new();
    let mut rest = run;
    while let Some(open) = rest.find('{') {
        let (before, after) = rest.split_at(open);
        line.push_str(before);
        let name_end = after.find('}');
        let name = name_end.map(|end| &after[1..end]);
        let named = name.filter(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && !before.ends_with('$')
        });
        let (Some(name), Some(end)) = (named, name_end) else {
            // A brace of the shell's own: `${VAR}`, `{a,b}`.
            line.push('{');
            rest = &after[1..];
            continue;
        };
        let value = if let Some(value) = args.get(name) {
            value.clone()
        } else if gave.contains_key(name) {
            home.join(format!("{name}.txt")).display().to_string()
        } else if name == "playbook" {
            home.display().to_string()
        } else {
            return Err(format!(
                "{{{name}}} in `{run}` is no argument, no step before it, nor `playbook`"
            ));
        };
        line.push_str(&quoted(&value));
        rest = &after[end + 1..];
    }
    line.push_str(rest);
    // A first word naming a file of the playbook's: that file.
    let first = line
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned();
    if !first.is_empty() && !first.contains('/') && home.join(&first).is_file() {
        let path = quoted(&home.join(&first).display().to_string());
        line = line.replacen(&first, &path, 1);
    }
    Ok(line)
}

/// `value` as one word for the shell, whatever it holds.
fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The task of an agent of a step: `brief`, its arguments and `part`
/// filled in, then what the steps it `gets` gave.
fn briefed(
    brief: &str,
    args: &HashMap<String, String>,
    part: Option<&str>,
    gets: &[String],
    gave: &HashMap<String, String>,
) -> String {
    let mut task = brief.to_owned();
    for (name, value) in args {
        task = task.replace(&format!("{{{name}}}"), value);
    }
    if let Some(part) = part {
        task = task.replace("{part}", part);
    }
    for got in gets {
        if let Some(given) = gave.get(got) {
            task.push_str(&format!(
                "\n\n## What the step {got} gave\n\n{}",
                cut(given, GIVEN_MOST)
            ));
        }
    }
    task
}

/// `text`, or its first `most` characters and how many it leaves out.
fn cut(text: &str, most: usize) -> String {
    match text.char_indices().nth(most) {
        Some((at, _)) => format!(
            "{}\n[{} more characters left out]",
            &text[..at],
            text[at..].chars().count()
        ),
        None => text.to_owned(),
    }
}

/// A skill's playbook, with the folder its files are in, or why it cannot
/// be read.
pub type Found = Result<(Playbook, PathBuf), String>;

/// The playbooks of the person's skills, offered to the leads of their
/// chats as one tool, which runs them.
#[derive(Debug, Clone)]
pub struct Playbooks {
    skills: Skills,
}

impl Playbooks {
    /// The playbooks beside `skills`.
    #[must_use]
    pub fn new(skills: Skills) -> Self {
        Self { skills }
    }

    /// The skills that have a playbook, with it, and the folder its files
    /// are in. One that cannot be read is listed with why.
    #[must_use]
    pub fn found(&self) -> Vec<(Skill, Found)> {
        self.skills
            .list()
            .into_iter()
            .filter_map(|skill| {
                let folder = skill.folder()?;
                match Playbook::read(&folder) {
                    Ok(Some(playbook)) => Some((skill, Ok((playbook, folder)))),
                    Ok(None) => None,
                    Err(why) => Some((skill, Err(why))),
                }
            })
            .collect()
    }

    /// Whether `agent` runs playbooks: the lead of a chat of the person's,
    /// never an agent another started, nor one a playbook started.
    fn serves(agent: &str) -> bool {
        !agent.contains('/')
    }
}

/// A copy of the files of `folder`, a skill's, in `home`: what the sandbox
/// reads there, where the person's own folder of skills it may not.
fn copied(folder: &Path, home: &Path) -> Result<(), String> {
    std::fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
    let entries = std::fs::read_dir(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let to = home.join(entry.file_name());
            std::fs::copy(&path, &to).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

impl<M> Extension<M> for Playbooks
where
    M: ChatModel + 'static,
{
    fn tools(&self, agent: &str) -> Vec<ToolSpec> {
        if !Self::serves(agent) || self.found().is_empty() {
            return Vec::new();
        }
        vec![ToolSpec {
            name: "playbook".into(),
            description: "Runs one of the person's playbooks: the steps of a recurring task, \
                          beside one of their skills, which iq runs itself, each on the model \
                          and the effort the playbook says, the steps without a model as \
                          commands. When a request is what a playbook is for, start it rather \
                          than doing its steps yourself: you get its last step's answer."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "skill": {"type": "string", "description": "The skill whose playbook runs."},
                    "args": {"type": "object", "description": "Its arguments, by name, each a string.", "additionalProperties": {"type": "string"}},
                },
                "required": ["skill"],
            }),
        }]
    }

    fn instructions(&self, agent: &str) -> Option<String> {
        if !Self::serves(agent) {
            return None;
        }
        let found = self.found();
        if found.is_empty() {
            return None;
        }
        let mut out = "Playbooks, which iq runs step by step with `playbook`: when a request is \
                       one a playbook is for, start it with its arguments rather than following \
                       its skill yourself."
            .to_owned();
        for (skill, playbook) in &found {
            match playbook {
                Ok((playbook, _)) => {
                    let what = if playbook.description.is_empty() {
                        skill.description.as_str()
                    } else {
                        playbook.description.as_str()
                    };
                    out.push_str(&format!("\n- {}: {what}", skill.name));
                    if !playbook.args.is_empty() {
                        out.push_str(&format!(" (args: {})", playbook.args.join(", ")));
                    }
                }
                Err(why) => out.push_str(&format!("\n- {}: cannot run, {why}", skill.name)),
            }
        }
        Some(out)
    }

    fn call(&self, crew: Crew<M>, agent: String, name: String, args: Value) -> Called {
        let found = self.found();
        Box::pin(async move {
            if name != "playbook" || !Self::serves(&agent) {
                return Err(format!("no tool named {name:?} is offered to {agent}"));
            }
            let wanted = args["skill"].as_str().unwrap_or_default();
            let Some((skill, playbook)) = found
                .into_iter()
                .find(|(skill, _)| slug(&skill.name) == slug(wanted))
            else {
                return Err(format!("no skill named {wanted:?} has a playbook"));
            };
            let (playbook, folder) = playbook?;
            let given: HashMap<String, String> = args["args"]
                .as_object()
                .map(|args| {
                    args.iter()
                        .map(|(name, value)| {
                            let value = value
                                .as_str()
                                .map_or_else(|| value.to_string(), str::to_owned);
                            (name.clone(), value)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let scratch = crew.scratch_of(&agent).ok_or(
                "iq keeps no folder of scratch here, where a playbook's files are copied to run",
            )?;
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let home = scratch
                .join("playbooks")
                .join(format!("{}-{stamp}", slug(&skill.name)));
            copied(&folder, &home)?;
            play(&crew, &agent, &skill.name, &playbook, &home, &given).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Stands for the crew: answers each command and agent from what it was
    /// given, and keeps what it was asked.
    #[derive(Default)]
    struct Fake {
        asked: Mutex<Vec<String>>,
        failing: Option<&'static str>,
    }

    impl Hands for Fake {
        async fn command(
            &self,
            _lead: &str,
            line: String,
            _internet: bool,
            _host: bool,
        ) -> Result<(String, bool), String> {
            self.asked.lock().unwrap().push(format!("run {line}"));
            let ok = self.failing.is_none_or(|word| !line.contains(word));
            Ok((format!("part one\npart two\n(from {line})"), ok))
        }

        async fn agent(
            &self,
            _lead: &str,
            name: &str,
            model: &ModelId,
            task: String,
            effort: Option<Effort>,
            how: &str,
        ) -> Result<String, String> {
            let effort = effort.map_or("as set", Effort::as_str);
            self.asked
                .lock()
                .unwrap()
                .push(format!("{name} on {model} ({effort}, {how}): {task}"));
            Ok(format!("{name} answered"))
        }

        fn say(&self, _lead: &str, _text: String) {}
    }

    fn playbook(text: &str) -> Result<Playbook, String> {
        toml::from_str::<Playbook>(text)
            .map_err(|e| e.to_string())
            .and_then(Playbook::checked)
    }

    #[test]
    fn a_playbook_is_checked_before_it_runs() {
        let fine = playbook(
            "args = [\"run\"]\n[[step]]\nname = \"logs\"\nrun = \"logs.py {run}\"\n\
             [[step]]\nname = \"diagnosis\"\nmodel = \"lab/mid\"\neffort = \"medium\"\n\
             brief = \"diagnose.md\"\ngets = [\"logs\"]",
        );
        assert!(fine.is_ok(), "{fine:?}");
        for (wrong, said) in [
            ("[[step]]\nname = \"a\"", "either runs a command"),
            ("[[step]]\nname = \"a\"\nmodel = \"lab/m\"", "needs a brief"),
            (
                "[[step]]\nname = \"a\"\nrun = \"x\"\ngets = [\"b\"]",
                "no step before it",
            ),
            (
                "[[step]]\nname = \"a\"\nrun = \"x\"\n[[step]]\nname = \"a\"\nrun = \"y\"",
                "each its own",
            ),
            ("description = \"x\"", "has steps"),
        ] {
            let error = playbook(wrong).unwrap_err();
            assert!(error.contains(said), "{wrong}: {error}");
        }
        // A key no playbook knows is said, rather than left aside.
        assert!(playbook("[[step]]\nname = \"a\"\nrun = \"x\"\nmodels = \"m\"").is_err());
    }

    #[test]
    fn an_argument_is_one_word_whatever_it_holds_and_a_step_its_file() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("logs.py"), "print()").unwrap();
        let args: HashMap<String, String> =
            [("run".to_owned(), "42; rm -rf ~ 'x'".to_owned())].into();
        let gave: HashMap<String, String> = [("logs".to_owned(), "...".to_owned())].into();
        let line = command_line(
            "logs.py {run} {logs} ${HOME} {a,b}",
            &args,
            &gave,
            home.path(),
        )
        .unwrap();
        let script = home.path().join("logs.py").display().to_string();
        let logs = home.path().join("logs.txt").display().to_string();
        assert_eq!(
            line,
            format!("'{script}' '42; rm -rf ~ '\\''x'\\''' '{logs}' ${{HOME}} {{a,b}}")
        );
        assert!(command_line("x {nothing}", &args, &gave, home.path()).is_err());
    }

    #[tokio::test]
    async fn the_steps_run_in_order_their_agents_on_the_playbook_s_models() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("read.md"),
            "Read {part} of change {change}.",
        )
        .unwrap();
        std::fs::write(home.path().join("judge.md"), "Judge change {change}.").unwrap();
        let book = playbook(
            "args = [\"change\"]\n\
             [[step]]\nname = \"parts\"\nrun = \"split.sh {change}\"\n\
             [[step]]\nname = \"readers\"\nmodel = \"lab/cheap\"\nbrief = \"read.md\"\neach = \"parts\"\nparallel = 2\n\
             [[step]]\nname = \"judge\"\nmodel = \"lab/strong\"\neffort = \"high\"\nbrief = \"judge.md\"\ngets = [\"readers\"]",
        )
        .unwrap();
        let fake = Fake::default();
        let args: HashMap<String, String> = [("change".to_owned(), "7".to_owned())].into();
        let answer = play(&fake, "lead", "review", &book, home.path(), &args)
            .await
            .unwrap();
        assert!(answer.contains("its last, judge, gave"), "{answer}");
        assert!(answer.ends_with("judge answered"), "{answer}");
        let asked = fake.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1 + 3 + 1, "{asked:#?}");
        assert_eq!(asked[0], "run split.sh '7'");
        // One reader per part the first step listed, on the cheap model.
        assert!(asked[1].starts_with(
            "readers-1 on lab/cheap (as set, the playbook review): Read part one of change 7."
        ));
        assert!(asked[3].starts_with("readers-3 on lab/cheap"));
        // The judge, on the strong model, given what the readers gave.
        assert!(
            asked[4]
                .starts_with("judge on lab/strong (high, the playbook review): Judge change 7.")
        );
        assert!(
            asked[4].contains("## What the step readers gave"),
            "{}",
            asked[4]
        );
        assert!(
            asked[4].contains("## part two\n\nreaders-2 answered"),
            "{}",
            asked[4]
        );
        // What each gave is kept.
        assert!(home.path().join("readers.txt").is_file());
        // An argument missing, or a step failing, stops it.
        assert!(
            play(&fake, "lead", "review", &book, home.path(), &HashMap::new())
                .await
                .is_err()
        );
        let failing = Fake {
            failing: Some("split"),
            ..Fake::default()
        };
        let why = play(&failing, "lead", "review", &book, home.path(), &args)
            .await
            .unwrap_err();
        assert!(why.contains("stopped at its step parts"), "{why}");
        assert_eq!(failing.asked.lock().unwrap().len(), 1);
    }
}
