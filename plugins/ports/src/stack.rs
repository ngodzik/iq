//! A worktree's stack kept apart: its compose project, the files compose
//! reads there with the plugin's after them, the ports of its own its
//! services are moved to, what cannot be kept apart, and the variables
//! that make a command's compose the worktree's.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::PortsError;
use crate::compose::{self, Port};
use crate::docker::{self, STARTING};
use crate::store::{Kept, Moved, Place, Store};

/// The first of the ports a worktree's own are taken from: below the
/// range the system gives outgoing connections, so that none of them
/// takes one while its stack is down.
const FIRST: u16 = 20000;

/// How many there are to take from.
const SPAN: u16 = 10000;

/// A worktree's stack, kept apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    /// The worktree's folder, resolved.
    pub folder: PathBuf,
    /// The repository's name.
    pub repository: String,
    /// Its compose project.
    pub project: String,
    /// The project's compose files.
    pub files: Vec<PathBuf>,
    /// The plugin's, read after them.
    pub ports_file: PathBuf,
    /// Its services' ports moved.
    pub moved: Vec<Moved>,
    /// What cannot be kept apart, said to the person.
    pub notes: Vec<String>,
}

/// `text` as a compose project's name: lowercase letters, digits, `-` and
/// `_`, a letter or a digit first.
fn project_name(text: &str) -> String {
    let name: String = text
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_start_matches(['-', '_']).to_owned();
    if name.is_empty() {
        "stack".to_owned()
    } else {
        name
    }
}

/// A number of `text`'s, the same from one start to the next: where the
/// search for a free port starts.
fn spot(text: &str) -> u16 {
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    (hash % u64::from(SPAN)) as u16
}

/// Whether `port` of localhost is free for `protocol`.
fn free(port: u16, protocol: &str) -> bool {
    if protocol == "udp" {
        std::net::UdpSocket::bind(("127.0.0.1", port)).is_ok()
    } else {
        std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
    }
}

