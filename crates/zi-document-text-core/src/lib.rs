//! Shared deterministic document ingestion and chunk boundaries.

use anyhow::{Result, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub mod docx;

pub const CHUNK_CHARS: usize = 800;
pub const OVERLAP_CHARS: usize = 100;
pub const MAX_CHUNKS: usize = 2_000;

#[derive(Clone, Debug, Serialize)]
pub struct Chunk {
    pub location: String,
    pub ordinal: usize,
    pub text: String,
    pub sha256: String,
}

/// Split UTF-8 plain text at the same paragraph boundaries as native ingestion.
pub fn plain_paragraphs(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split("\n\n").enumerate()
}

/// Collect a bounded paragraph preview without allocating sections after the limit.
pub fn plain_text_sections(
    text: &str,
    max_sections: usize,
    max_bytes: usize,
) -> Result<Vec<(String, String)>> {
    let mut sections = Vec::new();
    let mut stored_bytes = 0_usize;
    for (index, paragraph) in plain_paragraphs(text) {
        let trimmed = paragraph.trim();
        if trimmed.is_empty() {
            continue;
        }
        ensure!(sections.len() < max_sections, "段落超出数量上限");
        ensure!(
            stored_bytes.saturating_add(paragraph.len()) <= max_bytes,
            "提取正文超出字节上限"
        );
        stored_bytes += trimmed.len();
        sections.push((format!("段落 {}", index + 1), trimmed.to_owned()));
    }
    Ok(sections)
}

pub fn chunks_from(sections: Vec<(String, String)>) -> Result<Vec<Chunk>> {
    let mut chunks = Vec::new();
    for (location, text) in sections {
        let chars: Vec<char> = text.chars().collect();
        let mut start = 0;
        while start < chars.len() {
            ensure!(chunks.len() < MAX_CHUNKS, "分块超过 2000 个");
            let end = (start + CHUNK_CHARS).min(chars.len());
            let part: String = chars[start..end].iter().collect();
            chunks.push(Chunk {
                location: location.clone(),
                ordinal: chunks.len() + 1,
                sha256: format!("{:x}", Sha256::digest(part.as_bytes())),
                text: part,
            });
            if end == chars.len() {
                break;
            }
            start = end - OVERLAP_CHARS;
        }
    }
    ensure!(!chunks.is_empty(), "未提取到可用正文");
    Ok(chunks)
}
