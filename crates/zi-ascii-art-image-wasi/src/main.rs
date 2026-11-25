//! Converts one approved original image to a raw ASCII text resource.

use image::{ImageError, ImageFormat, ImageReader, Limits, error::LimitErrorKind};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Cursor, Read, Write};

const PLUGIN: &str = "com.zicode.devtools.ascii-art-image";
const CAPABILITY: &str = "devtools.ascii.image.render";
const INPUT_PROFILE: &str = "resource.wasi.input.draft2";
const OUTPUT_PROFILE: &str = "resource.wasi.output.draft1";
const MAX_BYTES: usize = 10 * 1024 * 1024;
const MAX_EDGE: u32 = 20_000;
const MAX_PIXELS: u64 = 20_000_000;
const MAX_ENVELOPE_BYTES: usize = 16 * 1024;
type Error = &'static str;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: Value,
    input: Input,
    resource_input_profile: String,
    resource_output_profile: String,
    resources: Vec<Resource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    width: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Resource {
    index: usize,
    resource_id: String,
    content_version: String,
    mime_type: String,
    byte_length: usize,
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "zicode_resource_output_draft1")]
unsafe extern "C" {
    #[link_name = "create"]
    fn output_create(mime_index: i32, length: i64) -> i32;
    #[link_name = "write"]
    fn output_write(index: i32, source: *const u8, length: i32) -> i32;
    #[link_name = "finish"]
    fn output_finish(index: i32) -> i32;
}

fn execute() -> Result<(u32, String), Error> {
    let mut encoded = Vec::new();
    std::io::stdin()
        .take((MAX_ENVELOPE_BYTES + 1) as u64)
        .read_to_end(&mut encoded)
        .map_err(|_| "INVALID_REQUEST")?;
    if encoded.len() > MAX_ENVELOPE_BYTES {
        return Err("INVALID_REQUEST");
    }
    let request: Envelope = serde_json::from_slice(&encoded).map_err(|_| "INVALID_REQUEST")?;
    if request.plugin_id != PLUGIN
        || request.scene_id.is_empty()
        || request.capability_id != CAPABILITY
        || !request.command_id.is_null()
        || request.resource_input_profile != INPUT_PROFILE
        || request.resource_output_profile != OUTPUT_PROFILE
    {
        return Err("INVALID_REQUEST");
    }
    let width = request.input.width;
    if !(8..=120).contains(&width) {
        return Err("INVALID_WIDTH");
    }
    let [resource] = request.resources.as_slice() else {
        return Err("INVALID_RESOURCE");
    };
    if resource.index != 0
        || resource.resource_id.is_empty()
        || resource.content_version.len() != 64
        || !resource
            .content_version
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("INVALID_RESOURCE");
    }
    if resource.byte_length > MAX_BYTES {
        return Err("INPUT_TOO_LARGE");
    }
    if resource.byte_length == 0 {
        return Err("INVALID_IMAGE");
    }
    let format = match resource.mime_type.as_str() {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/webp" => ImageFormat::WebP,
        _ => return Err("UNSUPPORTED_MIME"),
    };
    let bytes = read(resource.byte_length)?;
    let actual = image::guess_format(&bytes).map_err(|_| "INVALID_IMAGE")?;
    if actual != format {
        return Err("IMAGE_MIME_MISMATCH");
    }
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    reader.limits(limits);
    let (source_width, source_height) = reader.into_dimensions().map_err(|error| match error {
        ImageError::Limits(error) if error.kind() == LimitErrorKind::DimensionError => {
            "IMAGE_DIMENSION_LIMIT"
        }
        _ => "INVALID_IMAGE",
    })?;
    if source_width == 0
        || source_height == 0
        || u64::from(source_width) * u64::from(source_height) > MAX_PIXELS
    {
        return Err("IMAGE_DIMENSION_LIMIT");
    }
    let text = zi_character_core::image_art_bytes(&bytes, width).map_err(|_| "INVALID_IMAGE")?;
    Ok((width, text))
}

#[cfg(target_arch = "wasm32")]
fn read(length: usize) -> Result<Vec<u8>, Error> {
    zi_resource_input_wasi::read_prefix(0, length, length)
}

#[cfg(not(target_arch = "wasm32"))]
fn read(length: usize) -> Result<Vec<u8>, Error> {
    // Native conformance counterpart. The WASI executable cannot accept a path.
    let path = std::env::args_os().nth(1).ok_or("RESOURCE_READ_FAILED")?;
    let file = std::fs::File::open(path).map_err(|_| "RESOURCE_READ_FAILED")?;
    let mut bytes = Vec::new();
    file.take((length + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "RESOURCE_READ_FAILED")?;
    if bytes.len() != length {
        return Err("RESOURCE_READ_FAILED");
    }
    Ok(bytes)
}

#[cfg(target_arch = "wasm32")]
fn publish(text: &[u8]) -> Result<(), Error> {
    // Output MIME ABI index 4 is text/plain. The core emits at most 120 x 121 bytes.
    let index = unsafe { output_create(4, text.len() as i64) };
    if index != 0
        || unsafe { output_write(index, text.as_ptr(), text.len() as i32) } != text.len() as i32
        || unsafe { output_finish(index) } != 0
    {
        return Err("OUTPUT_FAILED");
    }
    Ok(())
}

fn failure(code: Error) -> Value {
    let message = match code {
        "INVALID_WIDTH" => "Use an integer width from 8 to 120. 宽度须为 8 至 120 的整数。",
        "INPUT_TOO_LARGE" => "The original image exceeds 10 MiB. 原图超过 10 MiB。",
        "UNSUPPORTED_MIME" | "IMAGE_MIME_MISMATCH" => {
            "Choose an original PNG, JPEG or WebP image of the stated type. 请选择类型相符的 PNG、JPEG 或 WebP 原图。"
        }
        "IMAGE_DIMENSION_LIMIT" => {
            "Use at most 20,000 pixels per side and 20 million total. 图片每边最多 2 万像素，总计最多 2000 万像素。"
        }
        "INVALID_IMAGE" => "The original image is damaged or invalid. 原图已损坏或无效。",
        "OUTPUT_FAILED" => "The text result could not be written. 无法写入文本结果。",
        _ => "The approved image could not be read. 无法读取本次授权的图片。",
    };
    json!({"ok":false,"error":{"code":code,"message":message}})
}

fn main() -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    match execute() {
        Ok((width, text)) => {
            #[cfg(target_arch = "wasm32")]
            if let Err(code) = publish(text.as_bytes()) {
                serde_json::to_writer(&mut stdout, &failure(code))?;
                return Ok(());
            }
            serde_json::to_writer(
                &mut stdout,
                &json!({"ok":true,"width":width,
                    "height":text.len()/(width as usize+1),"byteLength":text.len()}),
            )?;
            #[cfg(not(target_arch = "wasm32"))]
            {
                stdout.write_all(b"\n")?;
                stdout.write_all(text.as_bytes())?;
            }
        }
        Err(code) => serde_json::to_writer(&mut stdout, &failure(code))?,
    }
    stdout.flush()
}
