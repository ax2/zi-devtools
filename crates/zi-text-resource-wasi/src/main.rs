//! Bounded URL-encode adapter for one explicitly granted UTF-8 text resource.
//! The guest has no filesystem, environment, or network imports.
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::fmt;
use std::io::{Read, Write};

const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_RESOURCE_BYTES: usize = 8192;
const IO_CHUNK: usize = 64 * 1024;

#[link(wasm_import_module = "zicode_resource_input_draft1")]
unsafe extern "C" {
    #[link_name = "size"]
    fn resource_size(index: i32) -> i64;
    #[link_name = "read"]
    fn resource_read(index: i32, offset: i64, destination: i32, length: i32) -> i32;
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    plugin_id: String,
    scene_id: String,
    capability_id: Option<String>,
    command_id: Option<String>,
    input: serde_json::Value,
    resource_input_profile: String,
    resources: Vec<ResourceDescriptor>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResourceDescriptor {
    index: usize,
    resource_id: String,
    content_version: String,
    mime_type: String,
    byte_length: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct PublicError {
    code: &'static str,
    message: &'static str,
}

impl fmt::Display for PublicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for PublicError {}

fn read_envelope() -> Result<Envelope> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_REQUEST_BYTES, "INVALID_REQUEST");
    Ok(serde_json::from_slice(&bytes)?)
}

fn validate_envelope(envelope: &Envelope) -> Result<&ResourceDescriptor> {
    ensure!(
        envelope.plugin_id == "com.zicode.devtools.url-text-resource",
        "INVALID_REQUEST"
    );
    ensure!(!envelope.scene_id.is_empty(), "INVALID_REQUEST");
    ensure!(
        envelope.capability_id.as_deref() == Some("devtools.transforms.url.encode"),
        "INVALID_REQUEST"
    );
    ensure!(envelope.command_id.is_none(), "INVALID_REQUEST");
    ensure!(
        envelope
            .input
            .as_object()
            .is_some_and(serde_json::Map::is_empty),
        "INVALID_REQUEST"
    );
    ensure!(
        envelope.resource_input_profile == "resource.wasi.input.draft2",
        "INVALID_REQUEST"
    );
    ensure!(envelope.resources.len() == 1, "INVALID_RESOURCE");
    let descriptor = &envelope.resources[0];
    ensure!(
        descriptor.index == 0 && !descriptor.resource_id.is_empty(),
        "INVALID_RESOURCE"
    );
    ensure!(
        descriptor.content_version.len() == 64
            && descriptor
                .content_version
                .bytes()
                .all(|b| b.is_ascii_hexdigit()),
        "INVALID_RESOURCE"
    );
    ensure!(descriptor.mime_type == "text/plain", "UNSUPPORTED_MIME");
    ensure!(descriptor.byte_length > 0, "EMPTY_RESOURCE");
    ensure!(
        descriptor.byte_length <= MAX_RESOURCE_BYTES,
        "INPUT_TOO_LARGE"
    );
    Ok(descriptor)
}

fn read_resource(length: usize) -> Result<Vec<u8>> {
    ensure!(length > 0, "EMPTY_RESOURCE");
    ensure!(length <= MAX_RESOURCE_BYTES, "INPUT_TOO_LARGE");
    let declared_size = unsafe { resource_size(0) };
    ensure!(
        declared_size >= 0 && usize::try_from(declared_size).ok() == Some(length),
        "RESOURCE_READ_FAILED"
    );
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length)?;
    let mut chunk = [0u8; IO_CHUNK];
    let mut offset = 0usize;
    while offset < length {
        let count = (length - offset).min(chunk.len());
        let read = unsafe {
            resource_read(
                0,
                offset as i64,
                chunk.as_mut_ptr() as usize as i32,
                count as i32,
            )
        };
        ensure!(read > 0 && read as usize <= count, "RESOURCE_READ_FAILED");
        bytes.extend_from_slice(&chunk[..read as usize]);
        offset += read as usize;
    }
    Ok(bytes)
}

