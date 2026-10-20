//! Portable runtime provenance for saved tool results.
//! This intentionally excludes machine names, user names and local paths.
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

fn valid_tool_version(version: &str) -> bool {
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

/// Captures the declared catalog version and portable runtime fields for a newly generated result.
pub fn current_for_tool(tool_id: &str) -> anyhow::Result<ToolProvenance> {
    let catalog: serde_json::Value = serde_json::from_str(include_str!("../docs/tools.json"))?;
    from_catalog(tool_id, &catalog)
}

fn from_catalog(tool_id: &str, catalog: &serde_json::Value) -> anyhow::Result<ToolProvenance> {
    anyhow::ensure!(
        !tool_id.is_empty()
            && tool_id.len() <= 128
            && tool_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte)),
        "工具标识无效"
    );
    let tools = catalog["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("工具目录缺少工具列表"))?;
    let mut matches = tools
        .iter()
        .filter(|tool| tool["id"].as_str() == Some(tool_id));
    let tool = matches
        .next()
        .ok_or_else(|| anyhow::anyhow!("工具目录未登记来源工具"))?;
    anyhow::ensure!(matches.next().is_none(), "工具目录中的工具标识重复");
    let status = tool["status"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("来源工具未登记可用状态"))?;
    anyhow::ensure!(
        matches!(status, "implemented" | "in-progress"),
        "规划中的工具不能生成结果来源证明"
    );
    let version = tool["tool_version"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("来源工具未登记独立版本"))?;
    anyhow::ensure!(valid_tool_version(version), "来源工具版本格式无效");
    Ok(ToolProvenance {
        tool_id: tool_id.to_owned(),
        tool_version: version.to_owned(),
        runtime_fingerprint: RuntimeFingerprint::current(),
    })
}

impl RuntimeFingerprint {
    pub fn current() -> Self {
        let mut value = Self {
            schema: 1,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            os: std::env::consts::OS.to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            build_profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
            .to_owned(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_fingerprint_is_portable_bounded_and_self_verifying() {
        let fingerprint = RuntimeFingerprint::current();
        fingerprint.validate().unwrap();
        let json = serde_json::to_string(&fingerprint).unwrap();
        if let Ok(username) = std::env::var("USERNAME")
            && !username.is_empty()
        {
            assert!(!json.contains(&username));
        }
        let mut changed = fingerprint;
        changed.os.push('x');
        assert!(changed.validate().is_err());
    }

    #[test]
    fn result_provenance_uses_catalog_versions_and_rejects_unknown_tools() {
        for (id, version) in [
            ("image-tools", "1.7.1"),
            ("screenshot-workbench", "0.15.0"),
            ("image-batch", "1.4.0"),
            ("image-metadata", "1.4.0"),
            ("image-workflow", "0.12.0"),
        ] {
            let provenance = current_for_tool(id).unwrap();
            assert_eq!(provenance.tool_id, id);
            assert_eq!(provenance.tool_version, version);
            provenance.runtime_fingerprint.validate().unwrap();
        }
        assert!(current_for_tool("unknown-image-result").is_err());
    }

    #[test]
    fn provenance_validation_rejects_malformed_tool_versions() {
        let mut provenance = current_for_tool("agent-runner").unwrap();
        provenance.validate().unwrap();
        for version in ["1.2", "1.02.3", "1.2.3-alpha", "1..3", "1.2.4294967296"] {
            provenance.tool_version = version.to_owned();
            assert!(provenance.validate().is_err(), "accepted {version}");
        }
    }

    #[test]
    fn catalog_provenance_rejects_missing_invalid_or_ambiguous_versions() {
        let missing = serde_json::json!({
            "tools": [{
                "id": "image-tools",
                "status": "implemented"
            }]
        });
        assert!(from_catalog("image-tools", &missing).is_err());

        for version in ["1.2", "1.02.3", "1.2.3-alpha", "1..3", "1.2.4294967296"] {
            let catalog = serde_json::json!({
                "tools": [{
                    "id": "image-tools",
                    "status": "implemented",
                    "tool_version": version
                }]
            });
            assert!(from_catalog("image-tools", &catalog).is_err(), "{version}");
        }

        let duplicate = serde_json::json!({
            "tools": [
                {
                    "id": "image-tools",
                    "status": "implemented",
                    "tool_version": "1.2.3"
                },
                {
                    "id": "image-tools",
                    "status": "implemented",
                    "tool_version": "1.2.4"
                }
            ]
        });
        assert!(from_catalog("image-tools", &duplicate).is_err());
    }

    #[test]
    fn catalog_provenance_accepts_only_available_tools() {
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../docs/tools.json")).unwrap();
        let tools = catalog["tools"].as_array().unwrap();
        let mut checked = 0;
        let mut planned = 0;
        for tool in tools {
            let id = tool["id"].as_str().unwrap();
            match tool["status"].as_str().unwrap() {
                "implemented" | "in-progress" => {
                    let provenance = from_catalog(id, &catalog).unwrap_or_else(|error| {
                        panic!("available tool {id} should produce provenance: {error}")
                    });
                    assert_eq!(provenance.tool_id, id);
                    assert_eq!(provenance.tool_version, tool["tool_version"]);
                    checked += 1;
                }
                "planned" => {
                    planned += 1;
                    assert!(
                        from_catalog(id, &catalog).is_err(),
                        "planned tool {id} must not produce provenance"
                    );
                }
                status => panic!("unhandled catalog status {status} for {id}"),
            }
        }
        assert!(checked > 0, "the catalog should have available tools");
        assert!(planned > 0, "the catalog should exercise the planned guard");
    }
}
