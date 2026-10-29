//! Finds (and rewrites) the sub-resources an HTML document needs to render.
//!
//! The archive endpoint stores each resource as a deduplicated content blob and
//! records the mapping in `capture_asset`. This module only does the parsing;
//! fetching lives in `crate::downloader`.

use std::collections::{HashMap, HashSet};

use ego_tree::NodeId;
use scraper::{ElementRef, Html, Node, Selector};
use url::Url;

/// Stable label stored in `capture_asset.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Stylesheet,
    Script,
    Image,
    Font,
    Media,
    Frame,
    Manifest,
    Icon,
    Other,
}

impl AssetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stylesheet => "stylesheet",
            Self::Script => "script",
            Self::Image => "image",
            Self::Font => "font",
            Self::Media => "media",
            Self::Frame => "frame",
            Self::Manifest => "manifest",
            Self::Icon => "icon",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtractedResource {
    pub url: Url,
    pub kind: AssetKind,
}

fn sel(css: &str) -> Selector {
    Selector::parse(css).expect("static selector must be valid")
}

/// Resolve `raw` against `base`, keeping only fetchable http(s) URLs.
///
/// Fragments are dropped: `sprite.svg#icon` and `sprite.svg#other` are the same
/// bytes and would otherwise create two content rows / asset entries.
pub(super) fn resolve(base: &Url, raw: &str) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let lower = raw.to_ascii_lowercase();
    if lower.starts_with("data:")
        || lower.starts_with("blob:")
        || lower.starts_with("javascript:")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
        || lower.starts_with("about:")
        || lower.starts_with('#')
    {
        return None;
    }

    let mut url = Url::options().base_url(Some(base)).parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    Some(url)
}

/// `<base href>` changes how every other relative URL resolves, so read it first.
fn document_base(document: &Html, page_url: &Url) -> Url {
    document
        .select(&sel("base[href]"))
        .next()
        .and_then(|el| el.attr("href"))
        .and_then(|href| resolve(page_url, href))
        .unwrap_or_else(|| page_url.clone())
}

/// `srcset` is a comma-separated list of `url [descriptor]` entries.
fn srcset_urls(value: &str) -> impl Iterator<Item = &str> {
    value.split(',').filter_map(|candidate| {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            None
        } else {
            candidate.split_whitespace().next()
        }
    })
}

fn kind_for_as(value: &str) -> AssetKind {
    match value.trim().to_ascii_lowercase().as_str() {
        "style" => AssetKind::Stylesheet,
        "script" => AssetKind::Script,
        "image" => AssetKind::Image,
        "font" => AssetKind::Font,
        "audio" | "video" | "track" => AssetKind::Media,
        "document" => AssetKind::Frame,
        _ => AssetKind::Other,
    }
}

fn kind_for_link(element: &ElementRef) -> Option<AssetKind> {
    let rel = element.attr("rel").unwrap_or("").to_ascii_lowercase();
    let rels: HashSet<&str> = rel.split_whitespace().collect();

    if rels.contains("stylesheet") {
        Some(AssetKind::Stylesheet)
    } else if rels.contains("icon")
        || rels.contains("apple-touch-icon")
        || rels.contains("mask-icon")
    {
        Some(AssetKind::Icon)
    } else if rels.contains("manifest") {
        Some(AssetKind::Manifest)
    } else if rels.contains("modulepreload") {
        Some(AssetKind::Script)
    } else if rels.contains("preload") || rels.contains("prefetch") {
        Some(kind_for_as(element.attr("as").unwrap_or("")))
    } else {
        None
    }
}

fn parent_tag<'a>(element: ElementRef<'a>) -> Option<&'a str> {
    element.parent().and_then(|parent| match parent.value() {
        Node::Element(el) => Some(el.name()),
        _ => None,
    })
}

