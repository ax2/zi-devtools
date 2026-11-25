//! Share native shortcut-profile parsing, merge rules and prefix comparison.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

/// Absence uses the default; an empty value explicitly removes the binding.
pub type Bindings = std::collections::BTreeMap<String, String>;

pub fn normalize_sequence(input: &str) -> Result<String, String> {
    let keys: Vec<_> = input.split_whitespace().collect();
    if input.len() > 64
        || keys.len() > 4
        || keys
            .iter()
            .any(|k| k.len() != 1 || !k.as_bytes()[0].is_ascii_alphabetic())
    {
        return Err("使用 1 至 4 个 A–Z 字母，以空格分隔；留空取消绑定".into());
    }
    Ok(keys.join(" ").to_ascii_uppercase())
}

/// Detect equal keys or an ambiguous prefix between normalized non-empty sequences.
pub fn sequences_overlap(left: &str, right: &str) -> bool {
    !left.is_empty()
        && !right.is_empty()
        && (left == right
            || left
                .strip_prefix(right)
                .is_some_and(|suffix| suffix.starts_with(' '))
            || right
                .strip_prefix(left)
                .is_some_and(|suffix| suffix.starts_with(' ')))
}

pub const LIMIT: usize = 256 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    version: u32,
    bindings: Vec<Binding>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    command: String,
    sequence: String,
}
fn validate_id(id: &str) -> Result<()> {
    ensure!(
        id.len() <= 512
            && (matches!(
                id,
                "search" | "recorder:start" | "recorder:pause" | "recorder:stop"
            ) || (id.starts_with("open:")
                && id.len() > 5
                && id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b":/._-".contains(&c)))),
        "配置含无效指令标识"
    );
    Ok(())
}
pub fn parse(bytes: &[u8]) -> Result<Bindings> {
    ensure!(bytes.len() <= LIMIT, "快捷配置最多 256 KiB");
    let profile: Profile = serde_json::from_slice(bytes).context("无法解析快捷配置 JSON")?;
    ensure!(profile.version == 1, "不支持的快捷配置版本");
    ensure!(profile.bindings.len() <= 4096, "快捷配置最多 4096 项");
    let mut result = Bindings::new();
    for binding in profile.bindings {
        validate_id(&binding.command)?;
        let sequence = normalize_sequence(&binding.sequence).map_err(anyhow::Error::msg)?;
        ensure!(
            binding.command != "search" || sequence == "K",
            "K 保留给搜索"
        );
        ensure!(
            result.insert(binding.command, sequence).is_none(),
            "配置含重复指令，拒绝覆盖"
        );
    }
    Ok(result)
}
pub fn encode(bindings: &Bindings) -> Result<Vec<u8>> {
    let profile = Profile {
        version: 1,
        bindings: bindings
            .iter()
            .map(|(id, s)| Binding {
                command: id.clone(),
                sequence: s.clone(),
            })
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&profile)?;
    parse(&bytes)?;
    Ok(bytes)
}
pub fn candidate(current: &Bindings, imported: &Bindings, merge: bool) -> Bindings {
    let mut result = if merge {
        current.clone()
    } else {
        Bindings::new()
    };
    result.extend(imported.clone());
    result
}
