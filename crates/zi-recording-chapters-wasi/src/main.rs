//! Convert one explicitly shared chapter plan to the native recorder metadata format.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{io::Read as _, time::Duration};
use zi_recording_chapters_core::{MAX_CHAPTERS, Marker, ffmetadata, timestamp};

const PLUGIN: &str = "com.zicode.devtools.recording-chapters";
const CAPABILITY: &str = "devtools.recording.chapters.export";
const INPUT: &str = "resource.wasi.input.draft2";
const OUTPUT: &str = "resource.wasi.output.draft1";
const MAX_RESOURCE: usize = 32 * 1024;
const MAX_REQUEST: usize = 16 * 1024;
const MAX_OUTPUT: usize = 48 * 1024;
const MAX_DURATION: u64 = 31 * 24 * 60 * 60 * 1000;

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
    input: Empty,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    duration_ms: u64,
    chapters: Vec<Chapter>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Chapter {
    start_ms: u64,
    #[serde(default)]
    title: String,
}

#[derive(Clone, Copy, Serialize)]
struct Failure {
    code: &'static str,
    message: &'static str,
}

const REQUEST: Failure = Failure {
    code: "INVALID_REQUEST",
    message: "章节导出请求无效。",
};
const RESOURCE: Failure = Failure {
    code: "INVALID_RESOURCE",
    message: "请单独确认一份本机 UTF-8 JSON 章节计划。",
};
const LIMIT: Failure = Failure {
    code: "INPUT_LIMIT",
    message: "章节计划文件最多 32 KiB。",
};
const FORMAT: Failure = Failure {
    code: "INVALID_PLAN",
    message: "计划必须是包含 durationMs 和 chapters 的 UTF-8 JSON；每项包含整数 startMs 和可选 title。",
};
const DURATION: Failure = Failure {
    code: "DURATION_LIMIT",
    message: "durationMs 必须是大于零且不超过 31 天的毫秒数。",
};
const COUNT: Failure = Failure {
    code: "CHAPTER_LIMIT",
    message: "一份计划最多 256 项。",
};
const EMPTY: Failure = Failure {
    code: "NO_CHAPTERS",
    message: "没有位于指定时长内的章节。",
};
const ORDER: Failure = Failure {
    code: "CHAPTER_ORDER",
    message: "位于时长内的章节起点必须严格递增，同一毫秒不能使用两次。",
};
const TITLE: Failure = Failure {
    code: "INVALID_TITLE",
    message: "可导出的章节标题须在 512 UTF-8 字节以内，且只允许换行和制表符作为控制字符。",
};
const OUTPUT_LIMIT: Failure = Failure {
    code: "OUTPUT_LIMIT",
    message: "转义后的完整章节元数据超过 48 KiB。请缩短标题后再试。",
};
const SAVE: Failure = Failure {
    code: "OUTPUT_FAILED",
    message: "无法生成或登记完整章节元数据。",
};

fn execute(
    read: impl FnOnce(usize) -> Result<Vec<u8>, Failure>,
    save: impl FnOnce(&[u8]) -> Result<(), Failure>,
) -> Result<Value, Failure> {
    let mut wire = Vec::new();
    std::io::stdin()
        .take((MAX_REQUEST + 1) as u64)
        .read_to_end(&mut wire)
        .map_err(|_| REQUEST)?;
    if wire.len() > MAX_REQUEST {
        return Err(REQUEST);
    }
    let request: Request = serde_json::from_slice(&wire).map_err(|_| REQUEST)?;
    let Request {
        plugin_id,
        capability_id,
        scene_id,
        command_id,
        resource_input_profile,
        resource_output_profile,
        resources,
        input: Empty {},
    } = request;
    if plugin_id != PLUGIN
        || capability_id != CAPABILITY
        || scene_id.is_empty()
        || command_id.is_some()
        || resource_input_profile != INPUT
        || resource_output_profile != OUTPUT
    {
        return Err(REQUEST);
    }
    let [resource] = resources.as_slice() else {
        return Err(RESOURCE);
    };
    if resource.index != 0
        || resource.resource_id.is_empty()
        || resource.content_version.len() != 64
        || !resource
            .content_version
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
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
    let source = read(resource.byte_length)?;
    let source = source.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&source);
    let Plan {
        duration_ms,
        chapters,
    } = serde_json::from_slice(source).map_err(|_| FORMAT)?;
    if !(1..=MAX_DURATION).contains(&duration_ms) {
        return Err(DURATION);
    }
    if chapters.len() > MAX_CHAPTERS {
        return Err(COUNT);
    }

    let count = chapters.len();
    let mut included = Vec::with_capacity(count);
    for chapter in chapters {
        if chapter.start_ms >= duration_ms {
            continue;
        }
        if included
            .last()
            .is_some_and(|marker: &Marker| marker.start_ms >= chapter.start_ms)
        {
            return Err(ORDER);
        }
        included.push(Marker {
            start_ms: chapter.start_ms,
            title: chapter.title,
        });
    }
    let Some(first) = included.first() else {
        return Err(EMPTY);
    };
    let metadata = ffmetadata(&included, Duration::from_millis(duration_ms)).map_err(|_| TITLE)?;
    if metadata.len() > MAX_OUTPUT {
        return Err(OUTPUT_LIMIT);
    }
    save(metadata.as_bytes())?;
    let included_count = included.len();
    let skipped = count - included_count;
    let text = format!(
        "已生成 {included_count} 个章节，跳过 {skipped} 个达到或超过时长的起点。\n起点 {} · 结束 {}\n格式 FFMETADATA1，Studio 以 .txt 保存。",
        timestamp(first.start_ms),
        timestamp(duration_ms)
    );
    Ok(
        json!({"ok":true,"data":{"durationMs":duration_ms,"firstStartMs":first.start_ms,
        "chapterCount":included_count,"skippedCount":skipped,"byteLength":metadata.len(),"text":text},"error":null}),
    )
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

    pub fn read(length: usize) -> Result<Vec<u8>, Failure> {
        if unsafe { resource_size(0) } != length as i64 {
            return Err(RESOURCE);
        }
        let mut bytes = vec![0_u8; length];
        if length != 0
            && unsafe { resource_read(0, 0, bytes.as_mut_ptr() as i32, length as i32) }
                != length as i32
        {
            return Err(RESOURCE);
        }
        Ok(bytes)
    }

    pub fn save(bytes: &[u8]) -> Result<(), Failure> {
        const TEXT_MIME: i32 = 4;
        if unsafe { output_create(TEXT_MIME, bytes.len() as i64) } != 0
            || unsafe { output_write(0, bytes.as_ptr() as i32, bytes.len() as i32) }
                != bytes.len() as i32
            || unsafe { output_finish(0) } != 0
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
    // Test adapter only; the Studio package contains the standalone WASI module.
    use std::io::Write as _;
    let mut args = std::env::args_os().skip(1);
    let source = args.next().expect("fixture needs an input file");
    let target = args.next().expect("fixture needs an output file");
    print_result(execute(
        |length| {
            let mut bytes = Vec::new();
            std::fs::File::open(source)
                .map_err(|_| RESOURCE)?
                .take((length + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| RESOURCE)?;
            if bytes.len() != length {
                return Err(RESOURCE);
            }
            Ok(bytes)
        },
        |output| {
            std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(target)
                .and_then(|mut file| file.write_all(output))
                .map_err(|_| SAVE)
        },
    ));
}

fn print_result(result: Result<Value, Failure>) {
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => println!("{}", json!({"ok":false,"data":null,"error":error})),
    }
}
