//! Pure validation of exported Agent records. It never executes a record or reads paths.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub mod provenance;

pub const MAX_CONTEXT_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceContext {
    pub source: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_id: Option<usize>,
    pub source_id: String,
    pub source_name: String,
    pub relative_path: String,
    pub location: String,
    pub file_sha256: String,
    pub chunk_sha256: String,
}

impl EvidenceRef {
    pub fn valid(&self) -> bool {
        let label = |value: &str, max: usize| {
            !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
        };
        let hash =
            |value: &str| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
        label(&self.source_id, 128)
            && self.citation_id.is_none_or(|id| (1..=40).contains(&id))
            && label(&self.source_name, 256)
            && label(&self.relative_path, 1024)
            && !self.relative_path.starts_with(['/', '\\'])
            && !self.relative_path.contains(':')
            && !self
                .relative_path
                .split(['/', '\\'])
                .any(|part| part == "..")
            && label(&self.location, 256)
            && hash(&self.file_sha256)
            && hash(&self.chunk_sha256)
    }
}

pub fn validated_citations(answer: &str, available: &BTreeSet<usize>) -> Result<Vec<usize>> {
    let bytes = answer.as_bytes();
    let mut citations = BTreeSet::new();
    let mut cursor = 0;
    // The exported marker is exactly [K followed by one or more ASCII digits and ].
    while let Some(offset) = bytes[cursor..].windows(2).position(|pair| pair == b"[K") {
        let digits = cursor + offset + 2;
        let end = digits
            + bytes[digits..]
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
        cursor = end;
        if end > digits && bytes.get(end) == Some(&b']') {
            let id = answer[digits..end]
                .parse::<usize>()
                .context("答案引用编号无效")?;
            ensure!(
                available.contains(&id),
                "答案引用 [K{id}] 不在本次已选入模型消息的来源中"
            );
            citations.insert(id);
            cursor += 1;
        }
    }
    Ok(citations.into_iter().collect())
}

