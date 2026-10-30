//! The reviewer: a model that answers for the person, the autopilot being
//! on, what the gate asks. It sees what stays within the sandbox, what only
//! reaches the internet when the person lets it, and what runs on the
//! machine itself when the person lets it, following the policy they
//! wrote for it (the gate refuses it the rest), so that what it lets
//! through is what the person would: never a secret, never a commit
//! published, never what changes what others share.

/// The reviewer's standing instructions.
pub(crate) const INSTRUCTIONS: &str = "You review one action that an AI coding agent wants to \
take, on behalf of the person who owns the machine, who is away. The action runs in a sandbox: \
it can write only the project's folder, and what would reach beyond it is never yours to \
approve, so the worst it can do is lose the project's own work. A rule held it because it may be hard to undo. You are told the \
agent's task as its chat started, what the person asked of it since, and its last actions with \
how they ended: the person's latest request is what the agent works on now. Approve the action \
when it plausibly serves that work and loses nothing the person would want kept (changes not \
committed, branches, stashes) that the work does not ask to lose. Refuse it otherwise, or when \
you cannot tell. Some actions reach the internet from the sandbox: the person's other files and \
secrets stay out of their reach, but what they send goes. Approve those that fetch: installing \
packages, cloning or fetching sources, reading documentation or an API. Refuse those that send \
the project's files or data away (an upload, a POST of a file, scp, rsync to a remote), and those \
that run code they download (curl ... | sh). Some actions run on the machine itself, outside \
the sandbox, with the person's own services and credentials, as if they typed them: the \
container daemon, a cluster, the forge's command line, a cloud's. Approve those that serve the \
work as the person's policy, when they wrote one, says (a policy may rest on the agent's last \
actions, which you are shown): the project's own services started, \
its database, migrations and tests run against them, what reads a cluster or the forge. Refuse \
what the policy does not cover and you cannot tell, what destroys or changes what others share, \
what mounts or reads folders outside the project, privileged containers, images the project \
does not name, and anything that sends data away. Answer with APPROVE or REFUSE as the first \
word, then one short sentence why.";

/// How much of each request the reviewer reads: enough to tell what is
/// asked, little enough that a pasted log does not drown the rest.
const REQUEST_CHARS: usize = 1500;

/// How much of a failed action's output it reads.
const ENDED_CHARS: usize = 160;

/// What the reviewer is told of an agent's work, beside the action.
#[derive(Debug, Clone, Default)]
pub(crate) struct Work {
    /// Its task, as its chat started.
    pub(crate) task: String,
    /// What the person asked of it since, the latest last.
    pub(crate) asked: Vec<String>,
    /// What started its turn under way.
    pub(crate) prompt: Option<String>,
    /// Its last actions and how they ended, the latest last.
    pub(crate) done: Vec<String>,
}

/// What the reviewer is asked about one action: the agent's work, the
/// action, where it runs (`place`), why it was held, and what the action
/// does when its name alone does not say (`does`): the recipes of a `make`
/// target.
pub(crate) fn prompt(
    work: &Work,
    action: &str,
    place: &str,
    reason: &str,
    does: Option<&str>,
) -> String {
    let mut text = format!(
        "The agent's task, as its chat started:\n{}",
        cut(&work.task, REQUEST_CHARS)
    );
    if !work.asked.is_empty() {
        text.push_str("\n\nWhat the person asked of it since, the latest last:");
        for asked in &work.asked {
            text.push_str(&format!("\n- {}", cut(asked, REQUEST_CHARS)));
        }
    }
    // What woke it, when it is neither its task nor the person's latest
    // request: a teammate's message.
    if let Some(prompt) = &work.prompt
        && *prompt != work.task
        && !work
            .asked
            .last()
            .is_some_and(|last| prompt.contains(last.as_str()))
    {
        text.push_str(&format!(
            "\n\nWhat started its turn under way:\n{}",
            cut(prompt, REQUEST_CHARS)
        ));
    }
    if !work.done.is_empty() {
        text.push_str("\n\nIts last actions and how they ended, the latest last:");
        for done in &work.done {
            text.push_str(&format!("\n- {done}"));
        }
    }
    text.push_str(&format!(
        "\n\nThe action it wants to take:\n{action}\n\nWhere it runs: {place}.\n\nWhy it was \
         held: {reason}."
    ));
    if let Some(does) = does {
        text.push_str(&format!(
            "\n\nWhat the make targets do, from the project's Makefile, each with what it depends \
             on; judge the action by this, not by the targets' names:\n{does}"
        ));
    }
    text.push_str("\n\nAPPROVE or REFUSE?");
    text
}

