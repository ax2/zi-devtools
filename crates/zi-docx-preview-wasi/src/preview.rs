//! DOCX preview using the standalone document ingestion's deterministic chunks.

use serde::Serialize;
use zi_document_text_core::{
    Chunk, MAX_CHUNKS, chunks_from,
    docx::{DocxError, read_paragraphs},
};

pub const MAX_DOCX_BYTES: usize = 128 * 1024;
const MAX_XML_BYTES: usize = 128 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 256;
const MAX_SHOWN_CHUNKS: usize = 8;
const MAX_SHOWN_TEXT_BYTES: usize = 6 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    source_byte_count: usize,
    xml_byte_count: usize,
    non_empty_paragraph_count: usize,
    total_chunks: usize,
    chunk_chars: usize,
    overlap_chars: usize,
    truncated: bool,
    chunks: Vec<Chunk>,
}

pub fn run(bytes: &[u8]) -> Result<Report, &'static str> {
    if bytes.len() > MAX_DOCX_BYTES {
        return Err("INPUT_TOO_LARGE");
    }
    let mut sections = Vec::new();
    let xml_byte_count =
        read_paragraphs(bytes, MAX_ARCHIVE_ENTRIES, MAX_XML_BYTES, |number, text| {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                anyhow::ensure!(sections.len() < MAX_CHUNKS, "Too many document paragraphs");
                sections.push((format!("段落 {number}"), trimmed.to_owned()));
            }
            Ok(())
        })
        .map_err(|error| {
            error
                .downcast_ref::<DocxError>()
                .map_or("PARAGRAPH_LIMIT", |error| error.code())
        })?;
    let non_empty_paragraph_count = sections.len();
    let mut chunks = if sections.is_empty() {
        Vec::new()
    } else {
        chunks_from(sections).map_err(|_| "CHUNK_LIMIT")?
    };
    let total_chunks = chunks.len();
    let mut preview_bytes = 0;
    let shown = chunks
        .iter()
        .take(MAX_SHOWN_CHUNKS)
        .take_while(|chunk| {
            preview_bytes += chunk.text.len();
            preview_bytes <= MAX_SHOWN_TEXT_BYTES
        })
        .count();
    chunks.truncate(shown);
    Ok(Report {
        source_byte_count: bytes.len(),
        xml_byte_count,
        non_empty_paragraph_count,
        total_chunks,
        chunk_chars: zi_document_text_core::CHUNK_CHARS,
        overlap_chars: zi_document_text_core::OVERLAP_CHARS,
        truncated: shown != total_chunks,
        chunks,
    })
}
