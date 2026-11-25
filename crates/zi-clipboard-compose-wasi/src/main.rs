//! Combine the ordered text of one explicitly shared JSON file.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Read as _;
use zi_clipboard_compose_core::{ComposeError, TextFormat, combine_text};

const PLUGIN: &str = "com.zicode.devtools.clipboard-compose";
const CAPABILITY: &str = "devtools.clipboard.text.compose";
const INPUT: &str = "resource.wasi.input.draft2";
const OUTPUT: &str = "resource.wasi.output.draft1";
const MAX_RESOURCE: usize = 32 * 1024;
const MAX_REQUEST: usize = 16 * 1024;
const MAX_OUTPUT: usize = 48 * 1024;
const MAX_ITEMS: usize = 64;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    plugin_id: String,
    capability_id: String,
    scene_id: String,
    command_id: Option<String>,
    resource_input_profile: String,
    resource_output_profile: String,
    resources: Vec<Resource>,
    input: Options,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Resource {
    index: usize,
    resource_id: String,
    mime_type: String,
    byte_length: usize,
    content_version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    format: Format,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Format {
    Paragraphs,
    Numbered,
    Csv,
    Markdown,
}

impl Format {
    fn output(self) -> (TextFormat, &'static str, &'static str) {
        match self {
            Self::Paragraphs => (TextFormat::Paragraphs, "text/plain", "段落 TXT"),
            Self::Numbered => (TextFormat::Numbered, "text/plain", "编号 TXT"),
            Self::Csv => (TextFormat::Csv, "text/csv", "单列 CSV"),
            Self::Markdown => (TextFormat::Markdown, "text/markdown", "Markdown 清单"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    order: usize,
    text: String,
    #[serde(rename = "source")]
    _source: String,
    #[serde(rename = "time")]
    _time: String,
    #[serde(rename = "labels")]
    _labels: Vec<String>,
}

#[derive(Clone, Copy, Serialize)]
struct Failure {
    code: &'static str,
    message: &'static str,
}

const REQUEST: Failure = Failure {
    code: "INVALID_REQUEST",
    message: "请选择段落、编号、CSV 或 Markdown 格式。",
};
const RESOURCE: Failure = Failure {
    code: "INVALID_RESOURCE",
    message: "请选择并单独确认一份本机 UTF-8 JSON 正文清单。",
};
const LIMIT: Failure = Failure {
    code: "INPUT_LIMIT",
    message: "本次确认的 JSON 文件最多 32 KiB。",
};
const FORMAT: Failure = Failure {
    code: "INVALID_JSON",
    message: "请选择 JSON 字符串数组或独立版的有序 JSON 记录。",
};
const ORDER: Failure = Failure {
    code: "RECORD_ORDER",
    message: "记录的 order 应从 1 开始并和文件顺序相同。",
};
const COUNT: Failure = Failure {
    code: "ITEM_LIMIT",
    message: "每次请选择 1–64 段非空正文。",
};
const OUTPUT_LIMIT: Failure = Failure {
    code: "OUTPUT_LIMIT",
    message: "完整组合超过 48 KiB，请减少正文后再试。",
};
const WRITE: Failure = Failure {
    code: "OUTPUT_FAILED",
    message: "无法生成或登记完整组合文件。",
};

fn parse_text(raw: &[u8]) -> Result<Vec<String>, Failure> {
    let raw = raw.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(raw);
    let text = if let Ok(plain) = serde_json::from_slice::<Vec<String>>(raw) {
        plain
    } else {
        let records: Vec<Record> = serde_json::from_slice(raw).map_err(|_| FORMAT)?;
        if records
            .iter()
            .enumerate()
            .any(|(index, record)| record.order != index + 1)
        {
            return Err(ORDER);
        }
        records.into_iter().map(|record| record.text).collect()
    };
    if !(1..=MAX_ITEMS).contains(&text.len()) || text.iter().any(String::is_empty) {
        return Err(COUNT);
    }
    Ok(text)
}

fn execute(
    read: impl FnOnce(usize) -> Result<Vec<u8>, Failure>,
    save: impl FnOnce(&str, &[u8]) -> Result<(), Failure>,
) -> Result<Value, Failure> {
    let mut encoded = Vec::new();
    std::io::stdin()
        .take((MAX_REQUEST + 1) as u64)
        .read_to_end(&mut encoded)
        .map_err(|_| REQUEST)?;
    if encoded.len() > MAX_REQUEST {
        return Err(REQUEST);
    }
    let request: Envelope = serde_json::from_slice(&encoded).map_err(|_| REQUEST)?;
    if request.plugin_id != PLUGIN
        || request.capability_id != CAPABILITY
        || request.scene_id.is_empty()
        || request.command_id.is_some()
        || request.resource_input_profile != INPUT
        || request.resource_output_profile != OUTPUT
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
            .all(|value| value.is_ascii_hexdigit())
        || !matches!(
            resource.mime_type.as_str(),
            "application/json" | "text/plain" | "application/octet-stream"
        )
    {
        return Err(RESOURCE);
    }
    if resource.byte_length > MAX_RESOURCE {
        return Err(LIMIT);
    }
    let text = parse_text(&read(resource.byte_length)?)?;
    let refs = text.iter().map(String::as_str).collect::<Vec<_>>();
    let (format, mime, label) = request.input.format.output();
    let output = combine_text(&refs, format, MAX_OUTPUT).map_err(|error| match error {
        ComposeError::TooLarge => OUTPUT_LIMIT,
        _ => WRITE,
    })?;
    save(mime, output.as_bytes())?;
    let count = text.len();
    let bytes = output.len();
    let summary = format!(
        "已将 {count} 段正文按文件顺序组合成{label}。\n完整结果 {bytes} 字节，可预览并确认保存。"
    );
    Ok(
        json!({"ok":true,"data":{"count":count,"mimeType":mime,"byteLength":bytes,"text":summary},"error":null}),
    )
}

#[cfg(target_arch = "wasm32")]
mod bridge {
    use super::*;

    #[link(wasm_import_module = "zicode_resource_output_draft1")]
    unsafe extern "C" {
        #[link_name = "create"]
        fn output_create(mime: i32, length: i64) -> i32;
        #[link_name = "write"]
        fn output_write(index: i32, source: *const u8, length: i32) -> i32;
        #[link_name = "finish"]
        fn output_finish(index: i32) -> i32;
    }

    pub fn read(length: usize) -> Result<Vec<u8>, Failure> {
        zi_resource_input_wasi::read_prefix(0, length, length).map_err(|_| RESOURCE)
    }

    pub fn write(mime: &str, bytes: &[u8]) -> Result<(), Failure> {
        let index = match mime {
            "text/plain" => 4,
            "text/csv" => 23,
            "text/markdown" => 24,
            _ => return Err(WRITE),
        };
        if unsafe { output_create(index, bytes.len() as i64) } != 0
            || unsafe { output_write(0, bytes.as_ptr(), bytes.len() as i32) } != bytes.len() as i32
            || unsafe { output_finish(0) } != 0
        {
            return Err(WRITE);
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    print_result(execute(bridge::read, bridge::write));
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    // The standalone adapter is for fixed Windows verification only.
    use std::io::Write as _;
    let mut args = std::env::args_os().skip(1);
    let source = args.next().expect("test input file");
    let target = args.next().expect("test output file");
    print_result(execute(
        |length| {
            let mut raw = Vec::new();
            std::fs::File::open(source)
                .map_err(|_| RESOURCE)?
                .take((length + 1) as u64)
                .read_to_end(&mut raw)
                .map_err(|_| RESOURCE)?;
            if raw.len() != length {
                return Err(RESOURCE);
            }
            Ok(raw)
        },
        |_, bytes| {
            std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(target)
                .and_then(|mut file| file.write_all(bytes))
                .map_err(|_| WRITE)
        },
    ));
}

fn print_result(result: Result<Value, Failure>) {
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => println!("{}", json!({"ok":false,"data":null,"error":error})),
    }
}
