//! Replay a standalone table recipe and publish only the confirmed result.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Read as _, sync::atomic::AtomicBool};
use zi_table_core::{
    DataFormat, Dataset,
    workflow::{Definition, Output, PipelineLimit, StepError, write_csv},
};

const PLUGIN: &str = "com.zicode.devtools.table-pipeline";
const CAPABILITY: &str = "devtools.table.pipeline.replay";
const INPUT: &str = "resource.wasi.input.draft2";
const OUTPUT: &str = "resource.wasi.output.draft1";
const RECIPE_BYTES: usize = 16 * 1024;
const TABLE_BYTES: usize = 16 * 1024;
const INTERMEDIATE_BYTES: usize = 16 * 1024;
const STEP_WORK: usize = 128 * 1024;
const CSV_BYTES: usize = 32 * 1024;
const REQUEST_BYTES: usize = 16 * 1024;
const ROWS: usize = 96;
const COLUMNS: usize = 16;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    plugin_id: String,
    capability_id: String,
    scene_id: String,
    command_id: Option<String>,
    resource_input_profile: String,
    resource_output_profile: String,
    resources: Vec<Resource>,
    input: Settings,
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
struct Settings {
    #[serde(default)]
    format: Format,
    #[serde(default = "first_position", rename = "recipePosition")]
    recipe_position: u8,
}

fn first_position() -> u8 {
    1
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Format {
    #[default]
    Csv,
    Tsv,
    Json,
}

impl Format {
    fn parser(&self) -> (DataFormat, u8) {
        match self {
            Self::Csv => (DataFormat::Csv, b','),
            Self::Tsv => (DataFormat::Csv, b'\t'),
            Self::Json => (DataFormat::Json, b','),
        }
    }
}

#[derive(Clone, Copy, Serialize)]
struct Failure {
    code: &'static str,
    message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    step: Option<usize>,
}

impl Failure {
    const fn new(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            step: None,
        }
    }
}

const REQUEST: Failure = Failure::new("INVALID_REQUEST", "请选择表格格式，配方位置可填 1 或 2。");
const RESOURCE: Failure = Failure::new(
    "INVALID_RESOURCE",
    "请选择并确认两份文件：一份管线 JSON 配方，一份数据表格。",
);
const TOO_LARGE: Failure = Failure::new("FILE_LIMIT", "管线配方和表格文件分别最多 16 KiB。");
const RECIPE: Failure = Failure::new(
    "INVALID_RECIPE",
    "指定的配方文件必须是独立版格式 1–3 的有效表格管线 JSON。",
);
const TABLE: Failure = Failure::new(
    "INVALID_TABLE",
    "数据文件须是所选格式的有效 UTF-8 表格，最多 96 行、16 列。",
);
const WORK: Failure = Failure::new(
    "WORK_LIMIT",
    "本次管线的中间表格或累计处理规模超过上限，请缩小表格或分段运行。",
);
const POLICY: Failure = Failure::new(
    "CSV_POLICY",
    "请在配方中启用 CSV 公式保护，或删除另存格式设置。",
);
const STEP: Failure = Failure::new(
    "STEP_FAILED",
    "管线步骤失败。请核对该步骤中的列名及数据类型。",
);
const SAVE: Failure = Failure::new("OUTPUT_FAILED", "无法生成可保存的 CSV，请缩小结果范围。");

