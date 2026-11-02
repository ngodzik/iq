//! A search of the web with no key: DuckDuckGo's page of plain HTML,
//! posted a query, its ten results read from its markup. A fallback, off
//! unless the settings choose it: the page is made for people, not an API.
//! Used moderately it answers; used often, its engine slows robots down or
//! blocks them, and another engine (a search API with a key) would be
//! needed. When the page holds no list of results nor says it found none,
//! that is said plainly: the engine refused, or its page changed.

use crate::error::WebError;
use crate::html::{Token, tokens};

/// Where the query is posted.
pub(crate) const ENGINE: &str = "https://lite.duckduckgo.com/lite/";

/// One result: its title, its address and the line it is said in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hit {
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) snippet: String,
}

/// Whether a tag's class list holds `class`.
fn classed(attrs: &std::collections::HashMap<String, String>, class: &str) -> bool {
    attrs
        .get("class")
        .is_some_and(|c| c.split_whitespace().any(|c| c == class))
}

/// The text of the tokens from `at` until the closing of `name`, and where
/// that is.
fn text_until(all: &[Token], at: usize, name: &str) -> (String, usize) {
    let mut text = String::new();
    let mut end = at;
    for (i, token) in all.iter().enumerate().skip(at) {
        end = i;
        match token {
            Token::Close(closed) if closed == name => break,
            Token::Text(t) => text.push_str(t),
            _ => {}
        }
    }
    (text.split_whitespace().collect::<Vec<_>>().join(" "), end)
}

/// The address a result's link leads to: its own, or the one the engine's
/// redirection carries.
fn target(href: &str) -> String {
    let href = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_owned()
    };
    match reqwest::Url::parse(&href) {
        Ok(url) if url.path() == "/l/" => url
            .query_pairs()
            .find(|(key, _)| key == "uddg")
            .map_or(href.clone(), |(_, value)| value.into_owned()),
        _ => href,
    }
}

/// The results of the engine's page `html`.
pub(crate) fn hits(html: &str) -> Result<Vec<Hit>, WebError> {
    let all = tokens(html);
    let mut hits = Vec::new();
    let mut at = 0;
    while at < all.len() {
        let Token::Open { name, attrs } = &all[at] else {
            at += 1;
            continue;
        };
        if name != "a" || !classed(attrs, "result-link") {
            at += 1;
            continue;
        }
        let url = target(attrs.get("href").map_or("", String::as_str));
        let (title, end) = text_until(&all, at + 1, "a");
        // Its line is in the rows after it, before the next result.
        let mut snippet = String::new();
        let mut next = end + 1;
        while next < all.len() {
            match &all[next] {
                Token::Open { name, attrs } if name == "a" && classed(attrs, "result-link") => {
                    break;
                }
                Token::Open { name, attrs } if name == "td" && classed(attrs, "result-snippet") => {
                    snippet = text_until(&all, next + 1, "td").0;
                    break;
                }
                _ => next += 1,
            }
        }
        if url.starts_with("http") && !title.is_empty() {
            hits.push(Hit {
                title,
                url,
                snippet,
            });
        }
        at = end + 1;
    }
    if hits.is_empty() && !html.contains("No results") && !html.contains("No more results") {
        return Err(WebError::Engine);
    }
    Ok(hits)
}

/// What DuckDuckGo finds for `query`.
pub(crate) async fn duckduckgo(query: &str) -> Result<Vec<Hit>, WebError> {
    let engine = reqwest::Url::parse(ENGINE).map_err(|_| WebError::Address(ENGINE.to_owned()))?;
    let page = crate::net::page(engine, Some(&[("q", query)])).await?;
    hits(&page.body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_are_read_with_the_line_in_the_row_after_their_link() {
        let page = include_str!("../tests/fixtures/lite-results.html");
        assert_eq!(
            hits(page).unwrap(),
            [
                Hit {
                    title: "The runtime - Docs".into(),
                    url: "https://docs.example.org/runtime/".into(),
                    snippet: "The runtime drives the I/O and the timers & schedules the tasks."
                        .into(),
                },
                Hit {
                    title: "A guide".into(),
                    url: "https://example.com/guide?a=1".into(),
                    snippet: "How to start, step by step.".into(),
                },
            ]
        );
    }

    #[test]
    fn a_page_without_results_says_so_and_another_is_the_engine_refusing() {
        assert_eq!(hits("<p>No results.</p>").unwrap(), []);
        assert!(matches!(
            hits("<html><body>Please confirm you are human</body></html>"),
            Err(WebError::Engine)
        ));
    }
}
