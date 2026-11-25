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
