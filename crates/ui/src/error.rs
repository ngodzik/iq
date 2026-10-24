//! What the person's settings for the window can get wrong.

/// A shortcut or a look the settings name that the window cannot take.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UiError {
    /// `[keys]` names an action the window does not have.
    #[error("there is no action `{0}` to bind a key to; the actions are {1}")]
    UnknownAction(String, String),
    /// A key the window cannot read.
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
    /// A look the window does not have.
    #[error("there is no theme `{0}`; the themes are {1}")]
    UnknownTheme(String, String),
}
