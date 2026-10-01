//! Explicit, metadata-first export of one finished Agent run.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeSet, io::Read, path::Path};

use crate::agent::{Outcome, Plan, Step};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Completed,
    Failed,
    Cancelled,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

pub struct Snapshot<'a> {
    pub plan: &'a Plan,
    pub steps: &'a [Step],
    pub outcome: Option<&'a Outcome>,
    pub status: Status,
    pub approved: bool,
    pub finished_at: &'a str,
    pub error: Option<&'a str>,
}

pub fn export_json(snapshot: Snapshot<'_>, include_content: bool) -> Result<String> {
    ensure!(snapshot.approved, "尚未批准执行，不能导出运行记录");
    ensure!(!snapshot.finished_at.is_empty(), "任务尚未结束");
    ensure!(
        snapshot.status != Status::Completed || snapshot.outcome.is_some(),
        "完成记录缺少最终结果"
    );
    let steps: Vec<Value> = snapshot
        .steps
        .iter()
        .map(|step| {
            json!({
                "tool": step.tool,
                "elapsed_ms": step.elapsed_ms,
                "content_items": step.content_items,
                "response_bytes": step.response_bytes,
                "model_excerpt_bytes": step.model_excerpt_bytes,
                "is_error": step.is_error,
            })
        })
        .collect();
    let mut record = json!({
        "schema": "zi-devtools-agent-run",
        "schema_version": 1,
        "finished_at_utc": snapshot.finished_at,
        "status": snapshot.status.label(),
        "model": snapshot.plan.config.model,
        "approved": snapshot.approved,
        "allowed_tools": snapshot.plan.config.selected,
        "max_calls": snapshot.plan.config.max_calls,
        "calls_made": snapshot.steps.len(),
        "model_tokens_reported": snapshot.outcome.map(|outcome| outcome.model_tokens),
        "steps": steps,
    });
    if include_content {
        record["content"] = json!({
            "goal": snapshot.plan.config.goal,
            "plan": snapshot.plan.text,
            "answer": snapshot.outcome.map(|outcome| outcome.answer.as_str()),
            "error": snapshot.error,
        });
    }
    Ok(serde_json::to_string_pretty(&record)?)
}

pub const MAX_IMPORT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedStep {
    pub tool: String,
    pub elapsed_ms: u128,
    pub content_items: usize,
    pub response_bytes: usize,
    pub model_excerpt_bytes: usize,
    pub is_error: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedContent {
    pub goal: String,
    pub plan: String,
    pub answer: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedRecord {
    schema: String,
    schema_version: u32,
    pub finished_at_utc: String,
    pub status: String,
    pub model: String,
    pub approved: bool,
    pub allowed_tools: Vec<String>,
    pub max_calls: usize,
    pub calls_made: usize,
    pub model_tokens_reported: Option<u64>,
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

pub fn parse_json(bytes: &[u8]) -> Result<ImportedRecord> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_IMPORT_BYTES,
        "运行记录文件须为 1 MiB 以内的 JSON"
    );
    let record: ImportedRecord = serde_json::from_slice(bytes).context("运行记录 JSON 结构无效")?;
    ensure!(
        record.schema == "zi-devtools-agent-run" && record.schema_version == 1,
        "不支持的 Agent 运行记录版本"
    );
    let timestamp = chrono::DateTime::parse_from_rfc3339(&record.finished_at_utc)
        .context("运行记录结束时间无效")?;
    ensure!(
        timestamp.offset().local_minus_utc() == 0,
        "运行记录结束时间须为 UTC"
    );
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
            step.content_items <= 10_000
                && step.response_bytes <= 4 * 1024 * 1024
                && step.model_excerpt_bytes <= 4 * 1024,
            "运行步骤数据超出限制"
        );
    }
    if record.status == "completed" {
        ensure!(
            record.model_tokens_reported.is_some(),
            "完成记录缺少模型用量"
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
        ensure!(
            record.status != "completed" || content.answer.is_some(),
            "完成记录缺少可选答案"
        );
    }
    Ok(record)
}

