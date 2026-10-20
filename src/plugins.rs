//! Versioned declarative extensions: no scripts, dynamic libraries or install hooks.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::signature;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    future::Future,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub const CATEGORIES: &[&str] = &[
    "数据与格式",
    "文本与编码",
    "网络与接口",
    "文件与系统",
    "时间与生成",
    "AI 与模型",
    "知识与检索",
    "扩展与集成",
    "MCP 与 Agent",
    "Java 与 JVM",
    "Python 与 Django",
];
const MANIFEST_LIMIT: usize = 256 * 1024;
pub const PLUGIN_MANIFEST_SCHEMA_VERSION: u32 = 1;
const PLUGIN_SIGNATURE_DOMAIN: &[u8] = b"ZiDevTools plugin manifest v1\0";
const CATALOG_SIGNATURE_DOMAIN: &[u8] = b"ZiDevTools plugin catalog v1\0";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TrustedPublisher {
    pub id: String,
    pub name: String,
    pub public_key: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublisherStoreFile {
    schema_version: u32,
    publishers: Vec<TrustedPublisher>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogSource {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_catalog_schema_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub last_snapshot: Vec<CatalogSnapshotEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogSnapshotEntry {
    pub plugin_id: String,
    pub name: String,
    pub version: String,
    pub publisher_id: String,
    pub entry_digest: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description_digest: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub manifest_url_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_app_version_req: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_manifest_schema_version: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogChangedField {
    Name,
    Version,
    Publisher,
    Description,
    ManifestUrl,
    HostAppVersionRequirement,
    PluginManifestSchemaVersion,
    LegacyDetails,
}

impl CatalogChangedField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "名称",
            Self::Version => "版本",
            Self::Publisher => "发布者",
            Self::Description => "说明",
            Self::ManifestUrl => "清单地址",
            Self::HostAppVersionRequirement => "宿主版本范围",
            Self::PluginManifestSchemaVersion => "清单 schema",
            Self::LegacyDetails => "其他目录声明（旧快照不含字段指纹）",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntryChange {
    pub previous: CatalogSnapshotEntry,
    pub current: CatalogSnapshotEntry,
    pub changed_fields: Vec<CatalogChangedField>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogDiff {
    pub had_previous_check: bool,
    pub schema_version_change: Option<(u32, u32)>,
    pub added: Vec<CatalogSnapshotEntry>,
    pub updated: Vec<CatalogEntryChange>,
    pub removed: Vec<CatalogSnapshotEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogSourceStoreFile {
    schema_version: u32,
    sources: Vec<CatalogSource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedManifestEnvelope {
    schema_version: u32,
    key_id: String,
    manifest: String,
    signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedManifest {
    pub manifest: Vec<u8>,
    pub publisher_id: String,
    pub publisher_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub plugin_id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub manifest_url: String,
    pub publisher_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_app_version_req: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_manifest_schema_version: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginCatalog {
    pub schema_version: u32,
    pub entries: Vec<CatalogEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedCatalog {
    pub catalog: PluginCatalog,
    pub publisher_id: String,
    pub publisher_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogCompatibility {
    LegacyUnspecified,
    Compatible,
    Incompatible(String),
}

pub fn current_host_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("Cargo package version must be valid SemVer")
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub tools: Vec<PluginTool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginToolVersionRecord {
    pub version: String,
    pub date: String,
    pub changes: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginTool {
    pub id: String,
    /// Independent tool revision; old manifests remain readable without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Append-only, publisher-declared changes for this tool's independent version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub version_history: Vec<PluginToolVersionRecord>,
    pub name: String,
    pub description: String,
    pub category: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Explicit localized or alternate names used for exact tool discovery.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub sample: String,
    #[serde(default)]
    pub model: String,
    pub adapter: Adapter,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Adapter {
    Builtin {
        tool: String,
        #[serde(default)]
        action: usize,
        #[serde(default)]
        pattern: String,
    },
    Http {
        url: String,
        method: String,
        #[serde(default)]
        body: Value,
        #[serde(default)]
        response_pointer: Option<String>,
    },
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn publisher_id(public_key: &[u8]) -> String {
    format!("{:x}", Sha256::digest(public_key))
}
fn validate_publishers(publishers: &[TrustedPublisher]) -> Result<()> {
    ensure!(publishers.len() <= 64, "最多固定 64 个插件发布者公钥");
    let mut ids = BTreeSet::new();
    for publisher in publishers {
        let public_key = STANDARD
            .decode(&publisher.public_key)
            .context("发布者公钥不是有效的 Base64")?;
        ensure!(
            public_key.len() == 32
                && publisher.id == publisher_id(&public_key)
                && ids.insert(&publisher.id),
            "发布者公钥指纹无效或重复"
        );
        ensure!(
            !publisher.name.trim().is_empty()
                && publisher.name.len() <= 80
                && !publisher.name.chars().any(char::is_control),
            "发布者名称须为 1–80 个非控制字符"
        );
    }
    Ok(())
}
fn read_publishers(path: &Path) -> Result<Option<Vec<TrustedPublisher>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 64 * 1024,
        "发布者公钥文件类型或大小无效"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 64 * 1024, "发布者公钥文件超过 64 KiB");
    let file: PublisherStoreFile = serde_json::from_slice(&bytes).context("发布者公钥文件无效")?;
    ensure!(file.schema_version == 1, "发布者公钥文件版本不受支持");
    validate_publishers(&file.publishers)?;
    Ok(Some(file.publishers))
}

fn validate_catalog_sources(sources: &[CatalogSource]) -> Result<()> {
    ensure!(sources.len() <= 32, "最多保存 32 个插件目录来源");
    let mut ids = BTreeSet::new();
    let mut urls = BTreeSet::new();
    for source in sources {
        let id = uuid::Uuid::parse_str(&source.id).context("插件目录来源 ID 无效")?;
        ensure!(
            id.to_string() == source.id && ids.insert(&source.id),
            "插件目录来源 ID 格式无效或重复"
        );
        ensure!(
            !source.name.trim().is_empty()
                && source.name.trim().len() <= 80
                && !source.name.chars().any(char::is_control),
            "插件目录来源名称须为 1–80 个非控制字符"
        );
        ensure!(source.url.len() <= 2048, "插件目录来源 URL 最多 2048 字节");
        let parsed = endpoint(&source.url).context("插件目录来源 URL 无效")?;
        ensure!(
            parsed.as_str() == source.url && urls.insert(&source.url),
            "插件目录来源 URL 必须规范且不得重复"
        );
        if let Some(publisher_id) = &source.publisher_id {
            ensure!(
                publisher_id.len() == 64
                    && publisher_id
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "插件目录来源签名者指纹无效"
            );
        }
        ensure!(
            source.last_snapshot.len() <= 256,
            "目录来源变更快照最多包含 256 项"
        );
        if let Some(version) = source.last_catalog_schema_version {
            ensure!(matches!(version, 1 | 2), "缓存目录 schema 版本无效");
        }
        if let Some(verified_at) = &source.last_verified_at {
            ensure!(
                chrono::DateTime::parse_from_rfc3339(verified_at).is_ok()
                    && source.publisher_id.is_some()
                    && source.last_catalog_schema_version.is_some(),
                "目录来源验签时间或摘要状态无效"
            );
        } else {
            ensure!(
                source.last_snapshot.is_empty() && source.last_catalog_schema_version.is_none(),
                "尚未验签的目录来源不能包含历史快照"
            );
        }
        let mut snapshot_ids = BTreeSet::new();
        for entry in &source.last_snapshot {
            ensure!(
                valid_id(&entry.plugin_id)
                    && snapshot_ids.insert(&entry.plugin_id)
                    && !entry.name.trim().is_empty()
                    && entry.name.len() <= 128
                    && valid_tool_version(&entry.version),
                "目录来源历史快照条目无效"
            );
            ensure!(
                entry.publisher_id.len() == 64
                    && entry
                        .publisher_id
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    && entry.entry_digest.len() == 64
                    && entry
                        .entry_digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "目录来源历史快照指纹无效"
            );
            for digest in [&entry.description_digest, &entry.manifest_url_digest] {
                ensure!(
                    digest.is_empty()
                        || (digest.len() == 64
                            && digest
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))),
                    "目录来源历史字段摘要无效"
                );
            }
            if let Some(requirement) = &entry.host_app_version_req {
                ensure!(
                    requirement.len() <= 256 && semver::VersionReq::parse(requirement).is_ok(),
                    "目录来源历史宿主版本范围无效"
                );
            }
            ensure!(
                entry
                    .plugin_manifest_schema_version
                    .is_none_or(|version| version > 0),
                "目录来源历史清单 schema 无效"
            );
        }
    }
    Ok(())
}

fn read_catalog_sources(path: &Path) -> Result<Option<Vec<CatalogSource>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() <= 8 * 1024 * 1024,
        "插件目录来源文件类型或大小无效"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 8 * 1024 * 1024, "插件目录来源文件超过 8 MiB");
    let file: CatalogSourceStoreFile =
        serde_json::from_slice(&bytes).context("插件目录来源文件无效")?;
    ensure!(file.schema_version == 1, "插件目录来源文件版本不受支持");
    validate_catalog_sources(&file.sources)?;
    Ok(Some(file.sources))
}
pub fn public_key_fingerprint(encoded: &str) -> Result<String> {
    let public_key = STANDARD
        .decode(encoded.trim())
        .context("发布者公钥不是有效的 Base64")?;
    ensure!(public_key.len() == 32, "Ed25519 公钥必须为 32 字节");
    Ok(publisher_id(&public_key))
}
fn verify_signed_payload(
    envelope_bytes: &[u8],
    publishers: &[TrustedPublisher],
    domain: &[u8],
    label: &str,
) -> Result<(Vec<u8>, TrustedPublisher)> {
    ensure!(
        envelope_bytes.len() <= MANIFEST_LIMIT,
        "签名内容最多 256 KiB"
    );
    validate_publishers(publishers)?;
    let envelope: SignedManifestEnvelope =
        serde_json::from_slice(envelope_bytes).context("签名清单信封格式无效")?;
    ensure!(envelope.schema_version == 1, "不支持的签名清单协议版本");
    let publisher = publishers
        .iter()
        .find(|publisher| publisher.id == envelope.key_id)
        .context("签名使用未固定的发布者公钥")?;
    let manifest = STANDARD
        .decode(&envelope.manifest)
        .context("签名内容 Base64 编码无效")?;
    ensure!(manifest.len() <= MANIFEST_LIMIT, "插件清单最多 256 KiB");
    let signature = STANDARD
        .decode(&envelope.signature)
        .context("Ed25519 签名 Base64 编码无效")?;
    ensure!(signature.len() == 64, "Ed25519 签名长度无效");
    let public_key = STANDARD
        .decode(&publisher.public_key)
        .context("已固定的发布者公钥格式无效")?;
    signature::UnparsedPublicKey::new(&signature::ED25519, &public_key)
        .verify(&[domain, &manifest].concat(), &signature)
        .map_err(|_| anyhow::anyhow!("{label}签名验证失败；内容未受信任"))?;
    Ok((manifest, publisher.clone()))
}
pub fn verify_signed_manifest(
    envelope_bytes: &[u8],
    publishers: &[TrustedPublisher],
) -> Result<AuthenticatedManifest> {
    let (manifest, publisher) = verify_signed_payload(
        envelope_bytes,
        publishers,
        PLUGIN_SIGNATURE_DOMAIN,
        "插件发布者",
    )?;
    parse(&manifest)?;
    Ok(AuthenticatedManifest {
        manifest,
        publisher_id: publisher.id,
        publisher_name: publisher.name,
    })
}
fn validate_catalog(catalog: &PluginCatalog) -> Result<()> {
    ensure!(
        matches!(catalog.schema_version, 1 | 2),
        "不支持的插件目录版本"
    );
    ensure!(catalog.entries.len() <= 256, "插件目录最多包含 256 项");
    let mut ids = BTreeSet::new();
    for entry in &catalog.entries {
        ensure!(
            valid_id(&entry.plugin_id)
                && entry.plugin_id != "enabled"
                && ids.insert(&entry.plugin_id),
            "插件目录包含无效或重复的插件 ID"
        );
        ensure!(
            !entry.name.trim().is_empty()
                && entry.name.len() <= 128
                && entry.description.len() <= 512
                && !entry.name.chars().any(char::is_control)
                && !entry.description.chars().any(char::is_control),
            "插件目录名称或说明长度无效"
        );
        ensure!(
            valid_tool_version(&entry.version),
            "插件目录版本须为 major.minor.patch"
        );
        ensure!(
            entry.publisher_id.len() == 64
                && entry
                    .publisher_id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "插件目录发布者指纹无效"
        );
        endpoint(&entry.manifest_url).context("插件目录清单 URL 无效")?;
        match catalog.schema_version {
            1 => ensure!(
                entry.host_app_version_req.is_none()
                    && entry.plugin_manifest_schema_version.is_none(),
                "目录 schema 1 不支持兼容性字段；请升级到 schema 2"
            ),
            2 => {
                let requirement = entry
                    .host_app_version_req
                    .as_deref()
                    .context("目录 schema 2 必须声明宿主版本范围")?;
                ensure!(
                    requirement.len() <= 256 && semver::VersionReq::parse(requirement).is_ok(),
                    "插件目录宿主版本范围不是有效的 SemVer 约束"
                );
                ensure!(
                    entry.plugin_manifest_schema_version.is_some(),
                    "目录 schema 2 必须声明插件清单 schema 版本"
                );
                ensure!(
                    entry
                        .plugin_manifest_schema_version
                        .is_some_and(|schema| schema > 0),
                    "插件清单 schema 版本必须大于 0"
                );
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

pub fn catalog_compatibility(
    entry: &CatalogEntry,
    app_version: &semver::Version,
    supported_manifest_schema: u32,
) -> CatalogCompatibility {
    match (
        entry.host_app_version_req.as_deref(),
        entry.plugin_manifest_schema_version,
    ) {
        (None, None) => CatalogCompatibility::LegacyUnspecified,
        (Some(requirement), Some(schema)) => {
            let Ok(requirement) = semver::VersionReq::parse(requirement) else {
                return CatalogCompatibility::Incompatible("目录宿主版本约束无效".into());
            };
            if !requirement.matches(app_version) {
                return CatalogCompatibility::Incompatible(format!(
                    "仅支持宿主版本 {requirement}，当前为 {app_version}"
                ));
            }
            if schema != supported_manifest_schema {
                return CatalogCompatibility::Incompatible(format!(
                    "需要插件清单 schema {schema}，当前支持 {supported_manifest_schema}"
                ));
            }
            CatalogCompatibility::Compatible
        }
        _ => CatalogCompatibility::Incompatible("目录兼容性声明不完整".into()),
    }
}

fn catalog_snapshot(catalog: &PluginCatalog) -> Result<Vec<CatalogSnapshotEntry>> {
    let mut snapshot = catalog
        .entries
        .iter()
        .map(|entry| {
            let encoded = serde_json::to_vec(entry)?;
            Ok(CatalogSnapshotEntry {
                plugin_id: entry.plugin_id.clone(),
                name: entry.name.clone(),
                version: entry.version.clone(),
                publisher_id: entry.publisher_id.clone(),
                entry_digest: format!("{:x}", Sha256::digest(encoded)),
                description_digest: format!("{:x}", Sha256::digest(entry.description.as_bytes())),
                manifest_url_digest: format!("{:x}", Sha256::digest(entry.manifest_url.as_bytes())),
                host_app_version_req: entry.host_app_version_req.clone(),
                plugin_manifest_schema_version: entry.plugin_manifest_schema_version,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    snapshot.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
    Ok(snapshot)
}

fn diff_catalog_snapshots(
    had_previous_check: bool,
    previous_schema: Option<u32>,
    previous: &[CatalogSnapshotEntry],
    current_schema: u32,
    current: &[CatalogSnapshotEntry],
) -> CatalogDiff {
    let old_by_id: BTreeMap<_, _> = previous
        .iter()
        .map(|entry| (entry.plugin_id.as_str(), entry))
        .collect();
    let new_by_id: BTreeMap<_, _> = current
        .iter()
        .map(|entry| (entry.plugin_id.as_str(), entry))
        .collect();
    let mut diff = CatalogDiff {
        had_previous_check,
        schema_version_change: had_previous_check
            .then(|| previous_schema.map(|old| (old, current_schema)))
            .flatten()
            .filter(|(old, new)| old != new),
        ..Default::default()
    };
    for entry in current {
        match old_by_id.get(entry.plugin_id.as_str()) {
            Some(previous) if previous.entry_digest != entry.entry_digest => {
                let previous = *previous;
                let mut changed_fields = Vec::new();
                if previous.name != entry.name {
                    changed_fields.push(CatalogChangedField::Name);
                }
                if previous.version != entry.version {
                    changed_fields.push(CatalogChangedField::Version);
                }
                if previous.publisher_id != entry.publisher_id {
                    changed_fields.push(CatalogChangedField::Publisher);
                }
                if previous.description_digest != entry.description_digest {
                    changed_fields.push(if previous.description_digest.is_empty() {
                        CatalogChangedField::LegacyDetails
                    } else {
                        CatalogChangedField::Description
                    });
                }
                if previous.manifest_url_digest != entry.manifest_url_digest {
                    changed_fields.push(if previous.manifest_url_digest.is_empty() {
                        CatalogChangedField::LegacyDetails
                    } else {
                        CatalogChangedField::ManifestUrl
                    });
                }
                if previous.host_app_version_req != entry.host_app_version_req {
                    changed_fields.push(CatalogChangedField::HostAppVersionRequirement);
                }
                if previous.plugin_manifest_schema_version != entry.plugin_manifest_schema_version {
                    changed_fields.push(CatalogChangedField::PluginManifestSchemaVersion);
                }
                changed_fields.dedup();
                if changed_fields.is_empty() {
                    changed_fields.push(CatalogChangedField::LegacyDetails);
                }
                diff.updated.push(CatalogEntryChange {
                    previous: previous.clone(),
                    current: entry.clone(),
                    changed_fields,
                });
            }
            None => diff.added.push(entry.clone()),
            _ => {}
        }
    }
    for entry in previous {
        if !new_by_id.contains_key(entry.plugin_id.as_str()) {
            diff.removed.push(entry.clone());
        }
    }
    diff
}

pub fn verify_signed_catalog(
    envelope_bytes: &[u8],
    publishers: &[TrustedPublisher],
) -> Result<AuthenticatedCatalog> {
    let (bytes, publisher) = verify_signed_payload(
        envelope_bytes,
        publishers,
        CATALOG_SIGNATURE_DOMAIN,
        "插件目录发布者",
    )?;
    let catalog: PluginCatalog = serde_json::from_slice(&bytes).context("插件目录 JSON 无效")?;
    validate_catalog(&catalog)?;
    Ok(AuthenticatedCatalog {
        catalog,
        publisher_id: publisher.id,
        publisher_name: publisher.name,
    })
}
pub fn generate_publisher_keypair(name: &str) -> Result<(Vec<u8>, TrustedPublisher)> {
    use ring::signature::{Ed25519KeyPair, KeyPair};
    ensure!(
        !name.trim().is_empty() && name.trim().len() <= 80 && !name.chars().any(char::is_control),
        "发布者名称须为 1–80 个非控制字符"
    );
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
        .map_err(|_| anyhow::anyhow!("无法生成 Ed25519 发布者密钥"))?;
    let key = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .map_err(|_| anyhow::anyhow!("生成的 Ed25519 发布者密钥无效"))?;
    let public_key = key.public_key().as_ref();
    let publisher = TrustedPublisher {
        id: publisher_id(public_key),
        name: name.trim().to_owned(),
        public_key: STANDARD.encode(public_key),
    };
    Ok((pkcs8.as_ref().to_vec(), publisher))
}
pub fn sign_manifest_with_pkcs8(manifest: &[u8], pkcs8: &[u8]) -> Result<Vec<u8>> {
    ensure!(manifest.len() <= MANIFEST_LIMIT, "插件清单最多 256 KiB");
    parse(manifest).context("只能签署有效的插件 JSON 清单")?;
    sign_payload_with_pkcs8(manifest, pkcs8, PLUGIN_SIGNATURE_DOMAIN)
}
pub fn sign_catalog_with_pkcs8(catalog: &[u8], pkcs8: &[u8]) -> Result<Vec<u8>> {
    ensure!(catalog.len() <= MANIFEST_LIMIT, "插件目录最多 256 KiB");
    let catalog_value: PluginCatalog =
        serde_json::from_slice(catalog).context("插件目录 JSON 无效")?;
    validate_catalog(&catalog_value)?;
    sign_payload_with_pkcs8(catalog, pkcs8, CATALOG_SIGNATURE_DOMAIN)
}
fn sign_payload_with_pkcs8(payload: &[u8], pkcs8: &[u8], domain: &[u8]) -> Result<Vec<u8>> {
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let key =
        Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|_| anyhow::anyhow!("Ed25519 私钥文件无效"))?;
    let key_id = publisher_id(key.public_key().as_ref());
    let signature = key.sign(&[domain, payload].concat());
    let envelope = serde_json::to_vec_pretty(&serde_json::json!({
        "schema_version": 1,
        "key_id": key_id,
        "manifest": STANDARD.encode(payload),
        "signature": STANDARD.encode(signature.as_ref()),
    }))?;
    ensure!(
        envelope.len() <= MANIFEST_LIMIT,
        "签名信封超过 256 KiB，请缩小插件清单"
    );
    Ok(envelope)
}
#[cfg(test)]
pub(crate) fn signed_test_envelope(
    manifest: &[u8],
    publisher_name: &str,
) -> (Vec<u8>, TrustedPublisher) {
    signed_test_payload(manifest, publisher_name, PLUGIN_SIGNATURE_DOMAIN)
}
#[cfg(test)]
pub(crate) fn signed_test_catalog_envelope(
    catalog: &[u8],
    publisher_name: &str,
) -> (Vec<u8>, TrustedPublisher) {
    signed_test_payload(catalog, publisher_name, CATALOG_SIGNATURE_DOMAIN)
}
#[cfg(test)]
fn signed_test_payload(
    payload: &[u8],
    publisher_name: &str,
    domain: &[u8],
) -> (Vec<u8>, TrustedPublisher) {
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let key = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let public_key = key.public_key().as_ref();
    let id = publisher_id(public_key);
    let publisher = TrustedPublisher {
        id: id.clone(),
        name: publisher_name.into(),
        public_key: STANDARD.encode(public_key),
    };
    let signature = key.sign(&[domain, payload].concat());
    let envelope = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "key_id": id,
        "manifest": STANDARD.encode(payload),
        "signature": STANDARD.encode(signature.as_ref()),
    }))
    .unwrap();
    (envelope, publisher)
}
pub fn endpoint(url: &str) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(url).context("接口 URL 无效")?;
    ensure!(
        u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none(),
        "接口地址不能包含账号、密码、查询参数或片段；令牌在运行页单独输入"
    );
    let local = matches!(
        u.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    ensure!(
        u.scheme() == "https" || (u.scheme() == "http" && local),
        "远程接口必须为 HTTPS；HTTP 仅允许 localhost/127.0.0.1/::1"
    );
    Ok(u)
}

fn validate_plugin_tool_history(tool: &PluginTool) -> Result<()> {
    ensure!(
        tool.version_history.len() <= 64,
        "每个插件工具最多保留 64 条版本记录"
    );
    if tool.version_history.is_empty() {
        return Ok(());
    }
    let current = tool
        .version
        .as_deref()
        .context("插件工具声明版本历史时必须声明当前工具版本")?;
    let mut previous_version: Option<&str> = None;
    let mut previous_date: Option<chrono::NaiveDate> = None;
    for record in &tool.version_history {
        ensure!(
            valid_tool_version(&record.version),
            "插件工具历史版本需为 major.minor.patch"
        );
        let date = chrono::NaiveDate::parse_from_str(&record.date, "%Y-%m-%d")
            .context("插件工具版本历史日期须为 YYYY-MM-DD")?;
        ensure!(
            date.format("%Y-%m-%d").to_string() == record.date,
            "插件工具版本历史日期格式无效"
        );
        ensure!(
            !record.changes.trim().is_empty()
                && record.changes.len() <= 512
                && !record.changes.chars().any(char::is_control),
            "插件工具版本变更说明须为 1–512 字节且不含控制字符"
        );
        if let Some(previous) = previous_version {
            ensure!(
                compare_versions(&record.version, previous)? == std::cmp::Ordering::Greater,
                "插件工具版本历史必须严格递增"
            );
        }
        ensure!(
            previous_date.is_none_or(|previous| date >= previous),
            "插件工具版本历史日期必须按时间递增"
        );
        previous_version = Some(&record.version);
        previous_date = Some(date);
    }
    ensure!(
        tool.version_history
            .last()
            .map(|record| record.version.as_str())
            == Some(current),
        "插件工具当前版本必须与最后一条历史记录一致"
    );
    Ok(())
}

pub fn parse(bytes: &[u8]) -> Result<Manifest> {
    ensure!(bytes.len() <= MANIFEST_LIMIT, "插件清单最多 256 KiB");
    let m: Manifest = serde_json::from_slice(bytes).context("插件 JSON 格式或字段无效")?;
    ensure!(
        m.schema_version == PLUGIN_MANIFEST_SCHEMA_VERSION,
        "不支持的插件协议版本"
    );
    ensure!(
        valid_id(&m.id) && m.id != "enabled",
        "插件 ID 仅允许小写字母、数字和连字符，长度 1–64"
    );
    ensure!(
        !m.name.trim().is_empty()
            && m.name.len() <= 128
            && !m.version.is_empty()
            && m.version.len() <= 40
            && m.description.len() <= 1024,
        "插件名称、版本或说明长度无效"
    );
    ensure!(
        !m.tools.is_empty() && m.tools.len() <= 64,
        "每个插件需包含 1–64 个工具"
    );
    let mut ids = BTreeSet::new();
    for t in &m.tools {
        ensure!(valid_id(&t.id) && ids.insert(&t.id), "工具 ID 无效或重复");
        ensure!(
            t.version.as_deref().is_none_or(valid_tool_version),
            "工具版本需为 major.minor.patch，不能有前导零"
        );
        validate_plugin_tool_history(t)?;
        ensure!(
            !t.name.trim().is_empty()
                && t.name.len() <= 128
                && t.description.len() <= 1024
                && t.sample.len() <= 16384
                && t.model.len() <= 256,
            "工具文本过长或名称为空"
        );
        ensure!(
            CATEGORIES.contains(&t.category.as_str()),
            "工具分类不在受支持分类中"
        );
        ensure!(
            t.keywords.len() <= 32 && t.keywords.iter().all(|k| k.len() <= 80),
            "搜索关键词过多或过长"
        );
        ensure!(
            t.aliases.len() <= 16
                && t.aliases.iter().all(|alias| {
                    !alias.trim().is_empty()
                        && alias.len() <= 128
                        && !alias.chars().any(char::is_control)
                }),
            "工具别名最多 16 项，每项需为 1–128 字节的非控制文本"
        );
        match &t.adapter {
            Adapter::Builtin {
                tool,
                action,
                pattern,
            } => {
                let kind = crate::tools::ToolKind::ALL
                    .into_iter()
                    .find(|k| k.id() == tool)
                    .context("插件引用了未知内置工具")?;
                ensure!(
                    *action < kind.actions().len() && pattern.len() <= 4096,
                    "内置操作或参数无效"
                );
            }
            Adapter::Http {
                url,
                method,
                response_pointer,
                ..
            } => {
                endpoint(url)?;
                ensure!(
                    matches!(method.as_str(), "GET" | "POST"),
                    "仅支持 GET / POST"
                );
                ensure!(
                    response_pointer
                        .as_ref()
                        .is_none_or(|s| s.is_empty() || s.starts_with('/')),
                    "结果路径必须为 JSON Pointer"
                );
            }
        }
    }
    Ok(m)
}

pub(crate) fn valid_tool_version(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && (part.len() == 1 || !part.starts_with('0'))
                && part.bytes().all(|c| c.is_ascii_digit())
                && part.parse::<u32>().is_ok()
        })
}

fn compare_versions(left: &str, right: &str) -> Result<std::cmp::Ordering> {
    fn parts(value: &str) -> Result<[u32; 3]> {
        ensure!(valid_tool_version(value), "插件版本需为 major.minor.patch");
        let mut values = value.split('.').map(str::parse::<u32>);
        Ok([
            values.next().context("插件版本无效")??,
            values.next().context("插件版本无效")??,
            values.next().context("插件版本无效")??,
        ])
    }
    Ok(parts(left)?.cmp(&parts(right)?))
}

fn permission_summary(adapter: &Adapter) -> String {
    match adapter {
        Adapter::Builtin { tool, action, .. } => format!("内置动作 {tool}#{action}"),
        Adapter::Http { url, method, .. } => format!("HTTP {method} {url}"),
    }
}

fn replace_file(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context("插件目录无效")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".zi-plugin-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        {
            use windows::{
                Win32::Storage::FileSystem::{
                    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
                },
                core::HSTRING,
            };
            unsafe {
                MoveFileExW(
                    &HSTRING::from(temporary.as_os_str()),
                    &HSTRING::from(path.as_os_str()),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            }
            .context("无法原子替换插件文件")?;
        }
        #[cfg(not(windows))]
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdatePlan {
    pub id: String,
    pub current_version: String,
    pub candidate_version: String,
    pub added_tools: Vec<String>,
    pub removed_tools: Vec<String>,
    pub changed_tools: Vec<String>,
    pub tool_version_changes: Vec<PluginToolVersionChange>,
    pub permission_changes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginToolVersionChange {
    pub tool_id: String,
    pub name: String,
    pub previous_version: Option<String>,
    pub candidate_version: Option<String>,
    pub added_history: Vec<PluginToolVersionRecord>,
    pub history_changed: bool,
}

#[derive(Clone)]
pub struct Installed {
    pub manifest: Manifest,
    pub digest: String,
    pub enabled: bool,
}
pub struct Store {
    pub root: PathBuf,
    pub packages: Vec<Installed>,
    tool_search_key: Vec<(String, String, bool)>,
    pub errors: Vec<String>,
    pub trusted_publishers: Vec<TrustedPublisher>,
    pub catalog_sources: Vec<CatalogSource>,
    enabled: BTreeMap<String, String>,
}
impl Store {
    pub fn load(root: PathBuf) -> Self {
        let enabled: BTreeMap<String, String> = fs::read(root.join("enabled.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let (trusted_publishers, trust_error) =
            match read_publishers(&root.join("trusted-publishers.json")) {
                Ok(publishers) => (publishers.unwrap_or_default(), None),
                Err(error) => (Vec::new(), Some(error.to_string())),
            };
        let (catalog_sources, source_error) =
            match read_catalog_sources(&root.join("catalog-sources.json")) {
                Ok(sources) => (sources.unwrap_or_default(), None),
                Err(error) => (Vec::new(), Some(error.to_string())),
            };
        let mut store = Self {
            root,
            packages: Vec::new(),
            tool_search_key: Vec::new(),
            errors: trust_error.into_iter().chain(source_error).collect(),
            trusted_publishers,
            catalog_sources,
            enabled,
        };
        if let Ok(entries) = fs::read_dir(&store.root) {
            for entry in entries.flatten().take(256) {
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "json")
                    || path.file_name().is_some_and(|n| {
                        n == "enabled.json"
                            || n == "trusted-publishers.json"
                            || n == "catalog-sources.json"
                    })
                {
                    continue;
                }
                let read = (|| -> Result<Installed> {
                    let bytes = read_manifest(&path)?;
                    let manifest = parse(&bytes)?;
                    ensure!(
                        path.file_stem().and_then(|s| s.to_str()) == Some(&manifest.id),
                        "插件文件名与 ID 不一致"
                    );
                    let digest = format!("{:x}", Sha256::digest(&bytes));
                    let enabled = store.enabled.get(&manifest.id) == Some(&digest);
                    Ok(Installed {
                        manifest,
                        digest,
                        enabled,
                    })
                })();
                match read {
                    Ok(p) => store.packages.push(p),
                    Err(e) => store.errors.push(format!(
                        "{}：{e}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    )),
                }
            }
        }
        store
            .packages
            .sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        store.tool_search_key = store
            .packages
            .iter()
            .map(|package| {
                (
                    package.manifest.id.clone(),
                    package.digest.clone(),
                    package.enabled,
                )
            })
            .collect();
        store
    }
    pub fn pin_publisher(&mut self, name: &str, encoded_public_key: &str) -> Result<String> {
        let public_key = STANDARD
            .decode(encoded_public_key.trim())
            .context("发布者公钥不是有效的 Base64")?;
        ensure!(public_key.len() == 32, "Ed25519 公钥必须为 32 字节");
        ensure!(
            !name.trim().is_empty()
                && name.trim().len() <= 80
                && !name.chars().any(char::is_control),
            "发布者名称须为 1–80 个非控制字符"
        );
        let id = publisher_id(&public_key);
        let mut publishers = self.trusted_publishers.clone();
        if let Some(existing) = publishers.iter_mut().find(|publisher| publisher.id == id) {
            existing.name = name.trim().to_owned();
        } else {
            ensure!(publishers.len() < 64, "最多固定 64 个插件发布者公钥");
            publishers.push(TrustedPublisher {
                id: id.clone(),
                name: name.trim().to_owned(),
                public_key: STANDARD.encode(public_key),
            });
        }
        validate_publishers(&publishers)?;
        self.save_publishers(publishers)?;
        Ok(id)
    }
    pub fn remove_publisher(&mut self, id: &str) -> Result<()> {
        let mut publishers = self.trusted_publishers.clone();
        let old_len = publishers.len();
        publishers.retain(|publisher| publisher.id != id);
        ensure!(publishers.len() != old_len, "找不到此发布者公钥");
        self.save_publishers(publishers)
    }
    pub fn add_catalog_source(&mut self, name: &str, url: &str) -> Result<String> {
        let name = name.trim();
        ensure!(
            !name.is_empty() && name.len() <= 80 && !name.chars().any(char::is_control),
            "插件目录来源名称须为 1–80 个非控制字符"
        );
        let url = endpoint(url.trim())?.to_string();
        ensure!(url.len() <= 2048, "插件目录来源 URL 最多 2048 字节");
        let mut sources = self.catalog_sources.clone();
        ensure!(
            !sources.iter().any(|source| source.url == url),
            "此插件目录来源已保存"
        );
        ensure!(sources.len() < 32, "最多保存 32 个插件目录来源");
        let id = uuid::Uuid::new_v4().to_string();
        sources.push(CatalogSource {
            id: id.clone(),
            name: name.to_owned(),
            url,
            publisher_id: None,
            last_verified_at: None,
            last_catalog_schema_version: None,
            last_snapshot: Vec::new(),
        });
        self.save_catalog_sources(sources)?;
        Ok(id)
    }
    pub fn remove_catalog_source(&mut self, id: &str) -> Result<()> {
        let mut sources = self.catalog_sources.clone();
        let old_len = sources.len();
        sources.retain(|source| source.id != id);
        ensure!(sources.len() != old_len, "找不到此插件目录来源");
        self.save_catalog_sources(sources)
    }
    pub fn bind_catalog_source_publisher(&mut self, id: &str, publisher_id: &str) -> Result<()> {
        ensure!(
            publisher_id.len() == 64
                && publisher_id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "插件目录来源签名者指纹无效"
        );
        ensure!(
            self.trusted_publishers
                .iter()
                .any(|publisher| publisher.id == publisher_id),
            "插件目录签名者尚未固定"
        );
        let mut sources = self.catalog_sources.clone();
        let source = sources
            .iter_mut()
            .find(|source| source.id == id)
            .context("找不到此插件目录来源")?;
        ensure!(
            source
                .publisher_id
                .as_deref()
                .is_none_or(|known| known == publisher_id),
            "此目录来源已绑定其他签名者；请先独立核对并重新建立来源"
        );
        source.publisher_id = Some(publisher_id.to_owned());
        self.save_catalog_sources(sources)
    }
    pub fn record_verified_catalog(
        &mut self,
        id: &str,
        catalog: &AuthenticatedCatalog,
    ) -> Result<CatalogDiff> {
        validate_catalog(&catalog.catalog)?;
        ensure!(
            self.trusted_publishers
                .iter()
                .any(|publisher| publisher.id == catalog.publisher_id),
            "插件目录签名者尚未固定"
        );
        let next_snapshot = catalog_snapshot(&catalog.catalog)?;
        let mut sources = self.catalog_sources.clone();
        let source = sources
            .iter_mut()
            .find(|source| source.id == id)
            .context("找不到此插件目录来源")?;
        ensure!(
            source
                .publisher_id
                .as_deref()
                .is_none_or(|known| known == catalog.publisher_id),
            "此目录来源已绑定其他签名者；拒绝更新历史"
        );
        let mut diff = diff_catalog_snapshots(
            source.last_verified_at.is_some(),
            source.last_catalog_schema_version,
            source.last_snapshot.as_slice(),
            catalog.catalog.schema_version,
            &next_snapshot,
        );
        source.publisher_id = Some(catalog.publisher_id.clone());
        source.last_verified_at =
            Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        source.last_catalog_schema_version = Some(catalog.catalog.schema_version);
        source.last_snapshot = next_snapshot;
        self.save_catalog_sources(sources)?;
        diff.added.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
        diff.updated
            .sort_by(|a, b| a.current.plugin_id.cmp(&b.current.plugin_id));
        diff.removed.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
        Ok(diff)
    }
    fn save_catalog_sources(&mut self, sources: Vec<CatalogSource>) -> Result<()> {
        validate_catalog_sources(&sources)?;
        let file = CatalogSourceStoreFile {
            schema_version: 1,
            sources: sources.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&file)?;
        ensure!(
            bytes.len() <= 8 * 1024 * 1024,
            "插件目录来源和变更历史超过 8 MiB"
        );
        replace_file(&self.root.join("catalog-sources.json"), &bytes)?;
        self.catalog_sources = sources;
        Ok(())
    }
    fn save_publishers(&mut self, publishers: Vec<TrustedPublisher>) -> Result<()> {
        validate_publishers(&publishers)?;
        let file = PublisherStoreFile {
            schema_version: 1,
            publishers: publishers.clone(),
        };
        replace_file(
            &self.root.join("trusted-publishers.json"),
            &serde_json::to_vec_pretty(&file)?,
        )?;
        self.trusted_publishers = publishers;
        Ok(())
    }
    pub fn install(&mut self, bytes: &[u8]) -> Result<()> {
        let m = parse(bytes)?;
        ensure!(self.packages.len() < 128, "最多安装 128 个插件");
        fs::create_dir_all(&self.root)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.root.join(format!("{}.json", m.id)))
            .context("同 ID 插件已存在；请先卸载旧版")?;
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all()?;
        *self = Self::load(self.root.clone());
        Ok(())
    }
    pub fn update_plan(&self, bytes: &[u8]) -> Result<UpdatePlan> {
        let candidate = parse(bytes)?;
        let current = self
            .packages
            .iter()
            .find(|p| p.manifest.id == candidate.id)
            .context("没有同 ID 的已安装插件可升级")?;
        ensure!(
            compare_versions(&candidate.version, &current.manifest.version)?
                == std::cmp::Ordering::Greater,
            "候选版本必须高于已安装版本；回滚请使用已保存的上一版本"
        );
        let old: BTreeMap<_, _> = current
            .manifest
            .tools
            .iter()
            .map(|tool| (tool.id.as_str(), tool))
            .collect();
        let new: BTreeMap<_, _> = candidate
            .tools
            .iter()
            .map(|tool| (tool.id.as_str(), tool))
            .collect();
        let added_tools = new
            .keys()
            .filter(|id| !old.contains_key(**id))
            .map(|id| (*id).to_owned())
            .collect();
        let removed_tools = old
            .keys()
            .filter(|id| !new.contains_key(**id))
            .map(|id| (*id).to_owned())
            .collect();
        let mut changed_tools = Vec::new();
        let mut tool_version_changes = Vec::new();
        let mut permission_changes = Vec::new();
        for (id, new_tool) in &new {
            let Some(old_tool) = old.get(id) else {
                continue;
            };
            let history_changed = old_tool.version_history != new_tool.version_history;
            if old_tool.version != new_tool.version || history_changed {
                tool_version_changes.push(PluginToolVersionChange {
                    tool_id: (*id).to_owned(),
                    name: new_tool.name.clone(),
                    previous_version: old_tool.version.clone(),
                    candidate_version: new_tool.version.clone(),
                    added_history: new_tool
                        .version_history
                        .iter()
                        .filter(|record| !old_tool.version_history.contains(record))
                        .cloned()
                        .collect(),
                    history_changed,
                });
            }
            if serde_json::to_value(old_tool)? != serde_json::to_value(new_tool)? {
                changed_tools.push((*id).to_owned());
            }
            if serde_json::to_value(&old_tool.adapter)? != serde_json::to_value(&new_tool.adapter)?
            {
                permission_changes.push(format!(
                    "{id}: {} → {}",
                    permission_summary(&old_tool.adapter),
                    permission_summary(&new_tool.adapter)
                ));
            }
        }
        Ok(UpdatePlan {
            id: candidate.id,
            current_version: current.manifest.version.clone(),
            candidate_version: candidate.version,
            added_tools,
            removed_tools,
            changed_tools,
            tool_version_changes,
            permission_changes,
        })
    }
    pub fn update(&mut self, bytes: &[u8]) -> Result<()> {
        let candidate = parse(bytes)?;
        let plan = self.update_plan(bytes)?;
        let package_path = self.root.join(format!("{}.json", plan.id));
        let current_bytes = read_manifest(&package_path).context("无法读取当前插件清单")?;
        let current_digest = format!("{:x}", Sha256::digest(&current_bytes));
        let installed = self
            .packages
            .iter()
            .find(|p| p.manifest.id == plan.id)
            .context("插件已被移除，请刷新")?;
        ensure!(
            current_digest == installed.digest,
            "已安装清单在预览后发生变化，请刷新并重新审核"
        );
        self.set_enabled(&plan.id, false)?;
        let rollback_path = self.root.join("rollback").join(format!("{}.json", plan.id));
        replace_file(&rollback_path, &current_bytes).context("保存上一版本失败，插件未更新")?;
        replace_file(&package_path, bytes).context("插件更新失败；原版本仍保留在回滚记录中")?;
        *self = Self::load(self.root.clone());
        ensure!(
            self.packages
                .iter()
                .any(|package| package.manifest.id == candidate.id
                    && package.manifest.version == candidate.version
                    && !package.enabled),
            "更新后插件未能以停用状态重新载入，请检查清单"
        );
        Ok(())
    }
    pub fn rollback_version(&mut self, id: &str) -> Result<()> {
        ensure!(valid_id(id), "插件 ID 无效");
        let current = self
            .packages
            .iter()
            .find(|package| package.manifest.id == id)
            .context("插件不存在")?;
        let package_path = self.root.join(format!("{id}.json"));
        let current_bytes = read_manifest(&package_path)?;
        ensure!(
            format!("{:x}", Sha256::digest(&current_bytes)) == current.digest,
            "当前清单已改变，请刷新后重试"
        );
        let rollback_path = self.root.join("rollback").join(format!("{id}.json"));
        let rollback_bytes = read_manifest(&rollback_path).context("没有可用的上一版本")?;
        let rollback_manifest = parse(&rollback_bytes)?;
        ensure!(rollback_manifest.id == id, "回滚清单 ID 不匹配");
        self.set_enabled(id, false)?;
        replace_file(&rollback_path, &current_bytes).context("保存当前版本失败，未执行回滚")?;
        if let Err(error) = replace_file(&package_path, &rollback_bytes) {
            let _ = replace_file(&rollback_path, &rollback_bytes);
            return Err(error).context("插件回滚失败；当前版本保持不变");
        }
        *self = Self::load(self.root.clone());
        ensure!(
            self.packages.iter().any(|package| package.manifest.id == id
                && package.manifest.version == rollback_manifest.version
                && !package.enabled),
            "回滚后插件未能以停用状态重新载入，请检查清单"
        );
        Ok(())
    }
    pub fn has_rollback(&self, id: &str) -> bool {
        self.available_rollback_version(id).is_some()
    }
    pub fn available_rollback_version(&self, id: &str) -> Option<String> {
        valid_id(id)
            .then(|| read_manifest(&self.root.join("rollback").join(format!("{id}.json"))))
            .and_then(Result::ok)
            .and_then(|bytes| parse(&bytes).ok())
            .filter(|manifest| manifest.id == id)
            .map(|manifest| manifest.version)
    }
    pub fn set_enabled(&mut self, id: &str, on: bool) -> Result<()> {
        let package = self
            .packages
            .iter()
            .find(|p| p.manifest.id == id)
            .context("插件不存在")?;
        let mut next = self.enabled.clone();
        if on {
            next.insert(id.into(), package.digest.clone());
        } else {
            next.remove(id);
        }
        replace_file(
            &self.root.join("enabled.json"),
            &serde_json::to_vec_pretty(&next)?,
        )?;
        *self = Self::load(self.root.clone());
        Ok(())
    }
    pub fn uninstall(&mut self, id: &str) -> Result<()> {
        ensure!(valid_id(id), "插件 ID 无效");
        self.set_enabled(id, false)?;
        fs::remove_file(self.root.join(format!("{id}.json")))?;
        *self = Self::load(self.root.clone());
        Ok(())
    }
    pub fn tool_refs(&self) -> impl Iterator<Item = (String, &PluginTool)> {
        self.packages.iter().filter(|p| p.enabled).flat_map(|p| {
            p.manifest
                .tools
                .iter()
                .map(|t| (format!("plugin:{}/{}", p.manifest.id, t.id), t))
        })
    }
    /// Exact invalidation key for plugin-derived search entries.
    pub fn tool_search_key(&self) -> &[(String, String, bool)] {
        &self.tool_search_key
    }
}
pub fn read_manifest(path: &Path) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= MANIFEST_LIMIT as u64,
        "请选择最多 256 KiB 的普通 JSON 文件"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MANIFEST_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MANIFEST_LIMIT, "插件清单最多 256 KiB");
    Ok(bytes)
}

/// Fetches a user-selected signed plugin manifest; this does not install or enable it.
pub fn fetch_manifest_cancellable(
    url: &str,
    publishers: &[TrustedPublisher],
    cancelled: &AtomicBool,
) -> Result<AuthenticatedManifest> {
    let bytes = fetch_signed_envelope_cancellable(url, cancelled)?;
    verify_signed_manifest(&bytes, publishers).context("远程内容未通过发布者签名验证")
}

/// Fetches a user-selected signed plugin catalog; catalog entries are still only candidates.
pub fn fetch_catalog_cancellable(
    url: &str,
    publishers: &[TrustedPublisher],
    cancelled: &AtomicBool,
) -> Result<AuthenticatedCatalog> {
    let bytes = fetch_signed_envelope_cancellable(url, cancelled)?;
    verify_signed_catalog(&bytes, publishers).context("远程内容未通过插件目录签名验证")
}

fn fetch_signed_envelope_cancellable(url: &str, cancelled: &AtomicBool) -> Result<Vec<u8>> {
    let url = endpoint(url)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let request = client
            .get(url)
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/plain;q=0.9",
            )
            .header(reqwest::header::ACCEPT_ENCODING, "identity");
        let response = cancellable(request.send(), cancelled)
            .await?
            .context("更新清单连接失败或超时")?;
        ensure!(
            response.status().is_success(),
            "更新清单返回 HTTP {}；不会跟随重定向",
            response.status().as_u16()
        );
        if let Some(length) = response.content_length() {
            ensure!(length <= MANIFEST_LIMIT as u64, "更新清单超过 256 KiB");
        }
        let mut response = response;
        let mut bytes = Vec::new();
        loop {
            let chunk = cancellable(response.chunk(), cancelled)
                .await?
                .context("更新清单读取失败")?;
            let Some(chunk) = chunk else { break };
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= MANIFEST_LIMIT,
                "更新清单超过 256 KiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    })
}
fn expand(value: &Value, input: &str, model: &str) -> Result<Value> {
    fn budget(v: &Value, input: &str, model: &str, remaining: &mut usize) -> Result<()> {
        let size = match v {
            Value::String(s) if s == "$input" || s == "$json" => input.len().saturating_mul(6),
            Value::String(s) if s == "$model" => model.len().saturating_mul(6),
            Value::String(s) => s.len().saturating_mul(6),
            Value::Object(o) => o.keys().map(|k| k.len().saturating_mul(6) + 4).sum(),
            _ => 16,
        };
        *remaining = remaining
            .checked_sub(size)
            .context("模板展开后的请求可能超过 2 MiB，请缩小输入")?;
        match v {
            Value::Array(a) => {
                for v in a {
                    budget(v, input, model, remaining)?;
                }
            }
            Value::Object(o) => {
                for v in o.values() {
                    budget(v, input, model, remaining)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    budget(value, input, model, &mut (2 * 1024 * 1024))?;
    expand_value(value, input, model)
}
fn expand_value(value: &Value, input: &str, model: &str) -> Result<Value> {
    Ok(match value {
        Value::String(s) if s == "$input" => Value::String(input.into()),
        Value::String(s) if s == "$model" => Value::String(model.into()),
        Value::String(s) if s == "$json" => {
            serde_json::from_str(input).context("此插件需要 JSON 输入")?
        }
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|v| expand_value(v, input, model))
                .collect::<Result<_>>()?,
        ),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| Ok((k.clone(), expand_value(v, input, model)?)))
                .collect::<Result<_>>()?,
        ),
        _ => value.clone(),
    })
}
pub fn uses_model(value: &Value) -> bool {
    match value {
        Value::String(s) => s == "$model",
        Value::Array(a) => a.iter().any(uses_model),
        Value::Object(o) => o.values().any(uses_model),
        _ => false,
    }
}
fn api_error(bytes: &[u8]) -> Option<String> {
    let v: Value = serde_json::from_slice(bytes).ok()?;
    let text = v
        .pointer("/error/message")
        .or_else(|| v.get("error"))
        .and_then(Value::as_str)?;
    Some(text.chars().filter(|c| !c.is_control()).take(300).collect())
}
pub fn execute(tool: &PluginTool, input: &str, model: &str, token: &str) -> Result<String> {
    execute_with_context(tool, input, model, token, None)
}
pub(crate) fn request_body(
    body: &Value,
    input: &str,
    model: &str,
    messages: Option<&[Value]>,
) -> Result<Vec<u8>> {
    let mut expanded = expand(body, input, model)?;
    if let Some(messages) = messages {
        expanded["messages"] = Value::Array(messages.to_vec());
    }
    let bytes = serde_json::to_vec(&expanded)?;
    ensure!(bytes.len() <= 2 * 1024 * 1024, "请求体超过 2 MiB");
    Ok(bytes)
}
async fn cancellable<F: Future>(future: F, cancelled: &AtomicBool) -> Result<F::Output> {
    let mut future = std::pin::pin!(future);
    loop {
        ensure!(
            !cancelled.load(Ordering::Relaxed),
            "客户端请求已取消；服务端可能仍在处理"
        );
        if let Ok(value) = tokio::time::timeout(Duration::from_millis(50), future.as_mut()).await {
            ensure!(
                !cancelled.load(Ordering::Relaxed),
                "客户端请求已取消；服务端可能仍在处理"
            );
            return Ok(value);
        }
    }
}
fn request_cancellable(
    url: reqwest::Url,
    method: &str,
    body: Option<Vec<u8>>,
    token: &str,
    cancelled: &AtomicBool,
) -> Result<(reqwest::StatusCode, Vec<u8>)> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let mut request = client
            .request(
                if method == "GET" {
                    reqwest::Method::GET
                } else {
                    reqwest::Method::POST
                },
                url,
            )
            .header("Accept", "application/json");
        if !token.trim().is_empty() {
            request = request.bearer_auth(token.trim());
        }
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(body);
        }
        let response = cancellable(request.send(), cancelled)
            .await?
            .context("连接失败或请求超时")?;
        let status = response.status();
        let mut response = response;
        let mut bytes = Vec::new();
        loop {
            let chunk = cancellable(response.chunk(), cancelled)
                .await?
                .context("响应读取失败")?;
            let Some(chunk) = chunk else { break };
            ensure!(
                bytes.len() + chunk.len() <= 2 * 1024 * 1024,
                "响应超过 2 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok((status, bytes))
    })
}
pub(crate) fn execute_with_context(
    tool: &PluginTool,
    input: &str,
    model: &str,
    token: &str,
    messages: Option<&[Value]>,
) -> Result<String> {
    execute_with_context_cancellable(tool, input, model, token, messages, None)
}
pub(crate) fn execute_with_context_cancellable(
    tool: &PluginTool,
    input: &str,
    model: &str,
    token: &str,
    messages: Option<&[Value]>,
    cancelled: Option<&AtomicBool>,
) -> Result<String> {
    if let Some(cancelled) = cancelled {
        ensure!(
            !cancelled.load(Ordering::Relaxed),
            "客户端请求已取消；服务端可能仍在处理"
        );
    }
    ensure!(
        messages.is_none() || crate::conversation::supported(tool),
        "此插件不支持多轮对话"
    );
    crate::tools_extra::bounded(input)?;
    ensure!(
        model.len() <= 256 && token.len() <= 8192,
        "模型名称或令牌过长"
    );
    match &tool.adapter {
        Adapter::Builtin {
            tool,
            action,
            pattern,
        } => {
            let kind = crate::tools::ToolKind::ALL
                .into_iter()
                .find(|k| k.id() == tool)
                .context("内置工具不存在")?;
            crate::tools::run_tool(kind, *action, input, pattern, None, 10)
        }
        Adapter::Http {
            url,
            method,
            body,
            response_pointer,
        } => {
            ensure!(
                method != "POST" || !uses_model(body) || !model.trim().is_empty(),
                "此插件需要模型名称，请先查看模型列表并填写实际名称"
            );
            let url = endpoint(url)?;
            let body = if method == "POST" {
                Some(request_body(body, input, model, messages)?)
            } else {
                None
            };
            let (status, bytes) = if let Some(cancelled) = cancelled {
                request_cancellable(url, method, body, token, cancelled)?
            } else {
                let client = reqwest::blocking::Client::builder()
                    .no_proxy()
                    .connect_timeout(Duration::from_secs(5))
                    .timeout(Duration::from_secs(30))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()?;
                let mut request = client
                    .request(
                        if method == "GET" {
                            reqwest::Method::GET
                        } else {
                            reqwest::Method::POST
                        },
                        url,
                    )
                    .header("Accept", "application/json");
                if !token.trim().is_empty() {
                    request = request.bearer_auth(token.trim());
                }
                if let Some(body) = body {
                    request = request
                        .header("Content-Type", "application/json")
                        .body(body);
                }
                let response = request.send().context("连接失败或请求超时")?;
                let status = response.status();
                let mut bytes = Vec::new();
                response.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= 2 * 1024 * 1024, "响应超过 2 MiB");
                (status, bytes)
            };
            ensure!(
                status.is_success(),
                "接口返回 HTTP {}{}",
                status.as_u16(),
                api_error(&bytes)
                    .map(|s| format!("：{s}"))
                    .unwrap_or_default()
            );
            let value: Value =
                serde_json::from_slice(&bytes).context("接口未返回 JSON；暂不支持 SSE/流式响应")?;
            let value = if let Some(pointer) = response_pointer {
                value.pointer(pointer).context("响应缺少插件指定字段")?
            } else {
                &value
            };
            if messages.is_some() {
                return Ok(value
                    .as_str()
                    .context("多轮响应须为文本；暂不支持工具调用或结构化内容")?
                    .to_owned());
            }
            Ok(if let Some(s) = value.as_str() {
                s.into()
            } else {
                serde_json::to_string_pretty(value)?
            })
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn chat_http_sends_full_context_and_rejects_non_text_completions() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            time::{Duration, Instant},
        };
        let manifest =
            super::parse(include_bytes!("../plugins-examples/openai-compatible.json")).unwrap();
        for text_response in [true, false] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let mut tool = manifest
                .tools
                .iter()
                .find(|t| t.id == "chat")
                .unwrap()
                .clone();
            if let super::Adapter::Http { url, .. } = &mut tool.adapter {
                *url = format!(
                    "http://{}/v1/chat/completions",
                    listener.local_addr().unwrap()
                );
            }
            let server = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((s, _)) => break s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline);
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => panic!("fixture accept failed"),
                    }
                };
                // Windows accepted sockets inherit the listener's nonblocking mode.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 1024];
                let (start, size) = loop {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 65536);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                        let size: usize = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .unwrap()
                            .trim()
                            .parse()
                            .unwrap();
                        break (end + 4, size);
                    }
                };
                while bytes.len() < start + size {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 65536);
                }
                let payload: serde_json::Value =
                    serde_json::from_slice(&bytes[start..start + size]).unwrap();
                let body = if text_response {
                    r#"{"choices":[{"message":{"content":"fixture-answer"}}]}"#
                } else {
                    r#"{"choices":[{"message":{"content":null}}]}"#
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                payload
            });
            let messages = vec![
                serde_json::json!({"role":"user","content":"$model"}),
                serde_json::json!({"role":"assistant","content":"$input"}),
                serde_json::json!({"role":"user","content":"next"}),
            ];
            let result =
                super::execute_with_context(&tool, "next", "actual-model", "", Some(&messages));
            let payload = server.join().unwrap();
            assert_eq!(payload["messages"], serde_json::json!(messages));
            assert_eq!(payload["model"], "actual-model");
            assert_eq!(result.is_ok(), text_response);
        }
    }
    #[test]
    fn conversation_payload_preserves_literal_history_after_template_expansion() {
        let body = serde_json::json!({"model":"$model", "messages":[{"role":"user", "content":"$input"}],"stream":false});
        let history = vec![
            serde_json::json!({"role":"user","content":"$model"}),
            serde_json::json!({"role":"assistant","content":"$input"}),
            serde_json::json!({"role":"user","content":"next"}),
        ];
        let bytes = super::request_body(&body, "next", "actual-model", Some(&history)).unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload["model"], "actual-model");
        assert_eq!(payload["messages"], serde_json::json!(history));
    }

    #[test]
    fn cancelling_http_plugin_drops_client_request_without_claiming_remote_rollback() {
        use std::{
            io::Read,
            net::TcpListener,
            sync::{Arc, mpsc},
            time::Instant,
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/fixture", listener.local_addr().unwrap());
        let tool = super::PluginTool {
            id: "cancel-test".into(),
            version: None,
            version_history: Vec::new(),
            name: "Cancel test".into(),
            description: String::new(),
            category: "网络与接口".into(),
            keywords: Vec::new(),
            aliases: Vec::new(),
            sample: String::new(),
            model: String::new(),
            adapter: super::Adapter::Http {
                url,
                method: "POST".into(),
                body: serde_json::json!({"input":"$input"}),
                response_pointer: None,
            },
        };
        let (accepted, request_seen) = mpsc::channel();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            let (body_start, body_len) = loop {
                let size = stream.read(&mut buffer).unwrap();
                assert!(size > 0);
                request.extend_from_slice(&buffer[..size]);
                let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse::<usize>()
                    .unwrap();
                break (header_end + 4, length);
            };
            while request.len() < body_start + body_len {
                let size = stream.read(&mut buffer).unwrap();
                assert!(size > 0);
                request.extend_from_slice(&buffer[..size]);
            }
            assert!(request.starts_with(b"POST /fixture "));
            accepted.send(()).unwrap();
            let mut byte = [0; 1];
            stream.read(&mut byte).unwrap() == 0
        });
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancelled);
        let (result_tx, result_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            result_tx
                .send(super::execute_with_context_cancellable(
                    &tool,
                    "sample",
                    "",
                    "",
                    None,
                    Some(&worker_cancel),
                ))
                .unwrap();
        });
        request_seen.recv_timeout(Duration::from_secs(3)).unwrap();
        let started = Instant::now();
        cancelled.store(true, Ordering::Relaxed);
        let result = result_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("服务端可能仍在处理")
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(server.join().unwrap());
    }
    use super::*;
    const DEMO: &[u8] = include_bytes!("../plugins-examples/local-text.json");
    #[test]
    fn remote_manifest_fetch_is_user_bounded_json_and_never_follows_redirects() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            sync::mpsc,
        };

        let bytes = DEMO.to_vec();
        let (envelope, publisher) = signed_test_envelope(&bytes, "fixture publisher");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/manifest.json", listener.local_addr().unwrap());
        let expected = envelope.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = [0; 2048];
            let size = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..size]).starts_with("GET /manifest.json "));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                expected.len()
            )
            .unwrap();
            stream.write_all(&expected).unwrap();
        });
        let fetched =
            fetch_manifest_cancellable(&source, &[publisher], &AtomicBool::new(false)).unwrap();
        assert_eq!(fetched.manifest, bytes);
        assert_eq!(parse(&fetched.manifest).unwrap().id, "local-text");
        server.join().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/manifest.json", listener.local_addr().unwrap());
        let (redirected, redirect_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            let size = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..size]).starts_with("GET /manifest.json "));
            stream
                .write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            redirected.send(()).unwrap();
        });
        let error = fetch_manifest_cancellable(&source, &[], &AtomicBool::new(false)).unwrap_err();
        assert!(error.to_string().contains("不会跟随重定向"));
        redirect_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        server.join().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/manifest.json", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            let _ = stream.read(&mut request).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MANIFEST_LIMIT + 1
            )
            .unwrap();
        });
        assert!(fetch_manifest_cancellable(&source, &[], &AtomicBool::new(false)).is_err());
        server.join().unwrap();
    }

    #[test]
    fn cancelling_remote_manifest_check_closes_pending_client_socket() {
        use std::{
            io::Read,
            net::TcpListener,
            sync::{Arc, mpsc},
            time::Instant,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/manifest.json", listener.local_addr().unwrap());
        let (request_seen_tx, request_seen_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 512];
            loop {
                let size = stream.read(&mut buffer).unwrap();
                assert!(size > 0);
                request.extend_from_slice(&buffer[..size]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            request_seen_tx.send(()).unwrap();
            let mut byte = [0; 1];
            stream.read(&mut byte).unwrap() == 0
        });
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancelled);
        let (result_tx, result_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            result_tx
                .send(fetch_manifest_cancellable(&source, &[], &worker_cancel))
                .unwrap();
        });
        request_seen_rx
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        let started = Instant::now();
        cancelled.store(true, Ordering::Relaxed);
        assert!(
            result_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .is_err()
        );
        worker.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(server.join().unwrap());
    }

    #[test]
    fn signed_manifest_requires_a_pinned_publisher_and_rejects_tampering() {
        let (envelope, publisher) = signed_test_envelope(DEMO, "Trusted fixture");
        let verified = verify_signed_manifest(&envelope, std::slice::from_ref(&publisher)).unwrap();
        assert_eq!(verified.manifest, DEMO);
        assert_eq!(verified.publisher_id, publisher.id);
        assert_eq!(verified.publisher_name, "Trusted fixture");
        assert!(verify_signed_manifest(&envelope, &[]).is_err());

        let mut changed: Value = serde_json::from_slice(&envelope).unwrap();
        let mut manifest = STANDARD
            .decode(changed["manifest"].as_str().unwrap())
            .unwrap();
        manifest.push(b' ');
        changed["manifest"] = Value::String(STANDARD.encode(manifest));
        let changed = serde_json::to_vec(&changed).unwrap();
        assert!(verify_signed_manifest(&changed, &[publisher]).is_err());
    }

    #[test]
    fn signed_catalog_requires_pinned_signer_and_is_domain_separated() {
        let (_, author) = signed_test_envelope(DEMO, "Plugin author");
        let catalog = serde_json::to_vec(&PluginCatalog {
            schema_version: 2,
            entries: vec![CatalogEntry {
                plugin_id: "local-text".into(),
                name: "本地文本工具".into(),
                version: "1.0.0".into(),
                description: "测试目录条目".into(),
                manifest_url: "https://plugins.example.test/local-text.json".into(),
                publisher_id: author.id,
                host_app_version_req: Some(">=0.82.0-dev.1, <0.83.0".into()),
                plugin_manifest_schema_version: Some(1),
            }],
        })
        .unwrap();
        let (envelope, catalog_signer) = signed_test_catalog_envelope(&catalog, "Directory signer");
        let verified =
            verify_signed_catalog(&envelope, std::slice::from_ref(&catalog_signer)).unwrap();
        assert_eq!(verified.catalog.entries.len(), 1);
        assert_eq!(verified.publisher_id, catalog_signer.id);
        assert!(verify_signed_catalog(&envelope, &[]).is_err());
        assert!(verify_signed_manifest(&envelope, &[catalog_signer.clone()]).is_err());

        let (manifest_envelope, _) = signed_test_envelope(DEMO, "Manifest signer");
        assert!(verify_signed_catalog(&manifest_envelope, &[catalog_signer.clone()]).is_err());

        let mut changed: Value = serde_json::from_slice(&envelope).unwrap();
        let mut payload = STANDARD
            .decode(changed["manifest"].as_str().unwrap())
            .unwrap();
        payload.push(b' ');
        changed["manifest"] = Value::String(STANDARD.encode(payload));
        assert!(
            verify_signed_catalog(&serde_json::to_vec(&changed).unwrap(), &[catalog_signer])
                .is_err()
        );
    }

    #[test]
    fn catalog_compatibility_negotiates_host_and_manifest_schema() {
        let entry = CatalogEntry {
            plugin_id: "local-text".into(),
            name: "Local text".into(),
            version: "1.0.0".into(),
            description: "Compatibility fixture".into(),
            manifest_url: "https://plugins.example.test/local-text.json".into(),
            publisher_id: "a".repeat(64),
            host_app_version_req: Some(">=0.82.0-dev.1, <0.83.0".into()),
            plugin_manifest_schema_version: Some(1),
        };
        let current = current_host_version();
        assert_eq!(
            catalog_compatibility(&entry, &current, PLUGIN_MANIFEST_SCHEMA_VERSION),
            CatalogCompatibility::Compatible
        );

        let mut too_new = entry.clone();
        too_new.host_app_version_req = Some(">=0.83.0".into());
        assert!(matches!(
            catalog_compatibility(&too_new, &current, PLUGIN_MANIFEST_SCHEMA_VERSION),
            CatalogCompatibility::Incompatible(reason) if reason.contains("当前为")
        ));

        let mut unsupported_schema = entry.clone();
        unsupported_schema.plugin_manifest_schema_version = Some(2);
        assert!(matches!(
            catalog_compatibility(&unsupported_schema, &current, PLUGIN_MANIFEST_SCHEMA_VERSION),
            CatalogCompatibility::Incompatible(reason) if reason.contains("schema 2")
        ));

        let mut legacy = entry.clone();
        legacy.host_app_version_req = None;
        legacy.plugin_manifest_schema_version = None;
        assert_eq!(
            catalog_compatibility(&legacy, &current, PLUGIN_MANIFEST_SCHEMA_VERSION),
            CatalogCompatibility::LegacyUnspecified
        );
    }

    #[test]
    fn catalog_schema_two_rejects_missing_or_invalid_compatibility_declarations() {
        let base = PluginCatalog {
            schema_version: 2,
            entries: vec![CatalogEntry {
                plugin_id: "local-text".into(),
                name: "Local text".into(),
                version: "1.0.0".into(),
                description: "Compatibility fixture".into(),
                manifest_url: "https://plugins.example.test/local-text.json".into(),
                publisher_id: "a".repeat(64),
                host_app_version_req: Some(">=0.82.0".into()),
                plugin_manifest_schema_version: Some(1),
            }],
        };
        validate_catalog(&base).unwrap();
        let mut missing = base.clone();
        missing.entries[0].host_app_version_req = None;
        assert!(validate_catalog(&missing).is_err());
        let mut invalid = base;
        invalid.entries[0].host_app_version_req = Some("not semver".into());
        assert!(validate_catalog(&invalid).is_err());
    }

    #[test]
    fn publisher_signing_utility_creates_a_verifiable_envelope() {
        use zeroize::Zeroize;
        let (mut private_key, publisher) = generate_publisher_keypair("Sign fixture").unwrap();
        let envelope = sign_manifest_with_pkcs8(DEMO, &private_key).unwrap();
        let verified = verify_signed_manifest(&envelope, &[publisher.clone()]).unwrap();
        assert_eq!(verified.manifest, DEMO);
        assert_eq!(verified.publisher_id, publisher.id);
        let catalog = serde_json::to_vec(&PluginCatalog {
            schema_version: 2,
            entries: vec![CatalogEntry {
                plugin_id: "local-text".into(),
                name: "Local text".into(),
                version: "1.0.0".into(),
                description: "Signed catalog fixture".into(),
                manifest_url: "https://plugins.example.test/local-text.json".into(),
                publisher_id: publisher.id.clone(),
                host_app_version_req: Some(">=0.82.0-dev.1, <0.83.0".into()),
                plugin_manifest_schema_version: Some(1),
            }],
        })
        .unwrap();
        let signed_catalog = sign_catalog_with_pkcs8(&catalog, &private_key).unwrap();
        let verified_catalog =
            verify_signed_catalog(&signed_catalog, std::slice::from_ref(&publisher)).unwrap();
        assert_eq!(verified_catalog.catalog.entries.len(), 1);
        assert_eq!(verified_catalog.publisher_id, publisher.id);
        private_key.zeroize();
    }

    #[test]
    fn publisher_pins_persist_and_can_be_revoked_without_touching_plugins() {
        let root =
            std::env::temp_dir().join(format!("zi-plugin-publishers-{}", uuid::Uuid::new_v4()));
        let (_, publisher) = signed_test_envelope(DEMO, "Publisher");
        let mut store = Store::load(root.clone());
        let id = store
            .pin_publisher(&publisher.name, &publisher.public_key)
            .unwrap();
        assert_eq!(id, publisher.id);
        let reloaded = Store::load(root.clone());
        assert_eq!(reloaded.trusted_publishers, vec![publisher.clone()]);
        assert!(reloaded.errors.is_empty());
        assert!(reloaded.packages.is_empty());

        let mut store = reloaded;
        store.remove_publisher(&id).unwrap();
        assert!(store.trusted_publishers.is_empty());
        assert!(Store::load(root.clone()).trusted_publishers.is_empty());
        assert!(store.remove_publisher(&id).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catalog_sources_persist_validate_and_bind_a_trusted_signer() {
        let limit_fixtures: Vec<_> = (0..=32)
            .map(|index| CatalogSource {
                id: uuid::Uuid::new_v4().to_string(),
                name: format!("目录 {index}"),
                url: format!("https://plugins-{index}.example.test/catalog.json"),
                publisher_id: None,
                last_verified_at: None,
                last_catalog_schema_version: None,
                last_snapshot: Vec::new(),
            })
            .collect();
        assert!(validate_catalog_sources(&limit_fixtures[..32]).is_ok());
        assert!(validate_catalog_sources(&limit_fixtures).is_err());

        let root = std::env::temp_dir().join(format!(
            "zi-plugin-catalog-sources-{}",
            uuid::Uuid::new_v4()
        ));
        let mut store = Store::load(root.clone());
        let id = store
            .add_catalog_source("团队目录", "https://plugins.example.test/catalog.json")
            .unwrap();
        assert_eq!(store.catalog_sources.len(), 1);
        assert!(
            store
                .add_catalog_source("重复目录", "https://plugins.example.test/catalog.json")
                .is_err()
        );
        assert!(
            store
                .add_catalog_source("外部 HTTP", "http://plugins.example.test/catalog.json")
                .is_err()
        );
        assert!(
            store
                .add_catalog_source(
                    "带查询参数",
                    "https://plugins.example.test/catalog.json?page=1"
                )
                .is_err()
        );

        let (_, signer) = signed_test_catalog_envelope(
            br#"{"schema_version":1,"entries":[]}"#,
            "Directory signer",
        );
        store
            .pin_publisher(&signer.name, &signer.public_key)
            .unwrap();
        store
            .bind_catalog_source_publisher(&id, &signer.id)
            .unwrap();
        let reloaded = Store::load(root.clone());
        assert!(reloaded.errors.is_empty());
        assert_eq!(reloaded.catalog_sources.len(), 1);
        assert_eq!(reloaded.catalog_sources[0].id, id);
        assert_eq!(
            reloaded.catalog_sources[0].publisher_id.as_deref(),
            Some(signer.id.as_str())
        );
        assert!(reloaded.packages.is_empty());

        let mut reloaded = reloaded;
        reloaded.remove_catalog_source(&id).unwrap();
        assert!(Store::load(root.clone()).catalog_sources.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verified_catalog_snapshots_report_added_updated_removed_and_persist() {
        let root =
            std::env::temp_dir().join(format!("zi-plugin-catalog-diff-{}", uuid::Uuid::new_v4()));
        let mut store = Store::load(root.clone());
        let source_id = store
            .add_catalog_source("变化测试", "https://plugins.example.test/catalog.json")
            .unwrap();
        let (_, signer) =
            signed_test_catalog_envelope(br#"{"schema_version":1,"entries":[]}"#, "Directory");
        store
            .pin_publisher(&signer.name, &signer.public_key)
            .unwrap();
        let entry = |plugin_id: &str, name: &str, version: &str| CatalogEntry {
            plugin_id: plugin_id.into(),
            name: name.into(),
            version: version.into(),
            description: "测试目录条目".into(),
            manifest_url: format!("https://plugins.example.test/{plugin_id}.json"),
            publisher_id: signer.id.clone(),
            host_app_version_req: None,
            plugin_manifest_schema_version: None,
        };
        let first = AuthenticatedCatalog {
            catalog: PluginCatalog {
                schema_version: 1,
                entries: vec![
                    entry("alpha", "Alpha", "1.0.0"),
                    entry("beta", "Beta", "0.1.0"),
                ],
            },
            publisher_id: signer.id.clone(),
            publisher_name: signer.name.clone(),
        };
        let baseline = store.record_verified_catalog(&source_id, &first).unwrap();
        assert!(!baseline.had_previous_check);
        assert_eq!(baseline.added.len(), 2);

        let mut changed_alpha = entry("alpha", "Alpha renamed", "1.1.0");
        changed_alpha.publisher_id = "b".repeat(64);
        changed_alpha.description = "更新后的功能说明".into();
        changed_alpha.manifest_url = "https://cdn.example.test/alpha.json".into();
        let second = AuthenticatedCatalog {
            catalog: PluginCatalog {
                schema_version: 1,
                entries: vec![changed_alpha, entry("gamma", "Gamma", "1.0.0")],
            },
            publisher_id: signer.id.clone(),
            publisher_name: signer.name.clone(),
        };
        let diff = store.record_verified_catalog(&source_id, &second).unwrap();
        assert!(diff.had_previous_check);
        assert_eq!(
            diff.added
                .iter()
                .map(|e| e.plugin_id.as_str())
                .collect::<Vec<_>>(),
            ["gamma"]
        );
        assert_eq!(diff.updated.len(), 1);
        assert_eq!(diff.updated[0].previous.version, "1.0.0");
        assert_eq!(diff.updated[0].current.version, "1.1.0");
        assert_eq!(
            diff.updated[0].changed_fields,
            [
                CatalogChangedField::Name,
                CatalogChangedField::Version,
                CatalogChangedField::Publisher,
                CatalogChangedField::Description,
                CatalogChangedField::ManifestUrl,
            ]
        );
        assert_eq!(
            diff.removed
                .iter()
                .map(|e| e.plugin_id.as_str())
                .collect::<Vec<_>>(),
            ["beta"]
        );

        let reloaded = Store::load(root.clone());
        assert!(reloaded.errors.is_empty());
        let source = &reloaded.catalog_sources[0];
        assert!(source.last_verified_at.is_some());
        assert_eq!(source.last_catalog_schema_version, Some(1));
        assert_eq!(source.last_snapshot.len(), 2);
        assert!(
            source
                .last_snapshot
                .iter()
                .any(|entry| entry.plugin_id == "alpha" && entry.version == "1.1.0")
        );

        let mut schema_two_catalog = second.catalog.clone();
        schema_two_catalog.schema_version = 2;
        for entry in &mut schema_two_catalog.entries {
            entry.host_app_version_req = Some(">=0.82.0-dev.1, <0.83.0".into());
            entry.plugin_manifest_schema_version = Some(1);
        }
        let schema_two = AuthenticatedCatalog {
            catalog: schema_two_catalog,
            publisher_id: signer.id.clone(),
            publisher_name: signer.name.clone(),
        };
        let schema_diff = store
            .record_verified_catalog(&source_id, &schema_two)
            .unwrap();
        assert_eq!(schema_diff.schema_version_change, Some((1, 2)));
        assert!(
            schema_diff.updated[0]
                .changed_fields
                .contains(&CatalogChangedField::HostAppVersionRequirement)
        );
        assert!(
            schema_diff.updated[0]
                .changed_fields
                .contains(&CatalogChangedField::PluginManifestSchemaVersion)
        );

        let mut legacy_snapshot = catalog_snapshot(&first.catalog).unwrap();
        let legacy_alpha = legacy_snapshot
            .iter_mut()
            .find(|entry| entry.plugin_id == "alpha")
            .unwrap();
        legacy_alpha.description_digest.clear();
        legacy_alpha.manifest_url_digest.clear();
        let current_snapshot = catalog_snapshot(&second.catalog).unwrap();
        let legacy_diff =
            diff_catalog_snapshots(true, Some(1), &legacy_snapshot, 1, &current_snapshot);
        assert!(
            legacy_diff.updated[0]
                .changed_fields
                .contains(&CatalogChangedField::LegacyDetails)
        );
        assert_eq!(
            legacy_diff.updated[0]
                .changed_fields
                .iter()
                .filter(|field| **field == CatalogChangedField::LegacyDetails)
                .count(),
            1
        );
        let legacy_entry: CatalogSnapshotEntry = serde_json::from_str(
            r#"{"plugin_id":"old-tool","name":"Old tool","version":"1.0.0","publisher_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","entry_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}"#,
        )
        .unwrap();
        assert!(legacy_entry.description_digest.is_empty());
        assert!(legacy_entry.manifest_url_digest.is_empty());

        let (_, replacement_signer) = signed_test_catalog_envelope(
            br#"{"schema_version":1,"entries":[]}"#,
            "Replacement directory",
        );
        let mut reloaded = reloaded;
        reloaded
            .pin_publisher(&replacement_signer.name, &replacement_signer.public_key)
            .unwrap();
        let replacement = AuthenticatedCatalog {
            catalog: second.catalog,
            publisher_id: replacement_signer.id,
            publisher_name: replacement_signer.name,
        };
        assert!(
            reloaded
                .record_verified_catalog(&source_id, &replacement)
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn install_enable_tamper_disable_and_remove() {
        let root = std::env::temp_dir().join(format!("zi-plugins-{}", uuid::Uuid::new_v4()));
        let mut store = Store::load(root.clone());
        let empty_key = store.tool_search_key().to_vec();
        store.install(DEMO).unwrap();
        let installed_key = store.tool_search_key().to_vec();
        assert_ne!(empty_key, installed_key);
        assert!(store.tool_refs().next().is_none());
        assert!(store.install(DEMO).is_err());
        store.set_enabled("local-text", true).unwrap();
        let enabled_key = store.tool_search_key().to_vec();
        assert_ne!(installed_key, enabled_key);
        assert_eq!(store.tool_refs().count(), 2);
        assert_eq!(
            enabled_key.as_slice(),
            Store::load(root.clone()).tool_search_key()
        );
        let mut bytes = DEMO.to_vec();
        bytes.push(b' ');
        fs::write(root.join("local-text.json"), bytes).unwrap();
        store = Store::load(root.clone());
        assert_ne!(enabled_key.as_slice(), store.tool_search_key());
        assert!(store.tool_refs().next().is_none());
        store.uninstall("local-text").unwrap();
        assert!(store.packages.is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn plugin_upgrade_previews_changes_disables_changed_digest_and_swaps_rollback() {
        let root = std::env::temp_dir().join(format!("zi-plugin-upgrade-{}", uuid::Uuid::new_v4()));
        let mut store = Store::load(root.clone());
        store.install(DEMO).unwrap();
        store.set_enabled("local-text", true).unwrap();

        let mut update: Value = serde_json::from_slice(DEMO).unwrap();
        update["version"] = Value::String("1.10.0".into());
        update["tools"][0]["version"] = Value::String("0.2.0".into());
        update["tools"][0]["version_history"] = serde_json::json!([
            {"version":"0.1.0","date":"2026-10-05","changes":"建立工具版本基线"},
            {"version":"0.2.0","date":"2026-10-14","changes":"加入独立工具升级记录"}
        ]);
        update["tools"][0]["name"] = Value::String("Updated formatter".into());
        update["tools"][0]["adapter"] = serde_json::json!({
            "type": "http",
            "url": "http://127.0.0.1:11434/api/generate",
            "method": "POST",
            "body": {"prompt":"$input"}
        });
        let update_bytes = serde_json::to_vec_pretty(&update).unwrap();
        let plan = store.update_plan(&update_bytes).unwrap();
        assert_eq!(plan.current_version, "1.0.0");
        assert_eq!(plan.candidate_version, "1.10.0");
        assert_eq!(plan.changed_tools, vec!["deduplicate"]);
        assert_eq!(plan.tool_version_changes.len(), 1);
        assert_eq!(plan.tool_version_changes[0].tool_id, "deduplicate");
        assert_eq!(
            plan.tool_version_changes[0].previous_version.as_deref(),
            Some("0.1.0")
        );
        assert_eq!(
            plan.tool_version_changes[0].candidate_version.as_deref(),
            Some("0.2.0")
        );
        assert_eq!(plan.tool_version_changes[0].added_history.len(), 2);
        assert_eq!(plan.permission_changes.len(), 1);
        assert!(plan.permission_changes[0].contains("deduplicate: 内置动作 lines#0 → HTTP POST"));
        store.update(&update_bytes).unwrap();
        assert_eq!(store.packages[0].manifest.version, "1.10.0");
        assert!(!store.packages[0].enabled);
        assert!(store.tool_refs().next().is_none());
        assert!(store.has_rollback("local-text"));
        assert!(store.update_plan(DEMO).is_err());

        store.rollback_version("local-text").unwrap();
        assert_eq!(store.packages[0].manifest.version, "1.0.0");
        assert!(!store.packages[0].enabled);
        assert!(store.has_rollback("local-text"));
        let rollback =
            parse(&read_manifest(&root.join("rollback/local-text.json")).unwrap()).unwrap();
        assert_eq!(rollback.version, "1.10.0");

        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn validates_manifest_and_endpoint_boundaries() {
        let s = String::from_utf8(DEMO.to_vec()).unwrap();
        assert!(parse(s.replace("local-text", "../escape").as_bytes()).is_err());
        for url in [
            "file:///secret",
            "http://example.com/api",
            "https://user:pass@example.com",
            "https://example.com?token=x",
        ] {
            assert!(endpoint(url).is_err());
        }
        assert!(endpoint("http://127.0.0.1:11434/api/chat").is_ok());
        assert!(endpoint("https://api.example.com/v1/chat/completions").is_ok());
        assert!(parse(&vec![b' '; 256 * 1024 + 1]).is_err());
    }

    #[test]
    fn localized_aliases_are_optional_bounded_and_preserved() {
        let mut document: Value = serde_json::from_slice(DEMO).unwrap();
        // Old manifests remain valid and do not receive synthesized aliases.
        document["tools"][0]
            .as_object_mut()
            .unwrap()
            .remove("aliases");
        let old = parse(&serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(old.tools[0].aliases.is_empty());

        document["tools"][0]["aliases"] = serde_json::json!(["translate text", "翻译文本"]);
        let parsed = parse(&serde_json::to_vec(&document).unwrap()).unwrap();
        assert_eq!(parsed.tools[0].aliases, ["translate text", "翻译文本"]);

        document["tools"][0]["aliases"] = serde_json::to_value(vec!["x".repeat(128); 16]).unwrap();
        let at_limit = parse(&serde_json::to_vec(&document).unwrap()).unwrap();
        assert_eq!(at_limit.tools[0].aliases.len(), 16);
        assert_eq!(at_limit.tools[0].aliases[0].len(), 128);

        for aliases in [
            serde_json::json!(std::iter::repeat_n("alias", 17).collect::<Vec<_>>()),
            serde_json::json!(["翻".repeat(43)]), // 129 UTF-8 bytes
            serde_json::json!(["   "]),
            serde_json::json!(["bad\nalias"]),
        ] {
            document["tools"][0]["aliases"] = aliases;
            assert!(parse(&serde_json::to_vec(&document).unwrap()).is_err());
        }
    }

    #[test]
    fn independent_tool_versions_preserve_legacy_manifests_and_reject_invalid_metadata() {
        let mut document: Value = serde_json::from_slice(DEMO).unwrap();
        // Historical manifests must not acquire invented version information.
        document["tools"][0]
            .as_object_mut()
            .unwrap()
            .remove("version");
        document["tools"][0]
            .as_object_mut()
            .unwrap()
            .remove("version_history");
        assert!(
            parse(&serde_json::to_vec(&document).unwrap())
                .unwrap()
                .tools[0]
                .version
                .is_none()
        );
        for version in ["0.0.0", "0.1.0", "12.34.56"] {
            document["tools"][0]["version"] = Value::String(version.into());
            let manifest = parse(&serde_json::to_vec(&document).unwrap()).unwrap();
            assert_eq!(manifest.tools[0].version.as_deref(), Some(version));
        }
        for version in ["", "1.0", "01.2.3", "1.2.3\n", "1.2.-1", "4294967296.0.0"] {
            document["tools"][0]["version"] = Value::String(version.into());
            assert!(parse(&serde_json::to_vec(&document).unwrap()).is_err());
        }
        document["tools"][0]["version"] = Value::String("0.2.0".into());
        document["tools"][0]["version_history"] = serde_json::json!([
            {"version":"0.1.0","date":"2026-10-05","changes":"建立工具版本基线"},
            {"version":"0.2.0","date":"2026-10-14","changes":"增加版本追踪"}
        ]);
        assert_eq!(
            parse(&serde_json::to_vec(&document).unwrap())
                .unwrap()
                .tools[0]
                .version_history
                .len(),
            2
        );
        let mut history_without_version = document.clone();
        history_without_version["tools"][0]
            .as_object_mut()
            .unwrap()
            .remove("version");
        assert!(parse(&serde_json::to_vec(&history_without_version).unwrap()).is_err());
        for (history, version) in [
            (
                serde_json::json!([
                    {"version":"0.2.0","date":"2026-10-14","changes":"new"},
                    {"version":"0.1.0","date":"2026-10-15","changes":"old"}
                ]),
                "0.2.0",
            ),
            (
                serde_json::json!([
                    {"version":"0.2.0","date":"2026-10-14","changes":"new"}
                ]),
                "0.3.0",
            ),
            (
                serde_json::json!([
                    {"version":"0.2.0","date":"2026-13-40","changes":"new"}
                ]),
                "0.2.0",
            ),
            (
                serde_json::json!([
                    {"version":"0.2.0","date":"2026-10-14","changes":""}
                ]),
                "0.2.0",
            ),
        ] {
            document["tools"][0]["version_history"] = history;
            document["tools"][0]["version"] = Value::String(version.into());
            assert!(parse(&serde_json::to_vec(&document).unwrap()).is_err());
        }
    }
    #[test]
    fn template_substitution_is_structural() {
        let value = serde_json::json!({"prompt":"$input","model":"$model","other":"prefix $input"});
        let result = expand(&value, "\"},\"attack\":true", "test").unwrap();
        assert_eq!(result["prompt"], "\"},\"attack\":true");
        assert_eq!(result["other"], "prefix $input");
        assert!(expand(&Value::String("$json".into()), "bad", "").is_err());
        assert!(
            expand(
                &serde_json::json!(["$input", "$input"]),
                &"a".repeat(200000),
                ""
            )
            .is_err()
        );
        let tool = parse(DEMO).unwrap().tools.remove(0);
        assert_eq!(execute(&tool, "z\na\nz", "", "").unwrap(), "z\na");
    }
    #[test]
    fn model_validation_and_bounded_api_errors() {
        let tool = parse(include_bytes!("../plugins-examples/ollama.json"))
            .unwrap()
            .tools
            .remove(1);
        assert!(
            execute(&tool, "hello", "  ", "")
                .unwrap_err()
                .to_string()
                .contains("模型")
        );
        assert!(uses_model(&serde_json::json!({"nested": ["$model"]})));
        assert!(!uses_model(
            &serde_json::json!({"literal": "prefix $model"})
        ));
        assert_eq!(
            api_error(br#"{"error":{"message":"model not found"}}"#).as_deref(),
            Some("model not found")
        );
        assert!(api_error(b"<html>upstream error</html>").is_none());
        let data = serde_json::to_vec(&serde_json::json!({"error":"x".repeat(500)})).unwrap();
        assert_eq!(api_error(&data).unwrap().chars().count(), 300);
    }
    #[test]
    fn http_adapter_works_with_local_fixture() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let read = socket.read(&mut request).unwrap();
            assert!(read > 0);
            let body = r#"{"message":{"content":"fixture answer"}}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let mut tool = parse(include_bytes!("../plugins-examples/ollama.json"))
            .unwrap()
            .tools
            .remove(1);
        if let Adapter::Http { url, .. } = &mut tool.adapter {
            *url = format!("http://{address}/api/chat");
        }
        assert_eq!(
            execute(&tool, "hello", "fixture", "").unwrap(),
            "fixture answer"
        );
        server.join().unwrap();
    }
}
