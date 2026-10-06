//! Independent signed sidecar; the original complete release manifest stays schema 1.
use super::{delta, portable, signed};
use anyhow::{Context, Result, ensure};
use ring::signature::Ed25519KeyPair;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
const DOMAIN: &[u8] = b"ZiDevTools portable delta v1\0";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub patch: signed::File,
    pub source: signed::File,
    pub target: signed::File,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub source_version: String,
    pub target_version: String,
    pub target_manifest_sha256: String,
    pub files: Vec<Entry>,
}
pub fn patch_name(prefix: &str, source: &str, target: &str) -> String {
    format!("{prefix}-{source}-to-{target}-windows-x64.zidelta")
}
pub fn asset_name(name: &str, target: &str) -> bool {
    if name.len() > 320 {
        return false;
    }
    ["ZiDevTools-", "ZiDevToolsMcp-"].iter().any(|prefix| {
        name.strip_prefix(prefix)
            .and_then(|s| s.strip_suffix(&format!("-to-{target}-windows-x64.zidelta")))
            .is_some_and(|s| semver::Version::parse(s).is_ok_and(|v| v.to_string() == s))
    })
}
fn hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid(m: &Manifest, target: &signed::Authenticated, raw: &[u8]) -> Result<()> {
    let old = semver::Version::parse(&m.source_version)?;
    let new = semver::Version::parse(&m.target_version)?;
    ensure!(
        m.schema == 1
            && m.files.len() == 2
            && m.source_version.len() <= 128
            && m.target_version.len() <= 128
            && new.cmp_precedence(&old).is_gt(),
        "增量清单版本或数量无效"
    );
    ensure!(
        m.target_version == target.manifest.version
            && hash(&m.target_manifest_sha256)
            && m.target_manifest_sha256 == portable::digest(raw),
        "增量清单不属于本次认证目标发布"
    );
    for (i, prefix) in ["ZiDevTools", "ZiDevToolsMcp"].iter().enumerate() {
        let e = &m.files[i];
        ensure!(
            e.patch.name == patch_name(prefix, &m.source_version, &m.target_version)
                && e.source.name == format!("{prefix}-{}-windows-x64.exe", m.source_version)
                && e.target.name == format!("{prefix}-{}-windows-x64.exe", m.target_version),
            "增量文件名称或顺序无效"
        );
        for f in [&e.patch, &e.source, &e.target] {
            ensure!(
                f.size > 0 && f.size <= delta::MAX_FILE as u64 && hash(&f.sha256),
                "增量文件大小或摘要无效"
            );
        }
        let expected = signed::expected_file(target, &new, &e.target.name, e.target.size)?;
        ensure!(
            expected.sha256 == e.target.sha256,
            "重建目标与完整包签名不符"
        );
        ensure!(e.patch.size < e.target.size, "无传输收益的补丁不能发布");
    }
    Ok(())
}
pub fn sign(
    m: &Manifest,
    target: &signed::Authenticated,
    raw: &[u8],
    key: &Ed25519KeyPair,
) -> Result<(Vec<u8>, Vec<u8>)> {
    valid(m, target, raw)?;
    let raw = serde_json::to_vec_pretty(m)?;
    let sig = signed::sign_detached(&raw, DOMAIN, key)?;
    Ok((raw, sig))
}
pub fn verify(
    raw: &[u8],
    sig: &[u8],
    target: &signed::Authenticated,
    target_raw: &[u8],
    trust: &signed::TrustStore,
) -> Result<Manifest> {
    let key = signed::verify_detached(raw, sig, trust, DOMAIN)?;
    ensure!(key == target.key_id, "增量与完整发布的签名公钥不一致");
    let m: Manifest = serde_json::from_slice(raw).context("增量清单格式无效")?;
    valid(&m, target, target_raw)?;
    Ok(m)
}
pub fn verify_official(
    raw: &[u8],
    sig: &[u8],
    target: &signed::Authenticated,
    target_raw: &[u8],
) -> Result<Manifest> {
    let trust = serde_json::from_str(include_str!("../../docs/update-trust.json"))?;
    verify(raw, sig, target, target_raw, &trust)
}
pub fn baseline(m: &Manifest, version: &str, sources: [&[u8]; 2]) -> bool {
    m.source_version == version
        && sources.iter().zip(&m.files).all(|(bytes, e)| {
            bytes.len() as u64 == e.source.size && portable::digest(bytes) == e.source.sha256
        })
}
pub fn reconstruct(
    m: &Manifest,
    index: usize,
    source: &[u8],
    patch: &[u8],
    cancel: &AtomicBool,
) -> Result<Vec<u8>> {
    let e = m.files.get(index).context("增量文件索引无效")?;
    ensure!(
        source.len() as u64 == e.source.size && portable::digest(source) == e.source.sha256,
        "增量源文件不匹配"
    );
    ensure!(
        patch.len() as u64 == e.patch.size && portable::digest(patch) == e.patch.sha256,
        "增量补丁大小或摘要不符"
    );
    let (h, bytes) = delta::reconstruct(Some(source), patch, cancel)?;
    ensure!(
        h.source_version == m.source_version
            && h.target_version == m.target_version
            && h.source_size == e.source.size
            && h.source_sha256 == e.source.sha256
            && bytes.len() as u64 == e.target.size
            && portable::digest(&bytes) == e.target.sha256,
        "重建结果与认证清单不符"
    );
    Ok(bytes)
}
/// Returns None rather than publishing a patch larger than an uncompressed EXE.
pub fn create(
    source_version: &str,
    target: &signed::Authenticated,
    raw: &[u8],
    sources: [&[u8]; 2],
    targets: [&[u8]; 2],
    cancel: &AtomicBool,
) -> Result<Option<(Manifest, [Vec<u8>; 2])>> {
    let mut patches = [Vec::new(), Vec::new()];
    let mut files = Vec::new();
    for (i, prefix) in ["ZiDevTools", "ZiDevToolsMcp"].iter().enumerate() {
        let (h, patch) = delta::create(
            sources[i],
            targets[i],
            source_version,
            &target.manifest.version,
            cancel,
        )?;
        if patch.len() >= targets[i].len() {
            return Ok(None);
        }
        let descriptor = |name, bytes: &[u8]| signed::File {
            name,
            size: bytes.len() as u64,
            sha256: portable::digest(bytes),
        };
        files.push(Entry {
            patch: descriptor(
                patch_name(prefix, source_version, &target.manifest.version),
                &patch,
            ),
            source: descriptor(
                format!("{prefix}-{source_version}-windows-x64.exe"),
                sources[i],
            ),
            target: descriptor(
                format!("{prefix}-{}-windows-x64.exe", target.manifest.version),
                targets[i],
            ),
        });
        ensure!(
            h.target_sha256 == files[i].target.sha256,
            "生成补丁目标不符"
        );
        patches[i] = patch;
    }
    let m = Manifest {
        schema: 1,
        source_version: source_version.into(),
        target_version: target.manifest.version.clone(),
        target_manifest_sha256: portable::digest(raw),
        files,
    };
    valid(&m, target, raw)?;
    for i in 0..2 {
        ensure!(
            reconstruct(&m, i, sources[i], &patches[i], cancel)? == targets[i],
            "发布补丁自验失败"
        );
    }
    Ok(Some((m, patches)))
}
#[cfg(test)]
#[path = "signed_delta_tests.rs"]
mod tests;
