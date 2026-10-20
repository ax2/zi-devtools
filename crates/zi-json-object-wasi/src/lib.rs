//! Bounded structured JSON object provider and receiver. Host permissions and
//! result storage remain outside this WASI adapter.
use serde_json::{Value, json};

pub const PLUGIN_ID: &str = "com.zicode.devtools.json-object";
pub const INSPECT: &str = "devtools.json.object.inspect";
pub const RECEIVE: &str = "devtools.json.object.receive";
pub const CONTRACT_VERSION: &str = "1.0.0-rc.1";
pub const REQUEST_LIMIT: usize = 48 * 1024;
pub const OBJECT_LIMIT: usize = 8 * 1024;
pub const RESULT_LIMIT: usize = 48 * 1024;

fn failure(code: &'static str, message: &'static str) -> Value {
    json!({"contractVersion":CONTRACT_VERSION,"ok":false,"data":null,"error":{"code":code,"message":message}})
}

fn execute(bytes: &[u8]) -> Value {
    if bytes.len() > REQUEST_LIMIT {
        return failure("INPUT_TOO_LARGE", "请求超过 48 KiB");
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return failure("INVALID_INPUT", "请求不是 UTF-8 JSON");
    };
    let Ok(request) = zi_text_core::parse_strict_json(text) else {
        return failure("INVALID_INPUT", "请求结构无效");
    };
    let Some(request) = request.as_object() else {
        return failure("INVALID_INPUT", "请求结构无效");
    };
    if request.len() != 5
        || request.get("pluginId").and_then(Value::as_str) != Some(PLUGIN_ID)
        || request
            .get("sceneId")
            .and_then(Value::as_str)
            .is_none_or(|id| id.is_empty() || id.len() > 128)
        || request.get("commandId") != Some(&Value::Null)
    {
        return failure("INVALID_INPUT", "插件或场景标识无效");
    }
    let Some(capability) = request.get("capabilityId").and_then(Value::as_str) else {
        return failure("INVALID_INPUT", "能力标识无效");
    };
    if ![INSPECT, RECEIVE].contains(&capability) {
        return failure("UNSUPPORTED_OPERATION", "不支持此能力");
    }
    let Some(input) = request.get("input").and_then(Value::as_object) else {
        return failure("INVALID_INPUT", "输入必须是包含 value 对象的结构");
    };
    if input.len() != 1 {
        return failure("INVALID_INPUT", "输入字段无效");
    }
    let Some(value) = input.get("value").filter(|value| value.is_object()) else {
        return failure("INVALID_INPUT", "value 必须是 JSON object");
    };
    let Ok(serialized) = serde_json::to_vec(value) else {
        return failure("INVALID_INPUT", "value 无法序列化");
    };
    if serialized.len() > OBJECT_LIMIT {
        return failure("INPUT_TOO_LARGE", "JSON object 超过 8 KiB");
    }

    let data = if capability == INSPECT {
        match zi_json_core::object_summary(value) {
            Ok(summary) => summary,
            Err(_) => return failure("INVALID_INPUT", "JSON object 超出结构限制"),
        }
    } else {
        let Some(object) = value.as_object() else {
            unreachable!()
        };
        let keys: Vec<_> = object.keys().cloned().collect();
        json!({"receivedValue":value,"rootKeyCount":object.len(),"rootKeys":keys})
    };
    let output = json!({"contractVersion":CONTRACT_VERSION,"ok":true,"data":data,"error":null});
    if serde_json::to_vec(&output).is_ok_and(|bytes| bytes.len() <= RESULT_LIMIT) {
        output
    } else {
        failure("OUTPUT_TOO_LARGE", "结果超过 48 KiB")
    }
}

