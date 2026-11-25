//! Embedded-text PDF parsing shared by the desktop ingestion and Studio plugin.

use lopdf::{Dictionary, Document, ObjectId, content::Operation};
use serde::Serialize;

pub const MAX_PDF_BYTES: usize = 128 * 1024;
pub const MAX_PAGES: usize = 32;
pub const MAX_TEXT_BYTES: usize = 6 * 1024;

pub fn open(bytes: &[u8]) -> lopdf::Result<Document> {
    Document::load_mem(bytes)
}

pub fn page_text(document: &Document, page: u32) -> lopdf::Result<String> {
    document.extract_text(&[page])
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageStatus {
    Text,
    Blank,
    ImageWithoutText,
    NoExtractableText,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub page: u32,
    pub page_count: usize,
    pub source_byte_count: usize,
    pub status: PageStatus,
    pub content: String,
    pub truncated: bool,
}

pub fn preview(bytes: &[u8], page: u32) -> Result<Preview, &'static str> {
    if bytes.len() > MAX_PDF_BYTES {
        return Err("INPUT_TOO_LARGE");
    }
    if !bytes.starts_with(b"%PDF-") {
        return Err("MALFORMED_PDF");
    }
    let doc = open(bytes).map_err(|_| "MALFORMED_PDF")?;
    if doc.is_encrypted() || doc.trailer.has(b"Encrypt") {
        return Err("ENCRYPTED_PDF");
    }
    let pages: Vec<_> = doc.page_iter().take(MAX_PAGES + 1).collect();
    if pages.is_empty() {
        return Err("MALFORMED_PDF");
    }
    if pages.len() > MAX_PAGES {
        return Err("PAGE_LIMIT");
    }
    let id = *pages
        .get(page.checked_sub(1).ok_or("PAGE_NOT_FOUND")? as usize)
        .ok_or("PAGE_NOT_FOUND")?;
    for stream in doc.get_page_contents(id) {
        doc.get_object(stream)
            .and_then(|object| object.as_stream())
            .map_err(|_| "MALFORMED_PDF")?;
    }
    let mut content = page_text(&doc, page).map_err(|_| "MALFORMED_PDF")?;
    let status = if content.trim().is_empty() {
        content.clear();
        let decoded = doc
            .get_and_decode_page_content(id)
            .map_err(|_| "MALFORMED_PDF")?;
        if decoded.operations.is_empty() {
            PageStatus::Blank
        } else if draws_direct_image(&doc, id, &decoded.operations) {
            PageStatus::ImageWithoutText
        } else {
            PageStatus::NoExtractableText
        }
    } else {
        PageStatus::Text
    };
    let truncated = content.len() > MAX_TEXT_BYTES;
    if truncated {
        let mut end = MAX_TEXT_BYTES;
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        content.truncate(end);
    }
    Ok(Preview {
        page,
        page_count: pages.len(),
        source_byte_count: bytes.len(),
        status,
        content,
        truncated,
    })
}

fn draws_direct_image(doc: &Document, page: ObjectId, operations: &[Operation]) -> bool {
    let Ok((direct, inherited)) = doc.get_page_resources(page) else {
        return false;
    };
    let resources = direct.into_iter().chain(
        inherited
            .into_iter()
            .filter_map(|id| doc.get_dictionary(id).ok()),
    );
    resources
        .filter_map(|resource| image_resources(doc, resource))
        .any(|images| {
            operations
                .iter()
                .filter(|op| op.operator == "Do")
                .filter_map(|op| op.operands.first()?.as_name().ok())
                .any(|name| {
                    images
                        .get_deref(name, doc)
                        .ok()
                        .and_then(|item| item.as_stream().ok())
                        .and_then(|stream| stream.dict.get(b"Subtype").ok())
                        .and_then(|subtype| subtype.as_name().ok())
                        == Some(b"Image")
                })
        })
}

fn image_resources<'a>(doc: &'a Document, resources: &'a Dictionary) -> Option<&'a Dictionary> {
    resources.get_deref(b"XObject", doc).ok()?.as_dict().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_raw_size_above_bound_and_non_pdf_without_parsing() {
        assert_eq!(
            preview(&vec![0; MAX_PDF_BYTES + 1], 1).unwrap_err(),
            "INPUT_TOO_LARGE"
        );
        assert_eq!(preview(b"Text only", 1).unwrap_err(), "MALFORMED_PDF");
    }
}
