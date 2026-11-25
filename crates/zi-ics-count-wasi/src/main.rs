//! Previews the inclusive cutoff of a single supported iCalendar COUNT recurrence.
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Read, Write};
use zi_ics_preview_core::count::{self, MAX_FILE_BYTES};

const PLUGIN: &str = "com.zicode.devtools.ics-count";
const CAPABILITY: &str = "devtools.calendar.ics.count_cutoff";
const PROFILE: &str = "resource.wasi.input.draft2";
const CONTRACT: &str = "1.0.0-rc.1";
const MAX_RESULT: usize = 2048;
type Failure = (&'static str, &'static str);

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: Value,
    input: Input,
    resource_input_profile: String,
    resource_output_profile: Option<String>,
    resources: Vec<Descriptor>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Descriptor {
    index: usize,
    resource_id: String,
    content_version: String,
    mime_type: String,
    byte_length: usize,
}

fn run() -> Result<Value, Failure> {
    const MAX_REQUEST: usize = 16 * 1024;
    const REQUEST: Failure = ("INVALID_REQUEST", "ICS 截止预览请求无效。");
    let mut envelope = Vec::new();
    std::io::stdin()
        .take((MAX_REQUEST + 1) as u64)
        .read_to_end(&mut envelope)
        .map_err(|_| REQUEST)?;
    if envelope.len() > MAX_REQUEST {
        return Err(REQUEST);
    }
    let request: Request = serde_json::from_slice(&envelope).map_err(|_| REQUEST)?;
    if request.plugin_id != PLUGIN
        || request.capability_id != CAPABILITY
        || request.scene_id.is_empty()
        || request.scene_id.len() > 128
        || !request.command_id.is_null()
        || request.resource_input_profile != PROFILE
        || request.resource_output_profile.is_some()
    {
        return Err(REQUEST);
    }
    let Input {} = request.input;
    const RESOURCE: Failure = ("INVALID_RESOURCE", "请选择并确认一份本机 ICS 文件。");
    let [file] = request.resources.as_slice() else {
        return Err(RESOURCE);
    };
    if file.index != 0
        || file.resource_id.is_empty()
        || file.content_version.len() != 64
        || !file
            .content_version
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(RESOURCE);
    }
    if !matches!(
        file.mime_type.as_str(),
        "text/plain" | "application/octet-stream"
    ) {
        return Err(("UNSUPPORTED_MIME", "请选择本机 ICS 文件。"));
    }
    if file.byte_length > MAX_FILE_BYTES {
        return Err(("INPUT_TOO_LARGE", "单份 ICS 文件最多 16 KiB。"));
    }
    let bytes = read(file.byte_length)
        .map_err(|_| ("RESOURCE_READ_FAILED", "无法读取本次确认的 ICS 文件。"))?;
    let report = count::read(&bytes).map_err(|code| (code, message(code)))?;
    serde_json::to_value(report).map_err(|_| ("RESULT_FAILED", "无法显示日程检查结果。"))
}

fn message(code: &str) -> &'static str {
    match code {
        "INVALID_UTF8" => "ICS 需要 UTF-8 编码。",
        "INVALID_CALENDAR" => "ICS 结构无效或行数超过 512。",
        "SINGLE_EVENT_REQUIRED" => "请选择恰好包含一个日程的 ICS 文件。",
        "UNSUPPORTED_COMPONENT" => {
            "支持单个 VEVENT，可忽略日程提醒；含时区定义、任务或其他组件的日历需先拆分。"
        }
        "UNSUPPORTED_CALENDAR" => "请选择公历中的普通日程文件。",
        "UNSUPPORTED_TIME" => "仅支持全天或浮动时间的重复规则。带时区或 UTC 的日程请在原日历检查。",
        "INVALID_START" => "开始日期需要在 1901–2099 年内，浮动时间须为整分钟。",
        "UNSUPPORTED_EXCEPTION" => "日程包含例外、附加日期或其他需原日历解析的条件。",
        "UNSUPPORTED_EVENT_STATUS" => "请选择已确认的日程。",
        "COUNT_REQUIRED" => "日程需要包含带 COUNT 的有限重复规则。",
        "INVALID_RULE" => {
            "重复规则无效或超出支持范围。COUNT 必须为正数，不得与 UNTIL 同用，末次开始须处于 1901–2099 年。"
        }
        _ => "无法查看该 ICS 日程的重复截止。",
    }
}

#[cfg(target_arch = "wasm32")]
fn read(length: usize) -> Result<Vec<u8>, &'static str> {
    zi_resource_input_wasi::read_window(0, length, 0, length)
}

#[cfg(not(target_arch = "wasm32"))]
fn read(length: usize) -> Result<Vec<u8>, &'static str> {
    const FAILED: &str = "RESOURCE_READ_FAILED";
    let path = std::env::args_os().nth(1).ok_or(FAILED)?;
    let file = std::fs::File::open(path).map_err(|_| FAILED)?;
    let mut bytes = Vec::new();
    file.take((length + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| FAILED)?;
    if bytes.len() != length {
        return Err(FAILED);
    }
    Ok(bytes)
}

fn error((code, message): Failure) -> Value {
    json!({"contractVersion":CONTRACT,"ok":false,"data":null,"error":{"code":code,"message":message}})
}

fn main() {
    let result = match run() {
        Ok(data) => json!({"contractVersion":CONTRACT,"ok":true,"data":data,"error":null}),
        Err(reason) => error(reason),
    };
    let mut bytes = serde_json::to_vec(&result).expect("valid ICS cutoff preview");
    if bytes.len() > MAX_RESULT {
        bytes = serde_json::to_vec(&error(("RESULT_TOO_LARGE", "预览结果超过输出限制。")))
            .expect("fixed result error");
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
