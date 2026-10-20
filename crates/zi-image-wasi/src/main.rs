//! Resource-mode image adapter. It has no filesystem or network imports.
use anyhow::{Context, Result, bail, ensure};
use image::ImageFormat;
use serde::Deserialize;
use std::io::{Read, Seek, SeekFrom, Write};

const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_RESOURCE_BYTES: usize = 32 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const PROFILE_MAX_EDGE: u32 = 4096;
const PROFILE_MAX_PIXELS: u64 = 16_000_000;
const IO_CHUNK: usize = 64 * 1024;

#[link(wasm_import_module = "zicode_resource_input_draft1")]
unsafe extern "C" {
    #[link_name = "size"]
    fn resource_size(index: i32) -> i64;
    #[link_name = "read"]
    fn resource_read(index: i32, offset: i64, destination: i32, length: i32) -> i32;
}

#[link(wasm_import_module = "zicode_resource_output_draft1")]
unsafe extern "C" {
    #[link_name = "create"]
    fn output_create(mime_index: i32, length: i64) -> i32;
    #[link_name = "write"]
    fn output_write(index: i32, source: i32, length: i32) -> i32;
    #[link_name = "finish"]
    fn output_finish(index: i32) -> i32;
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    plugin_id: String,
    scene_id: String,
    capability_id: Option<String>,
    command_id: Option<String>,
    input: Operation,
    resource_input_profile: String,
    resource_output_profile: String,
    resources: Vec<ResourceDescriptor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResourceDescriptor {
    index: usize,
    resource_id: String,
    content_version: String,
    mime_type: String,
    byte_length: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Operation {
    max_width: serde_json::Value,
    format: String,
    quality: serde_json::Value,
}

fn integer_parameter(value: &serde_json::Value, max_digits: usize) -> Result<u32> {
    match value {
        serde_json::Value::Number(number) => number
            .as_u64()
            .and_then(|number| u32::try_from(number).ok())
            .context("invalid operation integer"),
        serde_json::Value::String(text) => {
            ensure!(
                !text.is_empty()
                    && text.len() <= max_digits
                    && text.bytes().all(|byte| byte.is_ascii_digit()),
                "invalid operation integer"
            );
            text.parse::<u32>().context("invalid operation integer")
        }
        _ => bail!("invalid operation integer"),
    }
}

impl Operation {
    fn validated(&self) -> Result<(u32, u8, ImageFormat, i32, &'static str)> {
        let max_width = integer_parameter(&self.max_width, 4)?;
        let quality = integer_parameter(&self.quality, 2)?;
        ensure!(
            (1..=PROFILE_MAX_EDGE).contains(&max_width),
            "invalid operation width"
        );
        ensure!((35..=95).contains(&quality), "invalid operation quality");
        let (format, mime_index, mime_type) = encode_format(&self.format)?;
        Ok((max_width, quality as u8, format, mime_index, mime_type))
    }
}

struct BoundedOutput {
    bytes: Vec<u8>,
    position: usize,
}

impl BoundedOutput {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            position: 0,
        }
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let end = self
            .position
            .checked_add(bytes.len())
            .filter(|end| *end <= MAX_OUTPUT_BYTES)
            .ok_or_else(|| std::io::Error::other("output budget exceeded"))?;
        if end > self.bytes.len() {
            let additional = end - self.bytes.len();
            self.bytes
                .try_reserve_exact(additional)
                .map_err(std::io::Error::other)?;
            self.bytes.resize(end, 0);
        }
        self.bytes[self.position..end].copy_from_slice(bytes);
        self.position = end;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for BoundedOutput {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let next = match from {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::End(offset) => self.bytes.len() as i128 + i128::from(offset),
            SeekFrom::Current(offset) => self.position as i128 + i128::from(offset),
        };
        if !(0..=MAX_OUTPUT_BYTES as i128).contains(&next) {
            return Err(std::io::Error::other("output seek exceeds budget"));
        }
        self.position = next as usize;
        Ok(self.position as u64)
    }
}

fn read_request() -> Result<Envelope> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .context("cannot read request")?;
    ensure!(bytes.len() <= MAX_REQUEST_BYTES, "request too large");
    serde_json::from_slice(&bytes).context("invalid request")
}

