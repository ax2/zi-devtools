//! Pure text algorithms and the bounded Studio compute contract. No host access.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{
    Deserialize, Serialize,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Value, value::RawValue};
use sha2::{Digest, Sha256};

pub const REQUEST_LIMIT: usize = 48 * 1024;
pub const TEXT_LIMIT: usize = 8192;
pub const CONTRACT_VERSION: &str = "1.0.0-rc.1";

// Standalone policy preserves its existing wide integers and whitespace handling.
pub fn json(input: &str, pretty: bool) -> Result<String, serde_json::Error> {
    let value: Value = serde_json::from_str(input)?;
    if pretty {
        serde_json::to_string_pretty(&value)
    } else {
        serde_json::to_string(&value)
    }
}
pub fn encode(input: &str) -> String {
    STANDARD.encode(input.as_bytes())
}
pub fn decode(input: &str) -> Result<String, &'static str> {
    let bytes = STANDARD.decode(input).map_err(|_| "Base64 数据无效")?;
    String::from_utf8(bytes).map_err(|_| "解码结果不是 UTF-8 文本")
}
pub fn sha256(input: &str) -> String {
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultEnvelope {
    pub contract_version: &'static str,
    pub ok: bool,
    pub data: Option<Text>,
    pub error: Option<Error>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Text {
    pub text: String,
}
#[derive(Debug, Serialize)]
pub struct Error {
    pub code: &'static str,
    pub message: &'static str,
}
impl ResultEnvelope {
    pub fn failure(code: &'static str, message: &'static str) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            ok: false,
            data: None,
            error: Some(Error { code, message }),
        }
    }
    fn success(text: String) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            ok: true,
            data: Some(Text { text }),
            error: None,
        }
    }
}

