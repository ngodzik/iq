//! The system's notifications, sent through its own tool whatever the
//! terminal is: `notify-send` on Linux, which needs a notification daemon
//! as most desktops run, and `osascript` on macOS, where a terminal's own
//! escape shows nothing until the system allowed its notifications.

use std::process::{Command, Stdio};

/// Shows `text` as a notification of the system's, titled iq, on a thread
/// of its own: iq never waits for it, and one that cannot be shown is
/// left. Only iq's own fixed words go there, never the work's.
pub(crate) fn notify(text: &'static str) {
    std::thread::spawn(move || {
        let mut command = if cfg!(target_os = "macos") {
            let mut osascript = Command::new("osascript");
            osascript.args([
                "-e",
                &format!("display notification \"{text}\" with title \"iq\""),
            ]);
            osascript
        } else {
            let mut send = Command::new("notify-send");
            send.args(["iq", text]);
            send
        };
        let _ = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}
