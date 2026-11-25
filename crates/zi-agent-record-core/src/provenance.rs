//! Portable declared runtime fields and structural verification.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const DOMAIN: &str = "ZiDevTools runtime environment v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RuntimeFingerprint {
    pub schema: u32,
    pub app_version: String,
    pub os: String,
    pub architecture: String,
    pub build_profile: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ToolProvenance {
    pub tool_id: String,
    pub tool_version: String,
    pub runtime_fingerprint: RuntimeFingerprint,
}

pub fn valid_tool_version(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && (*part == "0" || !part.starts_with('0'))
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && part.parse::<u32>().is_ok()
        })
}

impl ToolProvenance {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.tool_id.is_empty()
                && self.tool_id.len() <= 128
                && self
                    .tool_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte)),
            "工具标识无效"
        );
        anyhow::ensure!(valid_tool_version(&self.tool_version), "工具版本格式无效");
        self.runtime_fingerprint.validate()
    }
}

impl RuntimeFingerprint {
    pub fn new(app_version: &str, os: &str, architecture: &str, build_profile: &str) -> Self {
        let mut value = Self {
            schema: 1,
            app_version: app_version.to_owned(),
            os: os.to_owned(),
            architecture: architecture.to_owned(),
            build_profile: build_profile.to_owned(),
            sha256: String::new(),
        };
        value.sha256 = value.calculate_sha256();
        value
    }

    fn calculate_sha256(&self) -> String {
        let canonical = format!(
            "{DOMAIN}\0{}\0{}\0{}\0{}",
            self.app_version, self.os, self.architecture, self.build_profile
        );
        Sha256::digest(canonical.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.schema == 1, "运行环境指纹版本不受支持");
        for (name, value, max) in [
            ("程序版本", self.app_version.as_str(), 64),
            ("操作系统", self.os.as_str(), 32),
            ("处理器架构", self.architecture.as_str(), 32),
            ("构建类型", self.build_profile.as_str(), 16),
        ] {
            anyhow::ensure!(
                !value.is_empty()
                    && value.len() <= max
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte)),
                "{name}字段无效"
            );
        }
        anyhow::ensure!(
            matches!(self.build_profile.as_str(), "debug" | "release"),
            "构建类型无效"
        );
        anyhow::ensure!(
            self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                && self.sha256 == self.calculate_sha256(),
            "运行环境指纹摘要不匹配"
        );
        Ok(())
    }
}
