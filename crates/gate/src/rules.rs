//! Rules on commands: a prefix of words, and what to do with a command that
//! starts with it. Of the rules a command matches, the most restrictive
//! decides.
//!
//! ```toml
//! [[rule]]
//! prefix = ["git", ["push", "send-email"]]
//! decision = "ask"
//! network = true
//! reason = "it publishes commits"
//! match = ["git push origin main"]
//! not_match = ["git pull"]
//! ```
//!
//! A word of the prefix may be a list of words, any of which matches. The
//! examples are checked as the rules are read, so that a rule that does not
//! do what it says is refused rather than trusted.

use serde::{Deserialize, Serialize};

use crate::error::GateError;
use crate::shell;

/// What a rule decides, the least restrictive first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// It runs.
    Allow,
    /// It waits for approval.
    Ask,
    /// It does not run.
    Deny,
}

/// One word of a prefix: a word, or any of several.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Token {
    One(String),
    Any(Vec<String>),
}

impl Token {
    fn matches(&self, word: &str) -> bool {
        match self {
            Token::One(one) => one == word,
            Token::Any(any) => any.iter().any(|one| one == word),
        }
    }
}

/// A rule on the commands that start with its prefix.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rule {
    prefix: Vec<Token>,
    pub(crate) decision: Decision,
    #[serde(default)]
    reason: Option<String>,
    /// Whether the command needs the network: allowed, it gets it.
    #[serde(default)]
    pub(crate) network: bool,
    #[serde(default, rename = "match")]
    matching: Vec<String>,
    #[serde(default)]
    not_match: Vec<String>,
}

impl Rule {
    fn matches(&self, words: &[String]) -> bool {
        words.len() >= self.prefix.len()
            && self
                .prefix
                .iter()
                .zip(words)
                .all(|(token, word)| token.matches(word))
    }

    /// Why it decides as it does, its prefix when it does not say.
    pub(crate) fn reason(&self) -> String {
        self.reason
            .clone()
            .unwrap_or_else(|| format!("a rule holds `{}`", self.written()))
    }

    fn written(&self) -> String {
        self.prefix
            .iter()
            .map(|token| match token {
                Token::One(one) => one.clone(),
                Token::Any(any) => format!("[{}]", any.join("|")),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn check(&self) -> Result<(), GateError> {
        let any_matches = |line: &str| {
            shell::read(line)
                .commands
                .iter()
                .any(|words| self.matches(words))
        };
        let failed = |example: &String, problem| GateError::Example {
            prefix: self.written(),
            example: example.clone(),
            problem,
        };
        if let Some(example) = self.matching.iter().find(|line| !any_matches(line)) {
            return Err(failed(example, "should match and does not"));
        }
        if let Some(example) = self.not_match.iter().find(|line| any_matches(line)) {
            return Err(failed(example, "should not match and does"));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    rule: Vec<Rule>,
}

/// The rules every workspace starts with.
const DEFAULTS: &str = include_str!("default_rules.toml");

/// A set of rules.
#[derive(Debug, Clone, Default)]
pub struct Rules(Vec<Rule>);

impl Rules {
    /// Reads rules written as TOML, checking each rule's examples.
    ///
    /// # Examples
    ///
    /// ```
    /// let rules = iq_gate::Rules::parse(r#"
    ///     [[rule]]
    ///     prefix = ["terraform", "apply"]
    ///     decision = "deny"
    ///     match = ["terraform apply -auto-approve"]
    /// "#)?;
    /// # Ok::<(), iq_gate::GateError>(())
    /// ```
    pub fn parse(text: &str) -> Result<Self, GateError> {
        let file: File = toml::from_str(text).map_err(GateError::Rules)?;
        file.rule.iter().try_for_each(Rule::check)?;
        Ok(Self(file.rule))
    }

    /// The rules every workspace starts with: nothing run as another user,
    /// and what publishes or throws away work asked first.
    pub fn defaults() -> Self {
        // Checked by a test, so that they always parse.
        Self::parse(DEFAULTS).unwrap_or_default()
    }

    /// These rules and `more`.
    #[must_use]
    pub fn and(mut self, more: Rules) -> Self {
        self.0.extend(more.0);
        self
    }

    /// The most restrictive rule a command matches, if any.
    pub(crate) fn decide(&self, words: &[String]) -> Option<&Rule> {
        self.0
            .iter()
            .filter(|rule| rule.matches(words))
            .max_by_key(|rule| rule.decision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_owned).collect()
    }

    #[test]
    fn the_defaults_parse() {
        assert!(
            Rules::parse(DEFAULTS).is_ok(),
            "{:?}",
            Rules::parse(DEFAULTS).err()
        );
        assert!(!Rules::defaults().0.is_empty());
    }

    #[test]
    fn the_most_restrictive_rule_decides() {
        let rules = Rules::parse(
            r#"
            [[rule]]
            prefix = ["git"]
            decision = "allow"
            [[rule]]
            prefix = ["git", ["push", "send-email"]]
            decision = "ask"
            network = true
            "#,
        )
        .unwrap()
        .and(Rules::defaults());
        let decided = |line| rules.decide(&words(line)).map(|rule| rule.decision);
        assert_eq!(decided("git status"), Some(Decision::Allow));
        assert_eq!(decided("git push origin"), Some(Decision::Ask));
        assert_eq!(decided("git send-email x"), Some(Decision::Ask));
        assert_eq!(decided("sudo ls"), Some(Decision::Deny));
        assert_eq!(decided("cargo test"), None);
        assert!(rules.decide(&words("git push")).unwrap().network);
    }

    #[test]
    fn a_rule_whose_examples_fail_is_refused() {
        let wrong = Rules::parse(
            r#"
            [[rule]]
            prefix = ["git", "push"]
            decision = "ask"
            match = ["git pull"]
            "#,
        );
        assert!(
            matches!(wrong, Err(GateError::Example { ref example, .. }) if example == "git pull"),
            "{wrong:?}"
        );
        let unknown = Rules::parse("[[rule]]\nprefix = [\"x\"]\ndecision = \"maybe\"\n");
        assert!(matches!(unknown, Err(GateError::Rules(_))));
    }
}
