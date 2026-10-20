use super::*;
use serde_json::json;
fn value(s: Result<String>) -> Value {
    serde_json::from_str(&s.unwrap()).unwrap()
}
#[test]
fn every_offline_sample_runs_and_ids_are_unique() {
    let mut ids = std::collections::BTreeSet::new();
    for tool in Tool::ALL {
        assert!(ids.insert(tool.id()));
        if !tool.environment() && tool != Tool::Actuator {
            let report = value(analyze(tool, tool.sample(), tool.second_sample()));
            assert_eq!(report["generatedBy"]["toolId"], tool.id(), "{tool:?}");
            let provenance = crate::tool_provenance::current_for_tool(tool.id()).unwrap();
            assert_eq!(
                report["generatedBy"]["toolVersion"], provenance.tool_version,
                "{tool:?}"
            );
            assert_eq!(report["generatedBy"]["runtimeFingerprint"]["schema"], 1);
        }
    }
}

#[test]
fn generated_reports_require_json_objects_and_preserve_the_report_body() {
    let generated = value(generated_report(
        Tool::Threads,
        r#"{"threadCount":2}"#.into(),
    ));
    assert_eq!(generated["threadCount"], 2);
    assert_eq!(generated["generatedBy"]["toolId"], "java-thread-dump");
    assert!(generated_report(Tool::Threads, "not json".into()).is_err());
    assert!(generated_report(Tool::Threads, "[]".into()).is_err());
}

