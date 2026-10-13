//! Installation ownership and optimistic lifecycle plans, independent of UI/MSI.
//! A committed ledger is not proof that its executable exists or its menu works.
use super::{OWNER, template_path};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

pub mod msi_operation;

#[cfg(windows)]
pub mod installer;
#[cfg(windows)]
pub mod store;

pub const MSI_UPGRADE_CODE: &str = "61AF00B6-E92A-4519-A0A3-382347FA4051";
const MAX_INSTALLATIONS: usize = 32;

/// Kind is part of identity; MSI upgrades preserve their family identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum Identity {
    Portable(String),
    Msi,
}
impl Identity {
    pub fn portable() -> Self {
        Self::Portable(uuid::Uuid::new_v4().to_string())
    }
    fn validate(&self) -> Result<()> {
        if let Self::Portable(value) = self {
            let parsed = uuid::Uuid::parse_str(value)?;
            ensure!(
                !parsed.is_nil() && parsed.to_string() == *value,
                "便携安装身份必须为规范的非空UUID"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installation {
    pub identity: Identity,
    /// New UUID for every update, even if the application version is unchanged.
    pub generation: String,
    pub executable: PathBuf,
    pub config: PathBuf,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_sha256: Option<String>,
    /// Exact declaration bound to executable_sha256; absent means the frozen
    /// legacy registration layout, never the maintaining program's new actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_capabilities: Option<String>,
}
impl Installation {
    pub fn new(identity: Identity, executable: PathBuf, config: PathBuf, version: String) -> Self {
        Self {
            identity,
            generation: uuid::Uuid::new_v4().to_string(),
            executable,
            config,
            version,
            executable_sha256: None,
            menu_capabilities: None,
        }
    }
    fn validate(&self) -> Result<()> {
        self.identity.validate()?;
        if let Some(declaration) = &self.menu_capabilities {
            crate::capability_manifest::Manifest::parse(declaration.as_bytes())
                .map_err(anyhow::Error::msg)?;
            ensure!(
                self.executable_sha256.is_some(),
                "菜单能力快照缺少绑定的程序摘要"
            );
        }
        if let Some(digest) = &self.executable_sha256 {
            ensure!(
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "程序摘要无效"
            );
        }
        Identity::Portable(self.generation.clone()).validate()?;
        ensure!(
            self.executable.is_absolute() && self.config.is_absolute(),
            "安装记录必须使用绝对路径"
        );
        let executable = self
            .executable
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("安装程序路径不是有效Unicode"))?;
        let config = self
            .config
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("配置路径不是有效Unicode"))?;
        ensure!(
            executable.encode_utf16().count() <= 32767 && config.encode_utf16().count() <= 32767,
            "安装路径过长"
        );
        semver::Version::parse(&self.version)?;
        // Reuse actual Shell template validation; do not silently resolve paths
        // against whichever working directory happens to read the ledger.
        template_path(&self.executable)?;
        template_path(&self.config)?;
        Ok(())
    }
    pub fn menu_declaration(&self) -> Result<crate::capability_manifest::Manifest> {
        // Protocols 1–3 emitted exactly this layout. Preserve registered legacy
        // behavior; do not infer new capability support from a version string.
        const LEGACY: &str = concat!(
            "ZiDevTools/ShellCapabilities/1\nchoose-files\n",
            "import:csv\nimport:tsv\nimport:json-data\nimport:json\n",
            "import:java-trace\nimport:django-trace\nimport:java-threads\n",
            "import:java-gc\nimport:django-sql\nimport:celery\nimport:files\n"
        );
        crate::capability_manifest::Manifest::parse(
            self.menu_capabilities
                .as_deref()
                .unwrap_or(LEGACY)
                .as_bytes(),
        )
        .map_err(anyhow::Error::msg)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub installation: Installation,
    pub updated_at_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    schema_version: u32,
    owner: String,
    revision: u64,
    active: Option<Identity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    msi_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activation_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_msi: Option<String>,
    // A list permits strict duplicate identity rejection during decoding.
    entries: Vec<Entry>,
}
impl Default for Ledger {
    fn default() -> Self {
        Self {
            schema_version: 4,
            owner: OWNER.into(),
            revision: 0,
            active: None,
            msi_cursor: None,
            activation_cursor: None,
            pending_msi: None,
            entries: Vec::new(),
        }
    }
}
impl Ledger {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    pub fn pending_msi_operation(&self) -> Option<&str> {
        self.pending_msi.as_deref()
    }
    pub fn active(&self) -> Option<&Installation> {
        self.entries
            .iter()
            .find(|entry| Some(&entry.installation.identity) == self.active.as_ref())
            .map(|entry| &entry.installation)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(matches!(self.schema_version, 1..=4), "安装记录协议不受支持");
        ensure!(
            self.schema_version >= 3 || self.pending_msi.is_none(),
            "旧安装协议不能携带待恢复操作"
        );
        if let Some(id) = &self.pending_msi {
            Identity::Portable(id.clone()).validate()?;
            ensure!(self.msi_cursor.is_some(), "待恢复操作缺少MSI游标");
            ensure!(self.revision > 0, "待恢复操作缺少有效修订号");
        }
        ensure!(
            self.schema_version != 1
                || (self.msi_cursor.is_none() && self.activation_cursor.is_none()),
            "旧安装协议不能携带操作游标"
        );
        for cursor in [&self.msi_cursor, &self.activation_cursor]
            .into_iter()
            .flatten()
        {
            Identity::Portable(cursor.clone()).validate()?;
        }
        ensure!(self.owner == OWNER, "安装记录不属于本程序");
        ensure!(self.entries.len() <= MAX_INSTALLATIONS, "安装记录数量超限");
        let mut identities = BTreeMap::new();
        let mut revisions = std::collections::BTreeSet::new();
        for entry in &self.entries {
            ensure!(
                self.schema_version >= 4 || entry.installation.menu_capabilities.is_none(),
                "旧登记协议不能携带菜单能力快照"
            );
            entry.installation.validate()?;
            ensure!(
                entry.updated_at_revision > 0 && entry.updated_at_revision <= self.revision,
                "安装记录修订号无效"
            );
            ensure!(
                revisions.insert(entry.updated_at_revision),
                "安装记录修订号重复"
            );
            ensure!(
                identities
                    .insert(&entry.installation.identity, entry)
                    .is_none(),
                "安装身份重复"
            );
        }
        if let Some(active) = &self.active {
            ensure!(identities.contains_key(active), "活动安装记录不存在");
        }
        if self.schema_version >= 2 {
            ensure!(
                self.active.is_none() || self.activation_cursor.is_some(),
                "活动入口缺少操作游标"
            );
            ensure!(
                !identities.contains_key(&Identity::Msi) || self.msi_cursor.is_some(),
                "MSI记录缺少操作游标"
            );
        }
        let reserved =
            usize::from(self.pending_msi.is_some() && !identities.contains_key(&Identity::Msi));
        ensure!(
            self.entries.len() + reserved <= MAX_INSTALLATIONS,
            "需保留MSI回滚位置，请先完成或恢复待处理安装操作"
        );
        Ok(())
    }
}

pub enum Change {
    Put {
        installation: Installation,
        /// None means first install, Some means update exactly this generation.
        expected_generation: Option<String>,
        activate: bool,
    },
    Remove {
        identity: Identity,
        expected_generation: String,
    },
    Activate(Identity),
}

/// Exact records verified by the caller for this operation, not cached IDs.
/// The eventual UI/installer must re-check files and identity before commit.
pub struct Eligible(pub Vec<Installation>);

#[derive(Clone)]
pub struct Prepared {
    before: Ledger,
    after: Ledger,
}
impl Prepared {
    pub fn result(&self) -> &Ledger {
        &self.after
    }
    pub fn require_current(&self, current: &Ledger) -> Result<()> {
        ensure!(current == &self.before, "安装记录已变化，请重新预览后操作");
        Ok(())
    }
}

pub(in crate::explorer) fn menu_plan(installation: &Installation) -> Result<super::Plan> {
    installation.validate()?;
    let digest = installation
        .executable_sha256
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("安装记录缺少程序摘要，需先核验此程序"))?;
    let mut result = super::plan_declared(
        &installation.executable,
        &installation.config,
        &installation.menu_declaration()?,
    )?;
    for value in &mut result.values {
        if value.name == "ZiCodeMenuProtocol" {
            value.value = "2".into();
        }
    }
    for root in super::ROOTS {
        for (name, value) in [
            (
                "ZiCodeActiveInstall",
                serde_json::to_string(&installation.identity)?,
            ),
            ("ZiCodeInstallGeneration", installation.generation.clone()),
            ("ZiCodeBinarySha256", digest.clone()),
        ] {
            result.values.push(super::Value {
                key: root.into(),
                name: name.into(),
                value,
            });
        }
    }
    Ok(result)
}

pub fn prepare(before: &Ledger, change: Change, eligible: &Eligible) -> Result<Prepared> {
    before.validate()?;
    let changes_msi = match &change {
        Change::Put { installation, .. } => installation.identity == Identity::Msi,
        Change::Remove { identity, .. } => *identity == Identity::Msi,
        Change::Activate(_) => false,
    };
    ensure!(
        !changes_msi || before.pending_msi.is_none(),
        "存在待完成或恢复的MSI操作，拒绝替换其安装记录"
    );
    let explicitly_selected = match &change {
        Change::Put {
            installation,
            activate: true,
            ..
        } => Some(installation.identity.clone()),
        Change::Activate(identity) => Some(identity.clone()),
        _ => None,
    };
    let selects_entry = explicitly_selected.is_some();
    let mut after = before.clone();
    after.schema_version = 4;
    after.revision = before
        .revision
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("安装修订号耗尽"))?;
    match change {
        Change::Put {
            installation,
            expected_generation,
            activate,
        } => {
            installation.validate()?;
            let position = after
                .entries
                .iter()
                .position(|entry| entry.installation.identity == installation.identity);
            let current = position.map(|index| &after.entries[index].installation);
            ensure!(
                current.map(|entry| entry.generation.as_str()) == expected_generation.as_deref(),
                "安装代次已变化或安装身份冲突"
            );
            ensure!(
                current.is_none_or(|entry| entry.generation != installation.generation),
                "更新必须生成新的安装代次"
            );
            let identity = installation.identity.clone();
            let entry = Entry {
                installation,
                updated_at_revision: after.revision,
            };
            if let Some(position) = position {
                after.entries[position] = entry;
            } else {
                after.entries.push(entry);
            }
            if activate {
                after.active = Some(identity);
            }
        }
        Change::Remove {
            identity,
            expected_generation,
        } => {
            let position = after
                .entries
                .iter()
                .position(|entry| entry.installation.identity == identity)
                .ok_or_else(|| anyhow::anyhow!("待移除的安装记录不存在"))?;
            ensure!(
                after.entries[position].installation.generation == expected_generation,
                "拒绝旧安装代次移除新记录"
            );
            after.entries.remove(position);
            if after.active.as_ref() == Some(&identity) {
                after.active = None;
            }
        }
        Change::Activate(identity) => {
            ensure!(
                after
                    .entries
                    .iter()
                    .any(|entry| entry.installation.identity == identity),
                "目标安装记录不存在"
            );
            after.active = Some(identity);
        }
    }
    if let Some(identity) = explicitly_selected {
        ensure!(
            after
                .entries
                .iter()
                .any(|entry| entry.installation.identity == identity
                    && eligible.0.contains(&entry.installation)),
            "目标安装尚未通过本次可用性核验"
        );
    }
    // Only exact eligible records can own the active alias. Select the latest
    // validated remaining record on removal, never an unverified stale path.
    if !after
        .active()
        .is_some_and(|entry| eligible.0.contains(entry))
    {
        after.active = after
            .entries
            .iter()
            .filter(|entry| eligible.0.contains(&entry.installation))
            .max_by_key(|entry| entry.updated_at_revision)
            .map(|entry| entry.installation.identity.clone());
    }
    if changes_msi
        || (after.msi_cursor.is_none()
            && after
                .entries
                .iter()
                .any(|e| e.installation.identity == Identity::Msi))
    {
        after.msi_cursor = Some(uuid::Uuid::new_v4().to_string());
    }
    if selects_entry
        || before.active != after.active
        || (after.active.is_some() && after.activation_cursor.is_none())
    {
        after.activation_cursor = Some(uuid::Uuid::new_v4().to_string());
    }
    after.validate()?;
    Ok(Prepared {
        before: before.clone(),
        after,
    })
}

#[cfg(test)]
mod tests;
