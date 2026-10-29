//! Finds (and rewrites) the resources referenced by a CSS stylesheet.
//!
//! This is intentionally not a full CSS parser: archived stylesheets only need
//! `url(...)` and `@import "..."` references rewritten to local content URLs.
//! Comments and strings are skipped so `url(` inside them is not mistaken for a
//! reference.

use std::collections::{HashMap, HashSet};

use url::Url;

use super::html::{AssetKind, resolve};

struct RawRef<'a> {
    url: &'a str,
    start: usize,
    end: usize,
    kind: AssetKind,
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0c)
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && is_ws(bytes[i]) {
        i += 1;
    }
    i
}

fn find_comment_end(bytes: &[u8], mut i: usize) -> usize {
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            return i + 2;
        }
        i += 1;
    }
    bytes.len()
}

fn find_string_end(bytes: &[u8], mut i: usize, quote: u8) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b if b == quote => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn starts_with_ci(haystack: &str, needle: &str) -> bool {
    haystack
        .get(..needle.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(needle))
}

fn at_boundary(bytes: &[u8], i: usize) -> bool {
    i == 0 || !matches!(bytes[i - 1], b'-' | b'_') && !bytes[i - 1].is_ascii_alphanumeric()
}

fn next_char(css: &str, i: usize) -> usize {
    i + css[i..].chars().next().map_or(1, char::len_utf8)
}

/// Parse the value of `url(...)`. `open` points just after the opening paren.
/// Returns `(value_start, value_end, index_after_closing_paren)`.
fn parse_url_token(css: &str, open: usize) -> Option<(usize, usize, usize)> {
    let bytes = css.as_bytes();
    let mut i = skip_ws(bytes, open);

    let (start, end, mut next) = if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
        let quote = bytes[i];
        let value_start = i + 1;
        let value_end = find_string_end(bytes, value_start, quote);
        if value_end <= value_start || bytes[value_end - 1] != quote {
            return None;
        }
        (value_start, value_end - 1, value_end)
    } else {
        let value_start = i;
        while i < bytes.len() && bytes[i] != b')' {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        let mut value_end = i;
        while value_end > value_start && is_ws(bytes[value_end - 1]) {
            value_end -= 1;
        }
        (value_start, value_end, i)
    };

    next = skip_ws(bytes, next);
    if next < bytes.len() && bytes[next] == b')' {
        next += 1;
    }
    Some((start, end, next))
}

fn scan(css: &str) -> Vec<RawRef<'_>> {
    let bytes = css.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i = find_comment_end(bytes, i + 2);
            }
            b'"' | b'\'' => {
                i = find_string_end(bytes, i + 1, bytes[i]);
            }
            _ if starts_with_ci(&css[i..], "url(") && at_boundary(bytes, i) => {
                if let Some((start, end, next)) = parse_url_token(css, i + 4) {
                    out.push(RawRef {
                        url: &css[start..end],
                        start,
                        end,
                        kind: AssetKind::Other,
                    });
                    i = next;
                } else {
                    i += 1;
                }
            }
            _ if starts_with_ci(&css[i..], "@import") && at_boundary(bytes, i) => {
                let mut j = skip_ws(bytes, i + 7);
                while j + 1 < bytes.len() && bytes[j] == b'/' && bytes[j + 1] == b'*' {
                    j = skip_ws(bytes, find_comment_end(bytes, j + 2));
                }

                if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                    let quote = bytes[j];
                    let start = j + 1;
                    let end = find_string_end(bytes, start, quote);
                    if end > start && bytes[end - 1] == quote {
                        out.push(RawRef {
                            url: &css[start..end - 1],
                            start,
                            end: end - 1,
                            kind: AssetKind::Stylesheet,
                        });
                        i = end;
                    } else {
                        i += 7;
                    }
                } else if starts_with_ci(&css[j..], "url(") {
                    if let Some((start, end, next)) = parse_url_token(css, j + 4) {
                        out.push(RawRef {
                            url: &css[start..end],
                            start,
                            end,
                            kind: AssetKind::Stylesheet,
                        });
                        i = next;
                    } else {
                        i += 7;
                    }
                } else {
                    i += 7;
                }
            }
            _ => i = next_char(css, i),
        }
    }

    out
}

#[derive(Debug, Clone)]
pub struct CssResource {
    pub url: Url,
    pub kind: AssetKind,
}

