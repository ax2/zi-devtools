//! Explicit, metadata-first export of one finished Agent run.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeSet, io::Read, path::Path};

use crate::agent::{self, Outcome, Plan, Step};
pub use zi_agent_record_core::{
    ImportedContent, ImportedRecord, ImportedStep, MAX_IMPORT_BYTES, MAX_STEP_ELAPSED_MS,
    parse_json,
};

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
    let has_references = include_content
        && snapshot
            .steps
            .iter()
            .any(|step| !step.references.is_empty());
    let has_citation_ids = has_references
        && snapshot
            .steps
            .iter()
            .flat_map(|step| &step.references)
            .any(|reference| reference.citation_id.is_some());
    let has_context = include_content && !snapshot.plan.config.context.is_empty();
    let generated_by = crate::tool_provenance::current_for_tool("agent-runner")?;
    if has_citation_ids {
        ensure!(
            snapshot
                .steps
                .iter()
                .flat_map(|step| &step.references)
                .all(|reference| reference.citation_id.is_some()),
            "来源编号不完整"
        );
        let available: BTreeSet<usize> = snapshot
            .steps
            .iter()
            .flat_map(|step| &step.references)
            .filter_map(|reference| reference.citation_id)
            .collect();
        if let Some(outcome) = snapshot.outcome {
            ensure!(
                outcome.citations == agent::validated_citations(&outcome.answer, &available)?,
                "答案引用与来源编号不一致"
            );
        }
    }
    let steps: Vec<Value> = snapshot
        .steps
        .iter()
        .map(|step| {
            let mut value = json!({
                "tool": step.tool,
                "elapsed_ms": step.elapsed_ms,
                "content_items": step.content_items,
                "response_bytes": step.response_bytes,
                "model_excerpt_bytes": step.model_excerpt_bytes,
                "is_error": step.is_error,
            });
            if has_references && !step.references.is_empty() {
                value["references"] = json!(step.references);
            }
            value
        })
        .collect();
    let mut record = json!({
        "schema": "zi-devtools-agent-run",
        "schema_version": 5,
        "generated_by": generated_by,
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
        if has_context {
            record["content"]["context"] = json!(snapshot.plan.config.context);
        }
        if has_citation_ids {
            record["content"]["citations"] = json!(
                snapshot
                    .outcome
                    .map_or(&[][..], |outcome| outcome.citations.as_slice())
            );
        }
    }
    Ok(serde_json::to_string_pretty(&record)?)
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
    use crate::agent::EvidenceRef;
    use crate::{agent, mcp};
    use std::path::PathBuf;

    fn fixture() -> (Plan, Vec<Step>, Outcome) {
        let plan = Plan {
            text: "secret plan".into(),
            planning_tokens: 20,
            config: agent::Config {
                endpoint: "http://127.0.0.1:11434/api/chat".into(),
                model: "local-model".into(),
                server: mcp::Config {
                    executable: PathBuf::from("C:/secret/program.exe"),
                    args: vec!["secret argument".into()],
                },
                access_path: PathBuf::from("C:/secret/access.json"),
                goal: "secret goal".into(),
                context: Vec::new(),
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
            references: Vec::new(),
        }];
        let outcome = Outcome {
            answer: "secret answer".into(),
            steps: steps.clone(),
            model_tokens: 42,
            citations: Vec::new(),
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
        let mut legacy = record.clone();
        legacy["schema_version"] = json!(1);
        legacy.as_object_mut().unwrap().remove("generated_by");
        let imported = parse_json(legacy.to_string().as_bytes()).unwrap();
        assert_eq!(imported.schema_version, 1);
        assert!(imported.generated_by.is_none());
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
    fn handed_off_context_is_only_exported_with_opt_in_v5_content() {
        let (mut plan, steps, outcome) = fixture();
        plan.config.context.push(agent::SourceContext {
            source: "synthetic handoff".into(),
            content: "private report body".into(),
        });
        let snapshot = || Snapshot {
            plan: &plan,
            steps: &steps,
            outcome: Some(&outcome),
            status: Status::Completed,
            approved: true,
            finished_at: "2026-10-14T00:00:00Z",
            error: None,
        };

        let default: Value =
            serde_json::from_str(&export_json(snapshot(), false).unwrap()).unwrap();
        assert_eq!(default["schema_version"], 5);
        assert_eq!(default["generated_by"]["toolId"], "agent-runner");
        assert!(default.get("content").is_none());

        let with_content = export_json(snapshot(), true).unwrap();
        let imported = parse_json(with_content.as_bytes()).unwrap();
        assert_eq!(imported.schema_version, 5);
        assert_eq!(
            imported.content.unwrap().context.unwrap()[0].content,
            "private report body"
        );
        let mut legacy: Value = serde_json::from_str(&with_content).unwrap();
        legacy["schema_version"] = json!(4);
        legacy.as_object_mut().unwrap().remove("generated_by");
        let imported = parse_json(legacy.to_string().as_bytes()).unwrap();
        assert_eq!(imported.schema_version, 4);
        assert!(imported.generated_by.is_none());
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
            assert_eq!(imported.schema_version, 5);
            let generated_by = imported.generated_by.as_ref().unwrap();
            assert_eq!(generated_by.tool_id, "agent-runner");
            generated_by.validate().unwrap();
            assert_eq!(imported.calls_made, 1);
            assert_eq!(imported.steps[0].tool, "echo");
            assert_eq!(imported.content.is_some(), include_content);
            assert_eq!(imported.model_tokens_reported, Some(42));
        }
    }

    #[test]
    fn knowledge_references_require_explicit_content_export_in_v5() {
        let (mut plan, mut steps, _) = fixture();
        plan.config.selected = vec!["search_knowledge".into()];
        steps[0].tool = "search_knowledge".into();
        steps[0].references = vec![EvidenceRef {
            citation_id: None,
            source_id: "source-1".into(),
            source_name: "private source name".into(),
            relative_path: "private/path.md".into(),
            location: "paragraph 2".into(),
            file_sha256: "a".repeat(64),
            chunk_sha256: "b".repeat(64),
        }];
        let outcome = Outcome {
            answer: "done".into(),
            steps: steps.clone(),
            model_tokens: 42,
            citations: Vec::new(),
        };
        let snapshot = || Snapshot {
            plan: &plan,
            steps: &steps,
            outcome: Some(&outcome),
            status: Status::Completed,
            approved: true,
            finished_at: "2026-10-01T01:00:00Z",
            error: None,
        };
        let default = export_json(snapshot(), false).unwrap();
        assert!(!default.contains("private/path.md"));
        assert!(!default.contains("private source name"));
        assert_eq!(
            parse_json(default.as_bytes()).unwrap().steps[0]
                .references
                .as_ref()
                .map(Vec::len),
            None
        );
        let selected = export_json(snapshot(), true).unwrap();
        let imported = parse_json(selected.as_bytes()).unwrap();
        assert_eq!(imported.schema_version, 5);
        assert_eq!(
            imported.steps[0].references.as_ref().unwrap()[0].relative_path,
            "private/path.md"
        );
        let mut legacy: Value = serde_json::from_str(&selected).unwrap();
        legacy["schema_version"] = json!(2);
        legacy.as_object_mut().unwrap().remove("generated_by");
        let imported = parse_json(legacy.to_string().as_bytes()).unwrap();
        assert_eq!(imported.schema_version, 2);
        assert!(imported.generated_by.is_none());
        assert!(!selected.contains("secret tool response"));
        let mut malformed: Value = serde_json::from_str(&selected).unwrap();
        malformed["steps"][0]["references"][0]["file_sha256"] = json!("bad");
        assert!(parse_json(malformed.to_string().as_bytes()).is_err());
        let mut disguised: Value = serde_json::from_str(&selected).unwrap();
        disguised["schema_version"] = json!(1);
        assert!(parse_json(disguised.to_string().as_bytes()).is_err());
    }

    #[test]
    fn numbered_sources_roundtrip_as_v5_and_reject_forged_citations() {
        let (mut plan, mut steps, _) = fixture();
        plan.config.selected = vec!["search_knowledge".into()];
        steps[0].tool = "search_knowledge".into();
        steps[0].references = vec![EvidenceRef {
            citation_id: Some(1),
            source_id: "source-1".into(),
            source_name: "private source".into(),
            relative_path: "private/guide.md".into(),
            location: "paragraph 1".into(),
            file_sha256: "a".repeat(64),
            chunk_sha256: "b".repeat(64),
        }];
        let outcome = Outcome {
            answer: "Synthesized statement [K1]".into(),
            steps: steps.clone(),
            model_tokens: 42,
            citations: vec![1],
        };
        let snapshot = || Snapshot {
            plan: &plan,
            steps: &steps,
            outcome: Some(&outcome),
            status: Status::Completed,
            approved: true,
            finished_at: "2026-10-01T01:00:00Z",
            error: None,
        };
        let default = export_json(snapshot(), false).unwrap();
        assert!(!default.contains("private/guide.md"));
        assert_eq!(parse_json(default.as_bytes()).unwrap().schema_version, 5);
        let selected = export_json(snapshot(), true).unwrap();
        let imported = parse_json(selected.as_bytes()).unwrap();
        assert_eq!(imported.schema_version, 5);
        assert_eq!(
            imported.steps[0].references.as_ref().unwrap()[0].citation_id,
            Some(1)
        );
        assert_eq!(imported.content.unwrap().citations, Some(vec![1]));
        let mut legacy: Value = serde_json::from_str(&selected).unwrap();
        legacy["schema_version"] = json!(3);
        legacy.as_object_mut().unwrap().remove("generated_by");
        let imported = parse_json(legacy.to_string().as_bytes()).unwrap();
        assert_eq!(imported.schema_version, 3);
        assert!(imported.generated_by.is_none());
        let mut forged: Value = serde_json::from_str(&selected).unwrap();
        forged["content"]["answer"] = json!("Fabricated [K2]");
        assert!(parse_json(forged.to_string().as_bytes()).is_err());
        let mut bad_list: Value = serde_json::from_str(&selected).unwrap();
        bad_list["content"]["citations"] = json!([2]);
        assert!(parse_json(bad_list.to_string().as_bytes()).is_err());
        let mut disguised: Value = serde_json::from_str(&selected).unwrap();
        disguised["schema_version"] = json!(2);
        assert!(parse_json(disguised.to_string().as_bytes()).is_err());
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
        let mut missing_provenance = original.clone();
        missing_provenance
            .as_object_mut()
            .unwrap()
            .remove("generated_by");
        assert!(parse_json(missing_provenance.to_string().as_bytes()).is_err());
        let mut wrong_tool = original.clone();
        wrong_tool["generated_by"]["toolId"] = json!("image-tools");
        assert!(parse_json(wrong_tool.to_string().as_bytes()).is_err());
        let mut invalid_tool_version = original.clone();
        invalid_tool_version["generated_by"]["toolVersion"] = json!("1.02.0");
        assert!(parse_json(invalid_tool_version.to_string().as_bytes()).is_err());
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
