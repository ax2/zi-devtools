//! Convert one confirmed tabular resource into an integrity-checked SQLite database.
use rusqlite::{Connection, DatabaseName};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;
#[cfg(not(target_arch = "wasm32"))]
use std::io::Write;
use zi_table_core::{
    DataFormat, Dataset,
    sqlite::{create_verified_table, validate_identifiers},
};

const PLUGIN: &str = "com.zicode.devtools.table-sqlite";
const CAPABILITY: &str = "devtools.data.sqlite.export";
const INPUT_PROFILE: &str = "resource.wasi.input.draft2";
const OUTPUT_PROFILE: &str = "resource.wasi.output.draft1";
const MAX_SOURCE: usize = 16 * 1024;
const MAX_ROWS: usize = 128;
const MAX_COLUMNS: usize = 16;
const MAX_OUTPUT: usize = 128 * 1024;
const RESULT_VERSION: &str = "1.0.0-rc.1";
type Failure = (&'static str, &'static str);

const INVALID_REQUEST: Failure = ("INVALID_REQUEST", "表格另存 SQLite 请求无效。");
const INVALID_RESOURCE: Failure = ("INVALID_RESOURCE", "请选择并确认一份本机 UTF-8 表格。");
const READ_FAILED: Failure = ("RESOURCE_READ_FAILED", "无法读取已确认的表格。");
const OUTPUT_FAILED: Failure = ("OUTPUT_FAILED", "无法生成经完整性检查的 SQLite 文件。");

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    plugin_id: String,
    capability_id: String,
    scene_id: String,
    command_id: Value,
    resource_input_profile: String,
    resource_output_profile: Option<String>,
    resources: Vec<File>,
    input: Options,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct File {
    index: usize,
    resource_id: String,
    mime_type: String,
    content_version: String,
    byte_length: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Options {
    format: Format,
    table_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Format {
    Csv,
    Tsv,
    Json,
}

fn run() -> Result<Value, Failure> {
    const MAX_REQUEST: usize = 16 * 1024;
    let mut envelope = Vec::new();
    std::io::stdin()
        .take((MAX_REQUEST + 1) as u64)
        .read_to_end(&mut envelope)
        .map_err(|_| INVALID_REQUEST)?;
    if envelope.len() > MAX_REQUEST {
        return Err(INVALID_REQUEST);
    }
    let request: Request = serde_json::from_slice(&envelope).map_err(|_| INVALID_REQUEST)?;
    if request.plugin_id != PLUGIN
        || request.capability_id != CAPABILITY
        || request.resource_input_profile != INPUT_PROFILE
        || request.resource_output_profile.as_deref() != Some(OUTPUT_PROFILE)
        || request.scene_id.is_empty()
        || request.scene_id.len() > 128
        || !request.command_id.is_null()
    {
        return Err(INVALID_REQUEST);
    }
    let [source] = request.resources.as_slice() else {
        return Err(INVALID_RESOURCE);
    };
    if source.index != 0
        || source.resource_id.is_empty()
        || source.content_version.len() != 64
        || !source
            .content_version
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(INVALID_RESOURCE);
    }
    if !matches!(
        source.mime_type.as_str(),
        "application/octet-stream" | "application/json" | "text/plain" | "text/csv"
    ) {
        return Err(("UNSUPPORTED_MIME", "请选择一份 CSV、TSV 或 JSON 表格。"));
    }
    if source.byte_length > MAX_SOURCE {
        return Err(("INPUT_TOO_LARGE", "输入文件最多 16 KiB。"));
    }
    let bytes = read(source.byte_length)?;
    let text =
        std::str::from_utf8(&bytes).map_err(|_| ("INVALID_UTF8", "请选择 UTF-8 编码的表格。"))?;
    let (format, delimiter) = match request.input.format {
        Format::Csv => (DataFormat::Csv, b','),
        Format::Tsv => (DataFormat::Csv, b'\t'),
        Format::Json => (DataFormat::Json, b','),
    };
    let table = request
        .input
        .table_name
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or("data");
    let data = Dataset::parse_with_limits(text, format, delimiter, MAX_ROWS, MAX_COLUMNS).map_err(
        |_| {
            (
                "INVALID_TABLE",
                "请检查所选格式、列名和每行长度。表格最多 128 行、16 列。",
            )
        },
    )?;
    validate_identifiers(table, &data.headers).map_err(|_| {
        (
            "INVALID_IDENTIFIERS",
            "请使用不超过 512 字节的有效表名和列名。SQLite 保留表名和仅大小写不同的列名不可用。",
        )
    })?;
    let mut conn = Connection::open_in_memory().map_err(|_| OUTPUT_FAILED)?;
    let row_count =
        create_verified_table(&mut conn, table, &data, || Ok(())).map_err(|_| OUTPUT_FAILED)?;
    let output = conn
        .serialize(DatabaseName::Main)
        .map_err(|_| OUTPUT_FAILED)?;
    if output.len() > MAX_OUTPUT {
        return Err(("OUTPUT_TOO_LARGE", "SQLite 结果最多 128 KiB。"));
    }
    publish(&output)?;
    Ok(json!({
        "tableName": table, "rows": row_count, "columns": data.headers.len(), "byteLength": output.len(),
        "text": format!("已生成 {row_count} 行、{} 列的 SQLite 文件。请确认后另存。", data.headers.len()),
    }))
}

#[cfg(target_arch = "wasm32")]
fn read(length: usize) -> Result<Vec<u8>, Failure> {
    zi_resource_input_wasi::read_window(0, length, 0, length).map_err(|_| READ_FAILED)
}

#[cfg(not(target_arch = "wasm32"))]
fn read(length: usize) -> Result<Vec<u8>, Failure> {
    let path = std::env::args_os().nth(1).ok_or(READ_FAILED)?;
    let input = std::fs::File::open(path).map_err(|_| READ_FAILED)?;
    let mut bytes = Vec::new();
    input
        .take((length + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| READ_FAILED)?;
    if bytes.len() != length {
        return Err(READ_FAILED);
    }
    Ok(bytes)
}

#[cfg(target_arch = "wasm32")]
fn publish(bytes: &[u8]) -> Result<(), Failure> {
    const SQLITE: i32 = 26; // application/x-zicode-sqlite, from resource.output.types.v1
    #[link(wasm_import_module = "zicode_resource_output_draft1")]
    unsafe extern "C" {
        #[link_name = "create"]
        fn create(mime: i32, length: i64) -> i32;
        #[link_name = "write"]
        fn write(index: i32, pointer: *const u8, length: usize) -> i32;
        #[link_name = "finish"]
        fn finish(index: i32) -> i32;
    }
    let index = unsafe { create(SQLITE, bytes.len() as i64) };
    if index < 0 {
        return Err(OUTPUT_FAILED);
    }
    for block in bytes.chunks(64 * 1024) {
        if unsafe { write(index, block.as_ptr(), block.len()) } != block.len() as i32 {
            return Err(OUTPUT_FAILED);
        }
    }
    if unsafe { finish(index) } != 0 {
        return Err(OUTPUT_FAILED);
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn publish(bytes: &[u8]) -> Result<(), Failure> {
    let path = std::env::args_os().nth(2).ok_or(OUTPUT_FAILED)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| OUTPUT_FAILED)?;
    file.write_all(bytes).map_err(|_| OUTPUT_FAILED)
}

fn main() {
    let report = match run() {
        Ok(data) => {
            json!({"contractVersion": RESULT_VERSION, "ok": true, "data": data, "error": null})
        }
        Err((code, message)) => {
            json!({"contractVersion": RESULT_VERSION, "ok": false, "data": null, "error": {"code": code, "message": message}})
        }
    };
    if serde_json::to_writer(std::io::stdout(), &report).is_err() {
        std::process::exit(1);
    }
}
