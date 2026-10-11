use super::*;

fn request(capability: &str, input: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "contractVersion":CONTRACT_VERSION,"pluginId":"com.zicode.devtools.compare",
        "sceneId":"coding","capabilityId":capability,"commandId":null,"input":input
    }))
    .unwrap()
}

#[test]
fn text_reports_and_line_boundary() {
    let cases = [
        (
            "same".to_owned(),
            "same".to_owned(),
            Some("两份文本完全相同"),
        ),
        (
            "a\n".to_owned(),
            "b\n".to_owned(),
            Some("--- 原文\n+++ 修改后\n@@ -1 +1 @@\n-a\n+b\n"),
        ),
        ("\n".repeat(5001), String::new(), None),
    ];
    for (left, right, expected) in cases {
        let output = execute_request(&request(
            "devtools.compare.text.diff",
            serde_json::json!({"left":left,"right":right}),
        ));
        let result: Envelope = serde_json::from_slice(&output).unwrap();
        assert_eq!(result.contract_version, CONTRACT_VERSION);
        if let Some(expected) = expected {
            assert_eq!(result.data.unwrap().text, expected);
            assert!(result.ok)
        } else {
            assert!(!result.ok);
            assert_eq!(result.error.unwrap().code, "INVALID_INPUT")
        }
    }
}

#[test]
fn strict_input_and_old_protocol_rejection() {
    let valid = request(
        "devtools.compare.text.diff",
        serde_json::json!({"left":"a","right":"a"}),
    );
    for version in ["1.1.0-proposal.2", "1.0.0-rc.1"] {
        let bytes = String::from_utf8(valid.clone())
            .unwrap()
            .replace(CONTRACT_VERSION, version);
        let result: Envelope = serde_json::from_slice(&execute_request(bytes.as_bytes())).unwrap();
        assert!(!result.ok);
        assert_eq!(result.error.unwrap().code, "INVALID_INPUT");
    }
    for input in [
        serde_json::json!({"left":"x"}),
        serde_json::json!({"left":"a","right":"b","extra":true}),
        serde_json::json!({"left":1,"right":"b"}),
    ] {
        let result: Envelope = serde_json::from_slice(&execute_request(&request(
            "devtools.compare.text.diff",
            input,
        )))
        .unwrap();
        assert!(!result.ok);
    }
}

#[test]
fn four_operations_use_original_implementation() {
    for (capability, input) in [
        (
            "devtools.compare.json.path",
            serde_json::json!({"text":"[1]","query":"$[0]"}),
        ),
        (
            "devtools.compare.json.diff.ordered",
            serde_json::json!({"left":"[1]","right":"[2]"}),
        ),
        (
            "devtools.compare.json.diff.unordered",
            serde_json::json!({"left":"[1,2]","right":"[2,1]"}),
        ),
        (
            "devtools.compare.regex.matches",
            serde_json::json!({"text":"aa","pattern":"a"}),
        ),
    ] {
        let bytes = request(capability, input);
        let old = String::from_utf8(bytes.clone())
            .unwrap()
            .replace(CONTRACT_VERSION, zi_fields_wasi::CONTRACT_VERSION);
        let expected = String::from_utf8(zi_fields_wasi::execute_request(old.as_bytes()))
            .unwrap()
            .replace(zi_fields_wasi::CONTRACT_VERSION, CONTRACT_VERSION);
        assert_eq!(execute_request(&bytes), expected.as_bytes());
    }
}
