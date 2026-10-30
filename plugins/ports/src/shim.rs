//! The compose docker runs in a worktree, so that a command handing compose
//! its files itself keeps the stack apart too.
//!
//! Given `--file`, compose reads the files it names and no longer
//! `COMPOSE_FILE`, which lists the plugin's file after the project's: a
//! `make` target running `docker compose --file compose.yaml up -d` would
//! start the stack on the project's ports, under its containers' fixed
//! names. Given `--project-name`, it takes the project named, the person's
//! own checkout's. Nothing of the project is changed to reach it: docker
//! finds its compose plugin through the folder `DOCKER_CONFIG` names, and
//! the worktree's commands are pointed to the plugin's.
//!
//! There, the person's settings are as in theirs (their context, their
//! credentials helpers, their proxies, their other plugins, and the rest of
//! their folder linked), but the logins it holds itself, which are not
//! copied (the names of the registries a credentials helper holds the
//! logins of are); and docker's compose is iq-ports, which adds to the
//! files given one that moves their ports, puts the worktree's project in
//! place of another, then runs docker's own compose in its place.

use std::ffi::OsString;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{Map, Value};

use crate::PortsError;
use crate::compose;
use crate::stack::{self, hidden_of, quoted};
use crate::store::{Kept, Place, Store};

/// What iq-ports is run with as docker's compose: then the store, the
/// person's docker folder, and what docker hands its compose plugin.
pub const AS_COMPOSE: &str = "--as-compose";

/// The program docker runs as `docker compose`.
const PLUGIN: &str = "docker-compose";

/// Where docker looks for its plugins after the person's folders, on Linux
/// and macOS.
const SYSTEM: [&str; 4] = [
    "/usr/local/lib/docker/cli-plugins",
    "/usr/local/libexec/docker/cli-plugins",
    "/usr/lib/docker/cli-plugins",
    "/usr/libexec/docker/cli-plugins",
];

/// What docker's own folder holds that the plugin's does not link: the
/// settings, written there without the logins, and the plugins, its own.
const NOT_LINKED: [&str; 2] = ["config.json", "cli-plugins"];

/// The fields of a login the person's settings hold themselves.
const LOGIN: [&str; 3] = ["auth", "identitytoken", "registrytoken"];

/// iq-ports as docker's compose in a worktree, keeping the settings of the
/// person's docker folder.
#[derive(Debug, Clone)]
pub struct Shim {
    program: PathBuf,
    persons: PathBuf,
}

impl Shim {
    /// `program`, iq-ports, as docker's compose, keeping the settings of
    /// `persons`, the person's docker folder.
    #[must_use]
    pub fn new(program: impl Into<PathBuf>, persons: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            persons: persons.into(),
        }
    }

    /// This program, keeping the settings of the folder the person's docker
    /// reads: `docker_config`, their `DOCKER_CONFIG`, unless it is the
    /// plugin's own, else `~/.docker`.
    #[must_use]
    pub fn of_person(home: &Path, docker_config: Option<PathBuf>, store: &Store) -> Option<Self> {
        let program = std::env::current_exe().ok()?;
        let ours = store.docker();
        let persons = docker_config
            .filter(|folder| folder.is_absolute() && !folder.starts_with(&ours))
            .unwrap_or_else(|| home.join(".docker"));
        Some(Self::new(program, persons))
    }

    /// Sets up the folder a worktree's docker is pointed to, as the
    /// person's settings are now: that folder, and what of theirs does not
    /// reach it, said.
    ///
    /// # Errors
    ///
    /// docker's compose plugin is not where the person's docker looks for
    /// it, their settings cannot be read, or the store cannot be written.
    pub fn set_up(&self, store: &Store) -> Result<(PathBuf, Vec<String>), PortsError> {
        let ours = store.docker();
        if real_compose(&self.persons, &ours).is_none() {
            return Err(PortsError::NoComposePlugin(self.persons.clone()));
        }
        let plugins = ours.join("cli-plugins");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&plugins)
            .map_err(|source| PortsError::File {
                path: plugins.clone(),
                source,
            })?;
        let (settings, said) = settings(&self.persons, &plugins)?;
        store.write(&ours.join("config.json"), &settings)?;
        link(&self.persons, &ours)?;
        let script = format!(
            "#!/bin/sh\n\
             # Written by iq-ports: docker's compose in a worktree, the files a command\n\
             # hands it read with the one that moves their ports.\n\
             exec {} {AS_COMPOSE} {} {} \"$@\"\n",
            quoted(&self.program.display().to_string()),
            quoted(&store.folder().display().to_string()),
            quoted(&self.persons.display().to_string()),
        );
        store.write_program(&plugins.join(PLUGIN), &script)?;
        Ok((ours, said))
    }
}

