//! The assay's own settings, in `settings.toml` in its folder, never
//! iq's: the assay and iq change apart, so a key one does not know is
//! said and left aside, never a reason to refuse the file. Keys never go
//! there: the provider's comes from the environment, as iq's does.
//!
//! ```toml
//! grader = "<provider>/<model>"     # grades every run, always the same so that grades compare; needed to run and to grade
//! grader_effort = "high"            # how hard it thinks
//! candidates = ["models", "teams"]  # who may be tested: "models", "teams", or candidates by name; all unless set
//! effort = "medium"                 # how hard the candidates think; left to each model unless set
//! reference = "<provider>/<model>"  # the model the others are measured against, as your lead; none unless set
//! parallel = 2                      # runs at once, from 1 to 8
//! base_url = "https://router.requesty.ai/v1"  # the provider, unless --base-url or IQ_BASE_URL says; Requesty's router unless set
//! ```

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use iq_core::{Effort, ModelId};
use serde::Deserialize;

use crate::error::AssayError;
use crate::roster::Allowed;

/// The settings' file, in the assay's folder.
pub const FILE: &str = "settings.toml";

/// The variable that names the assay's folder, in place of the one the
/// system's conventions give.
pub const VARIABLE: &str = "IQ_ASSAY_HOME";

/// How many runs go at once unless the settings say.
const PARALLEL: usize = 2;

/// The most runs at once.
const MOST_PARALLEL: usize = 8;

/// Keys the assay once read in iq's own settings, under `[assay]`, and
/// what to write instead.
const MOVED: [(&str, &str); 3] = [
    (
        "assay",
        "the keys go at the top of the file, without [assay]",
    ),
    (
        "folder",
        "the assay's folder is the one IQ_ASSAY_HOME names, else iq-assay in the system's folder of data",
    ),
    ("judges", "one grader grades every run: `grader`"),
];

/// The settings as the file writes them.
#[derive(Debug, Default, Deserialize)]
struct Written {
    grader: Option<String>,
    grader_effort: Option<Effort>,
    #[serde(default)]
    candidates: Vec<String>,
    effort: Option<Effort>,
    reference: Option<String>,
    parallel: Option<usize>,
    base_url: Option<String>,
    /// Whatever else the file holds: said, and left aside.
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

/// The assay's settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Who grades every run, the same for all so that grades compare;
    /// none set, nothing is run or graded.
    pub grader: Option<ModelId>,
    /// How hard the grader thinks.
    pub grader_effort: Effort,
    /// Which candidates may be tested.
    pub allowed: Allowed,
    /// How hard the candidates think; `None` leaves it to each model.
    pub effort: Option<Effort>,
    /// The model the others are measured against, as the person's lead,
    /// when the settings name one.
    pub reference: Option<ModelId>,
    /// How many runs go at once.
    pub parallel: usize,
    /// The provider's address, when the settings name one.
    pub base_url: Option<String>,
    /// What the file holds that the assay does not read, said for the
    /// person to remove or move.
    pub notices: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            grader: None,
            grader_effort: Effort::High,
            allowed: Allowed::default(),
            effort: None,
            reference: None,
            parallel: PARALLEL,
            base_url: None,
            notices: Vec::new(),
        }
    }
}

impl Settings {
    /// The settings of the assay's folder `folder`, the defaults when it
    /// holds none.
    ///
    /// # Errors
    ///
    /// The file cannot be read, or a key it reads holds what it cannot be:
    /// a model that names none, an effort that is no level.
    pub fn load(folder: &Path) -> Result<Self, AssayError> {
        let file = folder.join(FILE);
        match std::fs::read_to_string(&file) {
            Ok(text) => Self::read(&text, &file),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(AssayError::Io { path: file, source }),
        }
    }

    /// The settings `text` writes, the file `path` holds it.
    ///
    /// # Errors
    ///
    /// As [`Self::load`].
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    ///
    /// let set = iq_assay::Settings::read("grader = \"lab/sage\"\nlater = 1\n", Path::new("s.toml"))?;
    /// assert_eq!(set.grader.unwrap().as_str(), "lab/sage");
    /// assert!(set.notices[0].contains("`later`"));
    /// # Ok::<(), iq_assay::AssayError>(())
    /// ```
    pub fn read(text: &str, path: &Path) -> Result<Self, AssayError> {
        let written: Written = toml::from_str(text).map_err(|source| AssayError::Toml {
            path: path.to_path_buf(),
            source,
        })?;
        let model = |key: &'static str, named: Option<String>| {
            named
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty())
                .map(ModelId::new)
                .transpose()
                .map_err(|source| AssayError::Model {
                    path: path.to_path_buf(),
                    key,
                    source,
                })
        };
        let notices = written
            .unknown
            .keys()
            .map(|key| match MOVED.iter().find(|(moved, _)| moved == key) {
                Some((_, instead)) => format!(
                    "{}: `{key}` is read no more, {instead}; it is left aside",
                    path.display()
                ),
                None => format!(
                    "{}: `{key}` is not one of the assay's settings; it is left aside",
                    path.display()
                ),
            })
            .collect();
        Ok(Self {
            grader: model("grader", written.grader)?,
            grader_effort: written.grader_effort.unwrap_or(Effort::High),
            allowed: Allowed::from_words(&written.candidates),
            effort: written.effort,
            reference: model("reference", written.reference)?,
            parallel: written.parallel.unwrap_or(PARALLEL).clamp(1, MOST_PARALLEL),
            base_url: written
                .base_url
                .map(|url| url.trim().to_owned())
                .filter(|url| !url.is_empty()),
            notices,
        })
    }
}

