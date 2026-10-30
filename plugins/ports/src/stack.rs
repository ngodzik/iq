//! A worktree's stack kept apart: its compose project, the files compose
//! reads there with the plugin's after them, the ports of its own its
//! services are moved to, what cannot be kept apart, and the variables
//! that make a command's compose the worktree's, docker's among them when
//! the plugin's compose takes its place.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::PortsError;
use crate::compose::{self, Port, Service};
use crate::docker::{self, STARTING};
use crate::shim::Shim;
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
    /// Its services' ports moved: those of its usual files first, then
    /// those other files compose was run with there moved.
    pub moved: Vec<Moved>,
    /// The folder docker is pointed to there, where the plugin's compose
    /// reads the files a command names with the plugin's: none when it
    /// could not be set up.
    pub docker: Option<PathBuf>,
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

/// A number of `text`'s, the same from one start to the next.
pub(crate) fn hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Where the search for a free port for `text` starts.
fn spot(text: &str) -> u16 {
    (hash(text) % u64::from(SPAN)) as u16
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
    /// person's `COMPOSE_FILE`, when they set one; `shim`, the compose
    /// that takes docker's place there, so that a command handing compose
    /// files of its own keeps it apart too.
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
        shim: Option<&Shim>,
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
        // Read the first way compose takes: every profile's services first.
        let mut printed =
            docker::compose(&top, &reading, &compose::config_args(compose::READINGS[0])).await?;
        for way in &compose::READINGS[1..] {
            if printed.ok {
                break;
            }
            printed = docker::compose(&top, &reading, &compose::config_args(*way)).await?;
        }
        if !printed.ok {
            return Err(PortsError::Command {
                command: "docker compose config".to_owned(),
                said: printed.said(),
            });
        }
        let services = compose::services(&printed.stdout)?;
        let before = kept.places.get(&top);
        let mut taken = taken_by_others(kept, &top);
        let (mut moved, mut notes) = moves(&project, &services, before, &mut taken)?;
        let text = compose::moving(&services, |name, port| hidden_of(&moved, name, port));
        let ports_file = store.write_ports_file(&project, &text)?;
        // Those other files moved stay its own: their containers may hold
        // them.
        for again in before.into_iter().flat_map(|place| &place.moved) {
            if !moved.iter().any(|m| m.same_port(again)) && taken.insert(again.hidden) {
                moved.push(again.clone());
            }
        }
        let docker = match shim.map(|shim| shim.set_up(store)) {
            Some(Ok((folder, said))) => {
                notes.extend(said);
                Some(folder)
            }
            Some(Err(error)) => {
                notes.push(format!(
                    "a command handing compose its files itself (`--file`) reads the \
                     project's alone, and is not kept apart: {error}"
                ));
                None
            }
            None => None,
        };
        Ok(Self {
            folder: top,
            repository,
            project,
            files,
            ports_file,
            moved,
            docker,
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
        let mut vars = BTreeMap::from([
            ("COMPOSE_PROJECT_NAME".to_owned(), self.project.clone()),
            ("COMPOSE_FILE".to_owned(), joined(&files)),
        ]);
        if let Some(docker) = &self.docker {
            vars.insert("DOCKER_CONFIG".to_owned(), docker.display().to_string());
        }
        vars
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
    /// TCP: the ones a browser and a client reach. A port two services
    /// publish leads to the one of the usual files.
    #[must_use]
    pub fn relayed(&self) -> HashMap<u16, u16> {
        let mut routes = HashMap::new();
        for m in self.moved.iter().filter(|m| m.protocol == "tcp") {
            routes.entry(m.usual).or_insert(m.hidden);
        }
        routes
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
    /// its files or its project itself where the plugin's compose does not
    /// take docker's: said, since that stack is not kept apart. It takes
    /// it once set up, but for `docker-compose` run alone.
    pub async fn unmoved_by(&self, up: &str) -> Option<String> {
        let target = up.trim().strip_prefix("make")?;
        let target = target.trim();
        let mut args = vec!["-n"];
        args.extend(target.split_whitespace());
        let dry = docker::run("make", &args, &self.folder, &self.vars(), docker::READING)
            .await
            .ok()?;
        dry.stdout.lines().find_map(|line| {
            let alone = line.contains("docker-compose");
            let compose = alone || (self.docker.is_none() && line.contains("docker compose"));
            let words: Vec<&str> = line.split_whitespace().collect();
            let named = words.iter().any(|word| {
                ["-f", "-p"].contains(word)
                    || word.starts_with("--file")
                    || word.starts_with("--project-name")
            });
            (compose && named).then(|| unmoved(up, line.trim(), &words, alone))
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
pub(crate) fn hidden_of(moved: &[Moved], service: &str, port: &Port) -> Option<u16> {
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

/// What is said of `up`, whose `line` hands compose its files or its
/// project, `alone` when it runs `docker-compose` rather than docker's:
/// what dropping a `--file` naming compose's own file keeps apart, since
/// compose reads that file without it.
fn unmoved(up: &str, line: &str, words: &[&str], alone: bool) -> String {
    let files: Vec<&str> = words
        .iter()
        .zip(words.iter().skip(1))
        .filter_map(|(word, next)| ["-f", "--file"].contains(word).then_some(*next))
        .chain(words.iter().filter_map(|word| word.strip_prefix("--file=")))
        .collect();
    let mut said = format!(
        "`{up}` hands compose its files or its project itself (`{line}`): this stack is not \
         kept apart from the others"
    );
    if alone {
        said.push_str(
            ", since `docker-compose` is not run through docker, whose compose in a worktree is \
             iq-ports's",
        );
    }
    if let [file] = files.as_slice()
        && compose::FILES.contains(file)
        && !words
            .iter()
            .any(|word| *word == "-p" || word.starts_with("--project-name"))
    {
        said.push_str(&format!(
            ". Without `--file {file}`, it would be: compose reads that file anyway"
        ));
    }
    said
}

/// The ports of localhost the stacks of other places than `folder` were
/// given.
pub(crate) fn taken_by_others(kept: &Kept, folder: &Path) -> BTreeSet<u16> {
    kept.places
        .iter()
        .filter(|(other, _)| *other != folder)
        .flat_map(|(_, place)| place.moved.iter().map(|m| m.hidden))
        .collect()
}

/// The ports `services` publish moved for `project`: each to the one it
/// was given `before`, else to a free one, none of `taken`, which takes
/// those given; and what cannot be moved, said.
///
/// # Errors
///
/// No port is free for one.
pub(crate) fn moves(
    project: &str,
    services: &[Service],
    before: Option<&Place>,
    taken: &mut BTreeSet<u16>,
) -> Result<(Vec<Moved>, Vec<String>), PortsError> {
    let mut moved = Vec::new();
    let mut notes = Vec::new();
    for service in services {
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
                None => pick(project, &service.name, port, taken)?,
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
    Ok((moved, notes))
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
pub(crate) fn quoted(text: &str) -> String {
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
            docker: None,
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
        let mut stack = stack;
        stack.docker = Some("/s/iq-ports/docker".into());
        assert_eq!(stack.vars()["DOCKER_CONFIG"], "/s/iq-ports/docker");
    }

    #[test]
    fn a_port_two_services_publish_leads_to_the_one_of_the_usual_files() {
        let moved = |service: &str, hidden: u16| Moved {
            service: service.into(),
            target: 80,
            protocol: "tcp".into(),
            usual: 8000,
            hidden,
        };
        let stack = Stack {
            folder: "/w/shop/lead".into(),
            repository: "shop".into(),
            project: "shop-lead".into(),
            files: Vec::new(),
            ports_file: "/s/iq-ports/shop-lead.yaml".into(),
            moved: vec![moved("api", 20001), moved("dev-api", 20002)],
            docker: None,
            notes: Vec::new(),
        };
        assert_eq!(stack.relayed(), HashMap::from([(8000, 20001)]));
    }

    #[test]
    fn a_line_handing_compose_its_file_is_said_with_what_would_keep_it_apart() {
        let said = |line: &str| {
            let words: Vec<&str> = line.split_whitespace().collect();
            unmoved("make up", line, &words, false)
        };
        assert!(said("docker compose --file compose.yaml up -d").ends_with(
            ". Without `--file compose.yaml`, it would be: compose reads that file anyway"
        ));
        assert!(said("docker compose --file=docker-compose.yml up -d").contains("Without"));
        assert!(!said("docker compose -f compose.yaml -f dev.yaml up -d").contains("Without"));
        assert!(!said("docker compose -p shop -f compose.yaml up -d").contains("Without"));
        assert!(!said("docker compose -f deploy/compose.yaml up -d").contains("Without"));
        let words = ["docker-compose", "-f", "compose.yaml", "up"];
        assert!(
            unmoved("make up", &words.join(" "), &words, true).contains("not run through docker")
        );
    }
}
