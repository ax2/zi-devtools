//! Bounded access to the body XML of a DOCX. No other archive entry is read.

use anyhow::{Context, Result, ensure};
use quick_xml::{Reader, events::Event};
use std::{
    fmt,
    io::{Cursor, Read},
};

#[derive(Debug, Clone, Copy)]
pub enum DocxError {
    Archive,
    EntryLimit,
    MissingBody,
    XmlLimit,
    InvalidXml,
}
impl DocxError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Archive => "INVALID_ARCHIVE",
            Self::EntryLimit => "ARCHIVE_ENTRIES_EXCEEDED",
            Self::MissingBody => "MISSING_DOCUMENT_BODY",
            Self::XmlLimit => "XML_TOO_LARGE",
            Self::InvalidXml => "INVALID_DOCUMENT_XML",
        }
    }
}
impl fmt::Display for DocxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Archive => "Word 文件 ZIP 结构无效",
            Self::EntryLimit => "Word 文件条目过多",
            Self::MissingBody => "缺少 word/document.xml",
            Self::XmlLimit => "Word 正文 XML 超过限制",
            Self::InvalidXml => "Word 正文 XML 格式无效",
        })
    }
}
impl std::error::Error for DocxError {}

pub fn read_paragraphs(
    bytes: &[u8],
    max_entries: usize,
    max_xml_bytes: usize,
    mut visitor: impl FnMut(usize, String) -> Result<()>,
) -> Result<usize> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).context(DocxError::Archive)?;
    ensure!(archive.len() <= max_entries, DocxError::EntryLimit);
    let file = archive
        .by_name("word/document.xml")
        .context(DocxError::MissingBody)?;
    ensure!(file.size() <= max_xml_bytes as u64, DocxError::XmlLimit);
    let mut xml = Vec::new();
    file.take(max_xml_bytes.saturating_add(1) as u64)
        .read_to_end(&mut xml)
        .context(DocxError::Archive)?;
    ensure!(xml.len() <= max_xml_bytes, DocxError::XmlLimit);

    let mut reader = Reader::from_reader(xml.as_slice());
    reader.config_mut().trim_text(false);
    let mut paragraph = String::new();
    let mut number = 0;
    let mut in_text = false;
    let mut depth = 0usize;
    let mut seen_root = false;
    loop {
        let event = reader.read_event().context(DocxError::InvalidXml)?;
        match &event {
            Event::Start(tag) | Event::Empty(tag) if depth == 0 => {
                ensure!(
                    !seen_root && tag.local_name().as_ref() == b"document",
                    DocxError::InvalidXml
                );
                seen_root = true;
            }
            Event::DocType(_) => return Err(DocxError::InvalidXml.into()),
            _ => {}
        }
        match event {
            Event::Start(tag) => {
                depth += 1;
                if tag.name().as_ref() == b"w:t" {
                    in_text = true;
                }
            }
            Event::End(tag) => {
                depth = depth.checked_sub(1).ok_or(DocxError::InvalidXml)?;
                match tag.name().as_ref() {
                    b"w:t" => in_text = false,
                    b"w:p" => {
                        number += 1;
                        visitor(number, std::mem::take(&mut paragraph))?;
                    }
                    _ => {}
                }
            }
            Event::Text(value) if in_text => {
                paragraph.push_str(
                    &quick_xml::escape::unescape(&value.decode().context(DocxError::InvalidXml)?)
                        .context(DocxError::InvalidXml)?,
                );
            }
            Event::GeneralRef(value) if in_text => {
                let encoded = format!("&{};", value.decode().context(DocxError::InvalidXml)?);
                paragraph.push_str(
                    &quick_xml::escape::unescape(&encoded).context(DocxError::InvalidXml)?,
                );
            }
            Event::Empty(tag) if tag.name().as_ref() == b"w:tab" => paragraph.push('\t'),
            Event::Empty(tag) if tag.name().as_ref() == b"w:br" => paragraph.push('\n'),
            Event::Eof => {
                ensure!(seen_root && depth == 0, DocxError::InvalidXml);
                break;
            }
            _ => {}
        }
    }
    Ok(xml.len())
}