/// Visit every resource reference in the document.
///
/// `attr` is the attribute to rewrite, `srcset` tells the rewriter that the
/// attribute contains several URL/descriptor pairs.
fn walk<F>(document: &Html, page_url: &Url, mut visit: F)
where
    F: FnMut(&ElementRef<'_>, &'static str, bool, Url, AssetKind),
{
    let base = document_base(document, page_url);

    let mut push = |el: &ElementRef, attr: &'static str, srcset: bool, raw: &str, kind| {
        let Some(url) = resolve(&base, raw) else {
            return;
        };
        // Guards against a document embedding itself (e.g. an iframe pointing at
        // the page URL), which would otherwise fetch/store the same document.
        if url == *page_url {
            return;
        }
        visit(el, attr, srcset, url, kind);
    };

    for el in document.select(&sel("link[href]")) {
        if let (Some(href), Some(kind)) = (el.attr("href"), kind_for_link(&el)) {
            push(&el, "href", false, href, kind);
        }
    }

    for el in document.select(&sel("script[src]")) {
        if let Some(src) = el.attr("src") {
            push(&el, "src", false, src, AssetKind::Script);
        }
    }

    for el in document.select(&sel("img")) {
        if let Some(src) = el.attr("src") {
            push(&el, "src", false, src, AssetKind::Image);
        }
        if let Some(srcset) = el.attr("srcset") {
            for url in srcset_urls(srcset) {
                push(&el, "srcset", true, url, AssetKind::Image);
            }
        }
    }

    for el in document.select(&sel("source")) {
        // <source> in <picture> is an image; in <video>/<audio> it is media.
        let kind = match parent_tag(el) {
            Some("picture") => AssetKind::Image,
            _ => AssetKind::Media,
        };
        if let Some(src) = el.attr("src") {
            push(&el, "src", false, src, kind);
        }
        if let Some(srcset) = el.attr("srcset") {
            for url in srcset_urls(srcset) {
                push(&el, "srcset", true, url, kind);
            }
        }
    }

    for el in document.select(&sel("video")) {
        if let Some(src) = el.attr("src") {
            push(&el, "src", false, src, AssetKind::Media);
        }
        if let Some(poster) = el.attr("poster") {
            push(&el, "poster", false, poster, AssetKind::Image);
        }
    }

    for el in document.select(&sel("audio[src]")) {
        if let Some(src) = el.attr("src") {
            push(&el, "src", false, src, AssetKind::Media);
        }
    }

    for el in document.select(&sel("track[src]")) {
        if let Some(src) = el.attr("src") {
            push(&el, "src", false, src, AssetKind::Media);
        }
    }

    for el in document.select(&sel("iframe[src]")) {
        if let Some(src) = el.attr("src") {
            push(&el, "src", false, src, AssetKind::Frame);
        }
    }

    for el in document.select(&sel("embed[src]")) {
        if let Some(src) = el.attr("src") {
            push(&el, "src", false, src, AssetKind::Frame);
        }
    }

    for el in document.select(&sel("object[data]")) {
        if let Some(data) = el.attr("data") {
            push(&el, "data", false, data, AssetKind::Frame);
        }
    }

    for el in document.select(&sel("input[type=image]")) {
        if let Some(src) = el.attr("src") {
            push(&el, "src", false, src, AssetKind::Image);
        }
    }

    // SVG <image href> / <use href>, including the legacy xlink:href form.
    // Attributes are matched by local name so both forms are found.
    for el in document.select(&sel("image, use")) {
        for (name, value) in el.value().attrs.iter() {
            if name.local.as_str() == "href" {
                push(&el, "href", false, value.as_ref(), AssetKind::Image);
            }
        }
    }

    // Social-card / platform tile images are not required for rendering, but
    // they are the "links" most users notice when an archived page is incomplete.
    for el in document.select(&sel("meta[content]")) {
        let key = el
            .attr("property")
            .or_else(|| el.attr("name"))
            .unwrap_or("")
            .to_ascii_lowercase();
        let kind = match key.as_str() {
            "og:image"
            | "og:image:url"
            | "og:image:secure_url"
            | "twitter:image"
            | "twitter:image:src"
            | "msapplication-tileimage" => AssetKind::Image,
            "og:video"
            | "og:video:url"
            | "og:video:secure_url"
            | "og:audio"
            | "og:audio:url"
            | "og:audio:secure_url" => AssetKind::Media,
            _ => continue,
        };
        if let Some(content) = el.attr("content") {
            push(&el, "content", false, content, kind);
        }
    }
}

