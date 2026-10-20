use crate::django;
use serde_json::Value;

#[test]
fn openapi_incremental_output_budget_preserves_every_fitting_report() {
    use serde_json::json;
    for count in [1, 40, 120, 220, 350] {
        for kind in ["operations", "nested", "components"] {
            let mut left = json!({"openapi":"3.1.1","paths":{}});
            let mut right = left.clone();
            for index in 0..count {
                let key = format!("field-{index}-中~\"/");
                match kind {
                    "operations" => left["paths"][format!("/p{index}")] = json!({"get":{}}),
                    "nested" => {
                        left["paths"]["/p"]["get"]["schema"]["properties"][&key] =
                            json!({"type":"string"});
                        right["paths"]["/p"]["get"]["schema"]["properties"][&key] =
                            json!({"type":"integer"});
                    }
                    _ => {
                        left["components"]["schemas"][&key] = json!({"type":"string"});
                        right["components"]["schemas"][&key] = json!({"type":"integer"});
                    }
                }
            }
            for (a, b) in [(&left, &right), (&right, &left)] {
                let (a, b) = (a.to_string(), b.to_string());
                let full = crate::contracts::openapi(&a, &b).unwrap();
                let limited = crate::contracts::openapi_limited(&a, &b, 48 * 1024);
                let escaped_bytes = serde_json::to_vec(&full).unwrap().len() - 2;
                if escaped_bytes <= 48 * 1024 {
                    assert_eq!(limited.unwrap(), full, "{kind} {count}");
                } else {
                    assert!(
                        limited
                            .unwrap_err()
                            .downcast_ref::<crate::ReportTooLarge>()
                            .is_some()
                    );
                }
            }
        }
    }
}

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
fn jcmd_thread_dump_json_preserves_container_metadata_and_virtual_type_evidence() {
    let old_jdk = r#"{
      "threadDump": {
        "processId": "12345",
        "time": "2026-10-17T00:00:00Z",
        "runtimeVersion": "21.0.12+1-LTS",
        "threadContainers": [{
          "container": "<root>", "parent": null, "owner": null,
          "threads": [
            {"tid":"1","name":"main","stack":["app.Main.run(Main.java:1)"]},
            {"tid":"2","name":"worker","stack":["java.base/java.lang.VirtualThread.run(VirtualThread.java:1)"]}
          ],
          "threadCount":"2"
        }]
      }
    }"#;
    let report: Value = serde_json::from_str(&crate::java::threads(old_jdk).unwrap()).unwrap();
    assert_eq!(report["sourceFormat"], "jcmd-json");
    assert_eq!(report["threadCount"], 2);
    assert_eq!(report["containerCount"], 1);
    assert_eq!(report["virtualThreadCount"], 0);
    assert_eq!(report["virtualTypeUnknownCount"], 2);
    assert_eq!(report["threads"][1]["threadType"], "unknown");
    assert_eq!(report["threads"][1]["containerParent"], Value::Null);
    assert_eq!(
        report["threads"][1]["frames"][0],
        "java.base/java.lang.VirtualThread.run(VirtualThread.java:1)"
    );
    assert_eq!(report["waitEdges"], serde_json::json!([]));
    assert_eq!(report["cycleThreadIndices"], serde_json::json!([]));

    let current_jdk = r#"{"threadDump":{"processId":12345,"runtimeVersion":"25","threadContainers":[{"container":"<root>","parent":null,"owner":null,"threads":[{"tid":3,"name":"main","state":"WAITING","virtual":false,"stack":[]},{"tid":4,"name":"工作线程","state":"TIMED_WAITING","virtual":true,"time":"2026-10-17T00:00:00Z","stack":["app.Worker.run(Worker.java:2)"]}],"threadCount":"2"}]}}"#;
    let report: Value = serde_json::from_str(&crate::java::threads(current_jdk).unwrap()).unwrap();
    assert_eq!(report["metadata"]["processId"], "12345");
    assert_eq!(report["virtualThreadCount"], 1);
    assert_eq!(report["virtualTypeUnknownCount"], 0);
    assert_eq!(report["threads"][0]["threadType"], "platform");
    assert_eq!(report["threads"][1]["threadType"], "virtual");
    assert_eq!(report["threads"][1]["state"], "TIMED_WAITING");
    assert_eq!(report["threads"][1]["containerOwner"], Value::Null);
    assert_eq!(report["threads"][1]["lockDataAvailable"], false);
}

