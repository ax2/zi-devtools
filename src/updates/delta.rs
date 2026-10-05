//! Versioned local reconstruction format, never an installer or authenticity proof.
use anyhow::{Context, Result, bail, ensure};
use flate2::{Compression, bufread::DeflateDecoder, write::DeflateEncoder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::atomic::{AtomicBool, Ordering},
};

const MAGIC: &[u8; 8] = b"ZIDELTA1";
pub const MAX_FILE: usize = 128 * 1024 * 1024;
const MAX_STREAM: usize = MAX_FILE + 1024 * 1024;
const MAX_PATCH: usize = MAX_STREAM + 64 * 1024;
const MAX_OPS: usize = 131_072;
const MAX_HEADER: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Delta,
    Full,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub schema: u32,
    pub source_version: String,
    pub target_version: String,
    pub kind: Kind,
    pub source_size: u64,
    pub target_size: u64,
    pub source_sha256: String,
    pub target_sha256: String,
    pub payload_sha256: String,
    pub payload_size: u64,
    pub copied_bytes: u64,
    pub literal_bytes: u64,
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
    Ok(())
}
fn hash(bytes: &[u8], cancel: &AtomicBool) -> Result<String> {
    let mut h = Sha256::new();
    for chunk in bytes.chunks(1024 * 1024) {
        cancelled(cancel)?;
        h.update(chunk);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn versions(source: &str, target: &str) -> Result<()> {
    ensure!(source.len() <= 128 && target.len() <= 128, "版本号过长");
    let source = semver::Version::parse(source).context("源版本号无效")?;
    let target = semver::Version::parse(target).context("目标版本号无效")?;
    ensure!(
        target.cmp_precedence(&source).is_gt(),
        "目标版本必须高于源版本（不以构建元数据区分）"
    );
    Ok(())
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Header {
    fn validate(&self) -> Result<()> {
        ensure!(self.schema == 1, "不支持的更新包版本");
        versions(&self.source_version, &self.target_version)?;
        ensure!(
            self.source_size <= MAX_FILE as u64
                && self.target_size <= MAX_FILE as u64
                && self.payload_size <= MAX_PATCH as u64,
            "更新包声明超过大小限制"
        );
        ensure!(
            valid_hash(&self.source_sha256)
                && valid_hash(&self.target_sha256)
                && valid_hash(&self.payload_sha256),
            "摘要格式无效"
        );
        ensure!(
            self.copied_bytes.checked_add(self.literal_bytes) == Some(self.target_size),
            "更新包统计无效"
        );
        ensure!(
            self.kind != Kind::Full || self.copied_bytes == 0,
            "完整内容包不能声明复制操作"
        );
        Ok(())
    }
}

// Content-defined boundaries resynchronize after insertion. This rolling value
// only selects boundaries; all reuse decisions use full SHA-256 block digests.
fn chunks(bytes: &[u8], cancel: &AtomicBool) -> Result<Vec<(usize, usize)>> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut rolling = 0u64;
    for (index, byte) in bytes.iter().enumerate() {
        if index % 65536 == 0 {
            cancelled(cancel)?;
        }
        let mut gear = (*byte as u64).wrapping_add(0x9e3779b97f4a7c15);
        gear = (gear ^ (gear >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        gear = (gear ^ (gear >> 27)).wrapping_mul(0x94d049bb133111eb);
        gear ^= gear >> 31;
        rolling = rolling.wrapping_shl(1).wrapping_add(gear);
        let len = index + 1 - start;
        if len >= 32768 || (len >= 2048 && rolling & 8191 == 0) {
            result.push((start, len));
            start = index + 1;
            rolling = 0;
        }
    }
    if start < bytes.len() {
        result.push((start, bytes.len() - start));
    }
    Ok(result)
}
fn compress(bytes: &[u8], cancel: &AtomicBool) -> Result<Vec<u8>> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    for chunk in bytes.chunks(65536) {
        cancelled(cancel)?;
        encoder.write_all(chunk)?;
    }
    cancelled(cancel)?;
    Ok(encoder.finish()?)
}
pub fn create(
    source: &[u8],
    target: &[u8],
    source_version: &str,
    target_version: &str,
    cancel: &AtomicBool,
) -> Result<(Header, Vec<u8>)> {
    cancelled(cancel)?;
    versions(source_version, target_version)?;
    ensure!(
        source.len() <= MAX_FILE && target.len() <= MAX_FILE,
        "单个文件最大128 MiB"
    );
    let mut index = HashMap::new();
    for (offset, len) in chunks(source, cancel)? {
        cancelled(cancel)?;
        index
            .entry(Sha256::digest(&source[offset..offset + len]).to_vec())
            .or_insert((offset, len));
    }
    let mut operations = Vec::new();
    let mut copied = 0u64;
    for (offset, len) in chunks(target, cancel)? {
        cancelled(cancel)?;
        let digest = Sha256::digest(&target[offset..offset + len]);
        if let Some(&(source_offset, source_len)) = index.get(digest.as_slice())
            && source_len == len
            && source[source_offset..source_offset + len] == target[offset..offset + len]
        {
            operations.push(0);
            operations.extend_from_slice(&(source_offset as u64).to_le_bytes());
            operations.extend_from_slice(&(len as u32).to_le_bytes());
            copied += len as u64;
        } else {
            operations.push(1);
            operations.extend_from_slice(&(len as u32).to_le_bytes());
            operations.extend_from_slice(&target[offset..offset + len]);
        }
    }
    operations.push(255);
    let delta = compress(&operations, cancel)?;
    let full = compress(target, cancel)?;
    let source_hash = hash(source, cancel)?;
    let target_hash = hash(target, cancel)?;
    let make_header = |kind: Kind, payload: &[u8], copied_bytes: u64| -> Result<Header> {
        Ok(Header {
            schema: 1,
            source_version: source_version.into(),
            target_version: target_version.into(),
            kind,
            source_size: source.len() as u64,
            target_size: target.len() as u64,
            source_sha256: source_hash.clone(),
            target_sha256: target_hash.clone(),
            payload_sha256: hash(payload, cancel)?,
            payload_size: payload.len() as u64,
            copied_bytes,
            literal_bytes: target.len() as u64 - copied_bytes,
        })
    };
    let delta_header = make_header(Kind::Delta, &delta, copied)?;
    let full_header = make_header(Kind::Full, &full, 0)?;
    let delta_json = serde_json::to_vec(&delta_header)?;
    let full_json = serde_json::to_vec(&full_header)?;
    let (header, payload, json) = if delta.len() + delta_json.len() < full.len() + full_json.len() {
        (delta_header, delta, delta_json)
    } else {
        (full_header, full, full_json)
    };
    ensure!(
        json.len() <= MAX_HEADER && payload.len() <= MAX_PATCH,
        "更新包超过限制"
    );
    let mut patch = Vec::with_capacity(12 + json.len() + payload.len());
    patch.extend_from_slice(MAGIC);
    patch.extend_from_slice(&(json.len() as u32).to_le_bytes());
    patch.extend_from_slice(&json);
    patch.extend_from_slice(&payload);
    Ok((header, patch))
}
pub fn inspect<'a>(patch: &'a [u8], cancel: &AtomicBool) -> Result<(Header, &'a [u8])> {
    cancelled(cancel)?;
    ensure!(
        patch.len() >= 12 && patch.len() <= MAX_PATCH + MAX_HEADER + 12 && &patch[..8] == MAGIC,
        "更新包标记或大小无效"
    );
    let len = u32::from_le_bytes(patch[8..12].try_into()?) as usize;
    ensure!(
        len > 0 && len <= MAX_HEADER && 12 + len <= patch.len(),
        "更新包头长度无效"
    );
    let header: Header = serde_json::from_slice(&patch[12..12 + len]).context("更新包头无效")?;
    header.validate()?;
    let payload = &patch[12 + len..];
    ensure!(
        payload.len() as u64 == header.payload_size
            && hash(payload, cancel)? == header.payload_sha256,
        "更新包内容摘要不匹配"
    );
    Ok((header, payload))
}
fn number<const N: usize>(stream: &[u8], position: &mut usize) -> Result<[u8; N]> {
    let end = position.checked_add(N).context("操作长度溢出")?;
    let bytes = stream.get(*position..end).context("更新包操作被截断")?;
    *position = end;
    Ok(bytes.try_into()?)
}
pub fn reconstruct(
    source: Option<&[u8]>,
    patch: &[u8],
    cancel: &AtomicBool,
) -> Result<(Header, Vec<u8>)> {
    let (header, payload) = inspect(patch, cancel)?;
    let source = if header.kind == Kind::Delta {
        let source = source.context("差分包需要匹配的源文件，请改用完整内容包")?;
        ensure!(
            source.len() as u64 == header.source_size
                && hash(source, cancel)? == header.source_sha256,
            "源文件不匹配，请使用对应源版本或完整内容包"
        );
        source
    } else {
        &[]
    };
    let limit = if header.kind == Kind::Full {
        header.target_size as usize
    } else {
        MAX_STREAM
    };
    let mut decoder = DeflateDecoder::new(payload);
    let mut stream = Vec::new();
    let mut buffer = [0; 65536];
    loop {
        cancelled(cancel)?;
        let count = decoder.read(&mut buffer).context("压缩内容损坏")?;
        if count == 0 {
            break;
        }
        ensure!(
            stream
                .len()
                .checked_add(count)
                .is_some_and(|size| size <= limit),
            "解压内容超过限制"
        );
        stream.extend_from_slice(&buffer[..count]);
    }
    ensure!(
        decoder.total_in() as usize == payload.len(),
        "压缩内容有未消费的尾随数据"
    );
    let target = if header.kind == Kind::Full {
        stream
    } else {
        let mut target = Vec::new();
        let mut pos = 0;
        let mut count = 0;
        let mut copied = 0u64;
        let mut literal = 0u64;
        loop {
            cancelled(cancel)?;
            count += 1;
            ensure!(count <= MAX_OPS, "更新包操作过多");
            let opcode = number::<1>(&stream, &mut pos)?[0];
            if opcode == 255 {
                ensure!(pos == stream.len(), "更新包操作尾随数据");
                break;
            }
            let bytes = match opcode {
                0 => {
                    let offset = usize::try_from(u64::from_le_bytes(number(&stream, &mut pos)?))?;
                    let len = u32::from_le_bytes(number(&stream, &mut pos)?) as usize;
                    ensure!(len > 0, "空复制操作无效");
                    let end = offset.checked_add(len).context("复制范围溢出")?;
                    copied = copied.checked_add(len as u64).context("复制统计溢出")?;
                    source.get(offset..end).context("复制操作超出源文件")?
                }
                1 => {
                    let len = u32::from_le_bytes(number(&stream, &mut pos)?) as usize;
                    ensure!(len > 0, "空数据操作无效");
                    let end = pos.checked_add(len).context("数据范围溢出")?;
                    let data = stream.get(pos..end).context("更新包数据被截断")?;
                    pos = end;
                    literal = literal.checked_add(len as u64).context("数据统计溢出")?;
                    data
                }
                _ => bail!("未知更新包操作"),
            };
            ensure!(
                target
                    .len()
                    .checked_add(bytes.len())
                    .is_some_and(|len| len as u64 <= header.target_size),
                "重建超出目标长度"
            );
            target.extend_from_slice(bytes);
        }
        ensure!(
            copied == header.copied_bytes && literal == header.literal_bytes,
            "操作统计与包头不一致"
        );
        target
    };
    ensure!(
        target.len() as u64 == header.target_size && hash(&target, cancel)? == header.target_sha256,
        "重建目标摘要或长度不匹配"
    );
    cancelled(cancel)?;
    Ok((header, target))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_files_and_decompression_limits_are_explicit() {
        let cancel = AtomicBool::new(false);
        let (_, patch) = create(b"", b"", "1.0.0", "2.0.0", &cancel).unwrap();
        assert!(reconstruct(None, &patch, &cancel).unwrap().1.is_empty());
        let (mut header, _) = create(b"old", b"x", "1.0.0", "2.0.0", &cancel).unwrap();
        let payload = compress(&vec![0; 65536], &cancel).unwrap();
        header.payload_size = payload.len() as u64;
        header.payload_sha256 = hash(&payload, &cancel).unwrap();
        let json = serde_json::to_vec(&header).unwrap();
        let mut patch = MAGIC.to_vec();
        patch.extend_from_slice(&(json.len() as u32).to_le_bytes());
        patch.extend_from_slice(&json);
        patch.extend_from_slice(&payload);
        assert!(
            reconstruct(None, &patch, &cancel)
                .unwrap_err()
                .to_string()
                .contains("超过限制")
        );
        header.target_size = MAX_FILE as u64 + 1;
        header.literal_bytes = header.target_size;
        assert!(header.validate().is_err());
    }
    fn random(len: usize) -> Vec<u8> {
        let mut state = 17u64;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    }
    #[test]
    fn insertions_reuse_blocks_and_exactly_reconstruct() {
        let source = random(512 * 1024);
        let mut target = source.clone();
        target.splice(12000..12000, b"tutorial update".iter().copied());
        let cancel = AtomicBool::new(false);
        let (header, patch) = create(&source, &target, "0.1.0", "0.2.0", &cancel).unwrap();
        assert_eq!(header.kind, Kind::Delta);
        assert!(header.copied_bytes > 400 * 1024);
        assert!(patch.len() < target.len() / 4);
        assert_eq!(
            reconstruct(Some(&source), &patch, &cancel).unwrap().1,
            target
        );
        assert!(reconstruct(Some(b"wrong"), &patch, &cancel).is_err());
        assert!(reconstruct(None, &patch, &cancel).is_err());
    }
    #[test]
    fn complete_payload_fallback_needs_no_source() {
        let cancel = AtomicBool::new(false);
        let (header, patch) = create(b"old", b"new", "1.0.0", "1.1.0", &cancel).unwrap();
        assert_eq!(header.kind, Kind::Full);
        assert_eq!(reconstruct(None, &patch, &cancel).unwrap().1, b"new");
        assert!(create(b"a", b"b", "1.1.0", "1.0.0", &cancel).is_err());
        assert!(create(b"a", b"b", "1.0.0+a", "1.0.0+b", &cancel).is_err());
    }
    #[test]
    fn corruption_truncation_unknown_schema_and_cancellation_are_rejected() {
        let cancel = AtomicBool::new(false);
        let (_, patch) = create(b"a", b"b", "1.0.0", "2.0.0", &cancel).unwrap();
        let mut corrupt = patch.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(inspect(&corrupt, &cancel).is_err());
        for cut in [0, 8, 12, patch.len() - 1] {
            assert!(inspect(&patch[..cut], &cancel).is_err());
        }
        let (mut header, payload) = inspect(&patch, &cancel).unwrap();
        header.schema = 2;
        let json = serde_json::to_vec(&header).unwrap();
        let mut future = MAGIC.to_vec();
        future.extend_from_slice(&(json.len() as u32).to_le_bytes());
        future.extend_from_slice(&json);
        future.extend_from_slice(payload);
        assert!(inspect(&future, &cancel).is_err());
        cancel.store(true, Ordering::Relaxed);
        assert!(create(b"a", b"b", "1.0.0", "2.0.0", &cancel).is_err());
        assert!(reconstruct(None, &patch, &cancel).is_err());
    }
    #[test]
    fn malicious_operations_and_compressed_tail_are_rejected() {
        let cancel = AtomicBool::new(false);
        let (mut header, patch) = create(b"source", b"target", "1.0.0", "2.0.0", &cancel).unwrap();
        let (_, payload) = inspect(&patch, &cancel).unwrap();
        let mut payload = payload.to_vec();
        payload.extend_from_slice(b"tail");
        header.payload_size = payload.len() as u64;
        header.payload_sha256 = hash(&payload, &cancel).unwrap();
        let json = serde_json::to_vec(&header).unwrap();
        let mut patch = MAGIC.to_vec();
        patch.extend_from_slice(&(json.len() as u32).to_le_bytes());
        patch.extend_from_slice(&json);
        patch.extend_from_slice(&payload);
        assert!(reconstruct(None, &patch, &cancel).is_err());
        header.kind = Kind::Delta;
        header.copied_bytes = 6;
        header.literal_bytes = 0;
        let mut stream = vec![0];
        stream.extend_from_slice(&99999u64.to_le_bytes());
        stream.extend_from_slice(&6u32.to_le_bytes());
        stream.push(255);
        let payload = compress(&stream, &cancel).unwrap();
        header.payload_size = payload.len() as u64;
        header.payload_sha256 = hash(&payload, &cancel).unwrap();
        let json = serde_json::to_vec(&header).unwrap();
        let mut patch = MAGIC.to_vec();
        patch.extend_from_slice(&(json.len() as u32).to_le_bytes());
        patch.extend_from_slice(&json);
        patch.extend_from_slice(&payload);
        assert!(reconstruct(Some(b"source"), &patch, &cancel).is_err());
    }
}
