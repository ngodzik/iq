//! The sandbox on macOS: Seatbelt, the system's own, through
//! `/usr/bin/sandbox-exec` and a profile written for each command.
//!
//! The profile starts from nothing allowed, as Chrome's and Codex's do, and
//! grants what a program needs to start, then the policy: reading the
//! system and what the policy lists, writing only what it lists, the
//! network only when it says. Seatbelt sees paths resolved, `/private/var`
//! for `/var`, so the policy's are resolved before they are written. Paths
//! reach the profile as parameters, never inside its text, so that no name
//! can change what it says.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::sandbox::Policy;

/// What any program needs to start and run on macOS, after Chrome's
/// profile and Codex's.
const BASE: &str = r#"(version 1)
(deny default)

(allow process-exec)
(allow process-fork)
(allow signal (target same-sandbox))
(allow process-info* (target same-sandbox))
(allow user-preference-read)
(allow sysctl-read)
(allow ipc-posix-sem)
(allow iokit-open (iokit-registry-entry-class "RootDomainUserClient"))
(allow mach-lookup
  (global-name "com.apple.system.opendirectoryd.libinfo")
  (global-name "com.apple.PowerManagement.control"))

(allow pseudo-tty)
(allow file-read* file-write* file-ioctl (literal "/dev/ptmx"))
(allow file-read* file-write*
  (require-all
    (regex #"^/dev/ttys[0-9]+")
    (extension "com.apple.sandbox.pty")))
(allow file-ioctl (regex #"^/dev/ttys[0-9]+"))

(allow file-read-metadata)
"#;

/// Where the system keeps what is not its own: the homes, other disks, and
/// the temporary files of every program, and the data volume that holds
/// them all under another path. Read only where the policy says.
const PRIVATE: &[&str] = &[
    "/Users",
    "/Volumes",
    crate::sandbox::DATA_VOLUME,
    "/private/var/folders",
    "/private/tmp",
    "/private/var/tmp",
];

/// Without the internet, localhost: the servers a command starts, and the
/// tests that talk to them.
const LOCALHOST: &str = r#"
(allow network-bind (local ip "localhost:*"))
(allow network-inbound (local ip "localhost:*"))
(allow network-outbound (remote ip "localhost:*"))
"#;

/// What the network needs beside sockets: the names, the configuration,
/// and the certificates' checks.
const NETWORK: &str = r#"
(allow network-outbound)
(allow network-inbound)
(allow system-socket)
(allow mach-lookup
  (global-name "com.apple.bsd.dirhelper")
  (global-name "com.apple.system.opendirectoryd.membership")
  (global-name "com.apple.SecurityServer")
  (global-name "com.apple.networkd")
  (global-name "com.apple.ocspd")
  (global-name "com.apple.trustd.agent")
  (global-name "com.apple.SystemConfiguration.DNSConfiguration")
  (global-name "com.apple.SystemConfiguration.configd"))
"#;

/// The arguments of `sandbox-exec` that run `program` with `args` under
/// `policy`.
pub(crate) fn arguments(policy: &Policy, program: OsString, args: Vec<OsString>) -> Vec<OsString> {
    let read = resolved(&policy.read);
    let write = resolved(&policy.write);
    let mut out: Vec<OsString> = vec![
        "-p".into(),
        profile(read.len(), write.len(), policy.network).into(),
    ];
    let params = read
        .iter()
        .enumerate()
        .map(|(i, path)| (format!("READ_{i}"), path))
        .chain(
            write
                .iter()
                .enumerate()
                .map(|(i, path)| (format!("WRITE_{i}"), path)),
        );
    for (name, path) in params {
        let mut param = OsString::from(format!("-D{name}="));
        param.push(path.as_os_str());
        out.push(param);
    }
    out.push("--".into());
    out.push(program);
    out.extend(args);
    out
}

/// The profile for `reads` folders to read and `writes` to write, named
/// `READ_0`, `WRITE_0` and on.
fn profile(reads: usize, writes: usize, network: bool) -> String {
    let mut profile = BASE.to_owned();
    let private: String = PRIVATE
        .iter()
        .map(|place| format!("\n    (require-not (subpath \"{place}\"))"))
        .collect();
    profile.push_str(&format!("(allow file-read*\n  (require-all{private}))\n"));
    let params = |prefix: &str, count: usize| -> String {
        (0..count)
            .map(|i| format!("\n  (subpath (param \"{prefix}_{i}\"))"))
            .collect()
    };
    if reads > 0 {
        profile.push_str(&format!("(allow file-read*{})\n", params("READ", reads)));
    }
    if writes > 0 {
        profile.push_str(&format!(
            "(allow file-read* file-write*{})\n",
            params("WRITE", writes)
        ));
    }
    profile.push_str(if network { NETWORK } else { LOCALHOST });
    profile
}

/// The paths as Seatbelt sees them: links resolved where they exist, the
/// others left out, since nothing is there to reach.
fn resolved(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = paths
        .iter()
        .filter_map(|path| Path::new(path).canonicalize().ok())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_reach_the_profile_as_parameters_only() {
        let dir = tempfile::tempdir().unwrap();
        let odd = dir.path().join("a \"quoted\") (allow default");
        std::fs::create_dir(&odd).unwrap();
        let policy = Policy {
            read: vec![PathBuf::from("/usr"), PathBuf::from("/no/such/folder")],
            write: vec![odd.clone()],
            network: false,
            shared: None,
        };
        let args = arguments(&policy, "/bin/sh".into(), vec!["-c".into(), "true".into()]);
        let text: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(text[0], "-p");
        let profile = &text[1];
        assert!(!profile.contains("quoted"), "{profile}");
        assert!(profile.starts_with("(version 1)\n(deny default)"));
        assert!(profile.contains("(subpath (param \"READ_0\"))"));
        assert!(!profile.contains("READ_1"), "a missing folder is left out");
        assert!(profile.contains("(subpath (param \"WRITE_0\"))"));
        assert!(profile.contains("(require-not (subpath \"/Users\"))"));
        assert!(profile.contains("(require-not (subpath \"/System/Volumes/Data\"))"));
        assert!(!profile.contains("(allow network-outbound)"));
        assert!(profile.contains("(remote ip \"localhost:*\")"));
        assert_eq!(text[2], "-DREAD_0=/usr");
        assert_eq!(
            text[3],
            format!("-DWRITE_0={}", odd.canonicalize().unwrap().display())
        );
        assert_eq!(text[4..], ["--", "/bin/sh", "-c", "true"]);
    }

    #[test]
    fn the_network_is_in_the_profile_only_when_given() {
        assert!(profile(0, 1, true).contains("(allow network-outbound)"));
        let closed = profile(0, 1, false);
        assert!(!closed.contains("(allow network-outbound)"), "{closed}");
        assert!(closed.contains("(allow network-outbound (remote ip \"localhost:*\"))"));
        // Parentheses balance, or sandbox-exec refuses the profile.
        for network in [false, true] {
            let text = profile(2, 3, network);
            let depth = text.chars().try_fold(0i32, |depth, c| {
                let depth = match c {
                    '(' => depth + 1,
                    ')' => depth - 1,
                    _ => depth,
                };
                (depth >= 0).then_some(depth)
            });
            assert_eq!(depth, Some(0), "{text}");
        }
    }
}