/// Execute one strict request and serialize exactly one bounded JSON response.
pub fn execute_request(bytes: &[u8]) -> Vec<u8> {
    let output = execute(bytes);
    serde_json::to_vec(&output).unwrap_or_else(|_| b"{\"contractVersion\":\"1.0.0-rc.1\",\"ok\":false,\"data\":null,\"error\":{\"code\":\"INTERNAL_ERROR\",\"message\":\"processing failed\"}}".to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(capability: &str, value: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({"pluginId":PLUGIN_ID,"sceneId":"test","capabilityId":capability,"commandId":null,"input":{"value":value}})).unwrap()
    }

    #[test]
    fn inspect_and_receiver_keep_the_object_typed_and_unchanged() {
        let original = json!({"名称":"Zi","active":true,"nested":{"count":3},"items":[null,1]});
        let inspected: Value =
            serde_json::from_slice(&execute_request(&request(INSPECT, original.clone()))).unwrap();
        assert_eq!(inspected["contractVersion"], CONTRACT_VERSION);
        assert_eq!(inspected["ok"], true);
        assert_eq!(inspected["error"], Value::Null);
        assert_eq!(inspected["data"]["value"], original);
        assert_eq!(inspected["data"]["keyCount"], 4);
        assert_eq!(inspected["data"]["valueTypes"]["nested"], "object");

        let received: Value = serde_json::from_slice(&execute_request(&request(
            RECEIVE,
            inspected["data"].clone(),
        )))
        .unwrap();
        assert_eq!(received["contractVersion"], CONTRACT_VERSION);
        assert_eq!(received["ok"], true);
        assert_eq!(received["error"], Value::Null);
        assert_eq!(received["data"]["receivedValue"], inspected["data"]);
        assert!(received["data"]["receivedValue"].is_object());
    }

    #[test]
    fn rejects_wrong_shapes_duplicates_and_limits() {
        for value in [json!("text"), json!([1, 2]), Value::Null] {
            let response: Value =
                serde_json::from_slice(&execute_request(&request(INSPECT, value))).unwrap();
            assert_eq!(response["error"]["code"], "INVALID_INPUT");
            assert_eq!(response["contractVersion"], CONTRACT_VERSION);
            assert_eq!(response["data"], Value::Null);
            assert_eq!(response.as_object().unwrap().len(), 4);
        }
        let duplicate = format!(
            "{{\"pluginId\":\"{PLUGIN_ID}\",\"sceneId\":\"x\",\"capabilityId\":\"{INSPECT}\",\"commandId\":null,\"input\":{{\"value\":{{\"a\":1,\"a\":2}}}}}}"
        );
        let response: Value =
            serde_json::from_slice(&execute_request(duplicate.as_bytes())).unwrap();
        assert_eq!(response["error"]["code"], "INVALID_INPUT");
        assert_eq!(response["contractVersion"], CONTRACT_VERSION);
        assert_eq!(response["data"], Value::Null);

        let large = json!({"v":"x".repeat(OBJECT_LIMIT)});
        let response: Value =
            serde_json::from_slice(&execute_request(&request(INSPECT, large))).unwrap();
        assert_eq!(response["error"]["code"], "INPUT_TOO_LARGE");
        assert_eq!(response["data"], Value::Null);

        let mut extra = json!({"pluginId":PLUGIN_ID,"sceneId":"x","capabilityId":INSPECT,"commandId":null,"input":{"value":{}}});
        extra
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), Value::Bool(true));
        let response: Value =
            serde_json::from_slice(&execute_request(&serde_json::to_vec(&extra).unwrap())).unwrap();
        assert_eq!(response["error"]["code"], "INVALID_INPUT");
        assert_eq!(response["data"], Value::Null);
    }

    #[test]
    fn json_object_output_examples_match_capture_and_reject_non_objects() {
        let output_schema: Value = serde_json::from_str(include_str!(
            "../../../plugins/json-object/output.schema.json"
        ))
        .unwrap();
        let manifest: Value =
            serde_json::from_str(include_str!("../../../plugins/json-object/plugin.json")).unwrap();
        let examples: Value = serde_json::from_str(include_str!(
            "../../../plugins/json-object/fixtures/result-json-cases.json"
        ))
        .unwrap();
        let good: Value =
            serde_json::from_slice(&execute_request(&request(INSPECT, json!({"x":1})))).unwrap();
        assert_eq!(output_schema["properties"]["data"]["type"], "object");
        assert_eq!(
            output_schema["properties"]["contractVersion"]["const"],
            CONTRACT_VERSION
        );
        assert_eq!(output_schema["properties"]["error"]["type"], "null");
        assert_eq!(
            output_schema["required"],
            json!(["contractVersion", "ok", "data", "error"])
        );
        assert!(good["ok"].as_bool().unwrap());
        assert!(good["data"].is_object());
        assert_eq!(
            manifest["requiredProfiles"]["result.json.v1"],
            "=1.0.0-preview.1"
        );
        let capabilities = manifest["contributes"]["capabilities"].as_array().unwrap();
        assert_eq!(capabilities.len(), 2);
        assert!(capabilities.iter().all(|capability| {
            capability["resultCaptureMimeType"] == "application/json"
                && capability["outputSchema"]["properties"]["data"]["type"] == "object"
                && capability["outputSchema"]["properties"]["contractVersion"]["const"]
                    == CONTRACT_VERSION
                && capability["outputSchema"]["properties"]["error"]["type"] == "null"
                && capability["inputSchema"]["properties"]["value"]["type"] == "object"
        }));
        let pi_tools = manifest["contributes"]["piTools"].as_array().unwrap();
        assert_eq!(pi_tools.len(), 1);
        assert_eq!(pi_tools[0]["capability"], INSPECT);
        assert_eq!(pi_tools[0]["inputSchema"], capabilities[0]["inputSchema"]);
        assert_eq!(
            manifest["permissions"]["optional"][0]["name"],
            "pi.tools.register"
        );
        let valid = &examples["validSuccess"];
        assert!(valid["ok"].as_bool().unwrap() && valid["data"].is_object());
        assert_eq!(valid["contractVersion"], CONTRACT_VERSION);
        assert_eq!(valid["error"], Value::Null);
        let invalid = &examples["invalidSuccessWithNonObjectData"];
        assert!(!invalid["data"].is_object());
        assert_eq!(invalid["data"].as_str(), Some("not-an-object"));
        assert!(examples["invalidSuccessMissingData"].get("data").is_none());
        let failure = &examples["validFailure"];
        assert_eq!(failure["contractVersion"], CONTRACT_VERSION);
        assert_eq!(failure["ok"], false);
        assert_eq!(failure["data"], Value::Null);
        assert!(failure["error"].is_object());
        let oversized = &examples["validLimitFailure"];
        assert_eq!(oversized["ok"], false);
        assert_eq!(oversized["data"], Value::Null);
        assert_eq!(oversized["error"]["code"], "INPUT_TOO_LARGE");
        let invalid_failure = &examples["invalidFailureWithData"];
        assert_eq!(invalid_failure["ok"], false);
        assert!(invalid_failure["data"].is_object());
    }
}
