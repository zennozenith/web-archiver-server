// TODO: handle <picture> element
// TODO: handle script[type=module]
// TODO: handle `import("/_app/immutable/entry/start.CcOUnjuJ.js")` inside script
// TODO: `<video src>`, `<video poster>`, `<audio src>`, `<source src>`, `<track src>` (subtitles)
// TODO: `<iframe src>`, `<embed src>`, `<object data>`
// TODO: `<input type=image src>`
// TODO: `<svg><image href>` and `<use href>` (also the older `xlink:href`)
// TODO: `<meta property="og:image">`, `twitter:image`, and `msapplication-TileImage`
// TODO: `<base href>`. This one changes how all relative URLs resolve, so read it first.

use ego_tree::NodeId;
use html5ever::{LocalName, QualName, ns};
use itertools::Itertools;
use scraper::{Html, Node, Selector};
use tokio::{fs, io::AsyncWriteExt as _};
use url::Url;
use uuid::Uuid;

#[derive(Debug)]
struct Link {
    node_id: NodeId,
    r#as: Option<String>,
    rel: Option<String>,
    href: String,
    url: Url,
}

#[derive(Debug)]
struct Script {
    node_id: NodeId,
    src: String,
    url: Url,
}

#[derive(Debug)]
struct Img {
    node_id: NodeId,
    src: Option<String>,
    srcset: Option<String>,
    url: Option<Url>,
}

// impl std::fmt::Debug for Link {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         f.debug_struct("Link")
//             .field("node_id", &self.node_id)
//             .field("as", &self.r#as)
//             .field("rel", &self.rel)
//             .field("href", &self.href)
//             .finish()
//     }
// }

fn parse_url_with_base(input: &str, base: &Url) -> Result<Url, url::ParseError> {
    // match Url::parse(input) {
    //     Err(ParseError::RelativeUrlWithoutBase) => {
    //         base.join(input)
    //     }
    //     other => other,
    // }
    Url::options().base_url(Some(base)).parse(input)
}

pub fn parse(base_url: &Url, html: &str) {
    let mut document = Html::parse_document(html);

    let links = parse_link_element(&document, base_url);
    let scripts = parse_script_element(&document, base_url);
    let imgs = parse_img_element(&document, base_url);
    // println!("{:#?}", links);
    let _t = links
        .iter()
        .map(|v| v.url.as_ref())
        .chain(scripts.iter().map(|v| v.url.as_ref()))
        .unique()
        .inspect(|v| println!("Url: {}", v))
        .collect::<Vec<_>>();

    for link in links.iter() {
        // match element.attr("rel") {
        //     Some(x) if ["stylesheet", "icon", "apple-touch-icon", "manifest"].contains(&x) => {
        //         todo!()
        //     }
        //     Some(x) if ["stylesheet", "icon", "apple-touch-icon", "manifest"].contains(&x) => {
        //         todo!()
        //     }
        //     _ => return None,
        // }
        if let Some(mut child) = document.tree.get_mut(link.node_id) {
            let Node::Element(el) = child.value() else {
                continue;
            };
            // let maybe_link = Url::parse(&link.href);
            // let u_href = match maybe_link {
            //     Ok(_) => link.href,
            //     Err(_) => ,
            // };
            let u_href = "https://example.com/link".into();
            let href = QualName::new(None, ns!(), LocalName::from("href"));
            if let Some(i) = el.attrs.iter().position(|attr| attr.0 == href) {
                el.attrs[i].1 = u_href;
            } else {
                el.attrs.push((href, u_href));
            }
            // println!("{:#?}", el);
        }
    }

    // {
    //     let mut file = std::fs::File::create("temp/temp.html").unwrap();
    //     std::io::Write::write_all(&mut file, &document.html().into_bytes()).unwrap();
    // }
}

fn parse_link_element(document: &Html, base_url: &Url) -> Vec<Link> {
    let selector = Selector::parse("link").expect("Invalid selector");

    document
        .select(&selector)
        .filter_map(|element| {
            let href = element.attr("href").map(String::from)?;
            let url = parse_url_with_base(&href, base_url).ok()?;
            Some(Link {
                node_id: element.id(),
                r#as: element.attr("as").map(String::from),
                rel: element.attr("rel").map(String::from),
                href,
                url,
            })
        })
        .collect::<Vec<_>>()
}

fn parse_script_element(document: &Html, base_url: &Url) -> Vec<Script> {
    let selector = Selector::parse("script").expect("Invalid selector");

    document
        .select(&selector)
        .filter_map(|element| {
            let src = element.attr("src").map(String::from)?;
            let url = parse_url_with_base(&src, base_url).ok()?;
            if element.attr("type") == Some("module") {
                tracing::error!("script[type=module]: {}", element.html())
            };
            Some(Script {
                node_id: element.id(),
                src,
                url,
            })
        })
        .collect::<Vec<_>>()
}

fn parse_img_element(document: &Html, base_url: &Url) -> Vec<Img> {
    // let selector = Selector::parse("img[src]").expect("Invalid selector");
    let selector = Selector::parse("img").expect("Invalid selector");

    document
        .select(&selector)
        .map(|element| {
            let src = element.attr("src").map(String::from);
            let url = src
                .as_ref()
                .and_then(|s| parse_url_with_base(s, base_url).ok());

            Img {
                node_id: element.id(),
                src,
                srcset: element.attr("srcset").map(String::from),
                url,
            }
        })
        .filter(|v| v.src.is_some() || v.srcset.is_some())
        .collect::<Vec<_>>()
}

async fn download_and_save(
    client: &reqwest::Client,
    url_str: &str,
    index: usize,
    tmp_dir: &std::path::Path,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Send HTTP GET request
    let response = client.get(url_str).send().await?;

    if !response.status().is_success() {
        return Err(format!("HTTP error status: {}", response.status()).into());
    }

    // Extract bytes from response
    let bytes = response.bytes().await?;

    // Generate a file name (fallback to index if parsing fails)

    let file_name = Uuid::now_v7().to_string();

    let file_path = tmp_dir.join(format!("{}_{}", index, file_name));

    // Write bytes asynchronously to disk
    let mut file = fs::File::create(&file_path).await?;
    file.write_all(&bytes).await?;

    Ok(file_path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn test_get() {
        let html_content = include_str!("../../temp/axum_doc/index.html");
        let base_url = Url::parse("http://zenitharch:9001/").unwrap();
        parse(&base_url, html_content);
    }
}