fn public_error(error: &anyhow::Error) -> PublicError {
    let code = error.to_string();
    match code.as_str() {
        "EMPTY_RESOURCE" => PublicError {
            code: "EMPTY_RESOURCE",
            message: "共享文本为空",
        },
        "INPUT_TOO_LARGE" => PublicError {
            code: "INPUT_TOO_LARGE",
            message: "共享文本超过 8192 字节限制",
        },
        "UNSUPPORTED_MIME" => PublicError {
            code: "UNSUPPORTED_MIME",
            message: "仅支持 text/plain 文本资源",
        },
        "INVALID_UTF8" => PublicError {
            code: "INVALID_UTF8",
            message: "共享内容不是有效的 UTF-8 文本",
        },
        "RESOURCE_READ_FAILED" => PublicError {
            code: "RESOURCE_READ_FAILED",
            message: "无法读取已授权文本资源",
        },
        "INVALID_RESOURCE" => PublicError {
            code: "INVALID_RESOURCE",
            message: "共享资源信息无效",
        },
        _ => PublicError {
            code: "INVALID_REQUEST",
            message: "请求或资源授权信息无效",
        },
    }
}

fn encode_resource(bytes: &[u8]) -> std::result::Result<serde_json::Value, PublicError> {
    if bytes.is_empty() {
        return Err(PublicError {
            code: "EMPTY_RESOURCE",
            message: "共享文本为空",
        });
    }
    if bytes.len() > MAX_RESOURCE_BYTES {
        return Err(PublicError {
            code: "INPUT_TOO_LARGE",
            message: "共享文本超过 8192 字节限制",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| PublicError {
        code: "INVALID_UTF8",
        message: "共享内容不是有效的 UTF-8 文本",
    })?;
    Ok(serde_json::json!({
        "ok": true,
        "encodedText": zi_text_core::transforms::url_encode(text),
        "sourceByteCount": bytes.len()
    }))
}

fn execute(envelope: Envelope) -> Result<serde_json::Value> {
    let descriptor = validate_envelope(&envelope)?;
    let resource = read_resource(descriptor.byte_length)?;
    encode_resource(&resource).map_err(anyhow::Error::new)
}

fn run() -> serde_json::Value {
    match read_envelope().and_then(execute) {
        Ok(value) => value,
        Err(error) => {
            if let Some(public) = error.downcast_ref::<PublicError>() {
                serde_json::json!({ "ok": false, "error": { "code": public.code, "message": public.message } })
            } else {
                let public = public_error(&error);
                serde_json::json!({ "ok": false, "error": { "code": public.code, "message": public.message } })
            }
        }
    }
}

fn main() {
    let result = run();
    let bytes = serde_json::to_vec(&result).expect("bounded JSON result serializes");
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encoding_matches_standalone_for_multilingual_text() {
        let value = encode_resource("ZiCode OCR test 2026 中 +".as_bytes()).unwrap();
        assert_eq!(
            value["encodedText"],
            "ZiCode%20OCR%20test%202026%20%E4%B8%AD%20%2B"
        );
        assert_eq!(value["sourceByteCount"], 26);
    }

    #[test]
    fn rejects_empty_text_invalid_utf8_and_oversized_text() {
        assert_eq!(encode_resource(&[]).unwrap_err().code, "EMPTY_RESOURCE");
        assert_eq!(encode_resource(&[0xff]).unwrap_err().code, "INVALID_UTF8");
        assert_eq!(
            encode_resource(&vec![b'a'; MAX_RESOURCE_BYTES + 1])
                .unwrap_err()
                .code,
            "INPUT_TOO_LARGE"
        );
    }

    #[test]
    fn accepts_exact_byte_limit() {
        let value = encode_resource(&vec![b'a'; MAX_RESOURCE_BYTES]).unwrap();
        assert_eq!(value["sourceByteCount"], MAX_RESOURCE_BYTES);
    }

    #[test]
    fn descriptor_rejects_wrong_mime_and_unexpected_payload() {
        let descriptor = ResourceDescriptor {
            index: 0,
            resource_id: "resource-id".into(),
            content_version: "a".repeat(64),
            mime_type: "image/png".into(),
            byte_length: 1,
        };
        // Runtime MIME rejection is checked through the same validator used before reads.
        let envelope = Envelope {
            plugin_id: "com.zicode.devtools.url-text-resource".into(),
            scene_id: "scene".into(),
            capability_id: Some("devtools.transforms.url.encode".into()),
            command_id: None,
            input: serde_json::json!({}),
            resource_input_profile: "resource.wasi.input.draft2".into(),
            resources: vec![descriptor],
        };
        assert_eq!(
            validate_envelope(&envelope).unwrap_err().to_string(),
            "UNSUPPORTED_MIME"
        );
    }
}
