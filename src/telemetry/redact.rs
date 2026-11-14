//! Wrappers that make values safe to put in logs.
//!
//! Each one is a [`Display`](std::fmt::Display) adapter, so call sites stay
//! cheap and read naturally: `tracing::warn!(url = %SafeUrl(&url), ...)`.

use std::fmt;

use url::Url;

/// `scheme://host[:port]/path` of a URL, without credentials, query string or
/// fragment. Signed URLs and tokens in archived pages usually live in the
/// query, so the query never reaches the logs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SafeUrl<'a>(pub(crate) &'a Url);

impl fmt::Display for SafeUrl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let url = self.0;
        write!(f, "{}://{}", url.scheme(), url.host_str().unwrap_or("-"))?;
        if let Some(port) = url.port() {
            write!(f, ":{port}")?;
        }
        f.write_str(url.path())
    }
}

/// An e-mail address reduced to its first character and domain (`a***@example.com`):
/// enough to tell accounts apart in a bug report, not enough to harvest.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MaskedEmail<'a>(pub(crate) &'a str);

impl fmt::Display for MaskedEmail<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.split_once('@') {
            Some((local, domain)) => {
                if let Some(first) = local.chars().next() {
                    write!(f, "{first}")?;
                }
                write!(f, "***@{domain}")
            }
            None => f.write_str("***"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_url_drops_credentials_query_and_fragment() {
        let url = Url::parse("https://user:secret@cdn.example.com:8443/a/b.css?sig=abc#frag")
            .expect("valid url");
        assert_eq!(
            SafeUrl(&url).to_string(),
            "https://cdn.example.com:8443/a/b.css"
        );
    }

    #[test]
    fn safe_url_omits_default_port() {
        let url = Url::parse("http://example.com/x?token=1").expect("valid url");
        assert_eq!(SafeUrl(&url).to_string(), "http://example.com/x");
    }

    #[test]
    fn masked_email_keeps_first_letter_and_domain() {
        assert_eq!(
            MaskedEmail("alice@example.com").to_string(),
            "a***@example.com"
        );
        assert_eq!(MaskedEmail("@example.com").to_string(), "***@example.com");
        assert_eq!(MaskedEmail("not-an-email").to_string(), "***");
    }
}
