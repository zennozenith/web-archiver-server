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
use scraper::{Html, Node, Selector};
use url::Url;

#[derive(Debug)]
struct Link {
    node_id: NodeId,
    r#as: Option<Box<str>>,
    rel: Option<Box<str>>,
    href: Box<str>,
}

#[derive(Debug)]
struct Anchor {
    node_id: NodeId,
    href: Box<str>,
}

#[derive(Debug)]
struct Script {
    node_id: NodeId,
    src: Box<str>,
}

#[derive(Debug)]
struct Img {
    node_id: NodeId,
    src: Option<Box<str>>,
    srcset: Option<Box<str>>,
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

pub fn parse(html: &str) {
    let base_url = Url::parse("https://crates.io/crates/scraper").unwrap();
    let document = Html::parse_document(html);

    // println!("{:#?}", document);
    let selector = Selector::parse("script").unwrap();
    for element in document.select(&selector) {
        println!("{:#?}", element);
        println!("{:#?}", element.attr("src"));
    }
    let links = parse_link_element(&document);
    let anchors = parse_anchor_element(&document);
    let scripts = parse_script_element(&document);
    let imgs = parse_img_element(&document);
    println!("{:#?}", imgs);

    // let rels = itertools::Itertools::unique(links.iter().filter_map(|v| v.rel.as_ref()))
    //     .collect::<Vec<_>>();
    // println!("{:#?}", rels);

    let document = hello(&base_url, document, &links, &anchors);
    {
        let mut file = std::fs::File::create("temp/temp.html").unwrap();
        std::io::Write::write_all(&mut file, &document.html().into_bytes()).unwrap();
    }
}

fn parse_link_element(document: &Html) -> Vec<Link> {
    let selector = Selector::parse("link").expect("Invalid selector");

    document
        .select(&selector)
        .filter_map(|element| {
            let href = element.attr("href").map(Box::from)?;
            Some(Link {
                node_id: element.id(),
                r#as: element.attr("as").map(Box::from),
                rel: element.attr("rel").map(Box::from),
                href,
            })
        })
        .collect::<Vec<_>>()
}

fn parse_script_element(document: &Html) -> Vec<Script> {
    let selector = Selector::parse("script").expect("Invalid selector");

    document
        .select(&selector)
        .filter_map(|element| {
            let src = element.attr("src").map(Box::from)?;
            if element.attr("type") == Some("module") {
                tracing::error!("script[type=module]: {}", element.html())
            };
            Some(Script {
                node_id: element.id(),
                src,
            })
        })
        .collect::<Vec<_>>()
}

fn parse_img_element(document: &Html) -> Vec<Img> {
    // let selector = Selector::parse("img[src]").expect("Invalid selector");
    let selector = Selector::parse("img").expect("Invalid selector");

    document
        .select(&selector)
        .map(|element| Img {
            node_id: element.id(),
            src: element.attr("src").map(Box::from),
            srcset: element.attr("srcset").map(Box::from),
        })
        .filter(|v| v.src.is_some() || v.srcset.is_some())
        .collect::<Vec<_>>()
}

fn parse_anchor_element(document: &Html) -> Vec<Anchor> {
    let selector = Selector::parse("link").expect("Invalid selector");

    document
        .select(&selector)
        .filter_map(|element| {
            let href = element.attr("href").map(Box::from)?;
            Some(Anchor {
                node_id: element.id(),
                href,
            })
        })
        .collect::<Vec<_>>()
}

fn hello(base_url: &Url, mut document: Html, links: &[Link], anchors: &[Anchor]) -> Html {
    // let u = (match Url::parse(href) {
    //     Ok(v) => Some(v),
    //     Err(url::ParseError::RelativeUrlWithoutBase) => base_url
    //         .join(href)
    //         .inspect_err(|e| tracing::error!("URL pares: {e}"))
    //         .ok(),
    //     Err(e) => {
    //         tracing::error!("URL pares: {e}");
    //         None
    //     }
    // })?;

    for id in links.iter().map(|v| v.node_id) {
        // match element.attr("rel") {
        //     Some(x) if ["stylesheet", "icon", "apple-touch-icon", "manifest"].contains(&x) => {
        //         todo!()
        //     }
        //     Some(x) if ["stylesheet", "icon", "apple-touch-icon", "manifest"].contains(&x) => {
        //         todo!()
        //     }
        //     _ => return None,
        // }
        if let Some(mut child) = document.tree.get_mut(id) {
            let Node::Element(el) = child.value() else {
                continue;
            };
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
    document
}
