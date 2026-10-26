//! Who does an exercise: one model in iq's loop, or a team of them working
//! in one of the crew's patterns.

use std::fmt;
use std::str::FromStr;

use iq_agent::Pattern;
use iq_core::ModelId;
use serde::{Deserialize, Serialize};

/// The kinds of candidates, kept apart in the results: a model is tested
/// alone in iq's loop, and a team is several agents. Set side by side, two
/// kinds would compare two things at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A model of a provider, in iq's own loop with its tools.
    Model,
    /// Agents working in one of the crew's patterns.
    Team,
}

impl Kind {
    /// Every kind, in the order the results give them.
    pub const ALL: [Kind; 2] = [Kind::Model, Kind::Team];

    /// Its name in the settings, plural as a list of them reads.
    #[must_use]
    pub fn plural(self) -> &'static str {
        match self {
            Kind::Model => "models",
            Kind::Team => "teams",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Model => "model",
            Kind::Team => "team",
        })
    }
}

/// One candidate: a pattern and the models it works with, the first
/// leading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Written", into = "Written")]
pub struct Candidate {
    /// How it works.
    pub pattern: Pattern,
    /// Its models: one alone; a writer and its reviewer; a lead and the
    /// cheaper ones it may give its agents; the voices of a debate.
    pub models: Vec<ModelId>,
}

/// A candidate as a record writes it.
#[derive(Serialize, Deserialize)]
struct Written {
    pattern: String,
    models: Vec<String>,
}

impl TryFrom<Written> for Candidate {
    type Error = String;

    fn try_from(written: Written) -> Result<Self, String> {
        let models = written
            .models
            .into_iter()
            .map(|m| ModelId::new(m).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        if models.is_empty() {
            return Err("a candidate has a model at least".into());
        }
        Ok(Self {
            pattern: written.pattern.parse()?,
            models,
        })
    }
}

impl From<Candidate> for Written {
    fn from(candidate: Candidate) -> Self {
        Self {
            pattern: candidate.pattern.name().to_owned(),
            models: candidate.models.iter().map(ToString::to_string).collect(),
        }
    }
}

impl Candidate {
    /// `model` alone.
    #[must_use]
    pub fn solo(model: ModelId) -> Self {
        Self {
            pattern: Pattern::Solo,
            models: vec![model],
        }
    }

    /// Its kind.
    #[must_use]
    pub fn kind(&self) -> Kind {
        match (self.pattern, self.models.as_slice()) {
            (Pattern::Solo, [_]) => Kind::Model,
            _ => Kind::Team,
        }
    }

    /// The model that leads it.
    #[must_use]
    pub fn lead(&self) -> &ModelId {
        &self.models[0]
    }

    /// How many agents it starts at the least, for guessing what it
    /// spends: a lead starts its own, two at a guess.
    #[must_use]
    pub fn agents(&self) -> usize {
        match self.pattern {
            Pattern::Solo => 1,
            Pattern::Pair => 2,
            Pattern::Lead => 3,
            Pattern::Debate => self.models.len().max(2),
        }
    }
}

impl fmt::Display for Candidate {
    /// `glm-5`, `pair: glm-5 + gpt-5-mini`: as it is written to be read
    /// back.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let models: Vec<&str> = self.models.iter().map(ModelId::as_str).collect();
        match self.pattern {
            Pattern::Solo => f.write_str(models[0]),
            pattern => write!(f, "{pattern}: {}", models.join(" + ")),
        }
    }
}

impl FromStr for Candidate {
    type Err = String;

    /// A model alone, or a pattern and its models joined by `+`:
    /// `openai/gpt-5-mini`, `pair: z-ai/glm-5 + openai/gpt-5-mini`.
    fn from_str(text: &str) -> Result<Self, String> {
        let text = text.trim();
        // A model's identifier holds no space or `:` before a `+`, but a
        // provider's may hold a `:`, as `qwen/qwen3:free`: the pattern is
        // read only when its name is one.
        let (pattern, models) = match text.split_once(':') {
            Some((head, rest)) if head.trim().parse::<Pattern>().is_ok() => {
                (head.trim().parse::<Pattern>()?, rest)
            }
            _ => (Pattern::Solo, text),
        };
        let models = models
            .split('+')
            .map(|m| ModelId::new(m.trim()).map_err(|e| format!("in `{text}`: {e}")))
            .collect::<Result<Vec<_>, _>>()?;
        if pattern == Pattern::Solo && models.len() > 1 {
            return Err(format!(
                "`{text}` names several models: say how they work together, as `pair: a + b`"
            ));
        }
        if pattern == Pattern::Pair && models.len() != 2 {
            return Err(format!(
                "`{text}`: a pair is a writer and a reviewer, two models"
            ));
        }
        Ok(Self { pattern, models })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_read_back_as_they_are_written() {
        for text in [
            "openai/gpt-5-mini",
            "anthropic/claude-haiku-5-5",
            "pair: z-ai/glm-5 + openai/gpt-5-mini",
            "debate: a/b + c/d + e/f",
            "qwen/qwen3:free",
        ] {
            let candidate: Candidate = text.parse().unwrap();
            assert_eq!(candidate.to_string(), text);
            let json = serde_json::to_string(&candidate).unwrap();
            assert_eq!(serde_json::from_str::<Candidate>(&json).unwrap(), candidate);
        }
        assert!("a/b + c/d".parse::<Candidate>().is_err());
        assert!("pair: a/b".parse::<Candidate>().is_err());
        assert!("pair: a/b + ".parse::<Candidate>().is_err());
    }

    #[test]
    fn each_kind_is_told_apart() {
        let kind = |text: &str| text.parse::<Candidate>().unwrap().kind();
        assert_eq!(kind("deepseek/deepseek-chat"), Kind::Model);
        assert_eq!(kind("anthropic/claude-opus-5-5"), Kind::Model);
        assert_eq!(kind("lead: anthropic/claude-opus-5-5 + a/b"), Kind::Team);
        assert_eq!("debate: a/b".parse::<Candidate>().unwrap().agents(), 2);
    }
}
