//! The reviewer: a model that answers for the person, the autopilot being
//! on, what the gate asks. It sees only what stays within the sandbox
//! (the gate refuses it the rest), so the worst it lets through is work of
//! the project's own lost, never a secret sent away or code run outside.

/// The reviewer's standing instructions.
pub(crate) const INSTRUCTIONS: &str = "You review one action that an AI coding agent wants to \
take, on behalf of the person who owns the machine, who is away. The action runs in a sandbox: \
it can write only the project's folder, and what would reach beyond it is never yours to \
approve, so the worst it can do is lose the project's own work. A rule held it because it may be hard to undo. Approve it when it \
plausibly serves the agent's task and loses nothing the person would want kept (changes not \
committed, branches, stashes) that the task does not ask to lose. Refuse it otherwise, or when \
you cannot tell. Answer with APPROVE or REFUSE as the first word, then one short sentence why.";

/// What the reviewer is asked about one action.
pub(crate) fn prompt(task: &str, action: &str, reason: &str) -> String {
    format!(
        "The agent's task:\n{task}\n\nThe action it wants to take:\n{action}\n\nWhy it was held: \
         {reason}.\n\nAPPROVE or REFUSE?"
    )
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
