use crate::django;
use serde_json::Value;

#[test]
fn generated_unicode_decimal_table_matches_locked_unicode_definition() {
    use regex_syntax::hir::{Class, HirKind};
    let hir = regex_syntax::Parser::new().parse(r"\d").unwrap();
    let HirKind::Class(Class::Unicode(class)) = hir.kind() else {
        panic!("decimal class")
    };
    let ranges: Vec<_> = class.iter().collect();
    for value in 0..=0x10ffff {
        if let Some(c) = char::from_u32(value) {
            let at = ranges.partition_point(|r| r.start() <= c);
            let expected = at > 0 && c <= ranges[at - 1].end();
            assert_eq!(crate::text_parse::digit(c), expected, "U+{value:X}");
        }
    }
}

#[test]
fn unicode_migration_names_dependencies_cycles_and_sql_evidence_are_preserved() {
    let report: Value = serde_json::from_str(&django::migrations("app\n[X] 0001_初始\n[ ] app.0002_更新 ... (app.0001_初始, app.0003)\n[ ] app.0003 ... (app.0002_更新)", "DROP TABLE old; ALTER TABLE new ADD x int;").unwrap()).unwrap();
    assert_eq!(report["migrations"].as_array().unwrap().len(), 3);
    assert_eq!(report["dependencies"].as_array().unwrap().len(), 3);
    assert_eq!(
        report["cycleNodes"],
        serde_json::json!(["app.0002_更新", "app.0003"])
    );
    assert_eq!(report["sqlRisks"].as_array().unwrap().len(), 2);
}

#[test]
fn url_unicode_combining_marks_and_malformed_converters_preserve_parameter_names() {
    let report: Value = serde_json::from_str(&django::urls(r#"[{"route":"<int:编号>/<名\u0301>/<bad:>/<:bad>/<outer<int:nested>>","name":"test"}]"#, r#"{"name":"test","kwargs":{"编号":1,"名\u0301":2,"nested":3}}"#).unwrap()).unwrap();
    assert_eq!(
        report["reverseCheck"]["candidates"][0]["parameterNamesMatch"],
        true
    );
}

#[test]
fn celery_preserves_retry_failures_unicode_names_and_never_copies_return_values() {
    let report: Value = serde_json::from_str(&django::celery("prefix Task 任务.发送[id] received\nTask 任务.发送[id] retry: Retry in 1.5s\nTask 任务.发送[id] raised unexpected: app.错误(secret)\nTask 任务.发送[id] succeeded in 0.25s: secret").unwrap()).unwrap();
    assert_eq!(report["tasks"][0]["retries"], 1);
    assert_eq!(report["tasks"][0]["failures"], 1);
    assert_eq!(report["tasks"][0]["events"][2]["exceptionType"], "app.错误");
    assert_eq!(report["tasks"][0]["events"][3]["durationSeconds"], 0.25);
    assert!(!report.to_string().contains("secret"));
    assert!(django::celery("Task one[id] received\nTask two[id] received").is_err());
}

#[test]
fn thread_report_minimum_and_plugin_budget_do_not_reduce_standalone_scope() {
    let minimum = serde_json::json!({"index":0,"name":"a","line":1,"state":"A","held":[],"wait":[],"releasedForWait":[],"frames":[]});
    assert!(serde_json::to_string_pretty(&minimum).unwrap().len() > 120);
    let input = (0..500)
        .map(|i| format!("\"t{i}\" #1\n"))
        .collect::<String>();
    assert!(crate::java::threads(&input).is_ok());
    assert!(
        crate::execute_plugin("java.threads", &input, "", 49152)
            .unwrap_err()
            .downcast_ref::<crate::ReportTooLarge>()
            .is_some()
    );
}

#[test]
fn configuration_fast_json_preserves_yaml_and_legacy_duplicate_resolution() {
    assert_eq!(
        crate::contracts::config(r#"{"x":1,"x":2}"#, "{}").unwrap(),
        crate::contracts::config("x: 2", "{}").unwrap()
    );
    assert_eq!(
        crate::contracts::config(r#"{"a":{"x":1,"x":2}}"#, "{}").unwrap(),
        crate::contracts::config("a:\n  x: 2", "{}").unwrap()
    );
    let json = crate::contracts::config(
        r#"{"server":{"port":8080},"secret":"hidden"}"#,
        r#"{"server":{"port":8081},"secret":"changed"}"#,
    )
    .unwrap();
    let yaml = crate::contracts::config(
        "server:\n  port: 8080\nsecret: hidden",
        "server:\n  port: 8081\nsecret: changed",
    )
    .unwrap();
    assert_eq!(json, yaml);
    assert!(crate::contracts::config("{server: {port: 8080}}", "{}").is_ok());
}
