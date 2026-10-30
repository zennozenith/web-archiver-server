use std::borrow::Cow;

use url::{Url, form_urlencoded};

use crate::error::{AppError, Result};

type QueryPairs<'a> = Vec<(Cow<'a, str>, Cow<'a, str>)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scheme {
    Http,
    Https,
}

impl Scheme {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }
}

pub(crate) struct CanonicalUrl {
    pub url: String,
    pub scheme: Scheme,
    pub host: String,
    pub path: String,
    pub query: Option<String>,
}

pub(crate) fn canonicalize_url(raw_url: &str) -> Result<CanonicalUrl> {
    let mut parsed = Url::parse(raw_url.trim()).map_err(|e| AppError::InvalidUrl(e.to_string()))?;

    // Remove fragment/hash (not part of server-side resource identification)
    parsed.set_fragment(None);

    // These only fail for cannot-be-a-base URLs, which have no credentials to clear.
    let _username_result = parsed.set_username("");
    let _password_result = parsed.set_password(None);

    // Normalize and sort query parameters alphabetically
    if let Some(query) = parsed.query() {
        let mut pairs: QueryPairs<'_> = form_urlencoded::parse(query.as_bytes()).collect();

        // Sort by key, then by value
        pairs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        // pairs.sort_by(|a, b| a.0.cmp(&b.0));

        // Re-serialization rewrites encoding.
        // form_urlencoded::Serializer turns %20 into + and ?flag into ?flag=.
        // That’s fine for deduplication, but it means the stored URL may not be
        // byte-identical to what you actually fetch. If you fetch using the
        // canonical URL, this is harmless. If you fetch the user’s original,
        // keep the original in capture or metadata
        let sorted_query = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish();

        if sorted_query.is_empty() {
            parsed.set_query(None);
        } else {
            parsed.set_query(Some(&sorted_query));
        }
    }

    // Extract components matching your SQLite schema
    let scheme = match parsed.scheme() {
        "http" => Scheme::Http,
        "https" => Scheme::Https,
        _ => {
            return Err(AppError::InvalidUrl(
                "only http and https are supported".to_owned(),
            ));
        }
    };

    let host = parsed.host_str().unwrap_or("").to_owned();

    let path = parsed.path().to_owned();
    let query = parsed.query().map(std::borrow::ToOwned::to_owned);

    // The final normalized string representation (automatically strips default ports)
    let canonical_url = parsed.to_string();

    Ok(CanonicalUrl {
        url: canonical_url,
        scheme,
        host,
        path,
        query,
    })
}
