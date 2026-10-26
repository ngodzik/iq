//! The assay as the terminal holds it: what the program read of its store,
//! and the runs at work, as they go.

pub use iq_assay::{JUDGE, Kind, Overview, Phase, Progress, Row, State, Summary, Table};

/// The assay, as last told.
#[derive(Debug, Clone, Default)]
pub struct AssayView {
    overview: Overview,
    running: Vec<Progress>,
    /// Whether the program told it once.
    told: bool,
}

impl AssayView {
    /// What the store holds, as last read.
    #[must_use]
    pub fn overview(&self) -> &Overview {
        &self.overview
    }

    /// The runs at work, and those waiting.
    #[must_use]
    pub fn running(&self) -> &[Progress] {
        &self.running
    }

    /// Whether the program told it once: before, there is nothing to show.
    #[must_use]
    pub fn told(&self) -> bool {
        self.told
    }

    /// The judge the person talks with, as the settings name it.
    #[must_use]
    pub fn judge(&self) -> Option<&str> {
        self.overview.judges.first().map(String::as_str)
    }

    pub(crate) fn set(&mut self, overview: Overview) {
        self.overview = overview;
        self.told = true;
    }

    pub(crate) fn set_running(&mut self, running: Vec<Progress>) {
        self.running = running;
    }
}
