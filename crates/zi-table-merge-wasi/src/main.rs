//! Bounded two-table operations using the native table parser and join.
use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    sync::atomic::AtomicBool,
};
use zi_table_core::{
    DataFormat, Dataset,
    join::{Mode, combine_bounded},
};

const PLUGIN: &str = "com.zicode.devtools.table-merge";
const PREFIX: &str = "devtools.table-merge.";
const CONTRACT: &str = "1.0.0-rc.1";
const FIELD_BYTES: usize = 8192;
const IPC_BYTES: usize = 48 * 1024;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Format {
    Csv,
    Tsv,
    Json,
}

impl Format {
    fn parse(self, text: &str) -> Result<Dataset> {
        let (format, delimiter) = match self {
            Self::Csv => (DataFormat::Csv, b','),
            Self::Tsv => (DataFormat::Csv, b'\t'),
            Self::Json => (DataFormat::Json, b','),
        };
        Dataset::parse(text, format, delimiter)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: Value,
    input: Tables,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Tables {
    left: String,
    right: String,
    left_format: Format,
    right_format: Format,
    #[serde(default)]
    left_key: String,
    #[serde(default)]
    right_key: String,
}

fn selected_key<'a>(headers: &[String], key: &'a str) -> &'a str {
    if headers.iter().any(|header| header == key) {
        key
    } else {
        key.trim()
    }
}

fn execute(input: &[u8]) -> Result<String> {
    ensure!(input.len() <= IPC_BYTES, "请求超过 48 KiB");
    let request: Request =
        serde_json::from_slice(input).map_err(|_| anyhow!("请求格式或表格格式无效"))?;
    ensure!(
        request.plugin_id == PLUGIN && !request.scene_id.is_empty() && request.command_id.is_null(),
        "请求身份无效"
    );
    let mode = match request.capability_id.strip_prefix(PREFIX) {
        Some("left") => Mode::Left,
        Some("inner") => Mode::Inner,
        Some("append") => Mode::Append,
        Some("columns") => Mode::Columns,
        _ => bail!("此合并操作不可用"),
    };
    let input = request.input;
    ensure!(
        input.left.len() <= FIELD_BYTES && input.right.len() <= FIELD_BYTES,
        "每张表最多 8 KiB"
    );
    ensure!(
        input.left_key.len() <= FIELD_BYTES && input.right_key.len() <= FIELD_BYTES,
        "关联列名最多 8 KiB"
    );
    let left = input
        .left_format
        .parse(&input.left)
        .context("左表无法解析")?;
    let right = input
        .right_format
        .parse(&input.right)
        .context("右表无法解析")?;
    let left_key = selected_key(&left.headers, &input.left_key);
    let right_key = selected_key(&right.headers, &input.right_key);
    let key = if left_key.is_empty() {
        0
    } else {
        left.headers
            .iter()
            .position(|header| header == left_key)
            .context("左表不存在指定关联列")?
    };
    let result = combine_bounded(
        &left,
        &right,
        mode,
        key,
        right_key,
        &AtomicBool::new(false),
        IPC_BYTES,
    )?;
    let indices: Vec<_> = (0..result.data.rows.len()).collect();
    let csv = result.data.export(&indices, DataFormat::Csv, b',')?;
    Ok(serde_json::to_string_pretty(
        &json!({"report":result.report,"data":result.data,"csv":csv}),
    )?)
}

fn error(code: &str, message: &str) -> Value {
    json!({"contractVersion":CONTRACT,"ok":false,"data":null,"error":{"code":code,"message":message}})
}

fn main() -> Result<()> {
    let mut input = Vec::new();
    std::io::stdin()
        .take((IPC_BYTES + 1) as u64)
        .read_to_end(&mut input)?;
    let result = match execute(&input) {
        Ok(text) => json!({"contractVersion":CONTRACT,"ok":true,"data":{"text":text},"error":null}),
        Err(reason) => error("INVALID_INPUT", &format!("{reason:#}")),
    };
    let serialized = serde_json::to_vec(&result)?;
    let mut stdout = std::io::stdout().lock();
    if serialized.len() <= IPC_BYTES {
        stdout.write_all(&serialized)?;
    } else {
        serde_json::to_writer(
            stdout,
            &error("RESULT_TOO_LARGE", "结果超过 48 KiB，请缩小输入范围"),
        )?;
    }
    Ok(())
}
