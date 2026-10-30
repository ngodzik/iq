//! What compose makes of a worktree: the files it reads there, the
//! services they resolve to as `docker compose config` prints them, and
//! the file that moves their ports and takes off their containers' fixed
//! names.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::PortsError;

/// The files compose reads in a folder, the first it finds.
pub(crate) const FILES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yml",
    "docker-compose.yaml",
];

/// How the project is read, the first way compose takes: every profile's
/// services, which a target may start, their env files left unread (the
/// ports do not rest on them, and a profile never started may name one
/// never made); then with them, for a compose without that flag; then
/// the services of no profile. Each is what goes before compose's
/// `config`, then after it.
pub(crate) const READINGS: [(&[&str], &[&str]); 3] = [
    (&["--profile", "*"], &["--no-env-resolution"]),
    (&["--profile", "*"], &[]),
    (&[], &[]),
];

/// compose's arguments for `reading`, one of [`READINGS`]: the project
/// resolved, as JSON.
pub(crate) fn config_args(
    (before, after): (&[&'static str], &[&'static str]),
) -> Vec<&'static str> {
    let mut args = before.to_vec();
    args.extend(["config", "--format", "json"]);
    args.extend_from_slice(after);
    args
}

/// The file it reads after it, when there is one.
const OVERRIDES: [&str; 4] = [
    "compose.override.yml",
    "compose.override.yaml",
    "docker-compose.override.yml",
    "docker-compose.override.yaml",
];

/// The compose files of `folder`, as compose reads them there: those
/// `COMPOSE_FILE` lists, from `listed` (the person's environment) or the
/// folder's `.env`, else the first of the usual names and its override.
/// Each is absolute, as the variable that lists them with the plugin's
/// own must be, wherever a command runs.
///
/// # Errors
///
/// The folder holds no compose file.
pub fn files(folder: &Path, listed: Option<&str>) -> Result<Vec<PathBuf>, PortsError> {
    let listed = listed
        .map(str::to_owned)
        .or_else(|| from_env_file(&folder.join(".env"), "COMPOSE_FILE"))
        .filter(|listed| !listed.trim().is_empty());
    if let Some(listed) = listed {
        let files: Vec<PathBuf> = listed
            .split(':')
            .filter(|file| !file.trim().is_empty())
            .map(|file| folder.join(file.trim()))
            .collect();
        if files.iter().all(|file| file.is_file()) && !files.is_empty() {
            return Ok(files);
        }
        return Err(PortsError::NoCompose(folder.to_path_buf()));
    }
    let first = FILES
        .iter()
        .map(|name| folder.join(name))
        .find(|file| file.is_file())
        .ok_or_else(|| PortsError::NoCompose(folder.to_path_buf()))?;
    let mut files = vec![first];
    files.extend(
        OVERRIDES
            .iter()
            .map(|name| folder.join(name))
            .find(|file| file.is_file()),
    );
    Ok(files)
}

/// The value `.env` file `file` sets `name` to, its quotes taken off.
/// Only that line is read.
fn from_env_file(file: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    text.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (key, value) = line.split_once('=')?;
        (key.trim() == name).then(|| {
            value
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_owned()
        })
    })
}

/// A port a service publishes, as compose resolved it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Port {
    /// The port in the container.
    pub target: u16,
    /// `tcp` or `udp`.
    #[serde(default = "tcp")]
    pub protocol: String,
    /// The address of the machine it is published on; all of them when
    /// none.
    #[serde(default)]
    pub host_ip: Option<String>,
    /// The machine's port, or range of them; none for one the system
    /// picks.
    #[serde(default)]
    pub published: Option<String>,
}

fn tcp() -> String {
    "tcp".to_owned()
}

impl Port {
    /// The one port of the machine it is published on, when it is one.
    #[must_use]
    pub fn usual(&self) -> Option<u16> {
        self.published.as_deref()?.parse().ok()
    }
}

