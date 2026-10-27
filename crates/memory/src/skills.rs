//! The person's skills: what they wrote for the agents they start to
//! follow in a kind of work, such as which team to start for code and how
//! it works, each a Markdown file with its name and a line on when it
//! serves, as Claude Code's are: a folder with its `SKILL.md`, or a file of
//! its own. An agent the person started is told which there are, reads one
//! whole with `skill` before the work it is for, and gives the agents it
//! starts what they need of it. The person changes a skill, and the next
//! agent reads it as it is.

use std::path::{Path, PathBuf};

use iq_agent::{Called, Crew, Extension};
use iq_core::ToolSpec;
use serde_json::{Value, json};

use crate::{parse, secret, slug};

/// The file a skill's folder holds it in.
const SKILL_FILE: &str = "SKILL.md";

/// The longest skill given, in bytes.
const SKILL_MOST: usize = 40_000;

/// The folders the person keeps skills in.
#[derive(Debug, Clone, Default)]
pub struct Skills {
    folders: Vec<PathBuf>,
}

/// One skill, as listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Its name.
    pub name: String,
    /// When it serves, in a line.
    pub description: String,
    /// Its file.
    pub file: PathBuf,
}

impl Skills {
    /// The skills kept in `folders`, the first folder's winning where two
    /// hold a skill of the same name.
    #[must_use]
    pub fn new(folders: Vec<PathBuf>) -> Self {
        Self { folders }
    }

    /// The skills, by name.
    #[must_use]
    pub fn list(&self) -> Vec<Skill> {
        let mut skills: Vec<Skill> = Vec::new();
        for folder in &self.folders {
            for skill in skills_in(folder) {
                if !skills.iter().any(|s| slug(&s.name) == slug(&skill.name)) {
                    skills.push(skill);
                }
            }
        }
        skills.sort_by_key(|skill| skill.name.to_lowercase());
        skills
    }

    /// A skill whole, by its name.
    ///
    /// # Errors
    ///
    /// There is no such skill, it cannot be read, or it holds what looks
    /// like a secret.
    pub fn read(&self, name: &str) -> Result<String, String> {
        let skills = self.list();
        let wanted = slug(name);
        let skill = skills
            .iter()
            .find(|skill| slug(&skill.name) == wanted)
            .ok_or_else(|| {
                let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
                format!(
                    "no skill is named \"{name}\": the skills are {}",
                    names.join(", ")
                )
            })?;
        let text = std::fs::read_to_string(&skill.file)
            .map_err(|e| format!("{}: {e}", skill.file.display()))?;
        if let Some(sign) = secret(&text) {
            return Err(format!(
                "the skill holds what looks like a secret (`{sign}…`), which is never given"
            ));
        }
        let (_, _, body) = parse(&text);
        let body = body.trim();
        let mut cut = body.len().min(SKILL_MOST);
        while !body.is_char_boundary(cut) {
            cut -= 1;
        }
        let more = if cut < body.len() {
            "\n[the rest of the skill is left out]"
        } else {
            ""
        };
        Ok(format!(
            "{}: {}\n\n{}{more}",
            skill.name,
            skill.description,
            &body[..cut]
        ))
    }

    /// What an agent is told of the skills; `None` when there is none.
    #[must_use]
    pub fn told(&self) -> Option<String> {
        let skills = self.list();
        if skills.is_empty() {
            return None;
        }
        let mut out = "Your skills, which the person wrote for you: before the work one is for, \
                       read it whole with skill, and follow it."
            .to_owned();
        for skill in &skills {
            out.push_str(&format!("\n- {}", skill.name));
            if !skill.description.is_empty() {
                out.push_str(&format!(": {}", skill.description));
            }
        }
        Some(out)
    }
}

/// The skills in `folder`: each folder with its `SKILL.md`, and each
/// Markdown file beside them, named as its head says, else as its folder
/// or file is.
fn skills_in(folder: &Path) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter_map(|path| {
            let file = if path.is_dir() {
                path.join(SKILL_FILE)
            } else if path.extension().is_some_and(|e| e == "md") {
                path.clone()
            } else {
                return None;
            };
            let text = std::fs::read_to_string(&file).ok()?;
            let (name, description, _) = parse(&text);
            let fallback = path.file_stem()?.to_string_lossy().into_owned();
            Some(Skill {
                name: name.unwrap_or(fallback),
                description: description.unwrap_or_default(),
                file,
            })
        })
        .collect()
}

impl<M> Extension<M> for Skills
where
    M: Send + Sync + 'static,
{
    fn tools(&self, _agent: &str) -> Vec<ToolSpec> {
        if self.list().is_empty() {
            return Vec::new();
        }
        vec![ToolSpec {
            name: "skill".into(),
            description: "Reads one of your skills whole: what the person wrote for you to \
                          follow in the work it is for."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"],
            }),
        }]
    }

    fn leads_only(&self) -> bool {
        true
    }

    fn instructions(&self, _agent: &str) -> Option<String> {
        self.told()
    }

    fn call(&self, _crew: Crew<M>, _agent: String, name: String, args: Value) -> Called {
        let result = match name.as_str() {
            "skill" => self.read(args["name"].as_str().unwrap_or_default()),
            _ => Err(format!("the skills have no tool named {name:?}")),
        };
        Box::pin(std::future::ready(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_are_read_from_their_folders_as_claude_code_keeps_them() {
        let mine = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let skills = Skills::new(vec![mine.path().into(), other.path().into()]);
        assert_eq!(skills.told(), None);
        assert!(<Skills as Extension<()>>::tools(&skills, "lead").is_empty());
        std::fs::create_dir_all(mine.path().join("teams")).unwrap();
        std::fs::write(
            mine.path().join("teams/SKILL.md"),
            "---\nname: teams\ndescription: which team to start for which work\n---\n\nFor code, a writer and a reviewer.\n",
        )
        .unwrap();
        std::fs::write(mine.path().join("compact.md"), "Compact between tasks.\n").unwrap();
        std::fs::write(
            other.path().join("teams.md"),
            "---\nname: Teams\ndescription: another\n---\nNever read.\n",
        )
        .unwrap();
        std::fs::write(mine.path().join("notes.txt"), "not a skill").unwrap();
        let told = skills.told().unwrap();
        assert!(told.contains("\n- compact\n"));
        assert!(told.contains("- teams: which team to start for which work"));
        assert_eq!(skills.list().len(), 2);
        assert!(
            skills
                .read("Teams")
                .unwrap()
                .ends_with("For code, a writer and a reviewer.")
        );
        assert!(
            skills
                .read("deploy")
                .unwrap_err()
                .contains("compact, teams")
        );
        assert_eq!(
            <Skills as Extension<()>>::tools(&skills, "lead")[0].name,
            "skill"
        );
    }
}