fn execute(
    mut read: impl FnMut(usize, usize) -> Result<Vec<u8>, Failure>,
    save: impl FnOnce(&[u8]) -> Result<(), Failure>,
) -> Result<Value, Failure> {
    let mut wire = Vec::new();
    std::io::stdin()
        .take((REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut wire)
        .map_err(|_| REQUEST)?;
    if wire.len() > REQUEST_BYTES {
        return Err(REQUEST);
    }
    let request: Request = serde_json::from_slice(&wire).map_err(|_| REQUEST)?;
    if request.plugin_id != PLUGIN
        || request.capability_id != CAPABILITY
        || request.scene_id.is_empty()
        || request.command_id.is_some()
        || request.resource_input_profile != INPUT
        || request.resource_output_profile != OUTPUT
    {
        return Err(REQUEST);
    }
    let [first, second] = request.resources.as_slice() else {
        return Err(RESOURCE);
    };
    let (recipe, table) = match request.input.recipe_position {
        1 => (first, second),
        2 => (second, first),
        _ => return Err(REQUEST),
    };
    if recipe.resource_id == table.resource_id
        || !matches!(
            recipe.mime_type.as_str(),
            "application/octet-stream" | "text/plain"
        )
        || !matches!(
            table.mime_type.as_str(),
            "application/octet-stream" | "text/plain" | "text/csv"
        )
    {
        return Err(RESOURCE);
    }
    if recipe.byte_length > RECIPE_BYTES || table.byte_length > TABLE_BYTES {
        return Err(TOO_LARGE);
    }
    for (index, resource) in request.resources.iter().enumerate() {
        if resource.index != index
            || resource.resource_id.is_empty()
            || resource.content_version.len() != 64
        {
            return Err(RESOURCE);
        }
    }
    let mut select = |resource: &Resource| {
        let bytes = read(resource.index, resource.byte_length)?;
        if format!("{:x}", Sha256::digest(&bytes)) != resource.content_version {
            return Err(RESOURCE);
        }
        Ok(bytes)
    };
    let recipe_bytes = select(recipe)?;
    let definition = Definition::parse(
        recipe_bytes
            .strip_prefix(b"\xef\xbb\xbf")
            .unwrap_or(&recipe_bytes),
    )
    .map_err(|_| RECIPE)?;
    if !matches!(
        definition.output,
        None | Some(Output::Csv {
            protect_formulas: true,
            ..
        })
    ) {
        return Err(POLICY);
    }
    let table_bytes = select(table)?;
    let text = std::str::from_utf8(&table_bytes).map_err(|_| TABLE)?;
    let (format, delimiter) = request.input.format.parser();
    let source =
        Dataset::parse_with_limits(text, format, delimiter, ROWS, COLUMNS).map_err(|_| TABLE)?;
    let preview = definition
        .preview_with_limits(
            &source,
            definition.steps.len(),
            &AtomicBool::new(false),
            INTERMEDIATE_BYTES,
            Some(STEP_WORK),
        )
        .map_err(|error| {
            if error.downcast_ref::<PipelineLimit>().is_some() {
                return WORK;
            }
            Failure {
                step: error.downcast_ref::<StepError>().map(|error| error.0),
                ..STEP
            }
        })?;
    let mut writer = csv::Writer::from_writer(Vec::new());
    write_csv(&preview.result, &mut writer, true, || Ok(())).map_err(|_| SAVE)?;
    let csv = writer.into_inner().map_err(|_| SAVE)?;
    if csv.len() > CSV_BYTES {
        return Err(SAVE);
    }
    save(&csv)?;
    let (before_rows, after_rows, columns) = (
        source.rows.len(),
        preview.result.rows.len(),
        preview.result.headers.len(),
    );
    let step_count = preview.steps.len();
    let text = format!(
        "管线已运行 {step_count} 步\n数据行 {before_rows} → {after_rows} · 结果列 {columns}\nCSV {} 字节 · 已启用公式保护\n请确认保存新文件。",
        csv.len()
    );
    Ok(
        json!({"ok":true,"data":{"text":text,"steps":step_count,"rowsBefore":before_rows,"rowsAfter":after_rows,
        "columns":columns,"csvBytes":csv.len(),"formulaProtection":true},"error":null}),
    )
}

#[cfg(target_arch = "wasm32")]
mod bridge {
    use super::*;

    #[link(wasm_import_module = "zicode_resource_output_draft1")]
    unsafe extern "C" {
        #[link_name = "create"]
        fn output_create(mime_index: i32, length: i64) -> i32;
        #[link_name = "write"]
        fn output_write(index: i32, source: i32, length: i32) -> i32;
        #[link_name = "finish"]
        fn output_finish(index: i32) -> i32;
    }

    pub fn read(index: usize, length: usize) -> Result<Vec<u8>, Failure> {
        zi_resource_input_wasi::read_window(index as i32, length, 0, length).map_err(|_| RESOURCE)
    }

    pub fn save(bytes: &[u8]) -> Result<(), Failure> {
        const CSV_MIME: i32 = 23;
        let length = bytes.len() as i32;
        let slot = unsafe { output_create(CSV_MIME, i64::from(length)) };
        if slot != 0
            || unsafe { output_write(slot, bytes.as_ptr() as i32, length) } != length
            || unsafe { output_finish(slot) } != 0
        {
            return Err(SAVE);
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    print_result(execute(bridge::read, bridge::save));
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    // Standalone parity adapter; this is never part of the plugin package.
    use std::io::Write as _;
    let mut args = std::env::args_os().skip(1);
    let paths: Vec<_> = args.by_ref().take(2).collect();
    let target = args.next().expect("fixture needs an output file");
    print_result(execute(
        |index, length| {
            let mut bytes = Vec::new();
            std::fs::File::open(&paths[index])
                .map_err(|_| RESOURCE)?
                .take((length + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| RESOURCE)?;
            if bytes.len() != length {
                return Err(RESOURCE);
            }
            Ok(bytes)
        },
        |csv| {
            std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(target)
                .and_then(|mut file| file.write_all(csv))
                .map_err(|_| SAVE)
        },
    ));
}

fn print_result(result: Result<Value, Failure>) {
    println!(
        "{}",
        match result {
            Ok(value) => value,
            Err(error) => json!({"ok":false,"data":null,"error":error}),
        }
    );
}
