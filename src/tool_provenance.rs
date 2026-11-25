//! Portable runtime provenance for native saved tool results.
use zi_agent_record_core::provenance::valid_tool_version;
pub use zi_agent_record_core::provenance::{RuntimeFingerprint, ToolProvenance};

pub fn current_runtime_fingerprint() -> RuntimeFingerprint {
    RuntimeFingerprint::new(
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    )
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
        runtime_fingerprint: current_runtime_fingerprint(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_fingerprint_is_portable_bounded_and_self_verifying() {
        let fingerprint = current_runtime_fingerprint();
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