/// The folders the person's settings in `persons` add to those docker
/// looks for plugins in.
fn extra_dirs(persons: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(persons.join("config.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|settings| settings.get("cliPluginsExtraDirs").cloned())
        .and_then(|dirs| serde_json::from_value::<Vec<PathBuf>>(dirs).ok())
        .unwrap_or_default()
}

/// Whether `path` is in the plugin's folder `ours`, through links or not.
fn is_ours(path: &Path, ours: &Path) -> bool {
    path.starts_with(ours)
        || path
            .canonicalize()
            .ok()
            .zip(ours.canonicalize().ok())
            .is_some_and(|(path, ours)| path.starts_with(ours))
}

/// docker's own compose plugin, where docker looks for it for the person
/// whose folder is `persons`: in the folders their settings add, theirs,
/// then the system's; none of the plugin's folder `ours`.
#[must_use]
pub fn real_compose(persons: &Path, ours: &Path) -> Option<PathBuf> {
    extra_dirs(persons)
        .into_iter()
        .chain(std::iter::once(persons.join("cli-plugins")))
        .chain(SYSTEM.iter().map(PathBuf::from))
        .map(|folder| folder.join(PLUGIN))
        .find(|path| {
            std::fs::metadata(path)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
                && !is_ours(path, ours)
        })
}

/// The settings of the plugin's folder: the person's, in `persons`, but the
/// logins they hold themselves (the names of the registries a credentials
/// helper holds the logins of kept), with the plugin's `plugins` folder first
/// where docker looks for plugins, then the person's; and what is not
/// carried, said.
fn settings(persons: &Path, plugins: &Path) -> Result<(String, Vec<String>), PortsError> {
    let file = persons.join("config.json");
    let mut settings: Map<String, Value> = match std::fs::read_to_string(&file) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|source| PortsError::DockerSettings { path: file, source })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(source) => return Err(PortsError::File { path: file, source }),
    };
    let mut said = Vec::new();
    if let Some(Value::Object(logins)) = settings.remove("auths") {
        // A credentials helper leaves an empty entry for each registry it
        // holds a login of: its name alone, which some projects look for
        // before they start their stack, so it is kept.
        let (held, named): (Vec<_>, Vec<_>) = logins.into_iter().partition(|(_, login)| {
            LOGIN.iter().any(|field| {
                login
                    .get(field)
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.is_empty())
            })
        });
        if !named.is_empty() {
            let named = named
                .into_iter()
                .map(|(registry, _)| (registry, Value::Object(Map::new())))
                .collect();
            settings.insert("auths".to_owned(), Value::Object(named));
        }
        if !held.is_empty() {
            said.push(
                "the registries docker logged in to without a credentials helper are not \
                 logged in to from a worktree: docker's config.json holds those logins \
                 itself, and they are not copied (a `credsStore` keeps them for both)"
                    .to_owned(),
            );
        }
    }
    let theirs = settings
        .get("cliPluginsExtraDirs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut dirs = vec![Value::String(plugins.display().to_string())];
    dirs.extend(theirs.into_iter().filter(Value::is_string));
    dirs.push(Value::String(
        persons.join("cli-plugins").display().to_string(),
    ));
    settings.insert("cliPluginsExtraDirs".to_owned(), Value::Array(dirs));
    let text = serde_json::to_string_pretty(&Value::Object(settings)).unwrap_or_default();
    Ok((text, said))
}