#[test]
fn jcmd_thread_dump_json_rejects_malformed_shape_and_respects_report_budget() {
    for input in [
        r#"{"threadDump":[]}"#,
        r#"{"threadDump":{"threadContainers":{}}}"#,
        r#"{"threadDump":{"threadContainers":[{"container":"<root>","threads":[{"tid":1,"name":"x","virtual":"true"}]}]}}"#,
        r#"{"threadDump":{"threadContainers":[{"container":"<root>","threads":[{"tid":1,"name":"x","stack":[1]}]}]}}"#,
    ] {
        assert!(crate::java::threads(input).is_err(), "accepted {input}");
    }
    let minimal = r#"{"threadDump":{"threadContainers":[{"container":"<root>","parent":null,"owner":null,"threads":[{"tid":1,"name":"worker"}]}]}}"#;
    assert!(crate::java::threads_limited(minimal, 128).is_err());
}

#[test]
fn sanitized_real_jdk21_json_fixture_keeps_unreported_virtual_type_unknown() {
    let input = include_str!("../tests/fixtures/jcmd-thread-dump-jdk21.json");
    let report: Value = serde_json::from_str(&crate::java::threads(input).unwrap()).unwrap();
    assert_eq!(report["metadata"]["runtimeVersion"], "21.0.12+1-LTS");
    assert_eq!(report["containerCount"], 5);
    assert!(report["threadCount"].as_u64().unwrap_or_default() > 0);
    assert_eq!(report["virtualThreadCount"], 0);
    assert_eq!(report["virtualTypeUnknownCount"], report["threadCount"]);
    let virtual_stack = report["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["name"] == "zi-json-virtual-fixture")
        .expect("real JDK 21 dump contains the running virtual thread");
    assert_eq!(virtual_stack["threadType"], "unknown");
    assert!(
        virtual_stack["frames"]
            .to_string()
            .contains("VirtualThread")
    );
}

#[test]
fn gradle_dependency_insight_reconstructs_reported_reverse_paths() {
    let input = r#"> Task :dependencyInsight
commons-codec:commons-codec:1.7
  Variant default:
    | Attribute Name    | Provided | Requested |
    |-------------------|----------|-----------|
    | org.gradle.status | release  |           |
  Selection reasons:
     - By conflict resolution: between versions 1.7 and 1.6

commons-codec:commons-codec:1.7
\--- scm

commons-codec:commons-codec:1.6 -> 1.7
\--- org.apache.httpcomponents:httpclient:4.3.6
     \--- org.eclipse.jgit:org.eclipse.jgit:4.9.2.201712150930-r
          \--- scm"#;
    let report: Value = serde_json::from_str(&crate::java::dependencies(input).unwrap()).unwrap();
    let insight = &report["dependencyInsight"];
    assert_eq!(insight["targetCount"], 2);
    assert_eq!(insight["truncated"], false);
    let paths = insight["paths"].as_array().unwrap();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0]["target"], "commons-codec:commons-codec");
    assert_eq!(paths[0]["requested"], "1.7");
    assert_eq!(paths[0]["selected"], "1.7");
    assert_eq!(
        paths[0]["rootToTarget"],
        serde_json::json!(["scm", "commons-codec:commons-codec:1.7"])
    );
    assert_eq!(paths[1]["requested"], "1.6");
    assert_eq!(
        paths[1]["rootToTarget"],
        serde_json::json!([
            "scm",
            "org.eclipse.jgit:org.eclipse.jgit:4.9.2.201712150930-r",
            "org.apache.httpcomponents:httpclient:4.3.6",
            "commons-codec:commons-codec:1.6"
        ])
    );

    let regular = crate::java::dependencies("+--- org.example:demo:1.0").unwrap();
    let regular: Value = serde_json::from_str(&regular).unwrap();
    assert!(regular.get("dependencyInsight").is_none());

    let mut many_paths = String::new();
    for index in 0..129 {
        many_paths.push_str(&format!(
            "Selection reasons:\norg.example:library-{index}:1.0\n\\--- configuration-{index}\n"
        ));
    }
    let report: Value =
        serde_json::from_str(&crate::java::dependencies(&many_paths).unwrap()).unwrap();
    assert_eq!(
        report["dependencyInsight"]["paths"]
            .as_array()
            .unwrap()
            .len(),
        128
    );
    assert_eq!(report["dependencyInsight"]["truncated"], true);
}

