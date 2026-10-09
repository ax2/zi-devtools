use super::*;
#[path = "legacy.rs"]
mod legacy;

fn outcome(result: Result<String>) -> std::result::Result<String, String> {
    result.map_err(|error| format!("{error:#}"))
}

#[test]
fn complete_path_semantics_and_errors_match_original() {
    let inputs = [
        r#"{"项":[{"a.b":1},{"a.b":2}]}"#,
        r#"{"b":2,"a":1}"#,
        r#"{"x":1,"x":2}"#,
        r#"{"x":[null,true,9007199254740993]}"#,
        r#"{"a\"b":{"/":3}}"#,
        "[]",
        "null",
        "false",
        "1",
        "{}",
        "not-json",
    ];
    let paths = [
        "$",
        "  $  ",
        "$.项[*][\"a.b\"]",
        "$[*]",
        "$.x",
        "$.x[0]",
        "$.x[03]",
        "$[\"a\\\"b\"][\"/\"]",
        "$.missing[0]",
        "$..a",
        "$[-1]",
        "$[?(@.x)]",
        "$[1:2]",
        "$[",
        "$[\"x\"",
        "$.a b",
        "",
        "a",
        "$[999999999999999999999999999]",
    ];
    for input in inputs {
        for path in paths {
            assert_eq!(
                outcome(json_path(input, path)),
                outcome(legacy::json_path(input, path)),
                "{input} {path}"
            );
        }
    }
    assert_eq!(
        json_path(r#"{"项":[{"a.b":1},{"a.b":2}]}"#, r#"$.项[*]["a.b"]"#).unwrap(),
        "[\n  1,\n  2\n]"
    );
    assert_eq!(
        json_path(r#"{"b":2,"a":1}"#, "$[*]").unwrap(),
        "[\n  1,\n  2\n]"
    );
    assert_eq!(json_path("{}", "$.missing[0]").unwrap(), "[]");
}

#[test]
fn query_bytes_steps_and_match_limits_are_exact() {
    assert_eq!(
        json_path("{}", &format!("${}", ".a".repeat(128))).unwrap(),
        "[]"
    );
    assert!(
        json_path("{}", &format!("${}", ".a".repeat(129)))
            .unwrap_err()
            .to_string()
            .contains("128")
    );
    let exact = format!("$.{}", "a".repeat(4094));
    assert_eq!(json_path("{}", &exact).unwrap(), "[]");
    assert!(
        json_path("{}", &(exact + "a"))
            .unwrap_err()
            .to_string()
            .contains("4096")
    );
    // UTF-8 bytes, not character count.
    assert!(
        json_path("{}", &format!("$.{}", "中".repeat(1365)))
            .unwrap_err()
            .to_string()
            .contains("4096")
    );
    for count in [10_000, 10_001] {
        let input = serde_json::to_string(&vec![0; count]).unwrap();
        let result = json_path(&input, "$[*]");
        assert_eq!(
            outcome(json_path(&input, "$[*]")),
            outcome(legacy::json_path(&input, "$[*]"))
        );
        if count == 10_000 {
            assert_eq!(
                serde_json::from_str::<Vec<Value>>(&result.unwrap())
                    .unwrap()
                    .len(),
                count
            );
        } else {
            assert!(result.unwrap_err().to_string().contains("10000"));
        }
    }
}

#[test]
fn ordered_unordered_diff_and_pointer_escaping_match_original() {
    let values = [
        "null",
        "true",
        "1",
        "2.0",
        "[]",
        "[1,2]",
        "[2,1]",
        "[1,1,2]",
        "[1,2,2]",
        r#"{"a/b~":1,"项":[1,2]}"#,
        r#"{"a/b~":null,"项":[2,1]}"#,
        r#"{"x":9007199254740993}"#,
        r#"{"a":1,"a":2}"#,
        "not-json",
    ];
    for left in values {
        for right in values {
            for unordered in [false, true] {
                assert_eq!(
                    outcome(json_diff(left, right, unordered)),
                    outcome(legacy::json_diff(left, right, unordered)),
                    "{left} {right} {unordered}"
                );
            }
        }
    }
    let report: Value =
        serde_json::from_str(&json_diff(r#"{"a/b~":1}"#, r#"{"a/b~":null}"#, false).unwrap())
            .unwrap();
    assert_eq!(report["changes"][0]["path"], "/a~1b~0");
    let report: Value = serde_json::from_str(&json_diff("[1,2]", "[2,1]", true).unwrap()).unwrap();
    assert_eq!(report["equal"], true);
    let report: Value =
        serde_json::from_str(&json_diff("[1,1,2]", "[1,2,2]", true).unwrap()).unwrap();
    assert_eq!(report["equal"], false);
}

#[test]
fn input_and_change_boundaries_preserve_full_standalone_range() {
    let exact = format!("\"{}\"", "a".repeat(1024 * 1024 - 2));
    assert!(json_path(&exact, "$").is_ok());
    assert!(json_diff(&exact, &exact, false).is_ok());
    let over = exact + " ";
    assert!(
        json_path(&over, "$")
            .unwrap_err()
            .to_string()
            .contains("1 MiB")
    );
    assert!(
        json_diff(&over, "null", false)
            .unwrap_err()
            .to_string()
            .contains("1 MiB")
    );
    assert!(
        json_diff("null", &over, false)
            .unwrap_err()
            .to_string()
            .contains("1 MiB")
    );
    for count in [10_000, 10_001] {
        let left = serde_json::to_string(&vec![0; count]).unwrap();
        let right = serde_json::to_string(&vec![1; count]).unwrap();
        let result = json_diff(&left, &right, false);
        assert_eq!(
            outcome(json_diff(&left, &right, false)),
            outcome(legacy::json_diff(&left, &right, false))
        );
        if count == 10_000 {
            let report: Value = serde_json::from_str(&result.unwrap()).unwrap();
            assert_eq!(report["changes"].as_array().unwrap().len(), count);
        } else {
            assert!(result.unwrap_err().to_string().contains("10000"));
        }
    }
}

#[test]
fn array_index_semantics_do_not_depend_on_pointer_width() {
    for path in [
        "$[4294967296]",
        "$[18446744073709551615]",
        "$.missing[4294967296]",
    ] {
        assert_eq!(json_path("[1]", path).unwrap(), "[]");
        #[cfg(target_pointer_width = "64")]
        assert_eq!(
            outcome(json_path("[1]", path)),
            outcome(legacy::json_path("[1]", path))
        );
    }
    assert!(
        json_path("[]", "$[18446744073709551616]")
            .unwrap_err()
            .to_string()
            .contains("数组索引过大")
    );
}
#[test]
fn result_budget_counts_only_actual_changes_and_preserves_full_report() {
    use crate::{ReportTooLarge, json_diff_with_result_budget};
    let prefix = "x".repeat(6000);
    let left_values: serde_json::Map<String, serde_json::Value> = (0..100)
        .map(|i| (format!("key{i}"), serde_json::json!(0)))
        .collect();
    let right_values: serde_json::Map<String, serde_json::Value> = (0..100)
        .map(|i| (format!("key{i}"), serde_json::json!(1)))
        .collect();
    let left = serde_json::to_string(&serde_json::json!({(prefix.clone()):left_values})).unwrap();
    let right = serde_json::to_string(&serde_json::json!({(prefix):right_values})).unwrap();
    for unordered in [false, true] {
        let full = crate::json_diff(&left, &right, unordered).unwrap();
        assert!(full.len() > 48 * 1024);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&full).unwrap()["changes"]
                .as_array()
                .unwrap()
                .len(),
            100
        );
        assert!(
            json_diff_with_result_budget(&left, &right, unordered, 48 * 1024)
                .unwrap_err()
                .is::<ReportTooLarge>()
        );
        assert_eq!(
            json_diff_with_result_budget(&left, &left, unordered, 0).unwrap(),
            crate::json_diff(&left, &left, unordered).unwrap()
        );
        assert!(
            !json_diff_with_result_budget(&left, "{", unordered, 0)
                .unwrap_err()
                .is::<ReportTooLarge>()
        );
    }
    // /a~1b~0 is seven bytes: exact lower bound remains eligible for final serialization.
    assert!(json_diff_with_result_budget("{\"a/b~\":0}", "{}", false, 7).is_ok());
    assert!(
        json_diff_with_result_budget("{\"a/b~\":0}", "{}", false, 6)
            .unwrap_err()
            .is::<ReportTooLarge>()
    );
}
