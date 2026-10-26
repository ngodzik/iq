//! The models the assay may give an exercise to: the provider's, as it
//! lists them, with what they charge. The program reads them; the assay
//! only chooses among them.

use iq_core::ModelId;

use crate::candidate::Kind;

/// A model of the provider, as its list gives it.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    /// Its identifier, as requests name it.
    pub id: String,
    /// Its name without the host serving it, when the list gives one.
    pub canonical: Option<String>,
    /// Dollars per input token.
    pub input: Option<f64>,
    /// Dollars per output token.
    pub output: Option<f64>,
    /// The most it reads at once, in tokens.
    pub context: Option<u64>,
    /// Whether it calls tools: one that does not cannot work in iq's loop.
    pub tools: Option<bool>,
    /// What the provider says of it.
    pub description: Option<String>,
}

/// Everyone the assay may test.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Roster {
    /// Who serves the models, as it names itself.
    pub provider: Option<String>,
    /// The provider's models.
    pub models: Vec<Offer>,
    /// Why the provider's models are not there, when they are not: no
    /// key, the list could not be fetched.
    pub missing: Option<String>,
}

impl Roster {
    /// The provider's model `id`, when it lists it.
    #[must_use]
    pub fn offer(&self, id: &str) -> Option<&Offer> {
        self.models.iter().find(|offer| offer.id == id)
    }

    /// The name a grade of `model` is kept under, whoever serves it: its
    /// canonical name when the provider gives one, else its identifier
    /// without the host, reduced to letters and digits, so that
    /// `z-ai/glm-5.3` and `glm-5-3` are one model.
    #[must_use]
    pub fn key(&self, model: &ModelId) -> String {
        let named = self
            .offer(model.as_str())
            .and_then(|offer| offer.canonical.clone())
            .unwrap_or_else(|| {
                let id = model.as_str();
                id.rsplit_once('/').map_or(id, |(_, name)| name).to_owned()
            });
        plain(&named)
    }

    /// Whether `model` may be tested: a model the provider lists that
    /// calls tools. A model the list does not hold is
    /// let through when there is no list, for the provider to answer.
    ///
    /// # Errors
    ///
    /// Why it may not.
    pub fn check(&self, model: &ModelId) -> Result<(), String> {
        if self.models.is_empty() {
            return Ok(());
        }
        match self.offer(model.as_str()) {
            Some(offer) if offer.tools == Some(false) => Err(format!(
                "{model} calls no tools, so it cannot work in iq's loop"
            )),
            Some(_) => Ok(()),
            None => Err(format!(
                "the provider lists no {model}: find its exact identifier with assay_roster"
            )),
        }
    }
}

/// A name reduced to its letters and digits, lowercase.
#[must_use]
pub fn plain(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Which candidates the settings let the assay test: by kind, or by name.
#[derive(Debug, Clone, PartialEq)]
pub struct Allowed {
    kinds: Vec<Kind>,
    names: Vec<String>,
}

impl Default for Allowed {
    fn default() -> Self {
        Self {
            kinds: Kind::ALL.to_vec(),
            names: Vec::new(),
        }
    }
}

impl Allowed {
    /// As the settings write it: `models`, `teams`, or
    /// candidates' names; every kind when empty.
    #[must_use]
    pub fn from_words(words: &[String]) -> Self {
        if words.is_empty() {
            return Self::default();
        }
        let mut allowed = Self {
            kinds: Vec::new(),
            names: Vec::new(),
        };
        for word in words {
            match Kind::ALL
                .into_iter()
                .find(|kind| kind.plural() == word.trim())
            {
                Some(kind) => allowed.kinds.push(kind),
                None => allowed.names.push(word.trim().to_owned()),
            }
        }
        allowed
    }

    /// Whether a candidate of `kind`, named `name`, may be tested.
    #[must_use]
    pub fn allows(&self, kind: Kind, name: &str) -> bool {
        self.kinds.contains(&kind) || self.names.iter().any(|n| n == name)
    }

    /// In words, for the judge.
    #[must_use]
    pub fn said(&self) -> String {
        let mut parts: Vec<String> = self.kinds.iter().map(|k| k.plural().to_owned()).collect();
        parts.extend(self.names.iter().cloned());
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(id: &str, canonical: Option<&str>, tools: bool) -> Offer {
        Offer {
            id: id.into(),
            canonical: canonical.map(Into::into),
            input: Some(1e-6),
            output: Some(4e-6),
            context: None,
            tools: Some(tools),
            description: None,
        }
    }

    #[test]
    fn a_model_is_one_whoever_serves_it() {
        let roster = Roster {
            provider: Some("Requesty".into()),
            models: vec![
                offer("z-ai/glm-5.3", Some("glm-5.3"), true),
                offer("old/plain", None, false),
            ],
            missing: None,
        };
        let key = |id: &str| roster.key(&ModelId::new(id).unwrap());
        assert_eq!(key("z-ai/glm-5.3"), "glm53");
        assert_eq!(key("elsewhere/GLM-5-3"), "glm53");
        let check = |id: &str| roster.check(&ModelId::new(id).unwrap());
        assert!(check("z-ai/glm-5.3").is_ok());
        assert!(check("old/plain").unwrap_err().contains("no tools"));
        assert!(check("made/up").unwrap_err().contains("assay_roster"));
    }

    #[test]
    fn the_settings_allow_by_kind_or_name() {
        let allowed = Allowed::from_words(&["models".into(), "pair: a/b + c/d".into()]);
        assert!(allowed.allows(Kind::Model, "x/y"));
        assert!(!allowed.allows(Kind::Team, "debate: a/b + c/d"));
        assert!(allowed.allows(Kind::Team, "pair: a/b + c/d"));
        assert!(Allowed::from_words(&[]).allows(Kind::Team, "pair: a + b"));
        assert_eq!(allowed.said(), "models, pair: a/b + c/d");
    }
}
