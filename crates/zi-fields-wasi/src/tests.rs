use super::*;
use serde_json::{Value, json};
fn request(capability: &str, input: Value) -> Value {
    json!({"contractVersion":CONTRACT_VERSION,"pluginId":"com.zicode.devtools.compare","sceneId":"coding","capabilityId":capability,"commandId":null,"input":input})
}
fn run(value: &Value) -> Value {
    serde_json::from_slice(&execute_request(&serde_json::to_vec(value).unwrap())).unwrap()
}
#[test]
fn all_three_operations_use_complete_shared_core() {
    for (cap, input, expected) in [
        (
            OPERATIONS[0],
            json!({"text":"{\"项\":[1,2]}","query":"$.项[*]"}),
            zi_json_core::json_path("{\"项\":[1,2]}", "$.项[*]").unwrap(),
        ),
        (
            OPERATIONS[1],
            json!({"left":"[1,2]","right":"[2,1]"}),
            zi_json_core::json_diff("[1,2]", "[2,1]", false).unwrap(),
        ),
        (
            OPERATIONS[2],
            json!({"left":"[1,1,2]","right":"[1,2,2]"}),
            zi_json_core::json_diff("[1,1,2]", "[1,2,2]", true).unwrap(),
        ),
    ] {
        let result = run(&request(cap, input));
        assert_eq!(result["ok"], true);
        assert_eq!(result["data"]["text"], expected);
        assert_eq!(result["contractVersion"], CONTRACT_VERSION);
    }
}
#[test]
fn identity_required_fields_types_and_duplicate_fields_are_rejected() {
    let seed = request(OPERATIONS[0], json!({"text":"{}","query":"$"}));
    for key in [
        "contractVersion",
        "pluginId",
        "sceneId",
        "capabilityId",
        "commandId",
        "input",
    ] {
        let mut missing = seed.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert_eq!(
            run(&missing)["error"]["code"],
            "INVALID_INPUT",
            "missing {key}"
        );
    }
    for (key, value) in [
        ("contractVersion", json!("1.0.0-rc.1")),
        ("pluginId", json!("com.zicode.devtools.text")),
        ("sceneId", json!("")),
        ("commandId", json!("run")),
        ("input", json!({"text":"{}"})),
        ("input", json!({"text":"{}","query":1})),
        ("input", json!({"text":"{}","query":"$","extra":true})),
    ] {
        let mut wrong = seed.clone();
        wrong[key] = value;
        assert_eq!(run(&wrong)["error"]["code"], "INVALID_INPUT");
    }
    let raw = serde_json::to_string(&seed)
        .unwrap()
        .replace("\"query\":\"$\"", "\"query\":\"$\",\"query\":\"$.x\"");
    let output: Value = serde_json::from_slice(&execute_request(raw.as_bytes())).unwrap();
    assert_eq!(output["error"]["code"], "INVALID_INPUT");
    assert_eq!(
        run(&request(
            "devtools.compare.regex.matches",
            json!({"text":"a","pattern":"a"})
        ))["error"]["code"],
        "UNSUPPORTED_OPERATION"
    );
}
#[test]
fn byte_limits_are_checked_before_algorithm_execution() {
    let mut seed = request(
        OPERATIONS[0],
        json!({"text":"{}","query":format!("$.{}","a".repeat(4094))}),
    );
    assert_eq!(run(&seed)["ok"], true);
    seed["input"]["query"] = json!(format!("$.{}", "a".repeat(4095)));
    assert_eq!(run(&seed)["error"]["code"], "INPUT_TOO_LARGE");
    seed["input"]["query"] = json!("中".repeat(1366));
    assert_eq!(run(&seed)["error"]["code"], "INPUT_TOO_LARGE");
    let large = json!({"left":"\"".repeat(8193),"right":"null"});
    assert_eq!(
        run(&request(OPERATIONS[1], large))["error"]["code"],
        "INPUT_TOO_LARGE"
    );
    let raw = vec![b' '; REQUEST_LIMIT + 1];
    let result: Value = serde_json::from_slice(&execute_request(&raw)).unwrap();
    assert_eq!(result["error"]["code"], "INPUT_TOO_LARGE");
}

#[test]
fn large_shared_core_report_is_rejected_after_real_serialization() {
    let prefix = "x".repeat(1000);
    let left_values: serde_json::Map<String, Value> =
        (0..100).map(|i| (format!("key{i}"), json!(0))).collect();
    let right_values: serde_json::Map<String, Value> =
        (0..100).map(|i| (format!("key{i}"), json!(1))).collect();
    let left = serde_json::to_string(&json!({(prefix.clone()):left_values})).unwrap();
    let right = serde_json::to_string(&json!({(prefix):right_values})).unwrap();
    assert!(left.len() <= 8192 && right.len() <= 8192);
    let standalone = zi_json_core::json_diff(&left, &right, false).unwrap();
    assert!(standalone.len() > REQUEST_LIMIT);
    let bytes = execute_request(
        &serde_json::to_vec(&request(OPERATIONS[1], json!({"left":left,"right":right}))).unwrap(),
    );
    assert!(bytes.len() <= REQUEST_LIMIT);
    let result: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(result["ok"], false);
    assert!(result["data"].is_null());
    assert_eq!(result["error"]["code"], "INPUT_TOO_LARGE");
    assert_eq!(result["error"]["message"], "结果超过 48 KiB");
}
