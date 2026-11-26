//! Validates one approved clock backup without actuating its contents.
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Read, Write};
use zi_clock_snapshot_core::{FILE_LIMIT, Policy, Snapshot};

const PLUGIN: &str = "com.zicode.devtools.clock-backup";
const CAPABILITY: &str = "devtools.clock.backup.inspect";
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

fn summarize(snapshot: Snapshot, bytes: usize) -> Value {
    let saved_at = snapshot
        .saved_at
        .to_rfc3339_opts(zi_clock_snapshot_core::SecondsFormat::AutoSi, true);
    let (policy, policy_label) = match snapshot.policy {
        Policy::Pause => ("pause", "按剩余时间暂停恢复"),
        Policy::Continue => ("continue", "按原到期时间继续"),
    };
    let audio = snapshot.audio.is_some();
    let legacy = snapshot.schema == 1;
    let timers = snapshot.timers.len();
    let alarms = snapshot.alarms.len();
    let laps = snapshot.laps.len();
    let notices = snapshot.notices.len();
    let zones = snapshot.zones.len();
    let windows = snapshot.window_sizes.len();
    let mut text = format!(
        "时钟备份 v{} 通过校验\n保存时间（UTC） {saved_at}\n{timers} 个计时器 · {alarms} 个闹钟 · {laps} 个秒表分段 · {notices} 条提醒\n{zones} 个世界时区 · {windows} 条小窗尺寸 · 声音设置{}\n备份中的倒计时恢复方式：{policy_label}",
        snapshot.schema,
        if audio { "已记录" } else { "未记录" },
    );
    if legacy {
        text.push_str("\n独立版确认恢复并保存后会升级 v1，并保留旧格式校验备份。");
    }
    json!({"schemaVersion":snapshot.schema,"savedAtUtc":saved_at,"byteLength":bytes,
           "timerCount":timers,"alarmCount":alarms,"lapCount":laps,"noticeCount":notices,
           "zoneCount":zones,"windowSizeCount":windows,"audioSettingsPresent":audio,
           "storedTimerPolicy":policy,"legacyV1":legacy,"text":text})
}

#[cfg(target_arch = "wasm32")]
mod scan {
    use core::arch::wasm32::*;

    // Each SIMD load is within a complete 16-byte slice chunk.
    #[target_feature(enable = "simd128")]
    pub(super) unsafe fn padding(bytes: &[u8]) -> bool {
        let (chunks, remainder) = bytes.as_chunks::<16>();
        chunks.iter().all(|chunk| {
            let word = unsafe { v128_load(chunk.as_ptr().cast()) };
            let horizontal = v128_or(
                u8x16_eq(word, u8x16_splat(b' ')),
                u8x16_eq(word, u8x16_splat(b'\t')),
            );
            let vertical = v128_or(
                u8x16_eq(word, u8x16_splat(b'\n')),
                u8x16_eq(word, u8x16_splat(b'\r')),
            );
            u8x16_all_true(v128_or(horizontal, vertical))
        }) && remainder
            .iter()
            .all(|byte| matches!(byte, b' ' | b'\n' | b'\t' | b'\r'))
    }

    #[target_feature(enable = "simd128")]
    pub(super) unsafe fn escapes(bytes: &[u8], limit: usize) -> bool {
        let mut count = 0;
        let (chunks, remainder) = bytes.as_chunks::<16>();
        for chunk in chunks {
            let word = unsafe { v128_load(chunk.as_ptr().cast()) };
            count += u8x16_bitmask(u8x16_eq(word, u8x16_splat(b'\\'))).count_ones() as usize;
            if count > limit {
                return false;
            }
        }
        count + remainder.iter().filter(|byte| **byte == b'\\').count() <= limit
    }
}

#[cfg(target_arch = "wasm32")]
fn json_whitespace(bytes: &[u8]) -> bool {
    // SIMD is validated by Wasmtime when it loads this module.
    unsafe { scan::padding(bytes) }
}

#[cfg(not(target_arch = "wasm32"))]
fn json_whitespace(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .all(|byte| matches!(byte, b' ' | b'\n' | b'\t' | b'\r'))
}