/// Every fetchable resource the stylesheet references, in document order.
pub fn extract_resources(css: &str, base: &Url) -> Vec<CssResource> {
    let mut seen = HashSet::new();
    scan(css)
        .into_iter()
        .filter_map(|raw| {
            let url = resolve(base, raw.url)?;
            seen.insert(url.as_str().to_string())
                .then_some(CssResource {
                    url,
                    kind: raw.kind,
                })
        })
        .collect()
}

/// Rewrite references to their archived equivalents.
///
/// `replacements` maps absolute, fragmentless resource URLs to the URL that
/// serves the captured bytes. Returns `None` when nothing matched.
pub fn rewrite_resources(
    css: &str,
    base: &Url,
    replacements: &HashMap<String, String>,
) -> Option<String> {
    let mut edits = Vec::new();

    for raw in scan(css) {
        let Some(key) = resolve(base, raw.url) else {
            continue;
        };
        let Some(replacement) = replacements.get(key.as_str()) else {
            continue;
        };

        let fragment = Url::options()
            .base_url(Some(base))
            .parse(raw.url.trim())
            .ok()
            .and_then(|url| url.fragment().map(str::to_string));
        let value = match fragment {
            Some(fragment) => format!("{replacement}#{fragment}"),
            None => replacement.clone(),
        };
        edits.push((raw.start, raw.end, value));
    }

    if edits.is_empty() {
        return None;
    }

    let mut out = String::with_capacity(css.len());
    let mut last = 0;
    for (start, end, value) in edits {
        if start < last {
            continue;
        }
        out.push_str(&css[last..start]);
        out.push_str(&value);
        last = end;
    }
    out.push_str(&css[last..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.com/assets/site.css").unwrap()
    }

    #[test]
    fn extracts_css_resources() {
        let css = r#"
            @import "theme.css";
            @font-face {
                font-family: "Outfit";
                src: url("../fonts/outfit.woff2") format("woff2"),
                     url(https://cdn.example.com/fallback.woff) format("woff");
            }
            .hero { background: url('/img/hero.png?size=2') no-repeat; }
            .inline { background: url(data:image/png;base64,AAAA); }
            /* url(commented-out.png) */
            .text { content: "url(not-a-resource.png)"; }
        "#;

        let got: Vec<String> = extract_resources(css, &base())
            .into_iter()
            .map(|r| r.url.to_string())
            .collect();

        assert_eq!(
            got,
            vec![
                "https://example.com/assets/theme.css",
                "https://example.com/fonts/outfit.woff2",
                "https://cdn.example.com/fallback.woff",
                "https://example.com/img/hero.png?size=2",
            ]
        );
    }

    #[test]
    fn rewrites_css_resources_and_keeps_fragments() {
        let css = r#"
            @import "theme.css";
            @font-face { src: url("../fonts/outfit.woff2") format("woff2"); }
            .icon { mask: url("/sprite.svg#icon"); }
            .inline { background: url(data:image/png;base64,AAAA); }
        "#;

        let replacements = HashMap::from([
            (
                "https://example.com/assets/theme.css".to_string(),
                "/api/content/theme".to_string(),
            ),
            (
                "https://example.com/fonts/outfit.woff2".to_string(),
                "/api/content/font".to_string(),
            ),
            (
                "https://example.com/sprite.svg".to_string(),
                "/api/content/sprite".to_string(),
            ),
        ]);

        let out = rewrite_resources(css, &base(), &replacements).unwrap();

        assert!(out.contains(r#"@import "/api/content/theme""#));
        assert!(out.contains(r#"url("/api/content/font") format("woff2")"#));
        assert!(out.contains(r#"url("/api/content/sprite#icon")"#));
        assert!(out.contains("url(data:image/png;base64,AAAA)"));
        assert!(!out.contains("outfit.woff2"));
    }

    #[test]
    fn handles_non_ascii_css_without_panicking() {
        let css = ".héro { content: \"héllo\"; background: url(hero.png); }";
        let got: Vec<String> = extract_resources(css, &base())
            .into_iter()
            .map(|r| r.url.to_string())
            .collect();
        assert_eq!(got, vec!["https://example.com/assets/hero.png"]);
    }

    #[test]
    fn rewrite_returns_none_when_nothing_matches() {
        let css = "body { background: url(missing.png); }";
        assert!(rewrite_resources(css, &base(), &HashMap::new()).is_none());
    }
}
