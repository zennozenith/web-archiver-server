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
pub(crate) enum AssetKind {
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
    pub(crate) const fn as_str(self) -> &'static str {
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
pub(crate) struct ExtractedResource {
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

fn kind_for_link(element: &ElementRef<'_>) -> Option<AssetKind> {
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

fn parent_tag(element: ElementRef<'_>) -> Option<&str> {
    element.parent().and_then(|parent| match parent.value() {
        Node::Element(el) => Some(el.name()),
        Node::Document
        | Node::Fragment
        | Node::Doctype(_)
        | Node::Comment(_)
        | Node::Text(_)
        | Node::ProcessingInstruction(_) => None,
    })
}

/// Visit every resource reference in the document.
///
/// `attr` is the attribute to rewrite, `srcset` tells the rewriter that the
/// attribute contains several URL/descriptor pairs.
#[expect(
    clippy::too_many_lines,
    reason = "one pass over every supported HTML resource element"
)]
fn walk<F>(document: &Html, page_url: &Url, mut visit: F)
where
    F: FnMut(&ElementRef<'_>, &'static str, bool, Url, AssetKind),
{
    let base = document_base(document, page_url);

    let mut push = |el: &ElementRef<'_>, attr: &'static str, srcset: bool, raw: &str, kind| {
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
        for (name, value) in &el.value().attrs {
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
pub(crate) fn extract_resources(page_url: &Url, html: &str) -> Vec<ExtractedResource> {
    let document = Html::parse_document(html);
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    walk(&document, page_url, |_el, _attr, _srcset, url, kind| {
        if seen.insert(url.as_str().to_owned()) {
            out.push(ExtractedResource { url, kind });
        }
    });

    out
}

fn rewrite_url(raw: &str, base: &Url, replacements: &HashMap<String, String>) -> Option<String> {
    let resolved = Url::options().base_url(Some(base)).parse(raw.trim()).ok()?;
    let fragment = resolved.fragment().map(str::to_owned);
    let mut key = resolved;
    key.set_fragment(None);

    let replacement = replacements.get(key.as_str())?;
    Some(fragment.map_or_else(
        || replacement.clone(),
        |fragment| format!("{replacement}#{fragment}"),
    ))
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
            let rewritten = rewrite_url(url, base, replacements).unwrap_or_else(|| url.to_owned());

            Some(if descriptors.is_empty() {
                rewritten
            } else {
                format!("{rewritten} {}", descriptors.join(" "))
            })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve a page link (`<a href>`, `<area href>`) against the document base.
///
/// Unlike resources, links are not served from the archive: their target is
/// another page, so relative hrefs are made absolute to the *original* site.
/// Fragment-only and non-navigational schemes are left untouched.
fn absolute_link(base: &Url, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('#') {
        return None;
    }

    let lower = raw.to_ascii_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
        || lower.starts_with("data:")
        || lower.starts_with("blob:")
        || lower.starts_with("about:")
    {
        return None;
    }

    let url = Url::options().base_url(Some(base)).parse(raw).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

/// Rewrite resource URLs to their archived equivalents, and make page links
/// point at the original site.
///
/// `replacements` maps absolute, fragmentless resource URLs to the URL that
/// serves the captured bytes (usually `/api/content/<blake3>`). Returns `None`
/// when nothing in the document matched, so the caller can avoid storing a
/// second identical HTML blob.
#[expect(
    clippy::too_many_lines,
    reason = "resource rewriting and link absolutization share one parsed document"
)]
pub(crate) fn rewrite_resources(
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

    // Anchors keep pointing at the live web so navigation still works from the
    // archive. Relative hrefs must be resolved now, while the original page URL
    // is still known.
    let mut links = Vec::new();
    for element in document.select(&sel("a[href], area[href]")) {
        let Some(href) = element.attr("href") else {
            continue;
        };
        let Some(absolute) = absolute_link(&base, href) else {
            continue;
        };
        if absolute != href {
            links.push((element.id(), absolute));
        }
    }

    if edits.is_empty() && links.is_empty() {
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

    for (node_id, href) in links {
        let Some(mut node) = document.tree.get_mut(node_id) else {
            continue;
        };
        let Node::Element(element) = node.value() else {
            continue;
        };
        if let Some((_, value)) = element
            .attrs
            .iter_mut()
            .find(|(name, _)| name.local.as_str() == "href")
        {
            *value = href.into();
        }
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
        assert!(got.contains(&"https://example.com/assets/site.css".to_owned()));
        assert!(got.contains(&"https://example.com/docs/favicon.ico".to_owned()));
        assert!(got.contains(&"https://example.com/fonts/inter.woff2".to_owned()));
        assert!(got.contains(&"https://example.com/docs/app.js".to_owned()));
        assert!(got.contains(&"https://cdn.example.com/card.png".to_owned()));
        assert!(got.contains(&"https://example.com/docs/hero.png".to_owned()));
        assert!(got.contains(&"https://example.com/docs/hero-2x.png".to_owned()));
        assert!(got.contains(&"https://example.com/docs/pic.webp".to_owned()));
        assert!(got.contains(&"https://example.com/docs/movie.mp4".to_owned()));
        assert!(got.contains(&"https://example.com/docs/poster.jpg".to_owned()));
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
                "https://example.com/assets/site.css".to_owned(),
                "/api/content/css".to_owned(),
            ),
            (
                "https://cdn.example.com/card.png".to_owned(),
                "/api/content/card".to_owned(),
            ),
            (
                "https://example.com/docs/hero.png".to_owned(),
                "/api/content/hero".to_owned(),
            ),
            (
                "https://example.com/docs/hero-2x.png".to_owned(),
                "/api/content/hero2".to_owned(),
            ),
            (
                "https://example.com/docs/hero-3x.png".to_owned(),
                "/api/content/hero3".to_owned(),
            ),
            (
                "https://example.com/docs/poster.jpg".to_owned(),
                "/api/content/poster".to_owned(),
            ),
            (
                "https://example.com/sprite.svg".to_owned(),
                "/api/content/sprite".to_owned(),
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
    fn absolutizes_navigation_links_to_the_original_site() {
        let html = br##"<!doctype html>
            <html><body>
              <a href="/about">About</a>
              <a href="docs/guide.html">Guide</a>
              <a href="https://other.example/x">External</a>
              <a href="#section">Section</a>
              <a href="mailto:hi@example.com">Mail</a>
              <area href="map.html">
            </body></html>"##;

        // No resources are rewritten; the links alone must still produce a blob.
        let base = Url::parse("https://example.com/docs/page.html").unwrap();
        let out = rewrite_resources(html, &base, &HashMap::new()).unwrap();
        let out = String::from_utf8(out).unwrap();

        assert!(out.contains(r#"href="https://example.com/about""#));
        assert!(out.contains(r#"href="https://example.com/docs/docs/guide.html""#));
        assert!(out.contains(r#"href="https://other.example/x""#));
        assert!(out.contains(r##"href="#section""##));
        assert!(out.contains(r#"href="mailto:hi@example.com""#));
        assert!(out.contains(r#"href="https://example.com/docs/map.html""#));
    }

    #[test]
    fn navigation_links_respect_base_href() {
        let html = br#"<base href="https://cdn.example.com/static/">
            <a href="guide.html">Guide</a>"#;
        let page = Url::parse("https://example.com/docs/page.html").unwrap();
        let out = rewrite_resources(html, &page, &HashMap::new()).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains(r#"href="https://cdn.example.com/static/guide.html""#));
    }

    #[test]
    fn rewrite_returns_none_when_nothing_matches() {
        let base = Url::parse("https://example.com/").unwrap();
        let html = br#"<img src="missing.png">"#;
        assert!(rewrite_resources(html, &base, &HashMap::new()).is_none());
    }
}