/// Links in the plugin's folder `ours` what the person's `persons` holds,
/// but its settings and plugins, so that docker finds there what it finds
/// in theirs: its contexts, its builders. Links to what is gone are taken
/// off; what docker made in the plugin's folder itself is left.
fn link(persons: &Path, ours: &Path) -> Result<(), PortsError> {
    let failed = |path: &Path| {
        let path = path.to_path_buf();
        move |source| PortsError::File { path, source }
    };
    for entry in std::fs::read_dir(ours).map_err(failed(ours))?.flatten() {
        let path = entry.path();
        let linked = path
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_symlink());
        if linked && !path.exists() {
            std::fs::remove_file(&path).map_err(failed(&path))?;
        }
    }
    let Ok(entries) = std::fs::read_dir(persons) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if NOT_LINKED.iter().any(|kept| name == *kept) {
            continue;
        }
        let target = entry.path();
        let link = ours.join(&name);
        match link.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => {
                if std::fs::read_link(&link).is_ok_and(|to| to == target) {
                    continue;
                }
                std::fs::remove_file(&link).map_err(failed(&link))?;
            }
            Ok(_) => continue,
            Err(_) => {}
        }
        std::os::unix::fs::symlink(&target, &link).map_err(failed(&link))?;
    }
    Ok(())
}

/// Runs as docker's compose, `args` as docker hands them, keeping the
/// store `store` and the person's docker folder `persons`: compose's
/// arguments made the worktree's, then docker's own compose run in this
/// program's place. It returns only when that cannot be.
pub fn run(store: &Store, persons: &Path, args: Vec<OsString>) -> std::io::Error {
    let Some(real) = real_compose(persons, &store.docker()) else {
        return std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "docker's compose plugin is not where docker looks for it for {}",
                persons.display()
            ),
        );
    };
    let args = match args.split_first() {
        Some((first, rest)) if first == "compose" => std::iter::once(first.clone())
            .chain(worktrees(store, persons, rest))
            .collect(),
        // docker asking what the plugin is, or anything else: as given.
        _ => args,
    };
    std::process::Command::new(real).args(args).exec()
}

/// compose's arguments `args` as the worktree's whose project
/// `--project-name` names when it is one kept apart, else the one
/// `COMPOSE_PROJECT_NAME` names: that project in place of another, and
/// the files given read with one that moves their ports. As given when
/// compose is not run for a worktree kept apart.
fn worktrees(store: &Store, persons: &Path, args: &[OsString]) -> Vec<OsString> {
    let as_given = || args.to_vec();
    let Some(words) = args
        .iter()
        .map(|arg| arg.to_str().map(str::to_owned))
        .collect::<Option<Vec<String>>>()
    else {
        return as_given();
    };
    let kept = store.load();
    let given = options(&words);
    let named = given.project.as_ref().map(|project| project.value.clone());
    let env = std::env::var("COMPOSE_PROJECT_NAME").ok();
    let Some((folder, place)) = named
        .as_deref()
        .and_then(|named| kept.of_project(named))
        .or_else(|| env.as_deref().and_then(|env| kept.of_project(env)))
    else {
        return as_given();
    };
    let project = place.project.clone();
    if let Some(named) = &named
        && *named != project
    {
        eprintln!("iq-ports: the project {named} is read as this worktree's, {project}");
    }
    let file = if given.files.is_empty() {
        None
    } else if given.files.iter().any(|file| file.value == "-") {
        eprintln!(
            "iq-ports: a compose file read from the standard input is read alone: the ports \
             of {project} are not moved"
        );
        None
    } else {
        match moving(store, persons, &kept, (folder, place), &given) {
            Ok(file) => Some(file),
            Err(error) => {
                eprintln!("iq-ports: the ports of {project} are not moved: {error}");
                None
            }
        }
    };
    rewritten(words, &given, &project, file.as_deref())
        .into_iter()
        .map(OsString::from)
        .collect()
}

