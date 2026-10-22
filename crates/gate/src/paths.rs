//! Paths as the operating system reaches them, and what they hold.

use std::path::{Component, Path, PathBuf};

/// `path`, made absolute against `base`, each link resolved where it exists
/// as the system would follow it (`link/..` is the link's target's parent,
/// not `.`), what does not exist yet kept as written.
pub(crate) fn resolve(path: &Path, base: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => out.push(component),
            Component::CurDir => {}
            Component::ParentDir => {
                out = out.canonicalize().unwrap_or(out);
                out.pop();
            }
            Component::Normal(name) => {
                out.push(name);
                if let Ok(real) = out.canonicalize() {
                    out = real;
                }
            }
        }
    }
    out
}

/// Files whose name says they hold secrets.
const SECRET_NAMES: &[&str] = &[
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    ".dockercfg",
    "credentials",
    "credentials.json",
    ".credentials.json",
    "auth.json",
    "kubeconfig",
];

/// Endings of files of keys and certificates with their private part.
const SECRET_ENDINGS: &[&str] = &[".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"];

/// Folders of the home that hold credentials, whatever their files are
/// named.
pub(crate) const SECRET_FOLDERS: &[&str] = &[
    ".ssh",
    ".aws",
    ".azure",
    ".gnupg",
    ".docker",
    ".kube",
    ".config/gh",
    ".config/gcloud",
    ".config/op",
    ".password-store",
    ".local/share/keyrings",
    "Library/Keychains",
    ".claude",
    ".codex",
    ".iq",
];

/// Whether `path` holds secrets, as its name or its folder says: a file of
/// variables (`.env`, `.env.local`, `prod.env`), a private key, a file of
/// credentials, anything in a folder of credentials of the home.
pub(crate) fn secret(path: &Path, home: Option<&Path>) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let key = ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"]
        .iter()
        .any(|key| name.starts_with(key))
        && !name.ends_with(".pub");
    variables(&name)
        || key
        || SECRET_NAMES.contains(&name.as_str())
        || SECRET_ENDINGS.iter().any(|ending| name.ends_with(ending))
        || home.is_some_and(|home| {
            SECRET_FOLDERS
                .iter()
                .any(|folder| path.starts_with(home.join(folder)))
        })
}

/// Whether a file's name says it holds variables: `.env`, `.env.local`,
/// `prod.env`.
pub(crate) fn variables(name: &str) -> bool {
    name == ".env" || name.starts_with(".env.") || name.ends_with(".env")
}

/// Where code written in a workspace runs later on its own, outside the
/// sandbox, the person doing nothing but what they always do: git's hooks
/// and settings, the folders hooks are kept in, direnv's file, the editor's
/// tasks, continuous integration.
pub(crate) const RUNS_LATER: &[&str] = &[
    ".git",
    ".githooks",
    ".husky",
    ".envrc",
    ".vscode",
    ".pre-commit-config.yaml",
    ".github/workflows",
    ".gitlab-ci.yml",
];

/// Whether code written at `relative`, a path within a workspace, runs
/// later on its own.
pub(crate) fn runs_later(relative: &Path) -> bool {
    RUNS_LATER.iter().any(|place| relative.starts_with(place))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_followed_as_the_system_follows_it() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("real/inner")).unwrap();
        std::os::unix::fs::symlink(base.join("real/inner"), base.join("link")).unwrap();
        assert_eq!(resolve(Path::new("link/../x"), &base), base.join("real/x"));
        assert_eq!(
            resolve(Path::new("./new/file"), &base),
            base.join("new/file")
        );
        assert_eq!(
            resolve(Path::new("a/../../b"), &base),
            base.parent().unwrap().join("b")
        );
    }

    #[test]
    fn secrets_are_known_by_name_and_folder() {
        let home = Path::new("/home/me");
        for path in [
            "/w/.env",
            "/w/.env.production",
            "/w/config/prod.env",
            "/w/certs/server.key",
            "/home/me/.ssh/config",
            "/home/me/.aws/config",
            "/w/id_ed25519",
        ] {
            assert!(secret(Path::new(path), Some(home)), "{path}");
        }
        for path in [
            "/w/src/main.rs",
            "/w/.envrc",
            "/home/me/.ssh.md",
            "/w/id_ed25519.pub",
        ] {
            assert!(!secret(Path::new(path), Some(home)), "{path}");
        }
    }

    #[test]
    fn hooks_and_ci_run_later() {
        assert!(runs_later(Path::new(".git/hooks/pre-commit")));
        assert!(runs_later(Path::new(".github/workflows/ci.yml")));
        assert!(!runs_later(Path::new(".github/CODEOWNERS")));
        assert!(!runs_later(Path::new("src/git.rs")));
    }
}