/// The assay's folder: the one `IQ_ASSAY_HOME` names, else `iq-assay` in
/// `XDG_DATA_HOME` when it is an absolute path, else in the `home`'s
/// `.local/share`; none without a home. Apart from iq's own folders, so
/// that what the assay keeps and what iq keeps never mix. `var` reads the
/// environment.
///
/// # Examples
///
/// ```
/// use std::path::{Path, PathBuf};
///
/// let none = |_: &str| None;
/// let home = Some(Path::new("/home/me"));
/// assert_eq!(
///     iq_assay::folder(none, home),
///     Some(PathBuf::from("/home/me/.local/share/iq-assay"))
/// );
/// let set = |name: &str| (name == "IQ_ASSAY_HOME").then(|| "/lab/assay".into());
/// assert_eq!(iq_assay::folder(set, home), Some(PathBuf::from("/lab/assay")));
/// ```
pub fn folder(var: impl Fn(&str) -> Option<OsString>, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(named) = var(VARIABLE).filter(|named| !named.is_empty()) {
        return Some(PathBuf::from(named));
    }
    var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|data| data.is_absolute())
        .or_else(|| home.map(|home| home.join(".local/share")))
        .map(|data| data.join("iq-assay"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::Kind;

    fn read(text: &str) -> Result<Settings, AssayError> {
        Settings::read(text, Path::new("/a/settings.toml"))
    }

    #[test]
    fn every_setting_is_read_and_held_within_its_bounds() {
        let none = read("").unwrap();
        assert_eq!(none, Settings::default());
        assert_eq!(none.grader, None);
        assert_eq!(none.grader_effort, Effort::High);
        let set = read(
            "grader = \" lab/sage \"\ngrader_effort = \"max\"\ncandidates = [\"models\"]\n\
             effort = \"low\"\nreference = \"lab/quill\"\nparallel = 40\n\
             base_url = \" https://router.example/v1 \"\n",
        )
        .unwrap();
        assert_eq!(set.grader.unwrap().as_str(), "lab/sage");
        assert_eq!(set.grader_effort, Effort::Max);
        assert!(set.allowed.allows(Kind::Model, "lab/quill"));
        assert!(!set.allowed.allows(Kind::Team, "pair: lab/quill + lab/sage"));
        assert_eq!(set.effort, Some(Effort::Low));
        assert_eq!(set.reference.unwrap().as_str(), "lab/quill");
        assert_eq!(set.parallel, 8);
        assert_eq!(set.base_url.as_deref(), Some("https://router.example/v1"));
        assert!(set.notices.is_empty());
        assert_eq!(read("parallel = 0").unwrap().parallel, 1);
    }

    #[test]
    fn a_key_the_assay_does_not_read_is_said_never_refused() {
        let set = read(
            "grader = \"lab/sage\"\nfolder = \"~/lab\"\nsomeday = true\n\n[assay]\nparallel = 3\n",
        )
        .unwrap();
        assert_eq!(set.grader.unwrap().as_str(), "lab/sage");
        // What sits under [assay] is not read: it is said, with where it goes.
        assert_eq!(set.parallel, PARALLEL);
        let said = set.notices.join("\n");
        assert_eq!(set.notices.len(), 3, "{said}");
        assert!(said.contains("`assay` is read no more, the keys go at the top"));
        assert!(said.contains(
            "`folder` is read no more, the assay's folder is the one IQ_ASSAY_HOME names"
        ));
        assert!(said.contains("/a/settings.toml: `someday` is not one of the assay's settings"));
        // What a key it reads holds is still checked.
        assert!(read("parallel = \"two\"").is_err());
        assert!(read("grader_effort = \"enormous\"").is_err());
        assert!(matches!(read("grader = 3"), Err(AssayError::Toml { .. })));
        assert_eq!(read("grader = \"  \"").unwrap().grader, None);
    }

    #[test]
    fn the_folder_is_the_assays_own() {
        let home = Some(Path::new("/home/me"));
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(value))
            }
        };
        assert_eq!(
            folder(env(&[("XDG_DATA_HOME", "/data")]), home),
            Some(PathBuf::from("/data/iq-assay"))
        );
        // A relative XDG_DATA_HOME is none, as the conventions say.
        assert_eq!(
            folder(env(&[("XDG_DATA_HOME", "data")]), home),
            Some(PathBuf::from("/home/me/.local/share/iq-assay"))
        );
        assert_eq!(
            folder(
                env(&[("XDG_DATA_HOME", "/data"), ("IQ_ASSAY_HOME", "/lab")]),
                home
            ),
            Some(PathBuf::from("/lab"))
        );
        assert_eq!(folder(env(&[("IQ_ASSAY_HOME", "")]), None), None);
    }
}