/// The file that moves the ports of the services the files `given` names
/// resolve to, for `project`: each to the one the worktree was given for
/// it, else to a free one, kept for the next start.
fn moving(
    store: &Store,
    persons: &Path,
    kept: &Kept,
    (folder, place): (&PathBuf, &Place),
    given: &Options,
) -> Result<PathBuf, PortsError> {
    let project = place.project.as_str();
    let mut args = vec!["compose".to_owned()];
    for file in &given.files {
        args.extend(["--file".to_owned(), file.value.clone()]);
    }
    for env_file in &given.env_files {
        args.extend(["--env-file".to_owned(), env_file.value.clone()]);
    }
    if let Some(directory) = &given.directory {
        args.extend(["--project-directory".to_owned(), directory.value.clone()]);
    }
    args.extend(["--project-name".to_owned(), project.to_owned()]);
    // Read the first way compose takes: every profile's services first.
    let mut printed = config(persons, &args, &compose::config_args(compose::READINGS[0]));
    for way in &compose::READINGS[1..] {
        if printed.is_ok() {
            break;
        }
        printed = config(persons, &args, &compose::config_args(*way));
    }
    let printed = printed?;
    let services = compose::services(&printed)?;
    let mut taken = stack::taken_by_others(kept, folder);
    let (moved, _) = stack::moves(project, &services, Some(place), &mut taken)?;
    let text = compose::moving(&services, |name, port| hidden_of(&moved, name, port));
    let file = store.ports_file_of(project, stack::hash(&key(given)));
    store.write(&file, &text)?;
    if moved.iter().any(|m| !place.moved.contains(m)) {
        // Kept as it is now: another command may have kept it meanwhile.
        let mut now = store.load();
        now.keep(
            folder.clone(),
            Place {
                project: project.to_owned(),
                moved,
            },
        );
        store.save(&now)?;
    }
    Ok(file)
}

/// What names the files `given` are read as: each file, resolved, and
/// what changes what they resolve to.
fn key(given: &Options) -> String {
    let here = std::env::current_dir().unwrap_or_default();
    let resolved = |value: &str| {
        let path = here.join(value);
        path.canonicalize().unwrap_or(path).display().to_string()
    };
    let mut key: Vec<String> = given.files.iter().map(|f| resolved(&f.value)).collect();
    key.extend(
        given
            .env_files
            .iter()
            .map(|f| format!("--env-file {}", resolved(&f.value))),
    );
    key.extend(
        given
            .directory
            .iter()
            .map(|d| format!("--project-directory {}", resolved(&d.value))),
    );
    key.join("\n")
}