#[test]
fn generated_report_provenance_covers_all_diagnostic_tools() {
    for tool in Tool::ALL {
        let generated = value(generated_report(tool, r#"{"fixture":true}"#.into()));
        let expected = crate::tool_provenance::current_for_tool(tool.id()).unwrap();
        assert_eq!(generated["generatedBy"]["toolId"], tool.id(), "{tool:?}");
        assert_eq!(
            generated["generatedBy"]["toolVersion"], expected.tool_version,
            "{tool:?}"
        );
        let fingerprint: crate::tool_provenance::RuntimeFingerprint =
            serde_json::from_value(generated["generatedBy"]["runtimeFingerprint"].clone()).unwrap();
        fingerprint.validate().unwrap();
    }
}

#[test]
fn framework_jobs_expose_only_generic_status_and_advance_per_tool_generation() {
    let mut state = State::default();
    state.select(Tool::Jfr);
    let mut history = crate::tasks::Center::default();

    let (sender, receiver) = std::sync::mpsc::channel::<Result<String, String>>();
    state.running = Some((
        Tool::Jfr,
        receiver,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ));
    state.jobs.entry(Tool::Jfr).or_default().begin();
    sender.send(Err("sensitive report detail".into())).unwrap();
    state.poll().unwrap();
    let failed = state.task_snapshots().pop().unwrap();
    assert_eq!(failed.key, "java-jfr");
    assert_eq!(failed.title, Tool::Jfr.label());
    assert_eq!(failed.phase, crate::tasks::Phase::Failed);
    assert!(failed.cancellable);
    assert_eq!(failed.summary, "处理失败；详细原因保留在诊断工作台");
    assert!(!failed.summary.contains("sensitive report detail"));
    assert!(state.result_text().contains("sensitive report detail"));
    history.observe(Some(failed.clone()));

    let (sender, receiver) = std::sync::mpsc::channel::<Result<String, String>>();
    state.running = Some((
        Tool::Jfr,
        receiver,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ));
    state.jobs.entry(Tool::Jfr).or_default().begin();
    sender
        .send(Ok(analyze(Tool::Jfr, Tool::Jfr.sample(), "").unwrap()))
        .unwrap();
    state.poll().unwrap();
    let completed = state.task_snapshots().pop().unwrap();
    assert_eq!(completed.phase, crate::tasks::Phase::Done);
    assert_eq!(completed.generation, failed.generation + 1);
    history.observe(Some(completed));
    assert_eq!(history.rows.len(), 2);
    assert!(
        history
            .rows
            .iter()
            .any(|row| row.phase == crate::tasks::Phase::Failed)
    );
    assert!(
        history
            .rows
            .iter()
            .any(|row| row.phase == crate::tasks::Phase::Done)
    );
}
#[test]
fn thread_wait_cycles_are_not_declared_deadlocks() {
    let v = value(java::threads(Tool::Threads.sample()));
    assert_eq!(v["cycleThreadIndices"], json!([0, 1]));
    assert_eq!(v["explicitJvmDeadlockReport"], false);
    let v = value(java::threads(&format!(
        "{}\nFound one Java-level deadlock:",
        Tool::Threads.sample()
    )));
    assert_eq!(v["explicitJvmDeadlockReport"], true);
    assert!(java::threads("{\"threadDump\":[]}").is_err());
    let no_cycle = Tool::Threads
        .sample()
        .replace("waiting to lock <0x01>", "waiting to lock <0x03>");
    assert_eq!(
        value(java::threads(&no_cycle))["cycleThreadIndices"],
        json!([])
    );
}
#[test]
fn dependency_paths_selection_and_constraints() {
    let v = value(java::dependencies(
        "+--- a:b:1 -> 2\n|    \\--- x:y:3 (c)\n\\--- a:b:2 (*)",
    ));
    assert_eq!(v["entries"][0]["selected"], "2");
    assert_eq!(v["entries"][1]["path"], json!(["a:b", "x:y"]));
    assert_eq!(v["entries"][1]["constraint"], true);
    let v = value(java::dependencies(
        "[INFO] +- (a:b:jar:1:compile - omitted for conflict with 2)",
    ));
    assert_eq!(v["entries"][0]["selected"], "2");
}
#[test]
fn gc_and_jfr_preserve_units_and_sample_meaning() {
    let v = value(java::gc(Tool::Gc.sample()));
    assert_eq!(v["maxPauseMs"], 8.0);
    assert_eq!(v["totalPauseMs"], 12.5);
    assert_eq!(v["timeline"][0]["heap"]["after"], "3M");
    assert!(java::gc("[gc] concurrent phase only").is_err());
    let v = value(java::jfr(Tool::Jfr.sample()));
    assert_eq!(v["sampleTopFrames"][0]["samples"], 1);
    assert_eq!(v["timeline"][1]["durationSeconds"], 0.004);
}
#[test]
fn migrations_require_explicit_edges_and_detect_inconsistency() {
    let v = value(django::migrations(
        "[ ] a.0001\n[X] a.0002 ... (a.0001)",
        "DROP TABLE old;\nALTER TABLE x ADD y int;",
    ));
    assert_eq!(v["dependencies"].as_array().unwrap().len(), 1);
    assert_eq!(v["warnings"].as_array().unwrap().len(), 1);
    assert_eq!(v["sqlRisks"].as_array().unwrap().len(), 2);
    let v = value(django::migrations("a\n [X] 0001\n [ ] 0002", ""));
    assert_eq!(v["dependencies"], json!([]));
    let v = value(django::migrations(
        "[ ] a.0001 ... (a.0002)\n[ ] a.0002 ... (a.0001)",
        "",
    ));
    assert_eq!(v["cycleNodes"], json!(["a.0001", "a.0002"]));
}
#[test]
fn sql_literals_comments_identifiers_and_request_scopes() {
    assert_eq!(
        django::sql_tokens(
            "SELECT \"a1\" FROM t WHERE x='it''s' AND y=$tag$hidden$tag$ /* comment */"
        )
        .unwrap()
        .join(" "),
        "select \"a1\" from t where x = ? and y = ?"
    );
    assert!(django::sql_tokens("select 'unfinished").is_err());
    let v = value(django::sql(Tool::Sql.sample()));
    assert_eq!(v["groups"][0]["count"], 3);
    assert!(
        v["groups"][0]["assessment"]
            .as_str()
            .unwrap()
            .contains("疑似")
    );
    let input = Tool::Sql.sample().replace("\"requestId\":\"demo\",", "");
    assert!(
        value(django::sql(&input))["groups"][0]["assessment"]
            .as_str()
            .unwrap()
            .contains("缺少请求")
    );
    assert!(django::sql(r#"[{"sql":"select 1","durationMs":-1}]"#).is_err());
}
#[test]
fn config_redaction_and_contract_breaking_changes() {
    let v = value(contracts::config(
        Tool::SpringConfig.sample(),
        Tool::SpringConfig.second_sample(),
    ));
    let text = v.to_string();
    assert!(!text.contains("example-only"));
    assert!(!text.contains("DB_PASSWORD"));
    assert!(text.contains("REDACTED"));
    let v = value(contracts::openapi(
        Tool::Drf.sample(),
        Tool::Drf.second_sample(),
    ));
    assert_eq!(v["operationChanges"][0]["change"], "removed");
    assert!(!v["componentChanges"].as_array().unwrap().is_empty());
    assert!(contracts::openapi(r#"{"swagger":"2.0","paths":{}}"#, Tool::Drf.sample()).is_err());
}
#[test]
fn url_names_checks_and_celery_are_evidence_limited() {
    let v = value(django::urls(
        Tool::Urls.sample(),
        Tool::Urls.second_sample(),
    ));
    assert_eq!(v["duplicateNames"].as_array().unwrap().len(), 1);
    assert_eq!(v["reverseCheck"]["candidates"][0]["missing"], json!(["pk"]));
    assert_eq!(
        value(django::checks(
            "System check identified no issues (0 silenced)."
        ))["explicitNoIssues"],
        true
    );
    assert!(django::checks("command crashed").is_err());
    let v = value(django::celery(Tool::Celery.sample()));
    assert_eq!(v["tasks"][0]["retries"], 1);
    assert_eq!(v["tasks"][0]["lastObservedState"], "succeeded");
    assert_eq!(v["tasks"][0]["events"][2]["durationSeconds"], 0.25);
}
#[test]
fn background_results_follow_the_original_tool() {
    let mut state = State::default();
    let (tx, rx) = mpsc::channel();
    state.running = Some((
        Tool::Threads,
        rx,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ));
    state.select(Tool::Sql);
    tx.send(Ok("fixture report".into())).unwrap();
    assert!(state.poll().is_some());
    assert_eq!(state.drafts[&Tool::Threads].output, "fixture report");
    assert_eq!(state.selected, Tool::Sql);
}

#[test]
fn task_center_cancellation_is_generation_bound_and_preserves_previous_report() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let mut state = State::default();
    state.select(Tool::JavaEnvironment);
    state
        .drafts
        .entry(Tool::JavaEnvironment)
        .or_default()
        .output = "previous report".into();
    let cancelled = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::channel();
    state.jobs.entry(Tool::JavaEnvironment).or_default().begin();
    let generation = state.jobs[&Tool::JavaEnvironment]
        .snapshot(
            Tool::JavaEnvironment.id(),
            Tool::JavaEnvironment.label(),
            true,
        )
        .unwrap()
        .generation;
    state.running = Some((Tool::JavaEnvironment, receiver, Arc::clone(&cancelled)));

    assert!(!state.cancel_task(Tool::JavaEnvironment.id(), generation + 1));
    assert!(!state.cancel_task(Tool::Actuator.id(), generation));
    assert!(state.cancel_task(Tool::JavaEnvironment.id(), generation));
    assert!(cancelled.load(Ordering::Relaxed));
    assert_eq!(
        state.task_snapshots()[0].phase,
        crate::tasks::Phase::Cancelling
    );

    sender.send(Ok("new report".into())).unwrap();
    state.poll().unwrap();
    assert_eq!(
        state.drafts[&Tool::JavaEnvironment].output,
        "previous report"
    );
    let row = state.task_snapshots().pop().unwrap();
    assert_eq!(row.phase, crate::tasks::Phase::Cancelled);
    assert_eq!(row.summary, "任务已取消；原有报告保留在诊断工作台");
}
#[test]
fn object_wait_releases_monitor_and_celery_failure_hides_arguments() {
    let input = "\"waiting\" #1 tid=0x1\njava.lang.Thread.State: WAITING\n- waiting on <0x01>\n- locked <0x01>\n\"blocked\" #2 tid=0x2\njava.lang.Thread.State: BLOCKED\n- waiting to lock <0x01>";
    assert_eq!(value(java::threads(input))["waitEdges"], json!([]));
    let result =
        django::celery("Task demo.work[id] raised unexpected: ValueError('private payload')")
            .unwrap();
    assert!(!result.contains("private payload"));
    assert_eq!(
        value(Ok(result))["tasks"][0]["events"][0]["exceptionType"],
        "ValueError"
    );
    let v = value(django::migrations(
        "[X] app.0001",
        "/* multiline\n comment */ DROP\nTABLE old;",
    ));
    assert_eq!(v["sqlRisks"][0]["statementIndex"], 0);
}

#[test]
fn imported_text_does_not_replace_a_running_diagnostic() {
    let mut state = State::default();
    state.import_text(Tool::Threads, "original".into()).unwrap();
    let (_tx, rx) = std::sync::mpsc::channel();
    state.running = Some((
        Tool::Threads,
        rx,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ));
    assert!(
        state
            .import_text(Tool::Threads, "replacement".into())
            .is_err()
    );
    assert_eq!(state.drafts[&Tool::Threads].input, "original");
    state.import_text(Tool::Sql, "SELECT 1".into()).unwrap();
    assert_eq!(state.drafts[&Tool::Threads].input, "original");
    assert_eq!(state.drafts[&Tool::Sql].input, "SELECT 1");
}

#[test]
fn diagnostic_handoff_rejects_unsupported_and_large_inputs_without_mutation() {
    let mut state = State::default();
    state.import_text(Tool::Gc, "old gc".into()).unwrap();
    let draft = state.drafts.get_mut(&Tool::Gc).unwrap();
    draft.second = "comparison".into();
    draft.output = "old result".into();
    draft.summary = "old summary".into();
    draft.executable = "preserved executable".into();
    assert!(
        state
            .receive_handoff(Tool::JavaEnvironment, "incoming".into())
            .is_err()
    );
    assert!(
        state
            .receive_handoff(Tool::Gc, "x".repeat(2 * 1024 * 1024 + 1))
            .is_err()
    );
    assert_eq!(state.selected, Tool::Gc);
    assert_eq!(state.drafts[&Tool::Gc].input, "old gc");
    assert_eq!(state.drafts[&Tool::Gc].output, "old result");
    state.receive_handoff(Tool::Gc, "new gc".into()).unwrap();
    let draft = &state.drafts[&Tool::Gc];
    assert_eq!(draft.input, "new gc");
    assert!(draft.output.is_empty() && draft.summary.is_empty());
    assert_eq!(draft.second, "comparison");
    assert_eq!(draft.executable, "preserved executable");
    assert!(!state.is_running());
}