fn read_resource(length: usize) -> Result<Vec<u8>> {
    ensure!(
        length > 0 && length <= MAX_RESOURCE_BYTES,
        "input size out of range"
    );
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .context("cannot reserve input buffer")?;
    let mut chunk = [0u8; IO_CHUNK];
    let mut offset = 0usize;
    while offset < length {
        let count = (length - offset).min(chunk.len());
        // Host copies are bounded to 64 KiB per call and 64 MiB per request.
        let read = unsafe {
            resource_read(
                0,
                offset as i64,
                chunk.as_mut_ptr() as usize as i32,
                count as i32,
            )
        };
        ensure!(read > 0 && read as usize <= count, "resource read failed");
        bytes.extend_from_slice(&chunk[..read as usize]);
        offset += read as usize;
    }
    Ok(bytes)
}

fn encode_format(name: &str) -> Result<(ImageFormat, i32, &'static str)> {
    match name {
        "png" => Ok((ImageFormat::Png, 1, "image/png")),
        "jpeg" => Ok((ImageFormat::Jpeg, 2, "image/jpeg")),
        "webp" => Ok((ImageFormat::WebP, 3, "image/webp")),
        _ => bail!("unsupported output format"),
    }
}

fn execute(envelope: Envelope) -> Result<serde_json::Value> {
    ensure!(
        envelope.plugin_id == "com.zicode.devtools.image",
        "invalid plugin id"
    );
    ensure!(!envelope.scene_id.is_empty(), "missing scene id");
    ensure!(
        envelope.capability_id.as_deref() == Some("devtools.image.resize")
            && envelope.command_id.is_none(),
        "invalid capability"
    );
    ensure!(
        envelope.resource_input_profile == "resource.wasi.input.draft2"
            && envelope.resource_output_profile == "resource.wasi.output.draft1",
        "unsupported resource profile"
    );
    ensure!(
        envelope.resources.len() == 1,
        "exactly one input image is required"
    );
    let descriptor = &envelope.resources[0];
    ensure!(descriptor.index == 0, "invalid resource index");
    ensure!(
        !descriptor.resource_id.is_empty() && descriptor.content_version.len() == 64,
        "invalid resource descriptor"
    );
    ensure!(
        matches!(
            descriptor.mime_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp"
        ),
        "unsupported input mime type"
    );
    let (max_width, quality, format, mime_index, mime_type) = envelope.input.validated()?;

    let declared_size = unsafe { resource_size(0) };
    ensure!(declared_size > 0, "missing input resource");
    ensure!(
        usize::try_from(declared_size).ok() == Some(descriptor.byte_length),
        "resource size mismatch"
    );
    let input_bytes = read_resource(descriptor.byte_length)?;
    let detected = image::guess_format(&input_bytes).context("unsupported input image")?;
    let declared_format = match descriptor.mime_type.as_str() {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/webp" => ImageFormat::WebP,
        _ => unreachable!(),
    };
    ensure!(
        detected == declared_format,
        "input format does not match resource mime type"
    );

    let (input_width, input_height) =
        zi_image_core::encoded_dimensions(&input_bytes).context("image header is invalid")?;
    zi_image_core::validate_dimensions_with_limits(
        input_width,
        input_height,
        PROFILE_MAX_EDGE,
        PROFILE_MAX_PIXELS,
    )
    .context("image dimensions exceed resource profile")?;
    let image = zi_image_core::decode(&input_bytes).context("image decode failed")?;
    // The decoded image owns its pixels; free the compressed guest input before
    // resize/encode so the resource frame bytes do not remain live unnecessarily.
    drop(input_bytes);
    let source_width = image.width();
    let source_height = image.height();
    let resized = zi_image_core::resize_to_width(&image, max_width)?;
    let mut output = BoundedOutput::new();
    zi_image_core::encode(&resized, format, quality, &mut output)?;
    let output_bytes = output.into_bytes();
    ensure!(!output_bytes.is_empty(), "empty encoded image");

    let output_index = unsafe { output_create(mime_index, output_bytes.len() as i64) };
    ensure!(output_index == 0, "host rejected image output");
    for chunk in output_bytes.chunks(IO_CHUNK) {
        let written = unsafe {
            output_write(
                output_index,
                chunk.as_ptr() as usize as i32,
                chunk.len() as i32,
            )
        };
        ensure!(written == chunk.len() as i32, "host rejected output chunk");
    }
    ensure!(
        unsafe { output_finish(output_index) } == 0,
        "host rejected output finish"
    );

    Ok(serde_json::json!({
        "ok": true,
        "sourceWidth": source_width,
        "sourceHeight": source_height,
        "width": resized.width(),
        "height": resized.height(),
        "mimeType": mime_type,
        "byteLength": output_bytes.len(),
    }))
}

