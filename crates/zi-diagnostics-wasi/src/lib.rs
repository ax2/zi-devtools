//! Diagnostics compute adapter; no file, environment, process or network access.
use serde::{
    Deserialize, Serialize,
    de::{self, Visitor},
};
pub const REQUEST_LIMIT: usize = 48 * 1024;
pub const TEXT_LIMIT: usize = 8192;
pub const CONTRACT_VERSION: &str = "1.0.0-rc.1";
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
    input: Input,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    text: String,
    #[serde(default)]
    secondary: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Text {
    pub text: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Error {
    pub code: String,
    pub message: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub contract_version: String,
    pub ok: bool,
    pub data: Option<Text>,
    pub error: Option<Error>,
}
impl Envelope {
    pub fn failure(code: &str, message: &str) -> Self {
        Self {
            contract_version: CONTRACT_VERSION.into(),
            ok: false,
            data: None,
            error: Some(Error {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}
pub fn execute_request(bytes: &[u8]) -> Envelope {
    if bytes.len() > REQUEST_LIMIT {
        return Envelope::failure("INPUT_TOO_LARGE", "请求超过48 KiB");
    }
    let Ok(request) = serde_json::from_slice::<Request>(bytes) else {
        return Envelope::failure("INVALID_INPUT", "请求结构无效");
    };
    if request.plugin_id != "com.zicode.devtools.diagnostics"
        || request.scene_id.is_empty()
        || request.scene_id.len() > 128
    {
        return Envelope::failure("INVALID_INPUT", "插件或场景标识无效");
    }
    if request.input.text.len() > TEXT_LIMIT || request.input.secondary.len() > TEXT_LIMIT {
        return Envelope::failure("INPUT_TOO_LARGE", "每份报告最多8192 UTF-8字节");
    }
    let capability = match (request.capability_id.0, request.command_id.0) {
        (Some(id), None) if !id.is_empty() && id.len() <= 128 => id,
        (None, Some(id)) if !id.is_empty() => {
            return Envelope::failure("UNSUPPORTED_OPERATION", "未注册后台命令");
        }
        _ => return Envelope::failure("INVALID_INPUT", "必须指定唯一处理器"),
    };
    let Some(action) = capability.strip_prefix("devtools.diagnostics.") else {
        return Envelope::failure("UNSUPPORTED_OPERATION", "未知分析能力");
    };
    if !zi_diagnostics_core::ACTIONS
        .iter()
        .any(|entry| entry.id == action)
    {
        return Envelope::failure("UNSUPPORTED_OPERATION", "未知分析能力");
    }
    match zi_diagnostics_core::execute_plugin(
        action,
        &request.input.text,
        &request.input.secondary,
        REQUEST_LIMIT,
    ) {
        Ok(text) => Envelope {
            contract_version: CONTRACT_VERSION.into(),
            ok: true,
            data: Some(Text { text }),
            error: None,
        },
        Err(error)
            if error
                .downcast_ref::<zi_diagnostics_core::ReportTooLarge>()
                .is_some() =>
        {
            Envelope::failure("OUTPUT_TOO_LARGE", "完整结果超过48 KiB，请缩小输入范围")
        }
        Err(_) => Envelope::failure("INVALID_REPORT", "报告格式或分析范围无效，请核对输入"),
    }
}
pub fn serialize_result(result: &Envelope) -> Vec<u8> {
    struct Bounded(Vec<u8>);
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > REQUEST_LIMIT - self.0.len() {
                return Err(std::io::Error::other("result byte budget"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded(Vec::new());
    if serde_json::to_writer(&mut output, result).is_ok() {
        output.0
    } else {
        serde_json::to_vec(&Envelope::failure(
            "OUTPUT_TOO_LARGE",
            "完整结果超过48 KiB，请缩小输入范围",
        ))
        .expect("fixed error envelope")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> serde_json::Value {
        serde_json::json!({"pluginId":"com.zicode.devtools.diagnostics","sceneId":"coding",
        "capabilityId":"devtools.diagnostics.django.checks","commandId":null,"input":{"text":"?: (security.W018) DEBUG enabled"}})
    }
    #[test]
    fn required_fields_unique_handlers_and_duplicates_are_rejected() {
        let value = request();
        assert!(execute_request(&serde_json::to_vec(&value).unwrap()).ok);
        for key in ["pluginId", "sceneId", "capabilityId", "commandId", "input"] {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(
                !execute_request(&serde_json::to_vec(&missing).unwrap()).ok,
                "{key}"
            );
        }
        let mut both = value.clone();
        both["commandId"] = "other".into();
        assert!(!execute_request(&serde_json::to_vec(&both).unwrap()).ok);
        let duplicate = serde_json::to_string(&value)
            .unwrap()
            .replace("\"input\":", "\"pluginId\":\"other\",\"input\":");
        assert!(!execute_request(duplicate.as_bytes()).ok);
    }
    #[test]
    fn byte_and_escaped_output_limits_do_not_publish_truncated_success() {
        assert!(!execute_request(&vec![b' '; REQUEST_LIMIT + 1]).ok);
        let mut value = request();
        value["input"]["text"] = "中".repeat(2731).into();
        assert_eq!(
            execute_request(&serde_json::to_vec(&value).unwrap())
                .error
                .unwrap()
                .code,
            "INPUT_TOO_LARGE"
        );
        let result = Envelope {
            contract_version: CONTRACT_VERSION.into(),
            ok: true,
            data: Some(Text {
                text: "\0".repeat(9000),
            }),
            error: None,
        };
        let parsed: Envelope = serde_json::from_slice(&serialize_result(&result)).unwrap();
        assert!(!parsed.ok && parsed.data.is_none());
    }
    #[test]
    fn serialized_result_accepts_exact_boundary_and_rejects_one_byte_over() {
        for prefix in ["", "中\0\"\\"] {
            let mut result = Envelope {
                contract_version: CONTRACT_VERSION.into(),
                ok: true,
                data: Some(Text {
                    text: prefix.into(),
                }),
                error: None,
            };
            let overhead = serde_json::to_vec(&result).unwrap().len();
            result
                .data
                .as_mut()
                .unwrap()
                .text
                .push_str(&"a".repeat(REQUEST_LIMIT - overhead));
            assert_eq!(serialize_result(&result).len(), REQUEST_LIMIT);
            assert!(
                serde_json::from_slice::<Envelope>(&serialize_result(&result))
                    .unwrap()
                    .ok
            );
            result.data.as_mut().unwrap().text.push('a');
            let rejected: Envelope = serde_json::from_slice(&serialize_result(&result)).unwrap();
            assert_eq!(rejected.error.unwrap().code, "OUTPUT_TOO_LARGE");
            assert!(rejected.data.is_none());
        }
    }
}
