//! OAuth for servers reached over HTTP that take nothing else, as the
//! protocol's authorization spec describes: the authorization server found
//! from the resource's metadata (where the server's refusal points, else
//! where it is looked for), a client registered for iq, the person's
//! consent in the browser with PKCE, and the grant kept on disk for the
//! address it was given for, renewed before it expires.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_lc_rs::rand::SecureRandom;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::AuthError;

/// How long the person has to consent in the browser.
const CONSENT_WITHIN: Duration = Duration::from_secs(300);

/// How long before it expires a grant is renewed.
const RENEW_BEFORE: u64 = 60;

/// What a server's refusal says of where to ask, from its
/// `WWW-Authenticate` header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Challenge {
    /// Where its metadata is (RFC 9728).
    pub(crate) resource_metadata: Option<String>,
    /// The scope to ask for.
    pub(crate) scope: Option<String>,
}

impl Challenge {
    /// Reads a `WWW-Authenticate` header: its `Bearer` parameters, quoted or
    /// not.
    pub(crate) fn read(header: &str) -> Self {
        let mut challenge = Self::default();
        let rest = header.trim();
        let Some(rest) = rest
            .strip_prefix("Bearer")
            .or_else(|| rest.strip_prefix("bearer"))
        else {
            return challenge;
        };
        let mut rest = rest.trim_start();
        while !rest.is_empty() {
            let Some((name, after)) = rest.split_once('=') else {
                break;
            };
            let name = name.trim().trim_start_matches(',').trim();
            let after = after.trim_start();
            let (value, next) = if let Some(quoted) = after.strip_prefix('"') {
                match quoted.find('"') {
                    Some(end) => (&quoted[..end], &quoted[end + 1..]),
                    None => (quoted, ""),
                }
            } else {
                match after.find(',') {
                    Some(end) => (after[..end].trim(), &after[end..]),
                    None => (after.trim(), ""),
                }
            };
            match name {
                "resource_metadata" => challenge.resource_metadata = Some(value.to_owned()),
                "scope" => challenge.scope = Some(value.to_owned()),
                _ => {}
            }
            rest = next.trim_start().trim_start_matches(',').trim_start();
        }
        challenge
    }
}

/// How a client proves itself at the token endpoint (RFC 7591).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClientAuth {
    /// A public client: its id alone, PKCE proving the rest.
    #[default]
    None,
    /// Its secret in the form.
    ClientSecretPost,
    /// Its secret in a Basic authorization header.
    ClientSecretBasic,
}

impl ClientAuth {
    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ClientSecretPost => "client_secret_post",
            Self::ClientSecretBasic => "client_secret_basic",
        }
    }

    /// What iq asks to be: a public client when the server takes one, else
    /// one with a secret as it takes it. A server that says nothing takes
    /// a public client.
    fn chosen(supported: Option<&Vec<Value>>) -> Self {
        let Some(supported) = supported else {
            return Self::None;
        };
        let takes = |method: Self| supported.iter().any(|m| m == method.name());
        [Self::None, Self::ClientSecretPost, Self::ClientSecretBasic]
            .into_iter()
            .find(|method| takes(*method))
            .unwrap_or(Self::None)
    }
}

/// Where the authorization server takes the person and the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Endpoints {
    issuer: String,
    authorize: String,
    token: String,
    register: String,
    /// The protected resource, as the token is asked for it (RFC 8707).
    resource: String,
    /// The scope asked for, when the server names one.
    scope: Option<String>,
    /// How iq registers to prove itself.
    auth: ClientAuth,
    /// Whether the browser comes back saying which server it comes from
    /// (RFC 9207).
    says_issuer: bool,
}

/// What a request carries to the server, and what renews it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Grant {
    /// The server's address it was given for: never sent elsewhere.
    url: String,
    client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_secret: Option<String>,
    #[serde(default)]
    auth: ClientAuth,
    token_endpoint: String,
    resource: String,
    pub(crate) access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    /// Seconds since 1970; `None` when the server did not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<u64>,
}