fn public_failure(error: &anyhow::Error) -> serde_json::Value {
    let detail = error
        .chain()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let (code, message) = if detail.contains("图片尺寸超出限制") {
        ("IMAGE_DIMENSION_LIMIT", "图片尺寸超过当前能力上限")
    } else if detail.contains("图片解码内存超限") {
        ("IMAGE_MEMORY_LIMIT", "图片解码内存超过当前能力上限")
    } else if detail.contains("image decode") || detail.contains("unsupported input image") {
        (
            "INVALID_IMAGE",
            "输入内容不是受支持的 PNG、JPEG 或 WebP 图片",
        )
    } else if detail.contains("output budget") {
        ("OUTPUT_LIMIT", "编码结果超过 32 MiB 输出限制")
    } else if detail.contains("invalid operation") {
        ("INVALID_PARAMETERS", "缩放宽度、格式或质量参数无效")
    } else if detail.contains("input size out of range") {
        ("INPUT_LIMIT", "输入图片为空或超过 32 MiB")
    } else {
        ("IMAGE_OPERATION_FAILED", "图片处理失败或超出运行资源限制")
    };
    serde_json::json!({"ok": false, "error": {"code": code, "message": message}})
}

fn main() {
    let value = match read_request().and_then(execute) {
        Ok(value) => value,
        Err(error) => public_failure(&error),
    };
    let bytes = serde_json::to_vec(&value).expect("serializing bounded result cannot fail");
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_form_and_legacy_strings_have_identical_parameters() {
        for width in [1, 16, 1600, 4096] {
            for quality in [35, 82, 95] {
                let numeric: Operation = serde_json::from_value(serde_json::json!({
                    "maxWidth":width,"format":"webp","quality":quality
                }))
                .unwrap();
                let legacy: Operation = serde_json::from_value(serde_json::json!({
                    "maxWidth":format!("{width:04}"),"format":"webp","quality":quality.to_string()
                }))
                .unwrap();
                assert_eq!(numeric.validated().unwrap(), legacy.validated().unwrap());
            }
        }
        for value in [
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({}),
            serde_json::json!(u64::MAX),
        ] {
            assert!(integer_parameter(&value, 4).is_err());
        }
    }

    #[test]
    fn validates_tool_center_string_parameters_before_resource_reads() {
        assert!(
            Operation {
                max_width: "1600".into(),
                format: "webp".into(),
                quality: "82".into(),
            }
            .validated()
            .is_ok()
        );
        for invalid in [
            Operation {
                max_width: "0".into(),
                format: "png".into(),
                quality: "82".into(),
            },
            Operation {
                max_width: "4097".into(),
                format: "png".into(),
                quality: "82".into(),
            },
            Operation {
                max_width: "1e3".into(),
                format: "png".into(),
                quality: "82".into(),
            },
            Operation {
                max_width: "1000".into(),
                format: "bmp".into(),
                quality: "82".into(),
            },
            Operation {
                max_width: "1000".into(),
                format: "png".into(),
                quality: "34".into(),
            },
            Operation {
                max_width: "1000".into(),
                format: "png".into(),
                quality: "96".into(),
            },
        ] {
            assert!(invalid.validated().is_err());
        }
    }
}
