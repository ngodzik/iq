//! What goes wrong reading the web.

/// What goes wrong fetching a page or searching.
#[derive(Debug, thiserror::Error)]
pub enum WebError {
    /// It is no address.
    #[error("{0} is no address")]
    Address(String),
    /// It is an address of something other than the web.
    #[error("{0}: addresses are read only of the web (http, https)")]
    NotWeb(String),
    /// It carries a login.
    #[error("an address with a login in it is not read")]
    Login,
    /// It leads to this machine, a private network or a link's own.
    #[error(
        "{0} is not of the public web: this machine, a private network and a link's own are never read"
    )]
    Private(String),
    /// Its name did not resolve.
    #[error("{host} could not be found")]
    Resolve {
        /// The name.
        host: String,
        /// Why.
        #[source]
        source: std::io::Error,
    },
    /// It could not be reached.
    #[error("{url} could not be reached")]
    Http {
        /// The address.
        url: String,
        /// Why.
        #[source]
        source: reqwest::Error,
    },
    /// It answered with an error.
    #[error("it answered HTTP {0}")]
    Status(u16),
    /// It redirected too many times.
    #[error("it redirected more times than are followed")]
    Redirections,
    /// It is not a page.
    #[error("it is {0}, not a page of text")]
    NotText(String),
    /// The engine gave no list of results.
    #[error(
        "the search engine answered with no list of results: it may be slowing down robots, or its page changed; try later, or read a page with fetch"
    )]
    Engine,
}
