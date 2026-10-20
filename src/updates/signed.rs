//! Domain-separated Ed25519 manifests. Only compiled publisher keys are trusted.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::signature::{self, Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub const MAX_MANIFEST: usize = 512 * 1024;
pub const MAX_SIGNATURE: usize = 1024;
const DOMAIN: &[u8] = b"ZiDevTools update manifest v1\0";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherKey {
    pub id: String,
    pub public_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustStore {
    pub schema: u32,
    pub keys: Vec<PublisherKey>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub status: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolChangeSummary {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
}
impl ToolChangeSummary {
    pub fn len(&self) -> usize {
        self.added.len() + self.updated.len() + self.removed.len()
    }
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.updated.is_empty() && self.removed.is_empty()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    pub source_commit: String,
    pub platform: String,
    pub minimum_windows_major: u32,
    pub files: Vec<File>,
    pub tools: Vec<Tool>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Signature {
    schema: u32,
    key_id: String,
    signature: String,
}
#[derive(Clone, Debug)]
pub struct Authenticated {
    pub manifest: Manifest,
    pub key_id: String,
}
fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn publisher(key: &Ed25519KeyPair) -> PublisherKey {
    PublisherKey {
        id: format!("{:x}", Sha256::digest(key.public_key().as_ref())),
        public_key: STANDARD.encode(key.public_key().as_ref()),
    }
}
#[cfg(test)]
fn payload(raw: &[u8]) -> Vec<u8> {
    [DOMAIN, raw].concat()
}
fn validate(manifest: &Manifest) -> Result<()> {
    ensure!(
        manifest.schema == 1
            && manifest.platform == "windows-x64"
            && manifest.minimum_windows_major == 10,
        "更新清单协议或系统范围不受支持"
    );
    ensure!(
        manifest.version.len() <= 128 && semver::Version::parse(&manifest.version).is_ok(),
        "更新版本无效"
    );
    ensure!(hex(&manifest.source_commit, 40), "更新源码提交无效");
    let names = [
        format!("ZiDevTools-{}-windows-x64.exe", manifest.version),
        format!("ZiDevTools-{}-windows-x64.msi", manifest.version),
        format!("ZiDevToolsMcp-{}-windows-x64.exe", manifest.version),
    ];
    let mut seen = HashSet::new();
    ensure!(manifest.files.len() == 3, "更新清单必须包含三个发布文件");
    for file in &manifest.files {
        ensure!(
            names.contains(&file.name)
                && seen.insert(&file.name)
                && file.size > 0
                && file.size <= 128 * 1024 * 1024
                && hex(&file.sha256, 64),
            "更新附件名称、大小或摘要无效"
        );
    }
    ensure!(
        !manifest.tools.is_empty() && manifest.tools.len() <= 2048,
        "工具版本清单数量无效"
    );
    let mut seen = HashSet::new();
    for tool in &manifest.tools {
        ensure!(
            !tool.id.is_empty()
                && tool.id.len() <= 160
                && tool
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_:/".contains(&b))
                && seen.insert(&tool.id),
            "工具标识无效或重复"
        );
        ensure!(
            !tool.name.trim().is_empty()
                && tool.name.len() <= 256
                && !tool.name.chars().any(char::is_control),
            "工具名称无效"
        );
        ensure!(
            tool.version.as_ref().is_none_or(
                |version| version.len() <= 128 && semver::Version::parse(version).is_ok()
            ) && matches!(
                tool.status.as_str(),
                "implemented" | "in-progress" | "planned"
            ),
            "工具版本或状态无效"
        );
    }
    Ok(())
}
pub fn sign(manifest: &Manifest, key: &Ed25519KeyPair) -> Result<(Vec<u8>, Vec<u8>)> {
    validate(manifest)?;
    let raw = serde_json::to_vec_pretty(manifest)?;
    ensure!(raw.len() <= MAX_MANIFEST, "更新清单超过大小限制");
    let signature = sign_detached(&raw, DOMAIN, key)?;
    Ok((raw, signature))
}
pub(super) fn sign_detached(raw: &[u8], domain: &[u8], key: &Ed25519KeyPair) -> Result<Vec<u8>> {
    ensure!(
        !raw.is_empty() && raw.len() <= MAX_MANIFEST,
        "签名内容大小无效"
    );
    let signature = Signature {
        schema: 1,
        key_id: publisher(key).id,
        signature: STANDARD.encode(key.sign(&[domain, raw].concat()).as_ref()),
    };
    Ok(serde_json::to_vec(&signature)?)
}
pub fn verify(raw: &[u8], signature: &[u8], trust: &TrustStore) -> Result<Authenticated> {
    let key_id = verify_detached(raw, signature, trust, DOMAIN)?;
    let manifest: Manifest = serde_json::from_slice(raw).context("更新清单格式无效")?;
    validate(&manifest)?;
    Ok(Authenticated { manifest, key_id })
}
pub(super) fn verify_detached(
    raw: &[u8],
    signature: &[u8],
    trust: &TrustStore,
    domain: &[u8],
) -> Result<String> {
    ensure!(
        !raw.is_empty() && raw.len() <= MAX_MANIFEST && signature.len() <= MAX_SIGNATURE,
        "签名或更新清单超过大小限制"
    );
    ensure!(
        trust.schema == 1 && !trust.keys.is_empty() && trust.keys.len() <= 8,
        "发布公钥配置不可用"
    );
    let signature: Signature = serde_json::from_slice(signature).context("签名文件格式无效")?;
    ensure!(
        signature.schema == 1 && hex(&signature.key_id, 64),
        "签名协议或公钥标识无效"
    );
    let mut seen = HashSet::new();
    let mut selected = None;
    for key in &trust.keys {
        let public = STANDARD
            .decode(&key.public_key)
            .context("发布公钥格式无效")?;
        ensure!(
            public.len() == 32
                && hex(&key.id, 64)
                && key.id == format!("{:x}", Sha256::digest(&public))
                && seen.insert(&key.id),
            "发布公钥配置无效或重复"
        );
        if key.id == signature.key_id {
            selected = Some(public);
        }
    }
    let key = selected.context("签名使用未受信任的发布公钥")?;
    let bytes = STANDARD
        .decode(&signature.signature)
        .context("签名编码无效")?;
    ensure!(bytes.len() == 64, "签名长度无效");
    signature::UnparsedPublicKey::new(&signature::ED25519, &key)
        .verify(&[domain, raw].concat(), &bytes)
        .map_err(|_| anyhow::anyhow!("发布签名验证失败，不能使用该更新清单"))?;
    Ok(signature.key_id)
}
pub fn verify_official(raw: &[u8], signature: &[u8]) -> Result<Authenticated> {
    let trust: TrustStore = serde_json::from_str(include_str!("../../docs/update-trust.json"))
        .context("内置发布公钥配置无效")?;
    verify(raw, signature, &trust)
}
pub fn expected_file<'a>(
    authenticated: &'a Authenticated,
    version: &semver::Version,
    name: &str,
    size: u64,
) -> Result<&'a File> {
    ensure!(
        authenticated.manifest.version == version.to_string(),
        "签名清单与发布版本不一致"
    );
    let file = authenticated
        .manifest
        .files
        .iter()
        .find(|f| f.name == name)
        .context("签名清单缺少所选附件")?;
    ensure!(file.size == size, "签名清单与发布附件大小不一致");
    Ok(file)
}
pub fn tools(catalog: &[u8], version: &str) -> Result<Vec<Tool>> {
    #[derive(Deserialize)]
    struct Catalog {
        version: String,
        tools: Vec<Entry>,
    }
    #[derive(Deserialize)]
    struct Entry {
        id: String,
        name: String,
        tool_version: Option<String>,
        status: String,
    }
    ensure!(catalog.len() <= 2 * 1024 * 1024, "工具目录超过大小限制");
    let catalog: Catalog = serde_json::from_slice(catalog).context("工具目录格式无效")?;
    ensure!(catalog.version == version, "工具目录与程序版本不一致");
    Ok(catalog
        .tools
        .into_iter()
        .map(|t| Tool {
            id: t.id,
            name: t.name,
            version: t.tool_version,
            status: t.status,
        })
        .collect())
}
fn status(value: &str) -> &str {
    match value {
        "implemented" => "已实现",
        "in-progress" => "开发中",
        "planned" => "规划中",
        _ => "未知",
    }
}
pub fn changes(next: &[Tool]) -> Result<ToolChangeSummary> {
    let current = tools(
        include_bytes!("../../docs/tools.json"),
        env!("CARGO_PKG_VERSION"),
    )?;
    Ok(diff_tools(&current, next))
}
fn diff_tools(current: &[Tool], next: &[Tool]) -> ToolChangeSummary {
    let mut changes = ToolChangeSummary::default();
    for tool in next {
        match current.iter().find(|old| old.id == tool.id) {
            None => changes.added.push(format!(
                "新增目录项 · {} · {} · {}",
                tool.name,
                tool.version.as_deref().unwrap_or("未记录"),
                status(&tool.status)
            )),
            Some(old) if old.version != tool.version || old.status != tool.status => {
                changes.updated.push(format!(
                    "{} · {} → {} · {} → {}",
                    tool.name,
                    old.version.as_deref().unwrap_or("未记录"),
                    tool.version.as_deref().unwrap_or("未记录"),
                    status(&old.status),
                    status(&tool.status)
                ))
            }
            _ => {}
        }
    }
    for old in current {
        if !next.iter().any(|t| t.id == old.id) {
            changes.removed.push(format!(
                "目标清单移除 · {} · {}",
                old.name,
                old.version.as_deref().unwrap_or("未记录")
            ));
        }
    }
    changes
}
pub fn changes_ui(ui: &mut eframe::egui::Ui, changes: &ToolChangeSummary, max_height: f32) {
    eframe::egui::ScrollArea::vertical()
        .max_height(max_height)
        .show(ui, |ui| {
            for (label, rows) in [
                ("新增工具", &changes.added),
                ("版本或状态变化", &changes.updated),
                ("从目录移除", &changes.removed),
            ] {
                if rows.is_empty() {
                    continue;
                }
                ui.strong(format!("{label}（{}）", rows.len()));
                for row in rows {
                    ui.label(row);
                }
                ui.add_space(4.0);
            }
            if changes.is_empty() {
                ui.label("目标与当前工具版本清单一致。");
            }
        });
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Ed25519KeyPair, Manifest, TrustStore) {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let key = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let manifest = Manifest {
            schema: 1,
            version: "1.2.3".into(),
            source_commit: "a".repeat(40),
            platform: "windows-x64".into(),
            minimum_windows_major: 10,
            files: [
                "ZiDevTools-1.2.3-windows-x64.exe",
                "ZiDevTools-1.2.3-windows-x64.msi",
                "ZiDevToolsMcp-1.2.3-windows-x64.exe",
            ]
            .iter()
            .map(|n| File {
                name: (*n).into(),
                size: 3,
                sha256: "a".repeat(64),
            })
            .collect(),
            tools: vec![Tool {
                id: "calculator".into(),
                name: "计算器".into(),
                version: Some("0.1.0".into()),
                status: "in-progress".into(),
            }],
        };
        let trust = TrustStore {
            schema: 1,
            keys: vec![publisher(&key)],
        };
        (key, manifest, trust)
    }
    #[test]
    fn signature_binds_exact_bytes_and_protocol_domain() {
        let (key, manifest, trust) = fixture();
        let (raw, sig) = sign(&manifest, &key).unwrap();
        assert_eq!(
            verify(&raw, &sig, &trust).unwrap().manifest.version,
            "1.2.3"
        );
        let mut changed = raw.clone();
        changed.push(b' ');
        assert!(verify(&changed, &sig, &trust).is_err());
        let wrong = serde_json::to_vec(&Signature {
            schema: 1,
            key_id: publisher(&key).id,
            signature: STANDARD.encode(key.sign(&raw).as_ref()),
        })
        .unwrap();
        assert!(verify(&raw, &wrong, &trust).is_err());
        let (_, _, foreign) = fixture();
        assert!(verify(&raw, &sig, &foreign).is_err());
    }
    #[test]
    fn signed_but_invalid_metadata_is_rejected() {
        let (key, mut manifest, trust) = fixture();
        for change in 0..5 {
            let old = manifest.clone();
            match change {
                0 => manifest.files[0].name = "../file.exe".into(),
                1 => manifest.files[0].size = 0,
                2 => manifest.tools.push(manifest.tools[0].clone()),
                3 => manifest.platform = "linux".into(),
                _ => manifest.source_commit = "bad".into(),
            };
            assert!(sign(&manifest, &key).is_err());
            let raw = serde_json::to_vec(&manifest).unwrap();
            let sig = serde_json::to_vec(&Signature {
                schema: 1,
                key_id: publisher(&key).id,
                signature: STANDARD.encode(key.sign(&payload(&raw)).as_ref()),
            })
            .unwrap();
            assert!(verify(&raw, &sig, &trust).is_err());
            manifest = old;
        }
    }
    #[test]
    fn signature_bounds_unknown_fields_and_duplicate_trust_are_rejected() {
        let (key, manifest, mut trust) = fixture();
        let (raw, sig) = sign(&manifest, &key).unwrap();
        assert!(verify(&vec![0; MAX_MANIFEST + 1], &sig, &trust).is_err());
        assert!(verify(&raw, &vec![0; MAX_SIGNATURE + 1], &trust).is_err());
        let mut value: serde_json::Value = serde_json::from_slice(&sig).unwrap();
        value["public_key"] = "not a trust source".into();
        assert!(verify(&raw, &serde_json::to_vec(&value).unwrap(), &trust).is_err());
        trust.keys.push(trust.keys[0].clone());
        assert!(verify(&raw, &sig, &trust).is_err());
    }
    #[test]
    fn tool_catalog_versions_are_bound_to_program_version() {
        let current = tools(
            include_bytes!("../../docs/tools.json"),
            env!("CARGO_PKG_VERSION"),
        )
        .unwrap();
        assert!(changes(&current).unwrap().is_empty());
        assert!(tools(include_bytes!("../../docs/tools.json"), "0.0.0").is_err());
        let mut changed = current;
        changed[0].version = Some("99.0.0".into());
        assert!(!changes(&changed).unwrap().is_empty());
    }
    #[test]
    fn tool_changes_are_grouped_by_add_update_and_removal() {
        let current = vec![
            Tool {
                id: "calculator".into(),
                name: "计算器".into(),
                version: Some("1.0.0".into()),
                status: "implemented".into(),
            },
            Tool {
                id: "clock".into(),
                name: "时钟".into(),
                version: Some("0.2.0".into()),
                status: "in-progress".into(),
            },
            Tool {
                id: "recorder".into(),
                name: "录屏".into(),
                version: Some("1.0.0".into()),
                status: "implemented".into(),
            },
        ];
        let next = vec![
            Tool {
                id: "calculator".into(),
                name: "计算器".into(),
                version: Some("1.1.0".into()),
                status: "implemented".into(),
            },
            Tool {
                id: "clock".into(),
                name: "时钟".into(),
                version: Some("0.2.0".into()),
                status: "implemented".into(),
            },
            Tool {
                id: "calendar".into(),
                name: "万年历".into(),
                version: Some("0.1.0".into()),
                status: "in-progress".into(),
            },
        ];

        let changes = diff_tools(&current, &next);
        assert_eq!(changes.added.len(), 1);
        assert_eq!(changes.updated.len(), 2);
        assert_eq!(changes.removed.len(), 1);
        assert_eq!(changes.len(), 4);
        assert!(changes.added[0].contains("万年历"));
        assert!(
            changes
                .updated
                .iter()
                .any(|row| { row.contains("计算器") && row.contains("1.0.0 → 1.1.0") })
        );
        assert!(
            changes
                .updated
                .iter()
                .any(|row| { row.contains("时钟") && row.contains("开发中 → 已实现") })
        );
        assert!(changes.removed[0].contains("录屏"));
        assert!(!changes.is_empty());
    }
    #[test]
    fn legacy_missing_versions_remain_unknown_and_binding_rejects_wrong_release() {
        let legacy=br#"{"version":"0.81.0","tools":[{"id":"example","name":"Example","status":"planned"}]}"#;
        assert!(tools(legacy, "0.81.0").unwrap()[0].version.is_none());
        let (key, manifest, trust) = fixture();
        let (raw, sig) = sign(&manifest, &key).unwrap();
        let authenticated = verify(&raw, &sig, &trust).unwrap();
        assert!(
            expected_file(
                &authenticated,
                &semver::Version::new(1, 2, 3),
                &manifest.files[0].name,
                3
            )
            .is_ok()
        );
        assert!(
            expected_file(
                &authenticated,
                &semver::Version::new(1, 2, 4),
                &manifest.files[0].name,
                3
            )
            .is_err()
        );
        assert!(
            expected_file(
                &authenticated,
                &semver::Version::new(1, 2, 3),
                &manifest.files[0].name,
                4
            )
            .is_err()
        );
    }
    #[test]
    fn embedded_publisher_key_has_a_valid_public_fingerprint() {
        let trust: TrustStore =
            serde_json::from_str(include_str!("../../docs/update-trust.json")).unwrap();
        assert_eq!(trust.schema, 1);
        assert!(!trust.keys.is_empty());
        for key in trust.keys {
            let bytes = STANDARD.decode(key.public_key).unwrap();
            assert_eq!(bytes.len(), 32);
            assert_eq!(format!("{:x}", Sha256::digest(bytes)), key.id);
        }
    }
}