/// One of an agent's actions as the reviewer reads it, with how it ended:
/// the call of the tool `name` on `input`, which gave `output`, `ok` or
/// not. `None` for what only reads or speaks, which a policy does not rest
/// on.
pub(crate) fn done(name: &str, input: &str, output: &str, ok: bool) -> Option<String> {
    let what = match name {
        "run" => format!("ran `{input}`"),
        "write_file" => format!("wrote {input}"),
        "edit_file" => format!("edited {input}"),
        "start_agent" => format!("started the agent {input}"),
        "start_team" => format!("started the team {input}"),
        _ => return None,
    };
    let ended = if ok {
        // A command says how it ended on its last lines.
        output
            .lines()
            .rev()
            .find(|line| {
                ["[exit status", "[stopped", "[ended by"]
                    .iter()
                    .any(|start| line.starts_with(start))
            })
            .map_or("done".to_owned(), |line| {
                line.trim_matches(['[', ']']).to_owned()
            })
    } else {
        let said = output.strip_prefix("Error: ").unwrap_or(output);
        format!(
            "failed: {}",
            cut(said.lines().next().unwrap_or_default(), ENDED_CHARS)
        )
    };
    Some(format!("{what}: {ended}"))
}

/// `text`, cut to `most` characters.
fn cut(text: &str, most: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= most {
        return text.to_owned();
    }
    let kept: String = text.chars().take(most).collect();
    format!("{kept}…")
}

/// Where an action of `scope` runs, for the reviewer.
pub(crate) fn place(scope: iq_gate::Scope) -> &'static str {
    match scope {
        iq_gate::Scope::Sandbox => "in the sandbox, which holds it to the project's folder",
        iq_gate::Scope::Internet => "in the sandbox, with the internet",
        iq_gate::Scope::Host => {
            "on the machine itself, outside the sandbox, with the person's own services and \
             credentials"
        }
        iq_gate::Scope::Beyond => "beyond the sandbox, which only the person may allow",
    }
}

/// The reviewer's answer: approved or not, and why. Anything but a clear
/// approval refuses.
pub(crate) fn verdict(answer: &str) -> (bool, String) {
    let answer = answer.trim();
    let rest = answer.trim_start_matches(|c: char| !c.is_alphabetic());
    let first: String = rest.chars().take_while(|c| c.is_alphabetic()).collect();
    let approved = first.eq_ignore_ascii_case("approve");
    if !approved && !first.eq_ignore_ascii_case("refuse") {
        return (false, format!("its answer was not clear: {answer}"));
    }
    let why = rest[first.len()..]
        .trim_start_matches([':', '.', ',', '-', ' ', '*', '\n'])
        .lines()
        .next()
        .unwrap_or_default()
        .trim();
    let why = if why.is_empty() {
        "no reason given"
    } else {
        why
    };
    (approved, why.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reviewer_reads_the_latest_request_and_what_was_just_done() {
        let work = Work {
            task: "is this a new chat, with context already?".to_owned(),
            asked: vec!["code the first option".to_owned()],
            prompt: Some("Message from person: code the first option".to_owned()),
            done: vec![
                done("run", "make session-check", "ok\n[exit status 0]", true).unwrap(),
                done(
                    "run",
                    "make up-db",
                    "Error: the reviewer refused it: no check",
                    false,
                )
                .unwrap(),
            ],
        };
        let asked = prompt(&work, "run `uv sync --frozen`", "here", "it may", None);
        assert!(
            asked.contains(
                "What the person asked of it since, the latest last:\n- code the first option"
            ),
            "{asked}"
        );
        // The turn the person's request started is not said twice.
        assert!(!asked.contains("What started its turn"), "{asked}");
        assert!(
            asked.contains(
                "Its last actions and how they ended, the latest last:\n- ran `make \
                 session-check`: exit status 0\n- ran `make up-db`: failed: the reviewer refused \
                 it: no check"
            ),
            "{asked}"
        );
        // A teammate's message that woke it is said.
        let woken = Work {
            prompt: Some("Message from lead: migrate the db".to_owned()),
            ..work
        };
        let asked = prompt(&woken, "run x", "here", "it may", None);
        assert!(
            asked.contains("What started its turn under way:\nMessage from lead: migrate the db"),
            "{asked}"
        );
        // What only reads is not an action it rests on.
        assert!(done("read_file", "src/x.py", "1\tx", true).is_none());
        assert_eq!(
            done("edit_file", "src/x.py", "Edited.", true).as_deref(),
            Some("edited src/x.py: done")
        );
    }

    #[test]
    fn only_a_clear_approval_approves() {
        assert_eq!(
            verdict("APPROVE: the task asks to start over."),
            (true, "the task asks to start over.".to_owned())
        );
        assert_eq!(
            verdict("**REFUSE** - it drops uncommitted work"),
            (false, "it drops uncommitted work".to_owned())
        );
        assert_eq!(verdict("approve"), (true, "no reason given".to_owned()));
        let (approved, why) = verdict("I think it is fine");
        assert!(!approved);
        assert!(why.contains("not clear"), "{why}");
        assert!(!verdict("").0);
    }
}
