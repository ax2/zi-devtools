//! Shared insertion of one verified document into the native v1 knowledge index.
use anyhow::Result;
use rusqlite::{Connection, params};
use zi_document_text_core::Chunk;

pub struct IndexedDocument<'a> {
    pub source_id: &'a str,
    pub relative: &'a str,
    pub sha256: &'a str,
    pub bytes: u64,
    pub text_bytes: u64,
    pub indexed_at: &'a str,
    pub chunks: &'a [Chunk],
}

/// Insert within the caller's transaction. The native FTS5 triggers own all
/// updates to the index so the standalone and static builders stay in sync.
pub fn insert_document(
    connection: &Connection,
    document: IndexedDocument<'_>,
    mut check_cancel: impl FnMut() -> Result<()>,
) -> Result<()> {
    connection.execute(
        "INSERT INTO indexed_files(source_id,relative,sha256,bytes,text_bytes,chunk_count,indexed_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            document.source_id, document.relative, document.sha256, document.bytes,
            document.text_bytes, document.chunks.len(), document.indexed_at
        ],
    )?;
    let mut insert = connection.prepare(
        "INSERT INTO chunks(source_id,relative,ordinal,location,sha256,text)
         VALUES(?1,?2,?3,?4,?5,?6)",
    )?;
    for chunk in document.chunks {
        check_cancel()?;
        insert.execute(params![
            document.source_id,
            document.relative,
            chunk.ordinal,
            chunk.location,
            chunk.sha256,
            chunk.text
        ])?;
    }
    Ok(())
}
