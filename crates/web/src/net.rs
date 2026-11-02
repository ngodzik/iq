//! Reaching the public web, and only it. An address is taken only of the
//! web (`http`, `https`), its host a name with a dot or a public address,
//! no login in it; its name is resolved first and every address it gives
//! must be public (not this machine, not a private network, not a link's
//! own, a cloud's metadata among them), then the connection is made to the
//! address checked, so that the name cannot resolve elsewhere meanwhile.
//! Each redirection is checked again. No cookie is kept, no header of the
//! person's sent; what comes back is bounded in size and in time.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::error::WebError;

/// The most bytes of a page read.
pub(crate) const PAGE_BYTES: usize = 2 << 20;

/// How long a page may take, redirections with it.
const PAGE_WITHIN: Duration = Duration::from_secs(20);

/// How many redirections are followed.
const REDIRECTIONS: usize = 5;

/// What a page answered: where it was in the end, its kind and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Page {
    pub(crate) url: reqwest::Url,
    pub(crate) kind: String,
    pub(crate) body: String,
}

/// Whether `ip` is an address of the public internet.
pub(crate) fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || a == 0
                // Shared by carriers, and the protocols' own.
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0 && v4.octets()[2] == 0)
                || (a == 198 && (18..20).contains(&b))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // Unique local, and a link's own.
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
        }
    }
}

/// `text` as an address of the public web, or why it is not one.
pub(crate) fn address(text: &str) -> Result<reqwest::Url, WebError> {
    let url = reqwest::Url::parse(text.trim()).map_err(|_| WebError::Address(text.to_owned()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(WebError::NotWeb(url.scheme().to_owned()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(WebError::Login);
    }
    match url.host() {
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.');
            if !name.contains('.')
                || name.eq_ignore_ascii_case("localhost")
                || name.ends_with(".localhost")
            {
                return Err(WebError::Private(name.to_owned()));
            }
        }
        Some(url::Host::Ipv4(ip)) if !public(IpAddr::V4(ip)) => {
            return Err(WebError::Private(ip.to_string()));
        }
        Some(url::Host::Ipv6(ip)) if !public(IpAddr::V6(ip)) => {
            return Err(WebError::Private(ip.to_string()));
        }
        Some(_) => {}
        None => return Err(WebError::Address(text.to_owned())),
    }
    Ok(url)
}

/// The address `url`'s host resolves to, every one of them public.
async fn resolved(url: &reqwest::Url) -> Result<SocketAddr, WebError> {
    let host = url
        .host_str()
        .ok_or_else(|| WebError::Address(url.to_string()))?
        .trim_matches(|c| c == '[' || c == ']')
        .to_owned();
    let port = url.port_or_known_default().unwrap_or(443);
    let found: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|source| WebError::Resolve {
            host: host.clone(),
            source,
        })?
        .collect();
    if let Some(private) = found.iter().find(|a| !public(a.ip())) {
        return Err(WebError::Private(format!(
            "{host}, which is {}",
            private.ip()
        )));
    }
    found.into_iter().next().ok_or(WebError::Resolve {
        host,
        source: std::io::Error::other("no address"),
    })
}

/// What `request` gets, made on a client of its own that connects only to
/// the address checked for `url` and follows no redirection by itself.
async fn sent(
    url: &reqwest::Url,
    request: impl FnOnce(&reqwest::Client) -> reqwest::RequestBuilder,
) -> Result<reqwest::Response, WebError> {
    let at = resolved(url).await?;
    let host = url.host_str().unwrap_or_default().to_owned();
    // No proxy of the environment's: one would connect where it resolves,
    // past the address checked.
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve(&host, at)
        .timeout(PAGE_WITHIN)
        .user_agent(concat!("iq/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|source| WebError::Http {
            url: url.to_string(),
            source,
        })?;
    request(&client)
        .send()
        .await
        .map_err(|source| WebError::Http {
            url: url.to_string(),
            source,
        })
}

/// The page at `url`, redirections followed as far as [`REDIRECTIONS`],
/// each checked; `form` posted rather than got, when given.
pub(crate) async fn page(
    url: reqwest::Url,
    form: Option<&[(&str, &str)]>,
) -> Result<Page, WebError> {
    let mut url = url;
    for _ in 0..=REDIRECTIONS {
        let response = sent(&url, |client| match form {
            Some(fields) => {
                let body = url::form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(fields)
                    .finish();
                client
                    .post(url.clone())
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(body)
            }
            None => client.get(url.clone()),
        })
        .await?;
        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|l| l.to_str().ok())
                .ok_or_else(|| WebError::Status(status.as_u16()))?;
            let next = url
                .join(location)
                .map_err(|_| WebError::Address(location.to_owned()))?;
            url = address(next.as_str())?;
            continue;
        }
        if !status.is_success() {
            return Err(WebError::Status(status.as_u16()));
        }
        let kind = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|t| t.to_str().ok())
            .unwrap_or("text/html")
            .to_ascii_lowercase();
        let body = bounded(response, &url).await?;
        return Ok(Page { url, kind, body });
    }
    Err(WebError::Redirections)
}

/// The body of `response`, as far as [`PAGE_BYTES`].
async fn bounded(mut response: reqwest::Response, url: &reqwest::Url) -> Result<String, WebError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|source| WebError::Http {
        url: url.to_string(),
        source,
    })? {
        body.extend_from_slice(&chunk);
        if body.len() >= PAGE_BYTES {
            body.truncate(PAGE_BYTES);
            break;
        }
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_public_web_is_reached() {
        for refused in [
            "http://localhost:8080/",
            "http://intranet/wiki",
            "http://127.0.0.1/",
            "http://10.0.0.7/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://user:pass@example.com/",
            "file:///etc/passwd",
            "ftp://example.com/x",
            "http://app.localhost/",
        ] {
            assert!(address(refused).is_err(), "{refused}");
        }
        for taken in [
            "https://example.com/a?b=c",
            "http://93.184.216.34/",
            "https://docs.rs/tokio",
        ] {
            assert!(address(taken).is_ok(), "{taken}");
        }
        assert!(!public("100.64.0.1".parse().unwrap()));
        assert!(!public("::ffff:10.0.0.1".parse().unwrap()));
        assert!(public("2606:4700::1".parse().unwrap()));
    }
}
