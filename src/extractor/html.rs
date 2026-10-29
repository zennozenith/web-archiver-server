//! Finds the sub-resources an HTML document needs in order to render.
//!
//! The archive endpoint stores each resource as a deduplicated content blob and
//! records the mapping in `capture_asset`. This module only does the parsing;
//! fetching lives in `crate::downloader`.

use std::collections::HashSet;

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
fn resolve(base: &Url, raw: &str) -> Option<Url> {
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

struct Collector<'a> {
    base: &'a Url,
    page_url: &'a Url,
    out: Vec<ExtractedResource>,
    seen: HashSet<String>,
}

impl<'a> Collector<'a> {
    fn new(base: &'a Url, page_url: &'a Url) -> Self {
        Self {
            base,
            page_url,
            out: Vec::new(),
            seen: HashSet::new(),
        }
    }

    fn push(&mut self, raw: &str, kind: AssetKind) {
        let Some(url) = resolve(self.base, raw) else {
            return;
        };
        // Guards against a document embedding itself (e.g. an iframe pointing at
        // the page URL), which would otherwise fetch/store the same document.
        if url == *self.page_url {
            return;
        }
        if self.seen.insert(url.as_str().to_string()) {
            self.out.push(ExtractedResource { url, kind });
        }
    }

    fn push_srcset(&mut self, raw: &str, kind: AssetKind) {
        for url in srcset_urls(raw) {
            self.push(url, kind);
        }
    }
}

/// Extract every resource the document references, in document order.
///
/// Handles `<base href>`, `<link>` (stylesheets/icons/manifests/preloads),
/// scripts, images (including `srcset`), `picture` sources, media, frames,
/// SVG `image`/`use`, and social-card metadata images.
pub fn extract_resources(page_url: &Url, html: &str) -> Vec<ExtractedResource> {
    let document = Html::parse_document(html);

    // <base href> changes how every other relative URL resolves, so read it first.
    let base = document
        .select(&sel("base[href]"))
        .next()
        .and_then(|el| el.attr("href"))
        .and_then(|href| resolve(page_url, href))
        .unwrap_or_else(|| page_url.clone());

    let mut c = Collector::new(&base, page_url);

    for el in document.select(&sel("link[href]")) {
        if let (Some(href), Some(kind)) = (el.attr("href"), kind_for_link(&el)) {
            c.push(href, kind);
        }
    }

    for el in document.select(&sel("script[src]")) {
        if let Some(src) = el.attr("src") {
            c.push(src, AssetKind::Script);
        }
    }

    for el in document.select(&sel("img")) {
        if let Some(src) = el.attr("src") {
            c.push(src, AssetKind::Image);
        }
        if let Some(srcset) = el.attr("srcset") {
            c.push_srcset(srcset, AssetKind::Image);
        }
    }

    for el in document.select(&sel("source")) {
        // <source> in <picture> is an image; in <video>/<audio> it is media.
        let kind = match parent_tag(el) {
            Some("picture") => AssetKind::Image,
            _ => AssetKind::Media,
        };
        if let Some(src) = el.attr("src") {
            c.push(src, kind);
        }
        if let Some(srcset) = el.attr("srcset") {
            c.push_srcset(srcset, kind);
        }
    }

    for el in document.select(&sel("video")) {
        if let Some(src) = el.attr("src") {
            c.push(src, AssetKind::Media);
        }
        if let Some(poster) = el.attr("poster") {
            c.push(poster, AssetKind::Image);
        }
    }

    for el in document.select(&sel("audio[src]")) {
        if let Some(src) = el.attr("src") {
            c.push(src, AssetKind::Media);
        }
    }

    for el in document.select(&sel("track[src]")) {
        if let Some(src) = el.attr("src") {
            c.push(src, AssetKind::Media);
        }
    }

    for el in document.select(&sel("iframe[src]")) {
        if let Some(src) = el.attr("src") {
            c.push(src, AssetKind::Frame);
        }
    }

    for el in document.select(&sel("embed[src]")) {
        if let Some(src) = el.attr("src") {
            c.push(src, AssetKind::Frame);
        }
    }

    for el in document.select(&sel("object[data]")) {
        if let Some(data) = el.attr("data") {
            c.push(data, AssetKind::Frame);
        }
    }

    for el in document.select(&sel("input[type=image]")) {
        if let Some(src) = el.attr("src") {
            c.push(src, AssetKind::Image);
        }
    }

    // SVG <image href> / <use href>, including the legacy xlink:href form.
    for el in document.select(&sel("image, use")) {
        if let Some(href) = el.attr("href").or_else(|| el.attr("xlink:href")) {
            c.push(href, AssetKind::Image);
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
            c.push(content, kind);
        }
    }

    c.out
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
}