/// Written by hand: the tokens and the secret never show.
impl fmt::Debug for Grant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hidden = |value: &Option<String>| value.as_ref().map(|_| "<redacted>");
        f.debug_struct("Grant")
            .field("url", &self.url)
            .field("client_id", &self.client_id)
            .field("client_secret", &hidden(&self.client_secret))
            .field("auth", &self.auth)
            .field("token_endpoint", &self.token_endpoint)
            .field("resource", &self.resource)
            .field("access_token", &"<redacted>")
            .field("refresh_token", &hidden(&self.refresh_token))
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl Grant {
    /// A grant of the tests' server at `base`, ending at `expires_at`.
    #[cfg(test)]
    pub(crate) fn for_test(base: &str, access: &str, refresh: &str, expires_at: u64) -> Self {
        Self {
            url: format!("{base}/mcp"),
            client_id: "c-1".into(),
            client_secret: None,
            auth: ClientAuth::None,
            token_endpoint: format!("{base}/token"),
            resource: format!("{base}/mcp"),
            access_token: access.into(),
            refresh_token: Some(refresh.into()),
            expires_at: Some(expires_at),
        }
    }

    /// Whether it should be renewed before it is used, at `now`.
    pub(crate) fn is_ending(&self, now: u64) -> bool {
        self.expires_at
            .is_some_and(|at| at <= now.saturating_add(RENEW_BEFORE))
    }
}

/// Seconds since 1970.
pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The grants kept, one file per server, readable by their owner only.
#[derive(Debug, Clone)]
pub(crate) struct Grants {
    dir: PathBuf,
}

impl Grants {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, server: &str) -> PathBuf {
        self.dir.join(format!("{server}.json"))
    }

    /// The grant kept for `server` at `url`: one given for another address,
    /// as when the settings moved the server, is not, so that its token
    /// never goes where it was not given for.
    pub(crate) fn load(&self, server: &str, url: &str) -> Option<Grant> {
        let text = std::fs::read_to_string(self.path(server)).ok()?;
        let grant: Grant = serde_json::from_str(&text).ok()?;
        (grant.url == url).then_some(grant)
    }

    pub(crate) fn save(&self, server: &str, grant: &Grant) -> Result<(), AuthError> {
        let path = self.path(server);
        // A grant always serializes: its fields are strings and numbers.
        let text = serde_json::to_string_pretty(grant).unwrap_or_default();
        write_private(&path, &text).map_err(|source| AuthError::Keep { path, source })
    }

    pub(crate) fn forget(&self, server: &str) {
        let _ = std::fs::remove_file(self.path(server));
    }
}

/// Writes `text` at `path`, readable and writable by its owner only, from
/// the moment it exists.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let partial = path.with_extension("part");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    std::io::Write::write_all(&mut options.open(&partial)?, text.as_bytes())?;
    std::fs::rename(&partial, path)
}

fn parse(url: &str) -> Result<reqwest::Url, AuthError> {
    reqwest::Url::parse(url).map_err(|source| AuthError::Address {
        url: url.to_owned(),
        source,
    })
}