fn object_bytes(bytes: &[u8]) -> Option<&[u8]> {
    let start = memchr::memchr(b'{', bytes)?;
    let end = memchr::memrchr(b'}', bytes)?;
    (start < end && json_whitespace(&bytes[..start]) && json_whitespace(&bytes[end + 1..]))
        .then(|| &bytes[start..=end])
}

const MAX_JSON_WORK: usize = 224 * 1024;
const ESCAPE_COST: usize = 6;
const PADDING_DIVISOR: usize = 4;

fn fits_json_budget(file_bytes: usize, document: &[u8]) -> bool {
    let padding = (file_bytes - document.len()).div_ceil(PADDING_DIVISOR);
    let Some(available) = MAX_JSON_WORK.checked_sub(document.len() + padding) else {
        return false;
    };
    let limit = available / ESCAPE_COST;
    #[cfg(target_arch = "wasm32")]
    // SIMD is validated by Wasmtime when it loads this module.
    return unsafe { scan::escapes(document, limit) };
    #[cfg(not(target_arch = "wasm32"))]
    memchr::memchr_iter(b'\\', document).nth(limit).is_none()
}

fn run() -> Result<Value, Failure> {
    const MAX_REQUEST: usize = 16 * 1024;
    const REQUEST: Failure = ("INVALID_REQUEST", "时钟备份检查请求无效。");
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
    const RESOURCE: Failure = ("INVALID_RESOURCE", "请选择并确认一份本机时钟备份。");
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
        "application/json" | "application/octet-stream"
    ) {
        return Err(("UNSUPPORTED_MIME", "请选择时钟备份 JSON。"));
    }
    if file.byte_length > FILE_LIMIT {
        return Err(("INPUT_TOO_LARGE", "单份时钟备份最多 512 KiB。"));
    }
    let bytes = read(file.byte_length)
        .map_err(|_| ("RESOURCE_READ_FAILED", "无法读取已确认的时钟备份。"))?;
    const INVALID_JSON: Failure = ("INVALID_JSON", "文件不是有效的时钟备份 JSON。");
    let content = object_bytes(&bytes).ok_or(INVALID_JSON)?;
    if !fits_json_budget(bytes.len(), content) {
        return Err((
            "SNAPSHOT_COMPLEXITY",
            "备份 JSON 内容或转义过密。请使用独立版原始保存文件，或在独立版检查这份备份。",
        ));
    }
    let snapshot: Snapshot = serde_json::from_slice(content)
        .map_err(|_| ("INVALID_JSON", "文件不是有效的时钟备份 JSON。"))?;
    if !(1..=3).contains(&snapshot.schema) {
        return Err(("UNSUPPORTED_VERSION", "支持独立版时钟备份 v1、v2 和 v3。"));
    }
    snapshot.validate().map_err(|_| {
        (
            "INVALID_SNAPSHOT",
            "此备份包含不一致或越界的时钟设置、记录或关联。",
        )
    })?;
    Ok(summarize(snapshot, bytes.len()))
}

#[cfg(target_arch = "wasm32")]
fn read(length: usize) -> Result<Vec<u8>, ()> {
    // The production Worker checks the digest before mapping these frozen source bytes.
    zi_resource_input_wasi::read_window(0, length, 0, length).map_err(|_| ())
}

#[cfg(not(target_arch = "wasm32"))]
fn read(length: usize) -> Result<Vec<u8>, ()> {
    let file = std::env::args_os().nth(1).ok_or(())?;
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .map_err(|_| ())?
        .take((length + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    (bytes.len() == length).then_some(bytes).ok_or(())
}

fn main() -> std::io::Result<()> {
    let result = match run() {
        Ok(data) => json!({"contractVersion":CONTRACT,"ok":true,"data":data,"error":null}),
        Err((code, message)) => {
            json!({"contractVersion":CONTRACT,"ok":false,"data":null,"error":{"code":code,"message":message}})
        }
    };
    let mut bytes = serde_json::to_vec(&result).expect("valid clock backup report");
    if bytes.len() > MAX_RESULT {
        bytes = br#"{"contractVersion":"1.0.0-rc.1","ok":false,"data":null,"error":{"code":"RESULT_TOO_LARGE","message":"Clock backup report exceeds 2 KiB."}}"#.to_vec();
    }
    std::io::stdout().write_all(&bytes)
}
