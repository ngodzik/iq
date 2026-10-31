//! What the person's settings for the terminal can get wrong.

/// A shortcut or a look the settings name that iq cannot take.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UiError {
    /// `[keys]` names an action iq does not have.
    #[error("there is no action `{0}` to bind a key to; the actions are {1}")]
    UnknownAction(String, String),
    /// A key iq cannot read.
    #[error("`{chord}` for `{action}` is not a key: {why}")]
    BadChord {
        /// The action it was meant for.
        action: String,
        /// What the settings wrote.
        chord: String,
        /// What is wrong with it.
        why: String,
    },
    /// Two actions on one key.
    #[error("{chord} is bound to both `{first}` and `{second}`")]
    Taken {
        /// The key.
        chord: String,
        /// One action.
        first: String,
        /// The other.
        second: String,
    },
    /// A look iq does not have.
    #[error("there is no theme `{0}`; the themes are {1}")]
    UnknownTheme(String, String),
}