/// A service, as far as what it publishes and what it is called go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    /// Its name.
    pub name: String,
    /// The ports it publishes.
    pub ports: Vec<Port>,
    /// Whether its container is given a fixed name, which a second
    /// worktree's would take.
    pub named: bool,
    /// Whether it shares the machine's network, where its ports cannot be
    /// moved.
    pub host_network: bool,
}

#[derive(Deserialize)]
struct Model {
    #[serde(default)]
    services: std::collections::BTreeMap<String, ServiceModel>,
}

#[derive(Deserialize)]
struct ServiceModel {
    #[serde(default)]
    ports: Vec<Port>,
    #[serde(default)]
    container_name: Option<String>,
    #[serde(default)]
    network_mode: Option<String>,
}

/// The services `docker compose config --format json` printed, in the
/// order of their names.
///
/// # Errors
///
/// It is not what compose prints.
pub fn services(printed: &str) -> Result<Vec<Service>, PortsError> {
    let model: Model = serde_json::from_str(printed).map_err(PortsError::Config)?;
    Ok(model
        .services
        .into_iter()
        .map(|(name, service)| Service {
            name,
            ports: service.ports,
            named: service.container_name.is_some(),
            host_network: service.network_mode.as_deref() == Some("host"),
        })
        .collect())
}

