//! Shared portable tool collection format and its read-only import plan.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug)]
pub struct ImportProblem {
    pub code: &'static str,
    message: String,
}

impl fmt::Display for ImportProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for ImportProblem {}

fn problem(code: &'static str, message: impl Into<String>) -> ImportProblem {
    ImportProblem {
        code,
        message: message.into(),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PlannedCollection {
    pub source_name: String,
    pub name: String,
    pub tool_ids: Vec<String>,
    pub renamed_to_avoid_conflict: bool,
}

pub const TOOL_COLLECTIONS_EXPORT_SCHEMA: &str = "zi-devtools/tool-collections";
pub const TOOL_COLLECTIONS_EXPORT_MAX_BYTES: usize = 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PortableToolCollections {
    pub schema: String,
    pub schema_version: u32,
    pub collections: Vec<PortableToolCollection>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PortableToolCollection {
    pub name: String,
    pub tool_ids: Vec<String>,
}

pub fn valid_collection_name(name: &str) -> Result<String> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "请输入工具集合名称");
    anyhow::ensure!(name.chars().count() <= 40, "工具集合名称最多40个字符");
    anyhow::ensure!(
        !name.chars().any(char::is_control),
        "工具集合名称不能含控制字符"
    );
    Ok(name.to_owned())
}

pub fn prepare_import<'a>(
    bytes: &[u8],
    existing_names: impl ExactSizeIterator<Item = &'a str>,
) -> Result<Vec<PlannedCollection>> {
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() <= TOOL_COLLECTIONS_EXPORT_MAX_BYTES,
        problem("INVALID_FILE", "导入文件必须为1 MiB以内的非空JSON")
    );
    let input: PortableToolCollections = serde_json::from_slice(bytes).context(problem(
        "INVALID_FILE",
        "工具集合文件格式无效或包含未知字段",
    ))?;
    anyhow::ensure!(
        input.schema == TOOL_COLLECTIONS_EXPORT_SCHEMA && input.schema_version == 1,
        problem("UNSUPPORTED_VERSION", "不支持的工具集合文件版本")
    );
    anyhow::ensure!(
        !input.collections.is_empty() && input.collections.len() <= 128,
        problem(
            "INVALID_COLLECTION_COUNT",
            "导入文件必须包含1到128个工具集合"
        )
    );
    anyhow::ensure!(
        existing_names.len() + input.collections.len() <= 128,
        problem("COLLECTION_LIMIT", "导入后工具集合将超过128个")
    );

    let mut incoming_names = std::collections::HashSet::new();
    let mut reserved_names = existing_names
        .map(str::to_lowercase)
        .collect::<std::collections::HashSet<_>>();
    let mut items = Vec::with_capacity(input.collections.len());
    for portable in input.collections {
        let source_name = valid_collection_name(&portable.name)
            .map_err(|error| problem("INVALID_NAME", error.to_string()))?;
        anyhow::ensure!(
            incoming_names.insert(source_name.to_lowercase()),
            problem(
                "DUPLICATE_NAME",
                format!("导入文件中存在重复的集合名称：{source_name}")
            )
        );
        anyhow::ensure!(
            portable.tool_ids.len() <= 512,
            problem(
                "TOOL_LIMIT",
                format!("集合“{source_name}”包含超过512个工具入口")
            )
        );
        let mut seen_tools = std::collections::HashSet::new();
        for tool_id in &portable.tool_ids {
            anyhow::ensure!(
                !tool_id.is_empty() && tool_id.len() <= 512,
                problem(
                    "INVALID_TOOL_ID",
                    format!("集合“{source_name}”包含无效工具 ID")
                )
            );
            anyhow::ensure!(
                seen_tools.insert(tool_id),
                problem(
                    "DUPLICATE_TOOL_ID",
                    format!("集合“{source_name}”包含重复工具 ID：{tool_id}")
                )
            );
        }

        let mut name = source_name.clone();
        let mut renamed_to_avoid_conflict = false;
        if reserved_names.contains(&name.to_lowercase()) {
            renamed_to_avoid_conflict = true;
            let mut suffix_number = 1usize;
            loop {
                let suffix = format!("（导入 {suffix_number}）");
                let prefix_len = 40usize.saturating_sub(suffix.chars().count());
                let prefix = source_name.chars().take(prefix_len).collect::<String>();
                name = format!("{prefix}{suffix}");
                if reserved_names.insert(name.to_lowercase()) {
                    break;
                }
                suffix_number += 1;
                anyhow::ensure!(
                    suffix_number <= 1000,
                    problem("COLLECTION_LIMIT", "无法为冲突集合生成唯一名称")
                );
            }
        } else {
            reserved_names.insert(name.to_lowercase());
        }
        items.push(PlannedCollection {
            source_name,
            name,
            tool_ids: portable.tool_ids,
            renamed_to_avoid_conflict,
        });
    }
    Ok(items)
}