/// What `docker compose` with `args`, then `more`, its `config` among
/// them, prints of the project: run as the person's docker runs it, so
/// that it does not come back here.
fn config(persons: &Path, args: &[String], more: &[&str]) -> Result<String, PortsError> {
    let mut command = std::process::Command::new("docker");
    command
        .args(args)
        .args(more)
        .env("DOCKER_CONFIG", persons)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // What docker told this plugin is not for the one asked.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("DOCKER_CLI_PLUGIN_") {
            command.env_remove(name);
        }
    }
    let failed = |said: String| PortsError::Command {
        command: "docker compose config".to_owned(),
        said,
    };
    let output = command
        .output()
        .map_err(|error| failed(error.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(failed(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

/// A value one of compose's options is given: `words[at]` is `prefix`,
/// then the value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Valued {
    at: usize,
    prefix: String,
    value: String,
}

/// What compose's own options, those before its command, are given.
#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    files: Vec<Valued>,
    project: Option<Valued>,
    env_files: Vec<Valued>,
    directory: Option<Valued>,
}

/// What an option of compose's that takes a value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Taking {
    File,
    Project,
    EnvFile,
    Directory,
    Other,
}

/// What compose's option `long`, or `short`, takes, when it takes a value.
fn taking(long: Option<&str>, short: Option<char>) -> Option<Taking> {
    match (long, short) {
        (Some("file"), _) | (_, Some('f')) => Some(Taking::File),
        (Some("project-name"), _) | (_, Some('p')) => Some(Taking::Project),
        (Some("env-file"), _) => Some(Taking::EnvFile),
        (Some("project-directory" | "workdir"), _) => Some(Taking::Directory),
        (Some("profile" | "ansi" | "parallel" | "progress"), _) => Some(Taking::Other),
        _ => None,
    }
}

/// What compose's own options are given in `words`, its arguments: read
/// up to its command, as compose reads them (`-f x`, `-fx`, `-f=x`,
/// `--file x`, `--file=x`).
fn options(words: &[String]) -> Options {
    let mut options = Options::default();
    let mut at = 0;
    while let Some(word) = words.get(at) {
        if word == "--" || word == "-" || !word.starts_with('-') {
            break;
        }
        let (takes, joined) = match word.strip_prefix("--") {
            Some(long) => match long.split_once('=') {
                Some((name, value)) => (taking(Some(name), None), Some(value)),
                None => (taking(Some(long), None), None),
            },
            None => {
                let mut chars = word[1..].chars();
                let short = chars.next();
                let rest = chars.as_str();
                let value = (!rest.is_empty()).then(|| rest.strip_prefix('=').unwrap_or(rest));
                (taking(None, short), value)
            }
        };
        let Some(takes) = takes else {
            at += 1;
            continue;
        };
        let valued = match joined {
            Some(value) => Valued {
                at,
                prefix: word[..word.len() - value.len()].to_owned(),
                value: value.to_owned(),
            },
            None => {
                at += 1;
                let Some(value) = words.get(at) else {
                    break;
                };
                Valued {
                    at,
                    prefix: String::new(),
                    value: value.clone(),
                }
            }
        };
        match takes {
            Taking::File => options.files.push(valued),
            Taking::Project => options.project = Some(valued),
            Taking::EnvFile => options.env_files.push(valued),
            Taking::Directory => options.directory = Some(valued),
            Taking::Other => {}
        }
        at += 1;
    }
    options
}

/// `words`, compose's arguments, `given` as their options: `project` in
/// place of another named, and `file` read after the files named.
fn rewritten(
    mut words: Vec<String>,
    given: &Options,
    project: &str,
    file: Option<&Path>,
) -> Vec<String> {
    if let Some(named) = &given.project
        && named.value != project
    {
        words[named.at] = format!("{}{project}", named.prefix);
    }
    if let (Some(file), Some(last)) = (file, given.files.last()) {
        let after = last.at + 1;
        words.splice(
            after..after,
            ["--file".to_owned(), file.display().to_string()],
        );
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn compose_s_options_are_read_up_to_its_command() {
        let given = options(&words(
            "--ansi never -f compose.yaml --file=dev.yaml -pshop --env-file .env.dev \
             --dry-run up -d -f",
        ));
        let values = |list: &[Valued]| list.iter().map(|v| v.value.clone()).collect::<Vec<_>>();
        assert_eq!(values(&given.files), ["compose.yaml", "dev.yaml"]);
        assert_eq!(given.files[1].prefix, "--file=");
        assert_eq!(given.files[1].at, 4);
        assert_eq!(
            given.project,
            Some(Valued {
                at: 5,
                prefix: "-p".into(),
                value: "shop".into()
            })
        );
        assert_eq!(values(&given.env_files), [".env.dev"]);
        // A command's own -f is not compose's: logs -f follows.
        assert!(options(&words("logs -f web")).files.is_empty());
        assert!(options(&words("-- -f x")).files.is_empty());
        assert_eq!(
            options(&words("-p=shop --project-directory /w ps")),
            Options {
                project: Some(Valued {
                    at: 0,
                    prefix: "-p=".into(),
                    value: "shop".into()
                }),
                directory: Some(Valued {
                    at: 2,
                    prefix: String::new(),
                    value: "/w".into()
                }),
                ..Options::default()
            }
        );
    }

    #[test]
    fn the_worktree_s_project_and_file_take_their_place() {
        let line = words("--project-name shop -f compose.yaml --profile dev up -d");
        let given = options(&line);
        assert_eq!(
            rewritten(
                line,
                &given,
                "shop-lead",
                Some(Path::new("/s/shop-lead.1.yaml"))
            ),
            words(
                "--project-name shop-lead -f compose.yaml --file /s/shop-lead.1.yaml \
                 --profile dev up -d"
            )
        );
        let line = words("-pshop ps");
        let given = options(&line);
        assert_eq!(
            rewritten(line, &given, "shop-lead", None),
            words("-pshop-lead ps")
        );
        let line = words("--file=compose.yaml up");
        let given = options(&line);
        assert_eq!(
            rewritten(line, &given, "shop-lead", Some(Path::new("/s/x.yaml"))),
            words("--file=compose.yaml --file /s/x.yaml up")
        );
    }

    /// An executable file at `path`.
    fn program(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_person_s_settings_are_kept_but_their_logins_and_their_folder_linked() {
        let home = tempfile::tempdir().unwrap();
        let persons = home.path().join(".docker");
        let extra = home.path().join("desktop/cli-plugins");
        program(&extra.join(PLUGIN));
        std::fs::create_dir_all(persons.join("contexts/meta")).unwrap();
        std::fs::create_dir_all(persons.join("buildx")).unwrap();
        std::fs::write(
            persons.join("config.json"),
            serde_json::json!({
                "auths": {
                    "registry.example": {"auth": "c2VjcmV0"},
                    "helped.example": {},
                    "token.example": {"identitytoken": "dG9rZW4="},
                },
                "credsStore": "desktop",
                "currentContext": "desktop-linux",
                "proxies": {"default": {"httpProxy": "http://proxy:3128"}},
                "cliPluginsExtraDirs": [extra],
            })
            .to_string(),
        )
        .unwrap();
        let store = Store::at(home.path().join("state"));
        let shim = Shim::new("/bin/iq-ports", &persons);
        let (folder, said) = shim.set_up(&store).unwrap();
        assert_eq!(folder, store.docker());
        assert_eq!(said.len(), 1, "{said:?}");
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(folder.join("config.json")).unwrap())
                .unwrap();
        assert_eq!(written["auths"], serde_json::json!({"helped.example": {}}));
        assert!(!written.to_string().contains("c2VjcmV0"));
        assert!(!written.to_string().contains("dG9rZW4="));
        assert_eq!(written["credsStore"], "desktop");
        assert_eq!(written["currentContext"], "desktop-linux");
        assert_eq!(
            written["proxies"]["default"]["httpProxy"],
            "http://proxy:3128"
        );
        assert_eq!(
            written["cliPluginsExtraDirs"],
            serde_json::json!([
                folder.join("cli-plugins"),
                extra,
                persons.join("cli-plugins")
            ])
        );
        assert_eq!(
            std::fs::read_link(folder.join("contexts")).unwrap(),
            persons.join("contexts")
        );
        assert!(folder.join("buildx").is_dir());
        let script = folder.join("cli-plugins").join(PLUGIN);
        let text = std::fs::read_to_string(&script).unwrap();
        assert!(text.ends_with(&format!(
            "exec /bin/iq-ports {AS_COMPOSE} {} {} \"$@\"\n",
            store.folder().display(),
            persons.display()
        )));
        let mode = std::fs::metadata(&script).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        // docker's own compose is the person's, never the plugin's.
        assert_eq!(real_compose(&persons, &folder), Some(extra.join(PLUGIN)));
        // Set up again, as the person's folder now is.
        std::fs::remove_dir_all(persons.join("buildx")).unwrap();
        shim.set_up(&store).unwrap();
        assert!(folder.join("buildx").symlink_metadata().is_err());
        assert!(folder.join("contexts").is_dir());
    }

    #[test]
    fn without_docker_s_compose_nothing_takes_its_place() {
        let home = tempfile::tempdir().unwrap();
        let persons = home.path().join(".docker");
        let store = Store::at(home.path().join("state"));
        let no_compose = SYSTEM
            .iter()
            .all(|dir| !Path::new(dir).join(PLUGIN).exists());
        if no_compose {
            assert!(matches!(
                Shim::new("/bin/iq-ports", &persons).set_up(&store),
                Err(PortsError::NoComposePlugin(_))
            ));
        }
        std::fs::create_dir_all(&persons).unwrap();
        std::fs::write(persons.join("config.json"), "not json").unwrap();
        program(&persons.join("cli-plugins").join(PLUGIN));
        assert!(matches!(
            Shim::new("/bin/iq-ports", &persons).set_up(&store),
            Err(PortsError::DockerSettings { .. })
        ));
    }
}
