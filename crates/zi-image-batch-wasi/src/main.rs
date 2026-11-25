//! Bounded batch encoding of separately granted static images.
use image::codecs::png::{CompressionType, FilterType, PngDecoder, PngEncoder};
use image::{ColorType, DynamicImage, ImageDecoder, ImageEncoder, ImageError, Limits};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{Cursor, Read as _};

const PLUGIN: &str = "com.zicode.devtools.image-batch";
const CAPABILITY: &str = "devtools.image.batch.resize";
const INPUT: &str = "resource.wasi.input.draft2";
const OUTPUT: &str = "resource.wasi.output.draft1";
const MAX_FILES: usize = 2;
const MAX_FILE_BYTES: usize = 32 * 1024;
const MAX_EDGE: u32 = 64;
const MAX_RESIZED_WIDTH: u32 = 48;
const MIME: &str = "image/png";
const MAX_REQUEST: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    plugin_id: String,
    capability_id: String,
    scene_id: String,
    command_id: Option<String>,
    resource_input_profile: String,
    resource_output_profile: String,
    resources: Vec<Descriptor>,
    input: Settings,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Descriptor {
    index: usize,
    resource_id: String,
    mime_type: String,
    byte_length: usize,
    content_version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Settings {
    max_width: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    input_index: usize,
    output_index: usize,
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    byte_length: usize,
}

#[derive(Clone, Copy, Serialize)]
struct Failure {
    code: &'static str,
    message: &'static str,
}

const REQUEST: Failure = Failure {
    code: "INVALID_REQUEST",
    message: "宽度可留空保留原尺寸，或输入 1–48 像素。",
};
const RESOURCE: Failure = Failure {
    code: "INVALID_RESOURCE",
    message: "请选择并确认一至两张本机静态 PNG。",
};
const INVALID: Failure = Failure {
    code: "INVALID_IMAGE",
    message: "至少一张文件无法作为 PNG 读取。本批没有结果。",
};
const ANIMATED: Failure = Failure {
    code: "ANIMATED_IMAGE",
    message: "本工具只处理静态图片。本批没有结果。",
};
const INPUT_LIMIT: Failure = Failure {
    code: "INPUT_LIMIT",
    message: "每张 PNG 最多 32 KiB，宽高分别不超过 64 像素。",
};
const COLOR_DEPTH: Failure = Failure {
    code: "COLOR_DEPTH",
    message: "暂不支持 16 位通道 PNG。本批没有结果。",
};
const OUTPUT_LIMIT: Failure = Failure {
    code: "OUTPUT_LIMIT",
    message: "无法在本批预算内生成全部结果。请缩小目标尺寸。",
};
const REGISTER: Failure = Failure {
    code: "REGISTER_FAILED",
    message: "无法登记本批结果，请重新执行工具。",
};

fn decode_error(error: ImageError) -> Failure {
    match error {
        ImageError::Limits(_) => INPUT_LIMIT,
        _ => INVALID,
    }
}

fn decode_static(bytes: &[u8]) -> Result<DynamicImage, Failure> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(128 * 1024);
    let decoder = PngDecoder::with_limits(Cursor::new(bytes), limits).map_err(decode_error)?;
    if decoder.is_apng().map_err(decode_error)? {
        return Err(ANIMATED);
    }
    if !matches!(
        decoder.color_type(),
        ColorType::L8 | ColorType::La8 | ColorType::Rgb8 | ColorType::Rgba8
    ) {
        return Err(COLOR_DEPTH);
    }
    DynamicImage::from_decoder(decoder).map_err(decode_error)
}

fn encode_image(derived: &DynamicImage) -> Result<Vec<u8>, Failure> {
    let mut output = Vec::new();
    // This bounded format avoids adaptive PNG filtering but preserves every pixel.
    PngEncoder::new_with_quality(
        &mut output,
        CompressionType::Uncompressed,
        FilterType::NoFilter,
    )
    .write_image(
        derived.as_bytes(),
        derived.width(),
        derived.height(),
        derived.color().into(),
    )
    .map_err(|_| OUTPUT_LIMIT)?;
    Ok(output)
}

