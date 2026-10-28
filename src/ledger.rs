//! What iq keeps of its sessions in its folder, for the terminal to show
//! after them: each call billed to the provider, one line of JSON each.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use iq_core::Usd;
use iq_ui::costs::Spent;
use serde::{Deserialize, Serialize};

/// How long the calls billed to the provider are kept: a month and a few
/// days, for the bill to be followed over a whole month.
pub(crate) const BILLED_KEEP_SECS: u64 = 35 * 24 * 60 * 60;

/// One call billed to the provider, as kept.
#[derive(Debug, Serialize, Deserialize)]
struct Line {
    at: u64,
    agent: String,
    model: String,
    cost: Option<f64>,
}

/// The file kept, in iq's folder.
pub(crate) struct Ledger {
    calls: PathBuf,
    /// Held while a call is written, so that two never mix their lines.
    writing: Mutex<()>,
}

impl Ledger {
    /// The ledger of `folder`.
    pub(crate) fn open(folder: &Path) -> Self {
        Self {
            calls: folder.join("billed.jsonl"),
            writing: Mutex::new(()),
        }
    }

    /// The calls billed since `since`, in seconds since 1970, oldest first.
    pub(crate) fn billed(&self, since: u64) -> Vec<Spent> {
        let Ok(text) = std::fs::read_to_string(&self.calls) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|line| serde_json::from_str::<Line>(line).ok())
            .filter(|line| line.at >= since)
            .map(|line| Spent {
                at: line.at,
                model: line.model,
                cost: line.cost.map(Usd),
                tokens: 0,
            })
            .collect()
    }

    /// Keeps a call billed to the provider. A call that cannot be kept is
    /// still shown: the terminal counts it as it comes.
    pub(crate) fn bill(&self, agent: &str, model: &str, cost: Option<Usd>) {
        let line = Line {
            at: iq_ui::now(),
            agent: agent.to_owned(),
            model: model.to_owned(),
            cost: cost.map(|c| c.0),
        };
        let Ok(mut text) = serde_json::to_string(&line) else {
            return;
        };
        text.push('\n');
        let _writing = self.writing.lock();
        let written = crate::lock::holding(&self.calls, || {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&self.calls)
                .and_then(|mut file| file.write_all(text.as_bytes()))
        });
        if let Err(error) = written {
            tracing::warn!(
                "could not keep a billed call in {}: {error}",
                self.calls.display()
            );
        }
    }

    /// Drops calls older than the history kept, so that the file does not
    /// grow without end. Another iq adding a call meanwhile waits for it.
    pub(crate) fn trim(&self) {
        let _writing = self.writing.lock();
        crate::lock::holding(&self.calls, || self.trim_held());
    }

    fn trim_held(&self) {
        let since = iq_ui::now().saturating_sub(BILLED_KEEP_SECS);
        let Ok(text) = std::fs::read_to_string(&self.calls) else {
            return;
        };
        let kept: String = text
            .lines()
            .filter(|line| serde_json::from_str::<Line>(line).is_ok_and(|l| l.at >= since))
            .map(|line| format!("{line}\n"))
            .collect();
        if kept.len() < text.len() {
            let temporary = self.calls.with_extension("jsonl.new");
            if std::fs::write(&temporary, kept).is_ok() {
                let _ = std::fs::rename(&temporary, &self.calls);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billed_calls_outlive_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(dir.path());
        ledger.bill("lead", "openai/gpt-5-mini", Some(Usd(0.01)));
        ledger.bill("lead", "unknown/model", None);
        let again = Ledger::open(dir.path());
        let billed = again.billed(0);
        assert_eq!(billed.len(), 2);
        assert_eq!(billed[0].cost, Some(Usd(0.01)));
        assert_eq!(billed[1].cost, None);
        assert!(again.billed(iq_ui::now() + 10).is_empty());
        again.trim();
        assert_eq!(again.billed(0).len(), 2);
    }

    #[test]
    fn two_iq_billing_and_trimming_at_once_lose_no_call() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_path_buf();
        let billing = {
            let folder = folder.clone();
            std::thread::spawn(move || {
                let ledger = Ledger::open(&folder);
                for _ in 0..300 {
                    ledger.bill("lead", "lab/model", Some(Usd(0.01)));
                }
            })
        };
        // Another process's ledger: a call too old to keep, then a trim
        // that writes the file again, over and over.
        let other = Ledger::open(&folder);
        let old = format!(
            "{}\n",
            serde_json::to_string(&Line {
                at: 0,
                agent: "lead".into(),
                model: "lab/old".into(),
                cost: None,
            })
            .unwrap()
        );
        while !billing.is_finished() {
            crate::lock::holding(&other.calls, || {
                std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&other.calls)
                    .and_then(|mut file| file.write_all(old.as_bytes()))
                    .unwrap();
            });
            other.trim();
        }
        billing.join().unwrap();
        assert_eq!(Ledger::open(&folder).billed(1).len(), 300);
    }
}