/// Finds the authorization server of the resource at `url`, first where
/// its refusal points, and refuses one that iq cannot use.
pub(crate) async fn discover(
    client: &reqwest::Client,
    url: &str,
    challenge: &Challenge,
) -> Result<Endpoints, AuthError> {
    let resource = parse(url)?;
    let mut candidates: Vec<String> = challenge.resource_metadata.iter().cloned().collect();
    candidates.extend(well_known(&resource, "oauth-protected-resource"));
    let mut metadata = None;
    for candidate in candidates {
        if let Some(found) = get_json(client, &candidate).await {
            metadata = Some(found);
            break;
        }
    }
    let metadata = metadata.ok_or(AuthError::NoMetadata)?;
    let issuer = metadata["authorization_servers"][0]
        .as_str()
        .ok_or(AuthError::NoIssuer)?
        .to_owned();
    let at = parse(&issuer)?;
    let mut server = None;
    for candidate in well_known(&at, "oauth-authorization-server")
        .into_iter()
        .chain(well_known(&at, "openid-configuration"))
    {
        if let Some(found) = get_json(client, &candidate).await {
            server = Some(found);
            break;
        }
    }
    let server = server.ok_or(AuthError::NoServerMetadata)?;
    let s256 = server["code_challenge_methods_supported"]
        .as_array()
        .is_some_and(|m| m.iter().any(|m| m == "S256"));
    if !s256 {
        return Err(AuthError::NoPkce);
    }
    let field = |name: &str| server[name].as_str().map(str::to_owned);
    // The scope its refusal names, else all the resource says it has.
    let scope = challenge.scope.clone().or_else(|| {
        let listed: Vec<&str> = metadata["scopes_supported"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        (!listed.is_empty()).then(|| listed.join(" "))
    });
    Ok(Endpoints {
        issuer: field("issuer").unwrap_or(issuer),
        authorize: field("authorization_endpoint").ok_or(AuthError::NoEndpoint("authorization"))?,
        token: field("token_endpoint").ok_or(AuthError::NoEndpoint("token"))?,
        register: field("registration_endpoint").ok_or(AuthError::NoRegistration)?,
        resource: metadata["resource"]
            .as_str()
            .map_or_else(|| url.to_owned(), str::to_owned),
        scope,
        auth: ClientAuth::chosen(server["token_endpoint_auth_methods_supported"].as_array()),
        says_issuer: server["authorization_response_iss_parameter_supported"].as_bool()
            == Some(true),
    })
}

/// Where `url`'s metadata of `kind` may be: with its path, then at its
/// origin (RFC 8414 and RFC 9728).
fn well_known(url: &reqwest::Url, kind: &str) -> Vec<String> {
    let origin = url.origin().ascii_serialization();
    let path = url.path().trim_end_matches('/');
    let mut out = Vec::new();
    if !path.is_empty() {
        out.push(format!("{origin}/.well-known/{kind}{path}"));
    }
    out.push(format!("{origin}/.well-known/{kind}"));
    out
}

async fn get_json(client: &reqwest::Client, url: &str) -> Option<Value> {
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

/// iq as the authorization server registered it.
struct Registered {
    id: String,
    secret: Option<String>,
    auth: ClientAuth,
}

/// Registers iq with the authorization server, a public client when it
/// takes one.
async fn register(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    redirect: &str,
) -> Result<Registered, AuthError> {
    let mut asked = json!({
        "client_name": "iq",
        "redirect_uris": [redirect],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": endpoints.auth.name(),
    });
    if let Some(scope) = &endpoints.scope {
        asked["scope"] = json!(scope);
    }
    let response = client
        .post(&endpoints.register)
        .json(&asked)
        .send()
        .await
        .map_err(|source| AuthError::Http {
            url: endpoints.register.clone(),
            source,
        })?;
    let status = response.status();
    let body: Value = response.json().await.unwrap_or_default();
    if !status.is_success() {
        return Err(AuthError::RegistrationRefused(status.as_u16()));
    }
    let id = body["client_id"]
        .as_str()
        .ok_or(AuthError::NoClientId)?
        .to_owned();
    let secret = body["client_secret"].as_str().map(str::to_owned);
    // As the server registered it, whatever was asked.
    let auth = match body["token_endpoint_auth_method"].as_str() {
        Some("client_secret_post") => ClientAuth::ClientSecretPost,
        Some("client_secret_basic") => ClientAuth::ClientSecretBasic,
        Some("none") => ClientAuth::None,
        _ if secret.is_some() && endpoints.auth == ClientAuth::None => ClientAuth::ClientSecretPost,
        _ => endpoints.auth,
    };
    Ok(Registered { id, secret, auth })
}

/// `bytes` random bytes as URL-safe text.
fn random_text(bytes: usize) -> Result<String, AuthError> {
    let mut buffer = vec![0u8; bytes];
    aws_lc_rs::rand::SystemRandom::new()
        .fill(&mut buffer)
        .map_err(|_| AuthError::Random)?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

/// The PKCE challenge of `verifier`, as the tests' server checks it.
#[cfg(test)]
pub(crate) fn challenge_of(verifier: &str) -> String {
    pkce(verifier)
}

/// The PKCE challenge of `verifier`, by S256.
fn pkce(verifier: &str) -> String {
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest.as_ref())
}

/// The page the person consents on.
fn authorize_url(
    endpoints: &Endpoints,
    client_id: &str,
    redirect: &str,
    challenge: &str,
    state: &str,
) -> Result<String, AuthError> {
    let mut url = parse(&endpoints.authorize)?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", state)
            .append_pair("resource", &endpoints.resource);
        if let Some(scope) = &endpoints.scope {
            query.append_pair("scope", scope);
        }
    }
    Ok(url.into())
}

/// The person's consent for the resource at `url`, refused as `challenge`
/// says: `shown` is told the page to open, and `open` opens it, the browser
/// by default. Returns the grant.
pub(crate) async fn consent(
    client: &reqwest::Client,
    url: &str,
    challenge: &Challenge,
    shown: impl Fn(&str),
    open: impl Fn(&str) -> bool,
) -> Result<Grant, AuthError> {
    let endpoints = discover(client, url, challenge).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(AuthError::Listen)?;
    let port = listener.local_addr().map_err(AuthError::Listen)?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let registered = register(client, &endpoints, &redirect).await?;
    let verifier = random_text(32)?;
    let state = random_text(16)?;
    let page = authorize_url(
        &endpoints,
        &registered.id,
        &redirect,
        &pkce(&verifier),
        &state,
    )?;
    shown(&page);
    // Without a browser, the page shown is for the person to open.
    let _ = open(&page);
    let issuer = endpoints.says_issuer.then_some(endpoints.issuer.as_str());
    let code = tokio::time::timeout(CONSENT_WITHIN, callback(&listener, &state, issuer))
        .await
        .map_err(|_| AuthError::ConsentTimeout)??;
    let mut grant = Grant {
        url: url.to_owned(),
        client_id: registered.id,
        client_secret: registered.secret,
        auth: registered.auth,
        token_endpoint: endpoints.token,
        resource: endpoints.resource,
        access_token: String::new(),
        refresh_token: None,
        expires_at: None,
    };
    let response = token_request(
        client,
        &grant,
        &[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("code_verifier", verifier.as_str()),
        ],
    )
    .await?;
    granted(response, &mut grant).await?;
    Ok(grant)
}

/// `grant` renewed with its refresh token.
pub(crate) async fn renew(client: &reqwest::Client, grant: &Grant) -> Result<Grant, AuthError> {
    let refresh = grant.refresh_token.clone().ok_or(AuthError::NotRenewable)?;
    let response = token_request(
        client,
        grant,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
        ],
    )
    .await?;
    let mut renewed = grant.clone();
    granted(response, &mut renewed).await?;
    Ok(renewed)
}

