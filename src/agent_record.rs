//! Explicit, metadata-first export of one finished Agent run.
use anyhow::{Result, ensure};
use serde_json::{Value, json};

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
        let cancelled: Value = serde_json::from_str(
            &export_json(snapshot(Status::Cancelled, None, None), false).unwrap(),
        )
        .unwrap();
        assert_eq!(cancelled["status"], "cancelled");
        assert!(export_json(snapshot(Status::Completed, None, None), false).is_err());
    }
}
