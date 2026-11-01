//! What goes wrong reaching a server, and asking for its grant.

use std::path::PathBuf;

/// What goes wrong reaching a server or calling its tools.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// The settings give the server neither an address nor a command.
    #[error("neither a url nor a command is given")]
    Unreachable,
    /// Its program could not be started.
    #[error("{program} could not be started")]
    Start {
        /// The program.
        program: String,
        /// Why.
        #[source]
        source: std::io::Error,
    },
    /// Its program stopped, or closed its output.
    #[error("the server stopped{said}")]
    Stopped {
        /// The last lines it wrote of its errors, when it wrote some.
        said: String,
    },
    /// What was written to its program did not reach it.
    #[error("the server could not be written to")]
    Write(#[source] std::io::Error),
    /// Its output could not be read.
    #[error("the server could not be read")]
    Read(#[source] std::io::Error),
    /// Its address could not be reached.
    #[error("{url} could not be reached")]
    Http {
        /// The address.
        url: String,
        /// Why.
        #[source]
        source: reqwest::Error,
    },
    /// It answered with an HTTP error.
    #[error("{url} answered HTTP {status}: {said}")]
    Status {
        /// The address.
        url: String,
        /// The status.
        status: u16,
        /// The first line of what it answered.
        said: String,
    },
    /// It did not answer in time.
    #[error("no answer to {method} in {seconds} seconds")]
    Timeout {
        /// What was asked.
        method: String,
        /// How long it was waited for.
        seconds: u64,
    },
    /// It answered without the answer asked for.
    #[error("no answer to {method}")]
    NoAnswer {
        /// What was asked.
        method: String,
    },
    /// It answered with an error.
    #[error("{method} was refused: {message}")]
    Refused {
        /// What was asked.
        method: String,
        /// What it said.
        message: String,
    },
    /// The tool ran and failed, as it says.
    #[error("{0}")]
    Failed(String),
    /// No server offers a tool of that name.
    #[error("there is no tool {0}")]
    NoTool(String),
    /// The server is not connected, or no longer.
    #[error("{0} is not connected")]
    NotConnected(String),
    /// The permit is for another action than a server's tool.
    #[error("the permit is not for a server's tool")]
    NotATool,
    /// The arguments are not a JSON object.
    #[error("the arguments are not a JSON object")]
    Arguments(#[source] serde_json::Error),
    /// Its grant could not be had.
    #[error("no grant")]
    Auth(#[from] AuthError),
}

/// What goes wrong asking for a server's grant, as OAuth gives it.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// An address it gave is not one.
    #[error("{url} is not an address")]
    Address {
        /// What it gave.
        url: String,
        /// Why.
        #[source]
        source: url::ParseError,
    },
    /// An address could not be reached.
    #[error("{url} could not be reached")]
    Http {
        /// The address.
        url: String,
        /// Why.
        #[source]
        source: reqwest::Error,
    },
    /// No folder was given to keep grants in.
    #[error("no folder to keep its grant in")]
    NoFolder,
    /// The server says nothing of what authorizes it.
    #[error("it says nothing of its authorization server")]
    NoMetadata,
    /// Its metadata names no authorization server.
    #[error("it names no authorization server")]
    NoIssuer,
    /// Its authorization server says nothing of itself.
    #[error("its authorization server says nothing of itself")]
    NoServerMetadata,
    /// Its authorization server does not take PKCE as iq asks it.
    #[error("its authorization server does not take PKCE with S256")]
    NoPkce,
    /// Its authorization server lacks an endpoint iq needs.
    #[error("its authorization server has no {0} endpoint")]
    NoEndpoint(&'static str),
    /// Its authorization server registers no client.
    #[error("its authorization server registers no client: only the clients it knows may use it")]
    NoRegistration,
    /// Its authorization server refused to register iq.
    #[error(
        "its authorization server refused to register iq (HTTP {0}): it may take only the clients it knows"
    )]
    RegistrationRefused(u16),
    /// Registering gave no client id.
    #[error("registering gave no client id")]
    NoClientId,
    /// No random bytes could be had.
    #[error("no random bytes")]
    Random,
    /// No port could be opened to hear the consent on.
    #[error("no port to hear the consent on")]
    Listen(#[source] std::io::Error),
    /// The person did not consent in time.
    #[error("no consent within five minutes")]
    ConsentTimeout,
    /// The person, or the authorization server, refused.
    #[error("the consent was refused: {0}")]
    ConsentRefused(String),
    /// The browser came back for another request.
    #[error("the consent came back for another request")]
    WrongState,
    /// The browser came back from another authorization server.
    #[error("the consent came back from another authorization server")]
    WrongIssuer,
    /// The browser came back without a code.
    #[error("the consent came back without a code")]
    NoCode,
    /// The token endpoint refused.
    #[error("no token: HTTP {status} ({said})")]
    NoToken {
        /// Its status.
        status: u16,
        /// What it said of why.
        said: String,
    },
    /// The token endpoint answered without a token.
    #[error("the answer holds no token")]
    TokenMissing,
    /// The grant has no refresh token.
    #[error("the grant cannot be renewed")]
    NotRenewable,
    /// The grant could not be written.
    #[error("its grant could not be kept in {path}")]
    Keep {
        /// Where.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },
}

/// `error` and its causes, as one line.
pub(crate) fn said(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
