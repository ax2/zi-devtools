//! Metadata-only comparison of two explicitly imported Agent records.
use crate::{agent_record::ImportedRecord, agent_record_library::status_label};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct ToolMetrics {
    pub calls: usize,
    pub elapsed_ms: u64,
    pub response_bytes: u64,
    pub errors: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolComparison {
    pub name: String,
    pub baseline: ToolMetrics,
    pub candidate: ToolMetrics,
}

pub struct Comparison {
    pub baseline_status: String,
    pub candidate_status: String,
    pub baseline_model: String,
    pub candidate_model: String,
    pub baseline_calls: usize,
    pub candidate_calls: usize,
    pub baseline_tokens: Option<u64>,
    pub candidate_tokens: Option<u64>,
    pub allowed_added: Vec<String>,
    pub allowed_removed: Vec<String>,
    pub tools: Vec<ToolComparison>,
}

fn metrics(record: &ImportedRecord) -> BTreeMap<String, ToolMetrics> {
    let mut by_tool: BTreeMap<String, ToolMetrics> = record
        .allowed_tools
        .iter()
        .map(|name| (name.clone(), ToolMetrics::default()))
        .collect();
    for step in &record.steps {
        let item = by_tool.entry(step.tool.clone()).or_default();
        item.calls += 1;
        item.elapsed_ms += step.elapsed_ms as u64;
        item.response_bytes += step.response_bytes as u64;
        item.errors += usize::from(step.is_error);
    }
    by_tool
}

pub fn compare(baseline: &ImportedRecord, candidate: &ImportedRecord) -> Comparison {
    let baseline_tools = metrics(baseline);
    let candidate_tools = metrics(candidate);
    let baseline_allowed: BTreeSet<_> = baseline.allowed_tools.iter().cloned().collect();
    let candidate_allowed: BTreeSet<_> = candidate.allowed_tools.iter().cloned().collect();
    let tools = baseline_tools
        .keys()
        .chain(candidate_tools.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|name| ToolComparison {
            baseline: baseline_tools.get(&name).copied().unwrap_or_default(),
            candidate: candidate_tools.get(&name).copied().unwrap_or_default(),
            name,
        })
        .collect();
    Comparison {
        baseline_status: baseline.status.clone(),
        candidate_status: candidate.status.clone(),
        baseline_model: baseline.model.clone(),
        candidate_model: candidate.model.clone(),
        baseline_calls: baseline.calls_made,
        candidate_calls: candidate.calls_made,
        baseline_tokens: baseline.model_tokens_reported,
        candidate_tokens: candidate.model_tokens_reported,
        allowed_added: candidate_allowed
            .difference(&baseline_allowed)
            .cloned()
            .collect(),
        allowed_removed: baseline_allowed
            .difference(&candidate_allowed)
            .cloned()
            .collect(),
        tools,
    }
}

pub fn token_change(baseline: Option<u64>, candidate: Option<u64>) -> String {
    match (baseline, candidate) {
        (Some(a), Some(b)) => format!("{a} → {b} ({:+})", i128::from(b) - i128::from(a)),
        (Some(a), None) => format!("{a} → 未知"),
        (None, Some(b)) => format!("未知 → {b}"),
        (None, None) => "未知 → 未知".into(),
    }
}

impl Comparison {
    pub fn metadata_summary(&self) -> String {
        let mut lines = vec![
            "Agent 记录元数据对比（基线 → 候选）".to_owned(),
            format!(
                "状态：{} → {}",
                status_label(&self.baseline_status),
                status_label(&self.candidate_status)
            ),
            format!("模型：{} → {}", self.baseline_model, self.candidate_model),
            format!(
                "工具调用：{} → {}",
                self.baseline_calls, self.candidate_calls
            ),
            format!(
                "模型报告 token：{}",
                token_change(self.baseline_tokens, self.candidate_tokens)
            ),
            format!(
                "白名单新增：{}；移除：{}",
                if self.allowed_added.is_empty() {
                    "无".into()
                } else {
                    self.allowed_added.join("、")
                },
                if self.allowed_removed.is_empty() {
                    "无".into()
                } else {
                    self.allowed_removed.join("、")
                }
            ),
        ];
        for tool in &self.tools {
            lines.push(format!(
                "{}：调用 {}→{}，耗时 {}→{} ms，响应 {}→{} 字节，错误 {}→{}",
                tool.name,
                tool.baseline.calls,
                tool.candidate.calls,
                tool.baseline.elapsed_ms,
                tool.candidate.elapsed_ms,
                tool.baseline.response_bytes,
                tool.candidate.response_bytes,
                tool.baseline.errors,
                tool.candidate.errors
            ));
        }
        lines.push("记录由用户提供；差异不证明来源、答案质量或费用。".into());
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_record;
    use serde_json::json;

    fn record(status: &str, tokens: Option<u64>, tool: &str, secret: &str) -> ImportedRecord {
        agent_record::parse_json(
            json!({
                "schema":"zi-devtools-agent-run","schema_version":1,
                "finished_at_utc":"2026-10-01T01:00:00Z","status":status,
                "model":"local-model","approved":true,"allowed_tools":[tool],
                "max_calls":2,"calls_made":1,"model_tokens_reported":tokens,
                "steps":[{"tool":tool,"elapsed_ms":120,"content_items":1,
                    "response_bytes":512,"model_excerpt_bytes":128,"is_error":false}],
                "content":{"goal":secret,"plan":secret,"answer":secret,"error":null}
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn compare_uses_tool_union_and_never_copies_task_content() {
        let baseline = record("completed", Some(200), "search_knowledge", "PRIVATE_GOAL");
        let candidate = record(
            "completed",
            Some(150),
            "list_knowledge_sources",
            "PRIVATE_ANSWER",
        );
        let result = compare(&baseline, &candidate);
        assert_eq!(result.allowed_added, ["list_knowledge_sources"]);
        assert_eq!(result.allowed_removed, ["search_knowledge"]);
        assert_eq!(result.tools.len(), 2);
        let text = result.metadata_summary();
        assert!(text.contains("200 → 150 (-50)"));
        assert!(!text.contains("PRIVATE_GOAL"));
        assert!(!text.contains("PRIVATE_ANSWER"));
    }

    #[test]
    fn missing_tokens_stay_unknown() {
        let baseline = record("completed", Some(200), "search_knowledge", "private");
        let candidate = record("failed", None, "search_knowledge", "private");
        let text = compare(&baseline, &candidate).metadata_summary();
        assert!(text.contains("200 → 未知"));
        assert!(!text.contains("200 → 0"));
    }

    #[test]
    fn repeated_tool_calls_aggregate_without_making_failure_look_complete() {
        let baseline = record("completed", Some(200), "search_knowledge", "private");
        let candidate = agent_record::parse_json(
            json!({
                "schema":"zi-devtools-agent-run","schema_version":1,
                "finished_at_utc":"2026-10-01T02:00:00Z","status":"failed",
                "model":"local-model","approved":true,
                "allowed_tools":["search_knowledge"],"max_calls":2,"calls_made":2,
                "model_tokens_reported":null,
                "steps":[
                    {"tool":"search_knowledge","elapsed_ms":120,"content_items":1,"response_bytes":512,"model_excerpt_bytes":128,"is_error":false},
                    {"tool":"search_knowledge","elapsed_ms":80,"content_items":0,"response_bytes":32,"model_excerpt_bytes":0,"is_error":true}
                ]
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        let result = compare(&baseline, &candidate);
        assert_eq!(result.tools[0].candidate.calls, 2);
        assert_eq!(result.tools[0].candidate.elapsed_ms, 200);
        assert_eq!(result.tools[0].candidate.response_bytes, 544);
        assert_eq!(result.tools[0].candidate.errors, 1);
        assert!(result.metadata_summary().contains("200 → 未知"));
    }

    #[test]
    fn oversized_elapsed_time_is_rejected_before_aggregation() {
        let mut value: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&json!({
                "schema":"zi-devtools-agent-run","schema_version":1,
                "finished_at_utc":"2026-10-01T01:00:00Z","status":"completed",
                "model":"local-model","approved":true,"allowed_tools":["search_knowledge"],
                "max_calls":2,"calls_made":1,"model_tokens_reported":1,
                "steps":[{"tool":"search_knowledge","elapsed_ms":120,
                    "content_items":1,"response_bytes":512,"model_excerpt_bytes":128,"is_error":false}]
            }))
            .unwrap(),
        )
        .unwrap();
        value["steps"][0]["elapsed_ms"] = json!(u64::MAX);
        assert!(agent_record::parse_json(value.to_string().as_bytes()).is_err());
        value["steps"][0]["elapsed_ms"] = json!(120);
        value["steps"][0]["is_error"] = json!(true);
        assert!(agent_record::parse_json(value.to_string().as_bytes()).is_err());
    }
}
