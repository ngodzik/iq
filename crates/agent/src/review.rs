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
approve, so the worst it can do is lose the project's own work. A rule held it because it may be hard to undo. Approve it when it \
plausibly serves the agent's task and loses nothing the person would want kept (changes not \
committed, branches, stashes) that the task does not ask to lose. Refuse it otherwise, or when \
you cannot tell. Some actions reach the internet from the sandbox: the person's other files and \
secrets stay out of their reach, but what they send goes. Approve those that fetch: installing \
packages, cloning or fetching sources, reading documentation or an API. Refuse those that send \
the project's files or data away (an upload, a POST of a file, scp, rsync to a remote), and those \
that run code they download (curl ... | sh). Some actions run on the machine itself, outside \
the sandbox, with the person's own services and credentials, as if they typed them: the \
container daemon, a cluster, the forge's command line, a cloud's. Approve those that serve the \
task as the person's policy, when they wrote one, says: the project's own services started, \
its database, migrations and tests run against them, what reads a cluster or the forge. Refuse \
what the policy does not cover and you cannot tell, what destroys or changes what others share, \
what mounts or reads folders outside the project, privileged containers, images the project \
does not name, and anything that sends data away. Answer with APPROVE or REFUSE as the first \
word, then one short sentence why.";

/// What the reviewer is asked about one action: the task, the action,
/// where it runs (`place`), why it was held, and what the action does when
/// its name alone does not say (`does`): the recipes of a `make` target.
pub(crate) fn prompt(
    task: &str,
    action: &str,
    place: &str,
    reason: &str,
    does: Option<&str>,
) -> String {
    let does = match does {
        Some(does) => format!(
            "\n\nWhat the make targets do, from the project's Makefile, each with what it depends \
             on; judge the action by this, not by the targets' names:\n{does}"
        ),
        None => String::new(),
    };
    format!(
        "The agent's task:\n{task}\n\nThe action it wants to take:\n{action}\n\nWhere it runs: \
         {place}.\n\nWhy it was held: {reason}.{does}\n\nAPPROVE or REFUSE?"
    )
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
