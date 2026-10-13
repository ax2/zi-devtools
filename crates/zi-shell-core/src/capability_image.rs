//! Bounded reads of a desktop-only immutable PE declaration section.
//! No module loading or execution. Callers retain the file consistency lock and
//! bind the declaration to the same executable digest.
use crate::capability_manifest::{MAX_BYTES, Manifest};
use anyhow::{Result, ensure};
use std::io::{Read, Seek, SeekFrom};

struct Image<'a, R> {
    reader: &'a mut R,
    length: u64,
}
impl<R: Read + Seek> Image<'_, R> {
    fn bytes(&mut self, offset: u64, size: usize) -> Result<Vec<u8>> {
        ensure!(
            offset
                .checked_add(size as u64)
                .is_some_and(|end| end <= self.length),
            "PE range outside file"
        );
        self.reader.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; size];
        self.reader.read_exact(&mut bytes)?;
        Ok(bytes)
    }
}
fn word(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn dword(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// Missing section means unknown legacy capabilities, never all current actions.
pub fn read<R: Read + Seek>(reader: &mut R, length: u64) -> Result<Option<Manifest>> {
    ensure!(
        (64..=256 * 1024 * 1024).contains(&length),
        "PE outside supported size"
    );
    let mut image = Image { reader, length };
    let dos = image.bytes(0, 64)?;
    ensure!(&dos[..2] == b"MZ", "Not a PE image");
    let pe = u64::from(dword(&dos, 60));
    ensure!((64..=1024 * 1024).contains(&pe), "Invalid PE header offset");
    let header = image.bytes(pe, 24)?;
    ensure!(&header[..4] == b"PE\0\0", "Invalid PE signature");
    let count = usize::from(word(&header, 6));
    let optional_size = usize::from(word(&header, 20));
    ensure!(
        (1..=96).contains(&count) && (120..=4096).contains(&optional_size),
        "PE headers outside supported bounds"
    );
    let optional = image.bytes(pe + 24, optional_size)?;
    ensure!(
        matches!(word(&optional, 0), 0x10b | 0x20b),
        "Unknown PE optional header"
    );
    let table_end = pe + 24 + optional_size as u64 + count as u64 * 40;
    let sections = image.bytes(pe + 24 + optional_size as u64, count * 40)?;
    let candidates: Vec<_> = sections
        .chunks_exact(40)
        .filter(|s| &s[..8] == b".zicap\0\0")
        .collect();
    ensure!(candidates.len() <= 1, "Duplicate capability sections");
    let Some(section) = candidates.first() else {
        return Ok(None);
    };
    let size = dword(section, 16);
    let offset = u64::from(dword(section, 20));
    let flags = dword(section, 36);
    ensure!(
        flags & 0x4000_0040 == 0x4000_0040 && flags & 0xa000_0000 == 0,
        "Capability section is not immutable data"
    );
    ensure!(
        (12..=MAX_BYTES as u32 + 4096).contains(&size) && offset >= table_end,
        "Invalid capability section size/offset"
    );
    ensure!(
        offset + u64::from(size) <= length,
        "Capability section outside file"
    );
    for other in sections.chunks_exact(40) {
        if other.as_ptr() == section.as_ptr() {
            continue;
        }
        let other_start = u64::from(dword(other, 20));
        let other_end = other_start + u64::from(dword(other, 16));
        ensure!(
            other_start >= offset + u64::from(size) || other_end <= offset,
            "Capability section overlaps another section"
        );
    }
    let header = image.bytes(offset, 12)?;
    ensure!(
        &header[..8] == b"ZICAP001",
        "Unknown capability section envelope"
    );
    let payload_length = dword(&header, 8);
    ensure!(
        (1..=MAX_BYTES as u32).contains(&payload_length) && payload_length + 12 <= size,
        "Invalid capability payload length"
    );
    let bytes = image.bytes(offset + 12, payload_length as usize)?;
    Manifest::parse(&bytes)
        .map(Some)
        .map_err(anyhow::Error::msg)
}

#[cfg(test)]
#[path = "capability_image_tests.rs"]
pub(crate) mod tests;
