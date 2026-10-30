//! When iq syncs its folder of data with the repository the settings'
//! `[sync]` names, and what this machine is called in what it sends.

use std::time::Duration;

/// How long iq waits for a sync as it opens, before opening without it.
pub(crate) const OPENING: Duration = Duration::from_secs(20);

/// How long it waits for the last one as it closes.
pub(crate) const CLOSING: Duration = Duration::from_secs(20);

/// How often it looks for something to send while it runs.
pub(crate) const LOOK_EVERY: Duration = Duration::from_secs(120);

/// How often it syncs anyway, for what other machines sent.
pub(crate) const TAKE_EVERY: Duration = Duration::from_secs(10 * 60);

/// This machine's name, as the commits it sends say it.
pub(crate) fn machine() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        })
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "a machine".to_owned())
}