#[test]
fn openapi_local_schema_refs_propagate_changes_and_report_cycles_without_fetching() {
    let make_document = |profile_type: &str| {
        serde_json::json!({
            "openapi":"3.1.1",
            "paths":{
                "/users":{
                    "post":{
                        "requestBody":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/User"}}}},
                        "responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/User"}}}}}
                    }
                }
            },
            "components":{"schemas":{
                "User":{"type":"object","properties":{
                    "profile":{"$ref":"#/components/schemas/Profile"},
                    "self":{"$ref":"#/components/schemas/User"},
                    "exampleData":{"type":"object","example":{"$ref":"https://example.invalid/payload.json"}}
                }},
                "Profile":{"type":"object","properties":{"name":{"type":profile_type}}},
                "Remote":{"$ref":"https://example.invalid/schemas/Remote.json#/Widget"},
                "Foo/Bar":{"type":"string"}
            }}
        })
    };
    let old = make_document("string");
    let new = make_document("integer");
    let report: Value = serde_json::from_str(
        &crate::contracts::openapi(&old.to_string(), &new.to_string()).unwrap(),
    )
    .unwrap();

    let operation = &report["operationChanges"][0];
    assert_eq!(operation["operation"], "POST /users");
    assert_eq!(operation["change"], "changed");
    assert!(
        operation["fields"]
            .to_string()
            .contains("/profile/properties/name/type")
    );
    assert!(
        !operation["fields"]
            .to_string()
            .contains("example.invalid/payload")
    );
    assert!(
        !report["referenceAnalysis"]["left"]["cycles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !report["referenceAnalysis"]["right"]["cycles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        report["referenceAnalysis"]["left"]["unresolvedSchemaRefs"][0]["reference"],
        "https://example.invalid/schemas/Remote.json#/Widget"
    );
    assert_eq!(report["referenceAnalysis"]["left"]["truncated"], false);

    let escaped_pointer = serde_json::json!({
        "openapi":"3.0.3",
        "paths":{"/escaped":{"get":{"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Foo~1Bar"}}}}}}}},
        "components":{"schemas":{"Foo/Bar":{"type":"string"}}}
    })
    .to_string();
    let escaped: Value = serde_json::from_str(
        &crate::contracts::openapi(&escaped_pointer, &escaped_pointer).unwrap(),
    )
    .unwrap();
    assert_eq!(
        escaped["referenceAnalysis"]["left"]["resolvedSchemaRefs"],
        1
    );
    assert!(escaped["operationChanges"].as_array().unwrap().is_empty());

    let make_sibling_document = |version: &str, sibling_type: &str| {
        serde_json::json!({
            "openapi":version,
            "paths":{"/sibling":{"get":{"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Wrapper"}}}}}}}},
            "components":{"schemas":{
                "Base":{"type":"object","properties":{"name":{"type":"string"}}},
                "Wrapper":{"$ref":"#/components/schemas/Base","properties":{"name":{"type":sibling_type}}}
            }}
        })
    };
    let legacy_left = make_sibling_document("3.0.3", "string");
    let legacy_right = make_sibling_document("3.0.3", "integer");
    let legacy: Value = serde_json::from_str(
        &crate::contracts::openapi(&legacy_left.to_string(), &legacy_right.to_string()).unwrap(),
    )
    .unwrap();
    assert!(legacy["operationChanges"].as_array().unwrap().is_empty());
    assert!(legacy["componentChanges"].as_array().unwrap().is_empty());

    let modern_left = make_sibling_document("3.1.1", "string");
    let modern_right = make_sibling_document("3.1.1", "integer");
    let modern: Value = serde_json::from_str(
        &crate::contracts::openapi(&modern_left.to_string(), &modern_right.to_string()).unwrap(),
    )
    .unwrap();
    assert_eq!(modern["operationChanges"][0]["change"], "changed");
    assert!(
        modern["operationChanges"][0]["fields"]
            .to_string()
            .contains("/schema/allOf"),
        "unexpected reference propagation diff: {}",
        modern["operationChanges"][0]["fields"]
    );

    let boolean_schema = serde_json::json!({
        "openapi":"3.1.1",
        "paths":{"/boolean":{"get":{"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Denied"}}}}}}}},
        "components":{"schemas":{"Denied":false}}
    });
    let boolean_report: Value = serde_json::from_str(
        &crate::contracts::openapi(&boolean_schema.to_string(), &boolean_schema.to_string())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        boolean_report["referenceAnalysis"]["left"]["resolvedSchemaRefs"],
        1
    );
    assert!(
        boolean_report["referenceAnalysis"]["left"]["unresolvedSchemaRefs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn openapi_schema_ref_expansion_budget_is_reported() {
    let mut properties = serde_json::Map::new();
    for index in 0..4_097 {
        properties.insert(
            format!("field{index}"),
            serde_json::json!({"$ref":"#/components/schemas/Leaf"}),
        );
    }
    let document = serde_json::json!({
        "openapi":"3.1.1",
        "paths":{"/bounded":{"get":{"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Wide"}}}}}}}},
        "components":{"schemas":{
            "Leaf":{"type":"string"},
            "Wide":{"type":"object","properties":properties}
        }}
    });
    let report: Value = serde_json::from_str(
        &crate::contracts::openapi(&document.to_string(), &document.to_string()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["referenceAnalysis"]["left"]["resolvedSchemaRefs"],
        4_096
    );
    assert_eq!(report["referenceAnalysis"]["left"]["truncated"], true);
}