pub const MAX_IMPORT_BYTES: usize = 1024 * 1024;
pub const MAX_STEP_ELAPSED_MS: u128 = 24 * 60 * 60 * 1_000;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedStep {
    pub tool: String,
    pub elapsed_ms: u128,
    pub content_items: usize,
    pub response_bytes: usize,
    pub model_excerpt_bytes: usize,
    pub is_error: bool,
    pub references: Option<Vec<EvidenceRef>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedContent {
    pub goal: String,
    pub plan: String,
    pub answer: Option<String>,
    pub error: Option<String>,
    pub citations: Option<Vec<usize>>,
    pub context: Option<Vec<SourceContext>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedRecord {
    schema: String,
    pub schema_version: u32,
    pub finished_at_utc: String,
    pub status: String,
    pub model: String,
    pub approved: bool,
    pub allowed_tools: Vec<String>,
    pub max_calls: usize,
    pub calls_made: usize,
    pub model_tokens_reported: Option<u64>,
    #[serde(default)]
    pub generated_by: Option<provenance::ToolProvenance>,
    pub steps: Vec<ImportedStep>,
    pub content: Option<ImportedContent>,
}

fn identifier(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

fn content_text(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= max_bytes
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn parse_utc_timestamp(value: &str) -> Result<chrono::DateTime<chrono::FixedOffset>> {
    let timestamp = chrono::DateTime::parse_from_rfc3339(value).context("运行记录结束时间无效")?;
    ensure!(
        timestamp.offset().local_minus_utc() == 0,
        "运行记录结束时间须为 UTC"
    );
    Ok(timestamp)
}

impl ImportedRecord {
    /// Emit a canonical timestamp instead of copying the original input string.
    pub fn normalized_finished_at_utc(&self) -> Result<String> {
        Ok(parse_utc_timestamp(&self.finished_at_utc)?
            .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
    }
}

pub fn parse_json(bytes: &[u8]) -> Result<ImportedRecord> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_IMPORT_BYTES,
        "运行记录文件须为 1 MiB 以内的 JSON"
    );
    let record: ImportedRecord = serde_json::from_slice(bytes).context("运行记录 JSON 结构无效")?;
    ensure!(
        record.schema == "zi-devtools-agent-run" && matches!(record.schema_version, 1..=5),
        "不支持的 Agent 运行记录版本"
    );
    ensure!(
        (record.schema_version == 5) == record.generated_by.is_some(),
        "v5 运行记录必须包含生成工具来源，旧版不能声明该字段"
    );
    if let Some(generated_by) = &record.generated_by {
        generated_by.validate()?;
        ensure!(
            generated_by.tool_id == "agent-runner",
            "Agent 记录来源工具不匹配"
        );
    }
    ensure!(
        record.schema_version != 1 || record.steps.iter().all(|step| step.references.is_none()),
        "v1 记录不支持来源字段"
    );
    ensure!(
        record.schema_version != 2
            || (record.content.is_some()
                && record.steps.iter().any(|step| step.references.is_some())
                && record
                    .steps
                    .iter()
                    .flat_map(|step| step.references.iter().flatten())
                    .all(|reference| reference.citation_id.is_none())
                && record
                    .content
                    .as_ref()
                    .is_some_and(|content| content.citations.is_none())),
        "v2 来源记录缺少可选内容或来源"
    );
    ensure!(
        record.schema_version != 1
            || record
                .content
                .as_ref()
                .is_none_or(|content| content.citations.is_none()),
        "v1 记录不支持引用编号"
    );
    parse_utc_timestamp(&record.finished_at_utc)?;
    ensure!(
        matches!(record.status.as_str(), "completed" | "failed" | "cancelled"),
        "运行记录状态无效"
    );
    ensure!(record.approved, "运行记录缺少人工批准标志");
    ensure!(identifier(&record.model, 128), "模型名称无效");
    ensure!((1..=4).contains(&record.max_calls), "工具调用上限无效");
    ensure!(
        !record.allowed_tools.is_empty() && record.allowed_tools.len() <= 8,
        "工具白名单数量无效"
    );
    let names: BTreeSet<&str> = record.allowed_tools.iter().map(String::as_str).collect();
    ensure!(
        names.len() == record.allowed_tools.len()
            && record
                .allowed_tools
                .iter()
                .all(|name| identifier(name, 128)),
        "工具白名单名称无效或重复"
    );
    ensure!(
        record.calls_made == record.steps.len() && record.calls_made <= record.max_calls,
        "工具步骤数量与预算不一致"
    );
    for step in &record.steps {
        ensure!(
            names.contains(step.tool.as_str()),
            "运行步骤包含白名单之外的工具"
        );
        ensure!(
            step.elapsed_ms <= MAX_STEP_ELAPSED_MS
                && step.content_items <= 10_000
                && step.response_bytes <= 4 * 1024 * 1024
                && step.model_excerpt_bytes <= 4 * 1024,
            "运行步骤数据超出限制"
        );
        if let Some(references) = &step.references {
            ensure!(
                step.tool == "search_knowledge"
                    && !step.is_error
                    && !references.is_empty()
                    && references.len() <= 10
                    && references.iter().all(EvidenceRef::valid),
                "运行步骤来源字段无效"
            );
        }
    }
    let has_citation_ids = record
        .steps
        .iter()
        .flat_map(|step| step.references.iter().flatten())
        .any(|reference| reference.citation_id.is_some());
    if record.schema_version == 5 {
        ensure!(
            record
                .content
                .as_ref()
                .is_some_and(|content| content.citations.is_some())
                == has_citation_ids,
            "v5 来源编号与引用列表不匹配"
        );
    }
    let validates_citations = record.schema_version == 3
        || (record.schema_version == 4
            && record
                .content
                .as_ref()
                .is_some_and(|content| content.citations.is_some()))
        || (record.schema_version == 5 && has_citation_ids);
    if validates_citations {
        let references: Vec<&EvidenceRef> = record
            .steps
            .iter()
            .flat_map(|step| step.references.iter().flatten())
            .collect();
        let ids: BTreeSet<usize> = references
            .iter()
            .filter_map(|reference| reference.citation_id)
            .collect();
        ensure!(
            record.content.is_some()
                && !references.is_empty()
                && references
                    .iter()
                    .all(|reference| reference.citation_id.is_some())
                && ids.len() == references.len()
                && ids.iter().copied().eq(1..=ids.len()),
            "v3/v4 来源编号无效"
        );
        let content = record.content.as_ref().unwrap();
        let reported = content.citations.as_ref().context("v3 记录缺少引用列表")?;
        let expected = if let Some(answer) = &content.answer {
            validated_citations(answer, &ids)?
        } else {
            Vec::new()
        };
        ensure!(*reported == expected, "v3/v4 答案引用与来源不一致");
    }
    let has_context = record.content.as_ref().is_some_and(|content| {
        content
            .context
            .as_ref()
            .is_some_and(|context| !context.is_empty())
    });
    ensure!(
        (record.schema_version == 4) == has_context || record.schema_version == 5,
        "v4/v5 会话资料版本不匹配"
    );
    if record.status == "completed" {
        ensure!(
            record.model_tokens_reported.is_some(),
            "完成记录缺少模型用量"
        );
        ensure!(
            !record.steps.iter().any(|step| step.is_error),
            "完成记录不应包含工具错误"
        );
    } else {
        ensure!(
            record.model_tokens_reported.is_none(),
            "失败或取消记录不应声明完整模型用量"
        );
    }
    if let Some(content) = &record.content {
        ensure!(
            content_text(&content.goal, 2_000) && content_text(&content.plan, 128 * 1024),
            "可选任务或计划内容无效"
        );
        ensure!(
            content
                .answer
                .as_deref()
                .is_none_or(|answer| answer.len() <= 128 * 1024),
            "答案内容超出限制"
        );
        ensure!(
            content
                .error
                .as_deref()
                .is_none_or(|error| error.len() <= 128 * 1024),
            "错误内容超出限制"
        );
        if let Some(context) = &content.context {
            ensure!(
                !context.is_empty()
                    && serde_json::to_vec(context)?.len() <= MAX_CONTEXT_BYTES
                    && context.iter().all(|item| {
                        identifier(&item.source, 512)
                            && content_text(&item.content, MAX_CONTEXT_BYTES)
                    }),
                "接力资料无效或超过32 KiB"
            );
        }
        ensure!(
            record.status != "completed" || content.answer.is_some(),
            "完成记录缺少可选答案"
        );
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_citation_scan_matches_the_previous_regex() {
        let pattern = regex::Regex::new(r"\[K([0-9]+)\]").unwrap();
        let allowed = BTreeSet::from([1, 2, 3, 40]);
        let originals = [
            "中文 [K1] [K003] [K2] [K3] [K40]",
            "[[K1]] [K1[K2][K] [K一] [K٢] [K４] [K40",
            "[K9999999999999999999999999999999999999999999]",
            "[K4] [K1]",
            "unmatched [K42x] and [K1] followed by [K2]",
            "[K1][K002][K40]",
            "[K0]",
            "[] [] [ k1] K2] 中文 [K3] trailing [K",
        ];
        for answer in originals {
            let old = (|| -> Result<Vec<usize>> {
                let mut found = BTreeSet::new();
                for hit in pattern.captures_iter(answer) {
                    let id = hit[1].parse::<usize>().context("答案引用编号无效")?;
                    ensure!(
                        allowed.contains(&id),
                        "答案引用 [K{id}] 不在本次已选入模型消息的来源中"
                    );
                    found.insert(id);
                }
                Ok(found.into_iter().collect())
            })()
            .map_err(|error| error.to_string());
            assert_eq!(
                validated_citations(answer, &allowed).map_err(|error| error.to_string()),
                old,
                "{answer}"
            );
        }
    }
}
