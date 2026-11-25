//! Text extraction for saved HTML, shared with the native knowledge capture tool.
use anyhow::{Result, ensure};
use scraper::{ElementRef, Html, Selector};

pub const MAX_BODY: usize = 256 * 1024;

fn visible_text(element: ElementRef<'_>) -> String {
    element
        .descendants()
        .filter_map(|node| {
            let text = node.value().as_text()?;
            let hidden = node.ancestors().filter_map(ElementRef::wrap).any(|parent| {
                matches!(
                    parent.value().name(),
                    "script"
                        | "style"
                        | "noscript"
                        | "template"
                        | "nav"
                        | "footer"
                        | "aside"
                        | "header"
                )
            });
            (!hidden).then_some(text.trim().to_owned())
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn html_body(html: &str) -> Result<(String, String)> {
    let doc = Html::parse_document(html);
    let title_selector = Selector::parse("title").expect("static selector");
    let title = doc
        .select(&title_selector)
        .next()
        .map(visible_text)
        .unwrap_or_default();
    let mut best = String::new();
    for selector in ["main", "article", "body"] {
        let selector = Selector::parse(selector).expect("static selector");
        for element in doc.select(&selector) {
            let text = visible_text(element);
            if text.len() > best.len() {
                best = text;
            }
        }
        if best.chars().count() >= 80 {
            break;
        }
    }
    ensure!(
        !best.trim().is_empty(),
        "网页没有可提取的正文；请从浏览器复制可见文字"
    );
    ensure!(best.len() <= MAX_BODY, "网页提取正文超过 256 KiB");
    Ok((title, best))
}