#[test]
fn jfr_builds_bounded_root_to_leaf_inclusive_sample_call_tree() {
    let frame =
        |class: &str, name: &str| serde_json::json!({"method":{"type":{"name":class},"name":name}});
    let stack = serde_json::json!([
        frame("app/Worker", "work"),
        frame("app/Service", "handle"),
        frame("app/Main", "run")
    ]);
    let event = || serde_json::json!({"type":"jdk.ExecutionSample","values":{"stackTrace":{"frames":stack.clone()}}});
    let input = serde_json::json!({"recording":{"events":[event(),event()]}}).to_string();
    let report: Value = serde_json::from_str(&crate::java::jfr(&input).unwrap()).unwrap();
    let tree = &report["sampleCallTree"];
    let nodes = tree["nodes"].as_array().unwrap();

    assert_eq!(tree["rootToLeaf"], true);
    assert_eq!(tree["samples"], 2);
    assert_eq!(tree["truncated"], false);
    assert_eq!(nodes.len(), 3);
    assert_eq!(nodes[0]["method"], "app/Main.run");
    assert_eq!(nodes[0]["parentIndex"], Value::Null);
    assert_eq!(nodes[1]["method"], "app/Service.handle");
    assert_eq!(nodes[1]["parentIndex"], 0);
    assert_eq!(nodes[2]["method"], "app/Worker.work");
    assert_eq!(nodes[2]["parentIndex"], 1);
    assert!(nodes.iter().all(|node| node["inclusiveSamples"] == 2));
    assert_eq!(report["sampleTopFrames"][0]["method"], "app/Worker.work");

    let deep_frames: Vec<_> = (0..129)
        .map(|index| frame("deep/Stack", &format!("frame{index}")))
        .collect();
    let deep = serde_json::json!({"recording":{"events":[{"type":"jdk.ExecutionSample","values":{"stackTrace":{"frames":deep_frames}}}]}}).to_string();
    let deep_report: Value = serde_json::from_str(&crate::java::jfr(&deep).unwrap()).unwrap();
    assert_eq!(
        deep_report["sampleCallTree"]["nodes"]
            .as_array()
            .unwrap()
            .len(),
        128
    );
    assert_eq!(deep_report["sampleCallTree"]["truncated"], true);

    let wide_events: Vec<_> = (0..257)
        .map(|index| {
            serde_json::json!({"type":"jdk.ExecutionSample","values":{"stackTrace":{"frames":[frame("wide/Stack", &format!("frame{index}"))]}}})
        })
        .collect();
    let wide = serde_json::json!({"recording":{"events":wide_events}}).to_string();
    let wide_report: Value = serde_json::from_str(&crate::java::jfr(&wide).unwrap()).unwrap();
    assert_eq!(
        wide_report["sampleCallTree"]["nodes"]
            .as_array()
            .unwrap()
            .len(),
        256
    );
    assert_eq!(wide_report["sampleCallTree"]["truncated"], true);

    let missing_stack =
        serde_json::json!({"recording":{"events":[{"type":"jdk.ExecutionSample","values":{}}]}})
            .to_string();
    let missing_report: Value =
        serde_json::from_str(&crate::java::jfr(&missing_stack).unwrap()).unwrap();
    assert_eq!(missing_report["sampleCallTree"]["truncated"], true);
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