/// Extract every resource the document references, in document order.
pub fn extract_resources(page_url: &Url, html: &str) -> Vec<ExtractedResource> {
    let document = Html::parse_document(html);
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    walk(&document, page_url, |_el, _attr, _srcset, url, kind| {
        if seen.insert(url.as_str().to_string()) {
            out.push(ExtractedResource { url, kind });
        }
    });

    out
}

fn rewrite_url(raw: &str, base: &Url, replacements: &HashMap<String, String>) -> Option<String> {
    let resolved = Url::options().base_url(Some(base)).parse(raw.trim()).ok()?;
    let fragment = resolved.fragment().map(str::to_string);
    let mut key = resolved;
    key.set_fragment(None);

    let replacement = replacements.get(key.as_str())?;
    Some(match fragment {
        Some(fragment) => format!("{replacement}#{fragment}"),
        None => replacement.clone(),
    })
}

fn rewrite_srcset(raw: &str, base: &Url, replacements: &HashMap<String, String>) -> String {
    raw.split(',')
        .filter_map(|candidate| {
            let candidate = candidate.trim();
            if candidate.is_empty() {
                return None;
            }

            let mut parts = candidate.split_whitespace();
            let url = parts.next()?;
            let descriptors = parts.collect::<Vec<_>>();
            let rewritten = rewrite_url(url, base, replacements).unwrap_or_else(|| url.to_string());

            Some(if descriptors.is_empty() {
                rewritten
            } else {
                format!("{rewritten} {}", descriptors.join(" "))
            })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Rewrite resource URLs to their archived equivalents.
///
/// `replacements` maps absolute, fragmentless resource URLs to the URL that
/// serves the captured bytes (usually `/api/content/<blake3>`). Returns `None`
/// when nothing in the document matched, so the caller can avoid storing a
/// second identical HTML blob.
pub fn rewrite_resources(
    html: &[u8],
    page_url: &Url,
    replacements: &HashMap<String, String>,
) -> Option<Vec<u8>> {
    let text = String::from_utf8_lossy(html);
    let mut document = Html::parse_document(&text);
    let base = document_base(&document, page_url);

    let mut edits: HashSet<(NodeId, &'static str, bool)> = HashSet::new();
    walk(&document, page_url, |el, attr, srcset, url, _kind| {
        if replacements.contains_key(url.as_str()) {
            edits.insert((el.id(), attr, srcset));
        }
    });

    if edits.is_empty() {
        return None;
    }

    for (node_id, attr, srcset) in edits {
        let Some(mut node) = document.tree.get_mut(node_id) else {
            continue;
        };
        let Node::Element(element) = node.value() else {
            continue;
        };
        let Some((_, value)) = element
            .attrs
            .iter_mut()
            .find(|(name, _)| name.local.as_str() == attr)
        else {
            continue;
        };

        let raw = value.to_string();
        let rewritten = if srcset {
            rewrite_srcset(&raw, &base, replacements)
        } else {
            rewrite_url(&raw, &base, replacements).unwrap_or(raw)
        };
        *value = rewritten.into();
    }

    Some(document.html().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(html: &str) -> Vec<String> {
        let base = Url::parse("https://example.com/docs/page.html").unwrap();
        extract_resources(&base, html)
            .into_iter()
            .map(|r| r.url.to_string())
            .collect()
    }

    #[test]
    fn extracts_common_resources() {
        let html = r#"
            <html><head>
              <link rel="stylesheet" href="/assets/site.css">
              <link rel="icon" href="favicon.ico">
              <link rel="preload" as="font" href="/fonts/inter.woff2">
              <script src="app.js"></script>
              <meta property="og:image" content="https://cdn.example.com/card.png">
            </head><body>
              <img src="hero.png" srcset="hero-2x.png 2x, hero-3x.png 3x">
              <picture><source srcset="pic.webp"></picture>
              <video src="movie.mp4" poster="poster.jpg"></video>
            </body></html>"#;

        let got = urls(html);
        assert!(got.contains(&"https://example.com/assets/site.css".to_string()));
        assert!(got.contains(&"https://example.com/docs/favicon.ico".to_string()));
        assert!(got.contains(&"https://example.com/fonts/inter.woff2".to_string()));
        assert!(got.contains(&"https://example.com/docs/app.js".to_string()));
        assert!(got.contains(&"https://cdn.example.com/card.png".to_string()));
        assert!(got.contains(&"https://example.com/docs/hero.png".to_string()));
        assert!(got.contains(&"https://example.com/docs/hero-2x.png".to_string()));
        assert!(got.contains(&"https://example.com/docs/pic.webp".to_string()));
        assert!(got.contains(&"https://example.com/docs/movie.mp4".to_string()));
        assert!(got.contains(&"https://example.com/docs/poster.jpg".to_string()));
    }

    #[test]
    fn respects_base_href_and_skips_non_fetchable_urls() {
        let html = r#"
            <base href="https://cdn.example.com/static/">
            <img src="logo.png">
            <img src="data:image/png;base64,AAAA">
            <a href="/not-a-resource">nope</a>
        "#;

        let got = urls(html);
        assert_eq!(got, vec!["https://cdn.example.com/static/logo.png"]);
    }

    #[test]
    fn deduplicates_fragments() {
        let html = r#"
            <svg>
              <use href="/sprite.svg#a"></use>
              <use href="/sprite.svg#b"></use>
            </svg>
        "#;

        let got = urls(html);
        assert_eq!(got, vec!["https://example.com/sprite.svg"]);
    }

    #[test]
    fn rewrites_html_resource_urls() {
        let html = br#"<!doctype html>
            <html><head>
              <link rel="stylesheet" href="/assets/site.css">
              <meta property="og:image" content="https://cdn.example.com/card.png">
            </head><body>
              <img src="hero.png" srcset="hero-2x.png 2x, hero-3x.png 3x">
              <video poster="poster.jpg"></video>
              <svg><use href="/sprite.svg#icon"></use></svg>
            </body></html>"#;

        let replacements = HashMap::from([
            (
                "https://example.com/assets/site.css".to_string(),
                "/api/content/css".to_string(),
            ),
            (
                "https://cdn.example.com/card.png".to_string(),
                "/api/content/card".to_string(),
            ),
            (
                "https://example.com/docs/hero.png".to_string(),
                "/api/content/hero".to_string(),
            ),
            (
                "https://example.com/docs/hero-2x.png".to_string(),
                "/api/content/hero2".to_string(),
            ),
            (
                "https://example.com/docs/hero-3x.png".to_string(),
                "/api/content/hero3".to_string(),
            ),
            (
                "https://example.com/docs/poster.jpg".to_string(),
                "/api/content/poster".to_string(),
            ),
            (
                "https://example.com/sprite.svg".to_string(),
                "/api/content/sprite".to_string(),
            ),
        ]);

        let base = Url::parse("https://example.com/docs/page.html").unwrap();
        let out = rewrite_resources(html, &base, &replacements).unwrap();
        let out = String::from_utf8(out).unwrap();

        assert!(out.contains(r#"href="/api/content/css""#));
        assert!(out.contains(r#"content="/api/content/card""#));
        assert!(out.contains(r#"src="/api/content/hero""#));
        assert!(out.contains(r#"srcset="/api/content/hero2 2x, /api/content/hero3 3x""#));
        assert!(out.contains(r#"poster="/api/content/poster""#));
        assert!(out.contains(r#"href="/api/content/sprite#icon""#));
        assert!(!out.contains("site.css"));
        assert!(!out.contains("hero.png"));
    }

    #[test]
    fn rewrite_returns_none_when_nothing_matches() {
        let base = Url::parse("https://example.com/").unwrap();
        let html = br#"<img src="missing.png">"#;
        assert!(rewrite_resources(html, &base, &HashMap::new()).is_none());
    }
}