/// Asks `grant`'s token endpoint with `pairs`, the client proving itself
/// as it registered, the token asked for its resource.
async fn token_request(
    client: &reqwest::Client,
    grant: &Grant,
    pairs: &[(&str, &str)],
) -> Result<reqwest::Response, AuthError> {
    let mut fields: Vec<(&str, &str)> = pairs.to_vec();
    fields.push(("resource", grant.resource.as_str()));
    let secret = grant.client_secret.as_deref().unwrap_or("");
    let mut request = client
        .post(&grant.token_endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded");
    match grant.auth {
        ClientAuth::None => fields.push(("client_id", grant.client_id.as_str())),
        ClientAuth::ClientSecretPost => {
            fields.push(("client_id", grant.client_id.as_str()));
            fields.push(("client_secret", secret));
        }
        ClientAuth::ClientSecretBasic => {
            // Each part form-encoded before they are joined (RFC 6749, 2.3.1).
            let pair = format!("{}:{}", encoded(&grant.client_id), encoded(secret));
            request = request.header("Authorization", format!("Basic {}", STANDARD.encode(pair)));
        }
    }
    request
        .body(form(&fields))
        .send()
        .await
        .map_err(|source| AuthError::Http {
            url: grant.token_endpoint.clone(),
            source,
        })
}

/// `text` as a form encodes it.
fn encoded(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `pairs` as the body of a form, `application/x-www-form-urlencoded`.
fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", encoded(k), encoded(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// `grant` given what a token endpoint answered; a renewal that gives no
/// new refresh token keeps the one it had.
async fn granted(response: reqwest::Response, grant: &mut Grant) -> Result<(), AuthError> {
    let status = response.status();
    let body: Value = response.json().await.unwrap_or_default();
    if !status.is_success() {
        let said = body["error"].as_str().unwrap_or("refused").to_owned();
        return Err(AuthError::NoToken {
            status: status.as_u16(),
            said,
        });
    }
    grant.access_token = body["access_token"]
        .as_str()
        .ok_or(AuthError::TokenMissing)?
        .to_owned();
    if let Some(refresh) = body["refresh_token"].as_str() {
        grant.refresh_token = Some(refresh.to_owned());
    }
    grant.expires_at = body["expires_in"].as_u64().map(|s| now() + s);
    Ok(())
}

/// Waits for the browser to come back with the code, from `issuer` when
/// the authorization server says it tells, and answers it.
async fn callback(
    listener: &tokio::net::TcpListener,
    state: &str,
    issuer: Option<&str>,
) -> Result<String, AuthError> {
    loop {
        let (mut socket, _) = listener.accept().await.map_err(AuthError::Listen)?;
        let request = read_head(&mut socket).await;
        let Some(target) = request
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("GET "))
            .and_then(|rest| rest.split(' ').next())
        else {
            continue;
        };
        // A browser asks for its icon too: only the callback counts.
        let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
            continue;
        };
        if url.path() != "/callback" {
            let _ = socket
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            continue;
        }
        let param = |name: &str| {
            url.query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
        };
        let from_elsewhere = issuer.is_some_and(|issuer| {
            param("iss")
                .is_some_and(|iss| iss.trim_end_matches('/') != issuer.trim_end_matches('/'))
        });
        let outcome = match (param("code"), param("state"), param("error")) {
            _ if from_elsewhere => Err(AuthError::WrongIssuer),
            (_, _, Some(error)) => Err(AuthError::ConsentRefused(error)),
            (Some(code), Some(given), None) if given == state => Ok(code),
            (Some(_), _, None) => Err(AuthError::WrongState),
            _ => Err(AuthError::NoCode),
        };
        let page = match &outcome {
            Ok(_) => "iq has your consent: this page can be closed.".to_owned(),
            Err(why) => format!("iq did not get your consent: {why}."),
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        let _ = socket.write_all(response.as_bytes()).await;
        return outcome;
    }
}

/// What a browser sent up to the end of its headers, however TCP cut it,
/// at most 16 KiB.
async fn read_head(socket: &mut tokio::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    while buffer.len() < 16 * 1024 && !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
        match socket.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

/// Opens `url` in the person's browser: `open` on macOS, `xdg-open`
/// elsewhere. Whether it could be started.
pub(crate) fn open_browser(url: &str) -> bool {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let child = std::process::Command::new(program)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            // Waited for on a thread of its own, so that it leaves no
            // zombie behind.
            std::thread::spawn(move || child.wait());
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_is_s256_of_the_verifier() {
        // RFC 7636's example, its challenge worked out apart with Python's
        // hashlib and base64.
        assert_eq!(
            pkce("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        // 32 random bytes make the 43 characters the RFC asks for.
        let verifier = random_text(32).unwrap();
        assert_eq!(verifier.len(), 43);
        assert_ne!(verifier, random_text(32).unwrap());
    }

    #[test]
    fn a_form_is_encoded_as_browsers_encode_it() {
        assert_eq!(
            form(&[
                ("code", "a b/c+d"),
                ("redirect_uri", "http://127.0.0.1:9/cb")
            ]),
            "code=a%20b%2Fc%2Bd&redirect_uri=http%3A%2F%2F127.0.0.1%3A9%2Fcb"
        );
    }

    #[test]
    fn metadata_is_looked_for_with_the_path_then_at_the_origin() {
        let url = reqwest::Url::parse("https://mcp.example.com/mcp/").unwrap();
        assert_eq!(
            well_known(&url, "oauth-protected-resource"),
            [
                "https://mcp.example.com/.well-known/oauth-protected-resource/mcp",
                "https://mcp.example.com/.well-known/oauth-protected-resource",
            ]
        );
        let issuer = reqwest::Url::parse("https://auth.example.com").unwrap();
        assert_eq!(
            well_known(&issuer, "oauth-authorization-server"),
            ["https://auth.example.com/.well-known/oauth-authorization-server"]
        );
    }

    #[test]
    fn a_refusal_says_where_its_metadata_is_and_what_scope_to_ask() {
        let read = Challenge::read(
            r#"Bearer realm="OAuth", resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource/mcp", scope="read write""#,
        );
        assert_eq!(
            read,
            Challenge {
                resource_metadata: Some(
                    "https://mcp.example.com/.well-known/oauth-protected-resource/mcp".into()
                ),
                scope: Some("read write".into()),
            }
        );
        // Unquoted, without spaces after the commas.
        let read = Challenge::read("Bearer resource_metadata=https://a.example/m,scope=connect");
        assert_eq!(
            read.resource_metadata.as_deref(),
            Some("https://a.example/m")
        );
        assert_eq!(read.scope.as_deref(), Some("connect"));
        assert_eq!(Challenge::read("Basic realm=\"x\""), Challenge::default());
    }

    #[test]
    fn iq_registers_as_a_public_client_unless_the_server_takes_only_secrets() {
        let methods = |listed: &[&str]| listed.iter().map(|m| json!(m)).collect::<Vec<_>>();
        assert_eq!(ClientAuth::chosen(None), ClientAuth::None);
        assert_eq!(
            ClientAuth::chosen(Some(&methods(&["client_secret_basic", "none"]))),
            ClientAuth::None
        );
        assert_eq!(
            ClientAuth::chosen(Some(&methods(&[
                "client_secret_basic",
                "client_secret_post"
            ]))),
            ClientAuth::ClientSecretPost
        );
        assert_eq!(
            ClientAuth::chosen(Some(&methods(&["client_secret_basic"]))),
            ClientAuth::ClientSecretBasic
        );
    }

    #[tokio::test]
    async fn a_callback_cut_in_two_by_the_network_still_gives_its_code() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
            socket.write_all(b"GET /callback?code=c-9&").await.unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
            socket
                .write_all(b"state=s-1 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
                .await
                .unwrap();
            let mut answer = String::new();
            let _ = socket.read_to_string(&mut answer).await;
            assert!(answer.contains("iq has your consent"));
        });
        assert_eq!(
            callback(&listener, "s-1", None).await.unwrap(),
            "c-9".to_owned()
        );
    }

    #[tokio::test]
    async fn a_callback_from_another_authorization_server_is_refused() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
            socket
                .write_all(
                    b"GET /callback?code=c-9&state=s-1&iss=https%3A%2F%2Fother.example HTTP/1.1\r\n\r\n",
                )
                .await
                .unwrap();
            let mut answer = String::new();
            let _ = socket.read_to_string(&mut answer).await;
        });
        assert!(matches!(
            callback(&listener, "s-1", Some("https://auth.example")).await,
            Err(AuthError::WrongIssuer)
        ));
    }

    #[test]
    fn a_grant_is_kept_for_its_owner_and_its_address_only_and_never_shows_its_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let grants = Grants::new(dir.path().join("mcp-auth"));
        let grant = Grant {
            url: "https://mcp.example.com/mcp".into(),
            client_id: "c-1".into(),
            client_secret: Some("secret-client".into()),
            auth: ClientAuth::ClientSecretPost,
            token_endpoint: "https://auth.example.com/token".into(),
            resource: "https://mcp.example.com/mcp".into(),
            access_token: "secret-access".into(),
            refresh_token: Some("secret-refresh".into()),
            expires_at: Some(1_000),
        };
        grants.save("docs", &grant).unwrap();
        assert_eq!(
            grants.load("docs", "https://mcp.example.com/mcp"),
            Some(grant.clone())
        );
        // The server moved: its grant is not sent to the new address.
        assert_eq!(grants.load("docs", "https://elsewhere.example/mcp"), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.path().join("mcp-auth/docs.json");
            let mode = std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let shown = format!("{grant:?}");
        for secret in ["secret-access", "secret-refresh", "secret-client"] {
            assert!(!shown.contains(secret), "{shown}");
        }
        // Renewed a minute before it ends.
        assert!(!grant.is_ending(900));
        assert!(grant.is_ending(941));
        grants.forget("docs");
        assert_eq!(grants.load("docs", "https://mcp.example.com/mcp"), None);
    }
}