/// The name of the repository `folder` is in, and its top: as git says,
/// else the folder's own.
async fn named(folder: &Path) -> (String, PathBuf) {
    let top = docker::git(folder, &["rev-parse", "--show-toplevel"])
        .await
        .map_or_else(|| folder.to_path_buf(), PathBuf::from);
    let top = top.canonicalize().unwrap_or(top);
    let common = docker::git(
        &top,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await
    .map(PathBuf::from);
    let repository = common
        .as_deref()
        .and_then(|common| {
            if common.file_name()? == ".git" {
                common.parent()?.file_name()
            } else {
                common.file_stem()
            }
        })
        .or_else(|| top.file_name())
        .map_or_else(
            || "stack".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
    (repository, top)
}

impl Stack {
    /// The stack of the worktree `folder` is in, as compose resolves its
    /// files now: its ports moved to those it was given before, else to
    /// free ones, and the file that moves them written. `listed` is the
    /// person's `COMPOSE_FILE`, when they set one.
    ///
    /// # Errors
    ///
    /// The worktree has no compose file, compose cannot resolve them, or
    /// the store cannot be written.
    pub async fn prepare(
        folder: &Path,
        listed: Option<&str>,
        store: &Store,
        kept: &Kept,
    ) -> Result<Self, PortsError> {
        let (repository, top) = named(folder).await;
        let files = compose::files(&top, listed)?;
        let leaf = top
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let base = if leaf.is_empty() || leaf == repository {
            project_name(&repository)
        } else {
            project_name(&format!("{repository}-{leaf}"))
        };
        // Another folder's project of the same name, as two repositories
        // of one name would make, is not taken.
        let others: BTreeSet<&str> = kept
            .places
            .iter()
            .filter(|(other, _)| **other != top)
            .map(|(_, place)| place.project.as_str())
            .collect();
        let project = std::iter::once(base.clone())
            .chain((2..).map(|n| format!("{base}-{n}")))
            .find(|name| !others.contains(name.as_str()))
            .unwrap_or(base);
        let reading = BTreeMap::from([
            ("COMPOSE_PROJECT_NAME".to_owned(), project.clone()),
            ("COMPOSE_FILE".to_owned(), joined(&files)),
        ]);
        // Every profile's services, which a target may start.
        let mut printed = docker::compose(
            &top,
            &reading,
            &["--profile", "*", "config", "--format", "json"],
        )
        .await?;
        if !printed.ok {
            printed = docker::compose(&top, &reading, &["config", "--format", "json"]).await?;
        }
        if !printed.ok {
            return Err(PortsError::Command {
                command: "docker compose config".to_owned(),
                said: printed.said(),
            });
        }
        let services = compose::services(&printed.stdout)?;
        let before = kept.places.get(&top);
        let mut taken: BTreeSet<u16> = kept
            .places
            .iter()
            .filter(|(other, _)| **other != top)
            .flat_map(|(_, place)| place.moved.iter().map(|m| m.hidden))
            .collect();
        let mut moved = Vec::new();
        let mut notes = Vec::new();
        for service in &services {
            if service.host_network {
                notes.push(format!(
                    "{} shares the machine's network: its ports are its own and clash with \
                     another worktree's",
                    service.name
                ));
                continue;
            }
            for port in &service.ports {
                let Some(usual) = port.usual() else {
                    if let Some(range) = &port.published {
                        notes.push(format!(
                            "the ports {range} of {} are a range: they are not moved",
                            service.name
                        ));
                    }
                    continue;
                };
                let again = before.and_then(|place| {
                    place.moved.iter().find(|m| {
                        m.service == service.name
                            && m.target == port.target
                            && m.protocol == port.protocol
                            && !taken.contains(&m.hidden)
                    })
                });
                let hidden = match again {
                    Some(again) => again.hidden,
                    None => pick(&project, &service.name, port, &taken)?,
                };
                taken.insert(hidden);
                moved.push(Moved {
                    service: service.name.clone(),
                    target: port.target,
                    protocol: port.protocol.clone(),
                    usual,
                    hidden,
                });
            }
        }
        let text = compose::moving(&services, |name, port| hidden_of(&moved, name, port));
        let ports_file = store.write_ports_file(&project, &text)?;
        Ok(Self {
            folder: top,
            repository,
            project,
            files,
            ports_file,
            moved,
            notes,
        })
    }

    /// What it keeps of itself.
    #[must_use]
    pub fn place(&self) -> Place {
        Place {
            project: self.project.clone(),
            moved: self.moved.clone(),
        }
    }

    /// The variables that make a command's compose the worktree's.
    #[must_use]
    pub fn vars(&self) -> BTreeMap<String, String> {
        let mut files = self.files.clone();
        files.push(self.ports_file.clone());
        BTreeMap::from([
            ("COMPOSE_PROJECT_NAME".to_owned(), self.project.clone()),
            ("COMPOSE_FILE".to_owned(), joined(&files)),
        ])
    }

    /// The same, as a shell sets them.
    #[must_use]
    pub fn exports(&self) -> String {
        let vars: Vec<String> = self
            .vars()
            .iter()
            .map(|(name, value)| format!("{name}={}", quoted(value)))
            .collect();
        format!("export {}", vars.join(" "))
    }

    /// The usual ports of localhost led to the worktree's own, those of
    /// TCP: the ones a browser and a client reach.
    #[must_use]
    pub fn relayed(&self) -> HashMap<u16, u16> {
        self.moved
            .iter()
            .filter(|m| m.protocol == "tcp")
            .map(|m| (m.usual, m.hidden))
            .collect()
    }

    /// Whether some of its services run.
    pub async fn running(&self) -> bool {
        docker::compose(
            &self.folder,
            &self.vars(),
            &["ps", "--status", "running", "--quiet"],
        )
        .await
        .is_ok_and(|ps| ps.ok && !ps.stdout.trim().is_empty())
    }

    /// What a `make` target `up` names says it runs that hands compose
    /// its files itself, which reads the project's alone: said, since its
    /// ports then are not moved.
    pub async fn unmoved_by(&self, up: &str) -> Option<String> {
        let target = up.trim().strip_prefix("make")?;
        let target = target.trim();
        let mut args = vec!["-n"];
        args.extend(target.split_whitespace());
        let dry = docker::run("make", &args, &self.folder, &self.vars(), docker::READING)
            .await
            .ok()?;
        dry.stdout
            .lines()
            .find(|line| {
                (line.contains("docker compose") || line.contains("docker-compose"))
                    && line
                        .split_whitespace()
                        .any(|word| word == "-f" || word.starts_with("--file"))
            })
            .map(|line| {
                format!(
                    "`{up}` hands compose its files itself (`{}`): it reads the project's \
                     alone, so this stack takes the usual ports",
                    line.trim()
                )
            })
    }

    /// Starts it with `up`, as the person's settings say it starts.
    ///
    /// # Errors
    ///
    /// It failed, with the end of what it said.
    pub async fn start(&self, up: &str) -> Result<(), PortsError> {
        let ran = docker::shell(up, &self.folder, &self.vars(), STARTING).await?;
        if ran.ok {
            Ok(())
        } else {
            Err(PortsError::Command {
                command: up.to_owned(),
                said: ran.said(),
            })
        }
    }

    /// What the Team page says of it.
    #[must_use]
    pub fn said(&self, running: bool, served: bool) -> Vec<String> {
        let state = if running { "running" } else { "not started" };
        let mut line = format!("stack {}: {state}", self.project);
        let ports: Vec<String> = self
            .moved
            .iter()
            .filter(|m| m.protocol == "tcp")
            .map(|m| format!(":{} → {}", m.usual, m.hidden))
            .collect();
        if !ports.is_empty() {
            line.push_str(&format!(" · {}", ports.join(", ")));
        }
        if served {
            line.push_str(" · localhost leads here");
        }
        vec![line]
    }
}

/// The worktree's own port `port` of `service` was moved to.
fn hidden_of(moved: &[Moved], service: &str, port: &Port) -> Option<u16> {
    moved
        .iter()
        .find(|m| {
            m.service == service
                && m.target == port.target
                && m.protocol == port.protocol
                && Some(m.usual) == port.usual()
        })
        .map(|m| m.hidden)
}

/// A free port of localhost for `port` of `service` in `project`, none of
/// `taken`, from where its name puts it.
fn pick(
    project: &str,
    service: &str,
    port: &Port,
    taken: &BTreeSet<u16>,
) -> Result<u16, PortsError> {
    let start = spot(&format!(
        "{project}/{service}/{}/{}",
        port.target, port.protocol
    ));
    (0..SPAN)
        .map(|step| FIRST + (start + step) % SPAN)
        .find(|candidate| !taken.contains(candidate) && free(*candidate, &port.protocol))
        .ok_or_else(|| PortsError::NoPort(format!("{service}'s {}", port.target)))
}

/// `files` as `COMPOSE_FILE` lists them.
fn joined(files: &[PathBuf]) -> String {
    files
        .iter()
        .map(|file| file.display().to_string())
        .collect::<Vec<_>>()
        .join(":")
}

/// `text` as one word of a shell's line.
fn quoted(text: &str) -> String {
    if !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+=:@%".contains(c))
    {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_is_named_as_compose_takes_names() {
        assert_eq!(project_name("Shop-lead"), "shop-lead");
        assert_eq!(project_name("my shop.v2-fix"), "my-shop-v2-fix");
        assert_eq!(project_name("_x"), "x");
        assert_eq!(project_name("…"), "stack");
    }

    #[test]
    fn a_port_is_looked_for_from_the_same_spot_each_time_out_of_what_is_taken() {
        assert_eq!(
            spot("shop-lead/api/8000/tcp"),
            spot("shop-lead/api/8000/tcp")
        );
        let port = Port {
            target: 8000,
            protocol: "tcp".into(),
            host_ip: None,
            published: Some("8000".into()),
        };
        let first = pick("shop-lead", "api", &port, &BTreeSet::new()).unwrap();
        assert!((FIRST..FIRST + SPAN).contains(&first));
        let second = pick("shop-lead", "api", &port, &BTreeSet::from([first])).unwrap();
        assert_ne!(first, second);
        // Held by another program, it is passed over.
        let held = std::net::TcpListener::bind(("127.0.0.1", first)).unwrap();
        assert_ne!(
            pick("shop-lead", "api", &port, &BTreeSet::new()).unwrap(),
            first
        );
        drop(held);
    }

    #[test]
    fn what_a_command_is_given_lists_the_plugins_file_last() {
        let stack = Stack {
            folder: "/w/shop/lead".into(),
            repository: "shop".into(),
            project: "shop-lead".into(),
            files: vec!["/w/shop/lead/compose.yaml".into()],
            ports_file: "/s/iq-ports/shop-lead.yaml".into(),
            moved: vec![
                Moved {
                    service: "api".into(),
                    target: 8000,
                    protocol: "tcp".into(),
                    usual: 8000,
                    hidden: 20001,
                },
                Moved {
                    service: "dns".into(),
                    target: 53,
                    protocol: "udp".into(),
                    usual: 53,
                    hidden: 20002,
                },
            ],
            notes: Vec::new(),
        };
        assert_eq!(
            stack.vars()["COMPOSE_FILE"],
            "/w/shop/lead/compose.yaml:/s/iq-ports/shop-lead.yaml"
        );
        assert_eq!(
            stack.exports(),
            "export COMPOSE_FILE=/w/shop/lead/compose.yaml:/s/iq-ports/shop-lead.yaml \
             COMPOSE_PROJECT_NAME=shop-lead"
        );
        assert_eq!(stack.relayed(), HashMap::from([(8000, 20001)]));
        assert_eq!(
            stack.said(false, true),
            ["stack shop-lead: not started · :8000 → 20001 · localhost leads here"]
        );
        assert_eq!(quoted("/Users/Jo Doe"), "'/Users/Jo Doe'");
    }
}
