//! Read one original pixel from an explicitly shared static PNG.
use image::{
    ColorType, DynamicImage, GenericImageView, ImageDecoder, ImageError, Limits,
    codecs::png::PngDecoder,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{Cursor, Read as _};
use zi_image_core::{HexRgb, validate_dimensions_with_limits};

const PLUGIN: &str = "com.zicode.devtools.screenshot-pixel";
const CAPABILITY: &str = "devtools.screenshot.png.pixel";
const PROFILE: &str = "resource.wasi.input.draft2";
const MAX_BYTES: usize = 256 * 1024;
const MAX_PIXELS: u64 = 16_384;
const MAX_EDGE: u32 = 512;
const MAX_ENVELOPE: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    plugin_id: String,
    capability_id: String,
    scene_id: String,
    command_id: Option<String>,
    resource_input_profile: String,
    resource_output_profile: Option<String>,
    resources: Vec<Resource>,
    input: Coordinates,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Coordinates {
    x: u32,
    y: u32,
}

#[derive(Clone, Copy, Serialize)]
struct Failure {
    code: &'static str,
    message: &'static str,
}

const REQUEST: Failure = Failure {
    code: "INVALID_REQUEST",
    message: "请填写从 0 开始的整数像素坐标 x 和 y。",
};
const RESOURCE: Failure = Failure {
    code: "INVALID_RESOURCE",
    message: "请选择并确认一份本机静态 PNG 文件。",
};
const READ: Failure = Failure {
    code: "RESOURCE_READ_FAILED",
    message: "无法读取已确认的 PNG 文件。",
};
const BYTE_LIMIT: Failure = Failure {
    code: "INPUT_LIMIT",
    message: "每张 PNG 最多 256 KiB。",
};
const PIXEL_LIMIT: Failure = Failure {
    code: "DIMENSION_LIMIT",
    message: "PNG 每边最多 512 像素，合计不超过 16384 像素。",
};
const INVALID: Failure = Failure {
    code: "INVALID_PNG",
    message: "无法完整解码此 PNG。",
};
const ANIMATED: Failure = Failure {
    code: "ANIMATED_PNG",
    message: "此工具只读取静态 PNG 的像素。",
};
const DEPTH: Failure = Failure {
    code: "UNSUPPORTED_DEPTH",
    message: "当前工具保留精确的 8 位通道，暂不接受 16 位通道 PNG。",
};
const COORDINATES: Failure = Failure {
    code: "OUT_OF_BOUNDS",
    message: "指定坐标超过原始 PNG 边界。坐标原点是左上角的 (0, 0)。",
};

fn decode_error(error: ImageError) -> Failure {
    if matches!(error, ImageError::Limits(_)) {
        PIXEL_LIMIT
    } else {
        INVALID
    }
}

fn sample(bytes: &[u8], x: u32, y: u32) -> Result<Value, Failure> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(1024 * 1024);
    let decoder = PngDecoder::with_limits(Cursor::new(bytes), limits).map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    validate_dimensions_with_limits(width, height, MAX_EDGE, MAX_PIXELS)
        .map_err(|_| PIXEL_LIMIT)?;
    if decoder.is_apng().map_err(decode_error)? {
        return Err(ANIMATED);
    }
    if !matches!(
        decoder.color_type(),
        ColorType::L8 | ColorType::La8 | ColorType::Rgb8 | ColorType::Rgba8
    ) {
        return Err(DEPTH);
    }
    if x >= width || y >= height {
        return Err(COORDINATES);
    }
    let image = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    let rgba = image.get_pixel(x, y).0;
    let [red, green, blue, alpha] = rgba;
    let rgb = HexRgb([red, green, blue]).to_string();
    let alpha_hex = format!("{rgb}{alpha:02X}");
    let text = format!(
        "({x}, {y}) / {width}×{height} px\nRGB {red}, {green}, {blue} · Alpha {alpha}/255\n{rgb} · {alpha_hex}"
    );
    Ok(
        json!({"ok":true,"data":{"width":width,"height":height,"x":x,"y":y,
        "rgba":rgba,"hexRgb":rgb,"hexRgba":alpha_hex,"text":text},"error":null}),
    )
}

fn execute() -> Result<Value, Failure> {
    let mut wire = Vec::new();
    std::io::stdin()
        .take((MAX_ENVELOPE + 1) as u64)
        .read_to_end(&mut wire)
        .map_err(|_| REQUEST)?;
    if wire.len() > MAX_ENVELOPE {
        return Err(REQUEST);
    }
    let request: Envelope = serde_json::from_slice(&wire).map_err(|_| REQUEST)?;
    if request.plugin_id != PLUGIN
        || request.capability_id != CAPABILITY
        || request.scene_id.is_empty()
        || request.command_id.is_some()
        || request.resource_input_profile != PROFILE
        || request.resource_output_profile.is_some()
    {
        return Err(REQUEST);
    }
    let [resource] = request.resources.as_slice() else {
        return Err(RESOURCE);
    };
    if resource.index != 0
        || resource.resource_id.is_empty()
        || resource.content_version.len() != 64
        || !resource
            .content_version
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
        || resource.mime_type != "image/png"
    {
        return Err(RESOURCE);
    }
    if resource.byte_length > MAX_BYTES {
        return Err(BYTE_LIMIT);
    }
    let bytes = read(resource.byte_length)?;
    sample(&bytes, request.input.x, request.input.y)
}

#[cfg(target_arch = "wasm32")]
fn read(length: usize) -> Result<Vec<u8>, Failure> {
    zi_resource_input_wasi::read_prefix(0, length, length).map_err(|_| READ)
}

#[cfg(not(target_arch = "wasm32"))]
fn read(length: usize) -> Result<Vec<u8>, Failure> {
    let path = std::env::args_os().nth(1).ok_or(READ)?;
    let source = std::fs::File::open(path).map_err(|_| READ)?;
    let mut bytes = Vec::new();
    source
        .take((length + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| READ)?;
    if bytes.len() != length {
        return Err(READ);
    }
    Ok(bytes)
}

fn main() {
    match execute() {
        Ok(result) => println!("{result}"),
        Err(error) => println!("{}", json!({"ok":false,"data":null,"error":error})),
    }
}