fn execute(
    read: impl Fn(usize, usize) -> Result<Vec<u8>, Failure>,
    publish: impl Fn(usize, &[u8]) -> Result<(), Failure>,
) -> Result<Value, Failure> {
    let mut request = Vec::new();
    std::io::stdin()
        .take((MAX_REQUEST + 1) as u64)
        .read_to_end(&mut request)
        .map_err(|_| REQUEST)?;
    if request.len() > MAX_REQUEST {
        return Err(REQUEST);
    }
    let envelope: Envelope = serde_json::from_slice(&request).map_err(|_| REQUEST)?;
    if envelope.plugin_id != PLUGIN
        || envelope.capability_id != CAPABILITY
        || envelope.scene_id.is_empty()
        || envelope.command_id.is_some()
        || envelope.resource_input_profile != INPUT
        || envelope.resource_output_profile != OUTPUT
    {
        return Err(REQUEST);
    }
    if envelope
        .input
        .max_width
        .is_some_and(|width| !(1..=MAX_RESIZED_WIDTH).contains(&width))
    {
        return Err(REQUEST);
    }
    let max_width = envelope.input.max_width.unwrap_or(MAX_EDGE);
    if !(1..=MAX_FILES).contains(&envelope.resources.len()) {
        return Err(RESOURCE);
    }
    let mut inputs = Vec::with_capacity(envelope.resources.len());
    for (index, resource) in envelope.resources.into_iter().enumerate() {
        if resource.index != index
            || resource.resource_id.is_empty()
            || resource.content_version.len() != 64
            || !resource
                .content_version
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            || resource.mime_type != MIME
        {
            return Err(RESOURCE);
        }
        if !(1..=MAX_FILE_BYTES).contains(&resource.byte_length) {
            return Err(INPUT_LIMIT);
        }
        inputs.push(decode_static(&read(index, resource.byte_length)?)?);
    }

    // Stage every result before registration so a bad second file never produces a partial logical batch.
    let mut items = Vec::with_capacity(inputs.len());
    let mut outputs = Vec::with_capacity(inputs.len());
    let mut text = format!("已生成 {} 张 PNG。选择每张结果，确认后保存。", inputs.len());
    for (index, decoded) in inputs.into_iter().enumerate() {
        let (source_width, source_height) = (decoded.width(), decoded.height());
        let derived = if source_width <= max_width {
            decoded
        } else {
            zi_image_core::resize_to_width(&decoded, max_width).map_err(|_| OUTPUT_LIMIT)?
        };
        let (width, height) = (derived.width(), derived.height());
        let bytes = encode_image(&derived)?;
        let sequence = index + 1;
        use std::fmt::Write as _;
        let _ = write!(
            text,
            "\n输入 {sequence} → 结果 {sequence} · {source_width}×{source_height} → {width}×{height}"
        );
        items.push(Item {
            input_index: index,
            output_index: index,
            source_width,
            source_height,
            width,
            height,
            byte_length: bytes.len(),
        });
        outputs.push(bytes);
    }
    for (index, bytes) in outputs.iter().enumerate() {
        publish(index, bytes)?;
    }
    Ok(json!({"ok":true,"data":{"mimeType":MIME,"items":items,"text":text},"error":null}))
}

#[cfg(target_arch = "wasm32")]
mod bridge {
    use super::*;

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

    const CHUNK: usize = 64 * 1024;

    pub fn read(index: usize, length: usize) -> Result<Vec<u8>, Failure> {
        if unsafe { resource_size(index as i32) } != length as i64 {
            return Err(RESOURCE);
        }
        let mut bytes = vec![0_u8; length];
        for (chunk_index, chunk) in bytes.chunks_mut(CHUNK).enumerate() {
            let read = unsafe {
                resource_read(
                    index as i32,
                    (chunk_index * CHUNK) as i64,
                    chunk.as_mut_ptr() as i32,
                    chunk.len() as i32,
                )
            };
            if read != chunk.len() as i32 {
                return Err(RESOURCE);
            }
        }
        Ok(bytes)
    }

    pub fn publish(index: usize, bytes: &[u8]) -> Result<(), Failure> {
        const PNG_MIME_INDEX: i32 = 1;
        if unsafe { output_create(PNG_MIME_INDEX, bytes.len() as i64) } != index as i32 {
            return Err(REGISTER);
        }
        for chunk in bytes.chunks(CHUNK) {
            let written =
                unsafe { output_write(index as i32, chunk.as_ptr() as i32, chunk.len() as i32) };
            if written != chunk.len() as i32 {
                return Err(REGISTER);
            }
        }
        if unsafe { output_finish(index as i32) } != 0 {
            return Err(REGISTER);
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    print_result(execute(bridge::read, bridge::publish));
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    // Native fixture adapter. The Studio packaging scripts ship only the WASI module.
    let mut arguments = std::env::args_os().skip(1);
    let output = arguments.next().expect("fixture needs an output directory");
    let inputs: Vec<_> = arguments.collect();
    print_result(execute(
        |index, length| {
            let source =
                std::fs::File::open(inputs.get(index).ok_or(RESOURCE)?).map_err(|_| RESOURCE)?;
            let mut buffer = Vec::new();
            source
                .take((length + 1) as u64)
                .read_to_end(&mut buffer)
                .map_err(|_| RESOURCE)?;
            if buffer.len() != length {
                return Err(RESOURCE);
            }
            Ok(buffer)
        },
        |index, bytes| {
            use std::io::Write as _;
            std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(std::path::Path::new(&output).join(format!("{index}.bin")))
                .and_then(|mut file| file.write_all(bytes))
                .map_err(|_| REGISTER)
        },
    ));
}

fn print_result(result: Result<Value, Failure>) {
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => println!("{}", json!({"ok":false,"data":null,"error":error})),
    }
}