// A non-Option wrapper makes both nullable slots REQUIRED during deserialization.
struct Slot(Option<String>);
impl<'de> Deserialize<'de> for Slot {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Nullable;
        impl<'de> Visitor<'de> for Nullable {
            type Value = Slot;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("string or explicit null")
            }
            fn visit_unit<E: de::Error>(self) -> Result<Slot, E> {
                Ok(Slot(None))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Slot, E> {
                Ok(Slot(Some(value.to_owned())))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Slot, E> {
                Ok(Slot(Some(value)))
            }
        }
        deserializer.deserialize_any(Nullable)
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    plugin_id: String,
    scene_id: String,
    capability_id: Slot,
    command_id: Slot,
    input: Text,
}

pub fn execute_request(bytes: &[u8]) -> ResultEnvelope {
    if bytes.len() > REQUEST_LIMIT {
        return ResultEnvelope::failure("INPUT_TOO_LARGE", "请求超过 48 KiB");
    }
    let Ok(request) = serde_json::from_slice::<Request>(bytes) else {
        return ResultEnvelope::failure("INVALID_INPUT", "请求结构无效");
    };
    if request.plugin_id != "com.zicode.devtools.text" || request.scene_id.is_empty() {
        return ResultEnvelope::failure("INVALID_INPUT", "插件或场景标识无效");
    }
    if request.input.text.len() > TEXT_LIMIT {
        return ResultEnvelope::failure("INPUT_TOO_LARGE", "文本超过 8192 UTF-8 字节");
    }
    match (&request.capability_id.0, &request.command_id.0) {
        (Some(cap), None) if !cap.is_empty() => execute(cap, &request.input.text),
        (None, Some(command)) if !command.is_empty() => {
            ResultEnvelope::failure("UNSUPPORTED_OPERATION", "未注册后台命令")
        }
        _ => ResultEnvelope::failure("INVALID_INPUT", "必须指定唯一处理器"),
    }
}

pub fn execute(capability: &str, input: &str) -> ResultEnvelope {
    if input.len() > TEXT_LIMIT {
        return ResultEnvelope::failure("INPUT_TOO_LARGE", "文本超过 8192 UTF-8 字节");
    }
    let result = match capability {
        "devtools.text.base64.encode" => Ok(encode(input)),
        "devtools.text.base64.decode" => {
            decode(input).map_err(|_| ("INVALID_ENCODING", "Base64 或 UTF-8 编码无效"))
        }
        "devtools.text.sha256" => Ok(sha256(input)),
        "devtools.text.json.format" | "devtools.text.json.minify" => strict_json(input, 0)
            .and_then(|value| {
                if capability.ends_with("format") {
                    serde_json::to_string_pretty(&value)
                } else {
                    serde_json::to_string(&value)
                }
            })
            .map_err(|_| ("INVALID_INPUT", "JSON 无效、重复键、深度或整数范围超限")),
        _ => Err(("UNSUPPORTED_OPERATION", "未注册该工具能力")),
    };
    match result {
        Ok(text) => ResultEnvelope::success(text),
        Err((code, message)) => ResultEnvelope::failure(code, message),
    }
}

/// Never emit an oversized success, including escape expansion in JSON strings.
pub fn serialize_result(result: &ResultEnvelope) -> Vec<u8> {
    let bytes = serde_json::to_vec(result).expect("string-only result serialization");
    if bytes.len() <= REQUEST_LIMIT {
        bytes
    } else {
        serde_json::to_vec(&ResultEnvelope::failure(
            "INPUT_TOO_LARGE",
            "结果超过 48 KiB",
        ))
        .expect("fixed result")
    }
}

fn strict_json(input: &str, depth: usize) -> Result<Value, serde_json::Error> {
    // RawValue validates the complete syntax without discarding duplicate keys or
    // rounding integer tokens. Re-parse only children; bounded text/depth keep work finite.
    let raw: &RawValue = serde_json::from_str(input)?;
    let input = raw.get();
    if input.starts_with('{') || input.starts_with('[') {
        if depth >= 64 {
            return Err(de::Error::custom("depth"));
        }
        struct Container(usize);
        impl<'de> Visitor<'de> for Container {
            type Value = Value;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("container")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Value, M::Error> {
                let mut result = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if result.contains_key(&key) {
                        return Err(de::Error::custom("duplicate"));
                    }
                    let raw = map.next_value::<Box<RawValue>>()?;
                    let value = strict_json(raw.get(), self.0 + 1).map_err(de::Error::custom)?;
                    result.insert(key, value);
                }
                result.sort_keys();
                Ok(Value::Object(result))
            }
            fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Value, S::Error> {
                let mut result = Vec::new();
                while let Some(raw) = seq.next_element::<Box<RawValue>>()? {
                    result.push(strict_json(raw.get(), self.0 + 1).map_err(de::Error::custom)?);
                }
                Ok(Value::Array(result))
            }
        }
        let mut parser = serde_json::Deserializer::from_str(input);
        return serde::Deserializer::deserialize_any(&mut parser, Container(depth));
    }
    if (input.starts_with('-') || input.as_bytes()[0].is_ascii_digit()) && unsafe_integer(input) {
        return Err(de::Error::custom("unsafe integer"));
    }
    let value: Value = serde_json::from_str(input)?;
    if let Value::Number(number) = &value
        && let Some(float) = number.as_f64()
        && float.fract() == 0.0
        && float.abs() > 9_007_199_254_740_991.0
    {
        return Err(de::Error::custom("unsafe rounded integer"));
    }
    Ok(value)
}