/// The file compose reads after the project's: each service's ports in
/// place of its own, those `hidden` gives a port of the worktree's moved
/// to it on localhost, the others as they were; and the fixed names of
/// containers taken off. With nothing to move nor rename, its services
/// are an empty mapping, which compose takes where it refuses none.
#[must_use]
pub fn moving(services: &[Service], hidden: impl Fn(&str, &Port) -> Option<u16>) -> String {
    let quoted = |text: &str| serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned());
    let mut text = String::from(
        "# Written by iq-ports: this worktree's stack on ports of its own, its\n\
         # containers named by compose. Read after the project's files.\n\
         services:",
    );
    let mut any = false;
    for service in services {
        if service.host_network || (service.ports.is_empty() && !service.named) {
            continue;
        }
        if !any {
            text.push('\n');
            any = true;
        }
        text.push_str(&format!("  {}:\n", quoted(&service.name)));
        if service.named {
            text.push_str("    container_name: !reset null\n");
        }
        if service.ports.is_empty() {
            continue;
        }
        text.push_str("    ports: !override\n");
        for port in &service.ports {
            text.push_str(&format!("      - target: {}\n", port.target));
            text.push_str(&format!("        protocol: {}\n", quoted(&port.protocol)));
            match hidden(&service.name, port) {
                Some(hidden) => {
                    text.push_str("        host_ip: \"127.0.0.1\"\n");
                    text.push_str(&format!("        published: \"{hidden}\"\n"));
                }
                None => {
                    if let Some(ip) = &port.host_ip {
                        text.push_str(&format!("        host_ip: {}\n", quoted(ip)));
                    }
                    if let Some(published) = &port.published {
                        text.push_str(&format!("        published: {}\n", quoted(published)));
                    }
                }
            }
        }
    }
    if !any {
        text.push_str(" {}\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What compose prints of a stack: a back end with a fixed name and
    /// three ports, a front end, a tool, and one on the machine's network.
    const PRINTED: &str = r#"{
      "name": "shop",
      "services": {
        "api": {
          "container_name": "shop-api",
          "image": "x",
          "ports": [
            {"mode": "ingress", "target": 8000, "published": "8000", "protocol": "tcp"},
            {"mode": "ingress", "host_ip": "127.0.0.1", "target": 5432, "published": "5432", "protocol": "tcp"},
            {"mode": "ingress", "target": 9000, "protocol": "tcp"}
          ]
        },
        "front": {"ports": [{"mode": "ingress", "target": 80, "published": "3000-3001", "protocol": "tcp"}]},
        "hostnet": {"network_mode": "host"},
        "worker": {"image": "x"}
      }
    }"#;

    #[test]
    fn the_services_are_read_as_compose_resolved_them() {
        let services = services(PRINTED).unwrap();
        let names: Vec<&str> = services.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["api", "front", "hostnet", "worker"]);
        let api = &services[0];
        assert!(api.named && !api.host_network);
        assert_eq!(api.ports.len(), 3);
        assert_eq!(api.ports[0].usual(), Some(8000));
        assert_eq!(api.ports[1].host_ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(api.ports[2].usual(), None);
        // A range is no one port.
        assert_eq!(services[1].ports[0].usual(), None);
        assert!(services[2].host_network);
        assert!(super::services("not json").is_err());
    }

    #[test]
    fn the_file_moves_what_it_is_given_keeps_the_rest_and_takes_names_off() {
        let services = services(PRINTED).unwrap();
        let written = moving(&services, |service, port| match (service, port.usual()) {
            ("api", Some(8000)) => Some(20001),
            ("api", Some(5432)) => Some(20002),
            _ => None,
        });
        assert_eq!(
            written,
            "# Written by iq-ports: this worktree's stack on ports of its own, its\n\
             # containers named by compose. Read after the project's files.\n\
             services:\n  \
               \"api\":\n    \
                 container_name: !reset null\n    \
                 ports: !override\n      \
                   - target: 8000\n        protocol: \"tcp\"\n        host_ip: \"127.0.0.1\"\n        published: \"20001\"\n      \
                   - target: 5432\n        protocol: \"tcp\"\n        host_ip: \"127.0.0.1\"\n        published: \"20002\"\n      \
                   - target: 9000\n        protocol: \"tcp\"\n  \
               \"front\":\n    \
                 ports: !override\n      \
                   - target: 80\n        protocol: \"tcp\"\n        published: \"3000-3001\"\n"
        );
    }

    #[test]
    fn nothing_to_move_nor_rename_is_an_empty_mapping_of_services() {
        let services = services(r#"{"services": {"worker": {"image": "x"}}}"#).unwrap();
        for services in [&services[..], &[]] {
            assert!(
                moving(services, |_, _| None).ends_with("\nservices: {}\n"),
                "{}",
                moving(services, |_, _| None)
            );
        }
        assert_eq!(
            config_args(READINGS[0]),
            [
                "--profile",
                "*",
                "config",
                "--format",
                "json",
                "--no-env-resolution"
            ]
        );
        assert_eq!(config_args(READINGS[2]), ["config", "--format", "json"]);
    }

    #[test]
    fn the_files_are_those_compose_reads_there() {
        let folder = tempfile::tempdir().unwrap();
        let at = |name: &str| folder.path().join(name);
        assert!(files(folder.path(), None).is_err());
        std::fs::write(at("docker-compose.yml"), "services: {}").unwrap();
        assert_eq!(
            files(folder.path(), None).unwrap(),
            [at("docker-compose.yml")]
        );
        std::fs::write(at("compose.yaml"), "services: {}").unwrap();
        std::fs::write(at("compose.override.yaml"), "services: {}").unwrap();
        assert_eq!(
            files(folder.path(), None).unwrap(),
            [at("compose.yaml"), at("compose.override.yaml")]
        );
        // What COMPOSE_FILE lists, in the environment or the folder's
        // .env, comes first.
        std::fs::write(at("dev.yml"), "services: {}").unwrap();
        std::fs::write(
            at(".env"),
            "SECRET=x\nCOMPOSE_FILE=\"compose.yaml:dev.yml\"\n",
        )
        .unwrap();
        assert_eq!(
            files(folder.path(), None).unwrap(),
            [at("compose.yaml"), at("dev.yml")]
        );
        assert_eq!(
            files(folder.path(), Some("dev.yml")).unwrap(),
            [at("dev.yml")]
        );
        assert!(files(folder.path(), Some("gone.yml")).is_err());
    }
}