pub fn load_file(path: &Path) -> Result<ImportedRecord> {
    let file = std::fs::File::open(path).context("无法打开运行记录文件")?;
    ensure!(file.metadata()?.is_file(), "运行记录必须是普通文件");
    let mut bytes = Vec::new();
    file.take((MAX_IMPORT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .context("无法读取运行记录文件")?;
    parse_json(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{agent, mcp};
    use std::path::PathBuf;

    fn fixture() -> (Plan, Vec<Step>, Outcome) {
        let plan = Plan {
            text: "secret plan".into(),
            config: agent::Config {
                endpoint: "http://127.0.0.1:11434/api/chat".into(),
                model: "local-model".into(),
                server: mcp::Config {
                    executable: PathBuf::from("C:/secret/program.exe"),
                    args: vec!["secret argument".into()],
                },
                access_path: PathBuf::from("C:/secret/access.json"),
                goal: "secret goal".into(),
                selected: vec!["echo".into()],
                max_calls: 2,
            },
            tools: vec![json!({"secret_schema":"secret MCP content"})],
        };
        let steps = vec![Step {
            tool: "echo".into(),
            elapsed_ms: 123,
            result: "secret tool response".into(),
            content_items: 1,
            response_bytes: 78,
            model_excerpt_bytes: 27,
            is_error: false,
        }];
        let outcome = Outcome {
            answer: "secret answer".into(),
            steps: steps.clone(),
            model_tokens: 42,
        };
        (plan, steps, outcome)
    }

    #[test]
    fn default_record_exports_only_whitelisted_metadata() {
        let (plan, steps, outcome) = fixture();
        let text = export_json(
            Snapshot {
                plan: &plan,
                steps: &steps,
                outcome: Some(&outcome),
                status: Status::Completed,
                approved: true,
                finished_at: "2026-10-01T01:00:00Z",
                error: None,
            },
            false,
        )
        .unwrap();
        for private in [
            "secret plan",
            "secret goal",
            "secret answer",
            "secret tool response",
            "secret argument",
            "secret MCP content",
            "C:/secret",
            "127.0.0.1",
        ] {
            assert!(!text.contains(private), "record leaked {private}");
        }
        let record: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(record["status"], "completed");
        assert_eq!(record["steps"][0]["response_bytes"], 78);
        assert_eq!(record["model_tokens_reported"], 42);
        assert!(record.get("content").is_none());
    }

    #[test]
    fn opt_in_content_and_terminal_failures_are_distinct() {
        let (plan, steps, outcome) = fixture();
        let snapshot = |status, outcome, error| Snapshot {
            plan: &plan,
            steps: &steps,
            outcome,
            status,
            approved: true,
            finished_at: "2026-10-01T01:00:00Z",
            error,
        };
        let completed: Value = serde_json::from_str(
            &export_json(snapshot(Status::Completed, Some(&outcome), None), true).unwrap(),
        )
        .unwrap();
        assert_eq!(completed["content"]["goal"], "secret goal");
        assert_eq!(completed["content"]["answer"], "secret answer");
        assert!(!completed.to_string().contains("C:/secret"));
        let failed: Value = serde_json::from_str(
            &export_json(snapshot(Status::Failed, None, Some("model failed")), true).unwrap(),
        )
        .unwrap();
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["model_tokens_reported"], Value::Null);
        assert_eq!(failed["content"]["error"], "model failed");
        assert_eq!(
            parse_json(failed.to_string().as_bytes()).unwrap().status,
            "failed"
        );
        let cancelled: Value = serde_json::from_str(
            &export_json(snapshot(Status::Cancelled, None, None), false).unwrap(),
        )
        .unwrap();
        assert_eq!(cancelled["status"], "cancelled");
        assert_eq!(
            parse_json(cancelled.to_string().as_bytes()).unwrap().status,
            "cancelled"
        );
        assert!(export_json(snapshot(Status::Completed, None, None), false).is_err());
    }

    #[test]
    fn exports_roundtrip_into_read_only_record() {
        let (plan, steps, outcome) = fixture();
        for include_content in [false, true] {
            let exported = export_json(
                Snapshot {
                    plan: &plan,
                    steps: &steps,
                    outcome: Some(&outcome),
                    status: Status::Completed,
                    approved: true,
                    finished_at: "2026-10-01T01:00:00Z",
                    error: None,
                },
                include_content,
            )
            .unwrap();
            let imported = parse_json(exported.as_bytes()).unwrap();
            assert_eq!(imported.status, "completed");
            assert_eq!(imported.calls_made, 1);
            assert_eq!(imported.steps[0].tool, "echo");
            assert_eq!(imported.content.is_some(), include_content);
            assert_eq!(imported.model_tokens_reported, Some(42));
        }
    }

    #[test]
    fn import_rejects_tampering_and_unbounded_files() {
        let (plan, steps, outcome) = fixture();
        let exported = export_json(
            Snapshot {
                plan: &plan,
                steps: &steps,
                outcome: Some(&outcome),
                status: Status::Completed,
                approved: true,
                finished_at: "2026-10-01T01:00:00Z",
                error: None,
            },
            false,
        )
        .unwrap();
        let original: Value = serde_json::from_str(&exported).unwrap();
        for change in [
            ("schema_version", json!(2)),
            ("status", json!("running")),
            ("approved", json!(false)),
            ("calls_made", json!(2)),
            ("max_calls", json!(0)),
            ("finished_at_utc", json!("yesterday")),
            ("model_tokens_reported", Value::Null),
        ] {
            let mut record = original.clone();
            record[change.0] = change.1;
            assert!(
                parse_json(record.to_string().as_bytes()).is_err(),
                "accepted {}",
                change.0
            );
        }
        let mut other_tool = original.clone();
        other_tool["steps"][0]["tool"] = json!("delete_file");
        assert!(parse_json(other_tool.to_string().as_bytes()).is_err());
        let mut unknown = original.clone();
        unknown["unrecognized"] = json!("untrusted extra");
        assert!(parse_json(unknown.to_string().as_bytes()).is_err());
        let mut false_tokens = original.clone();
        false_tokens["status"] = json!("cancelled");
        assert!(parse_json(false_tokens.to_string().as_bytes()).is_err());

        let path =
            std::env::temp_dir().join(format!("zi-agent-import-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&path, vec![b' '; MAX_IMPORT_BYTES + 1]).unwrap();
        assert!(load_file(&path).is_err());
        std::fs::write(&path, exported).unwrap();
        assert_eq!(load_file(&path).unwrap().allowed_tools, vec!["echo"]);
        std::fs::remove_file(path).unwrap();
    }
}