// Compare mathematical integer tokens BEFORE f64 conversion, including exponent
// and decimal forms. Avoid allocating an exponent-sized string for hostile input.
fn unsafe_integer(token: &str) -> bool {
    let token = token.strip_prefix('-').unwrap_or(token);
    let (mantissa, exponent) = token.split_once(['e', 'E']).unwrap_or((token, "0"));
    let exponent = exponent
        .parse::<i64>()
        .unwrap_or(if exponent.starts_with('-') {
            i64::MIN
        } else {
            i64::MAX
        });
    let fractional = mantissa
        .split_once('.')
        .map_or(0, |(_, tail)| tail.len() as i64);
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return false;
    }
    let zeros = digits.len() - digits.trim_end_matches('0').len();
    let shift = exponent.saturating_sub(fractional);
    if shift < 0 && shift.unsigned_abs() > zeros as u64 {
        return false;
    }
    let length = (digits.len() as i64).saturating_add(shift);
    if length > 16 {
        return true;
    }
    if length < 16 {
        return false;
    }
    let normalized = if shift >= 0 {
        format!("{digits}{}", "0".repeat(shift as usize))
    } else {
        digits[..length as usize].to_owned()
    };
    normalized.as_str() > "9007199254740991"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contract_vectors() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../contracts/studio-devtools/v1/fixtures.json"
        ))
        .unwrap();
        for case in fixtures["cases"].as_array().unwrap() {
            let input = case.get("input").cloned().unwrap_or_else(|| serde_json::json!({"text":case["inputGenerator"]["textRepeat"].as_str().unwrap().repeat(case["inputGenerator"]["count"].as_u64().unwrap() as usize)}));
            let request = serde_json::json!({"pluginId":"com.zicode.devtools.text","sceneId":"coding","capabilityId":case["capabilityId"],"commandId":null,"input":input});
            let result = execute_request(&serde_json::to_vec(&request).unwrap());
            if let Some(code) = case.get("expectedError") {
                assert!(!result.ok, "{}", case["id"]);
                assert_eq!(
                    result.error.unwrap().code,
                    code.as_str().unwrap(),
                    "{}",
                    case["id"]
                );
            } else {
                assert!(result.ok, "{}: {:?}", case["id"], result.error);
                let expected = case
                    .get("expectedText")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| encode(input["text"].as_str().unwrap()));
                assert_eq!(result.data.unwrap().text, expected, "{}", case["id"]);
            }
        }
    }
    #[test]
    fn exact_integer_depth_and_strict_encoding() {
        for input in [
            "9007199254740992",
            "-9007199254740992",
            "9007199254740992.0",
            "9.007199254740992e15",
            "1e100",
            "1e999999999999999999999999",
        ] {
            assert!(strict_json(input, 0).is_err(), "{input}");
        }
        for input in [
            "9007199254740991",
            "-9007199254740991",
            "1.25",
            "1e-100",
            "0e100",
            "9.007199254740991e15",
        ] {
            assert!(strict_json(input, 0).is_ok(), "{input}");
        }
        assert!(strict_json(&format!("{}0{}", "[".repeat(64), "]".repeat(64)), 0).is_ok());
        assert!(strict_json(&format!("{}0{}", "[".repeat(65), "]".repeat(65)), 0).is_err());
        assert!(strict_json(r#"{"a":{"x":1,"\u0078":2}}"#, 0).is_err());
        for input in [" YQ==", "YQ==\n", "YQ", "YR==", "_w=="] {
            assert!(!execute("devtools.text.base64.decode", input).ok);
        }
        assert_eq!(json("9007199254740992", false).unwrap(), "9007199254740992");
    }
    #[test]
    fn envelope_limits_required_fields_and_no_input_echo() {
        let base = serde_json::json!({"pluginId":"com.zicode.devtools.text","sceneId":"coding","capabilityId":"devtools.text.sha256","commandId":null,"input":{"text":"secret"}});
        for key in ["pluginId", "sceneId", "capabilityId", "commandId", "input"] {
            let mut request = base.clone();
            request.as_object_mut().unwrap().remove(key);
            let output = serialize_result(&execute_request(&serde_json::to_vec(&request).unwrap()));
            let result: Value = serde_json::from_slice(&output).unwrap();
            assert_eq!(result["error"]["code"], "INVALID_INPUT", "{key}");
            assert!(!String::from_utf8(output).unwrap().contains("secret"));
        }
        assert!(!execute_request(&vec![b' '; REQUEST_LIMIT + 1]).ok);
        let mut command = base.clone();
        command["capabilityId"] = Value::Null;
        command["commandId"] = Value::String("unregistered".into());
        assert_eq!(
            execute_request(&serde_json::to_vec(&command).unwrap())
                .error
                .unwrap()
                .code,
            "UNSUPPORTED_OPERATION"
        );
        command["input"]["text"] = Value::String("a".repeat(TEXT_LIMIT + 1));
        assert_eq!(
            execute_request(&serde_json::to_vec(&command).unwrap())
                .error
                .unwrap()
                .code,
            "INPUT_TOO_LARGE"
        );
        let oversized = ResultEnvelope::success("\u{1}".repeat(8192));
        let output = serialize_result(&oversized);
        assert!(output.len() < REQUEST_LIMIT);
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap()["error"]["code"],
            "INPUT_TOO_LARGE"
        );
    }
}
