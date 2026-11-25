//! Convert an explicit offline conversation export into a new Markdown resource.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Read as _;
use zi_conversation_core::{parse_transcript, render_markdown};

const PLUGIN: &str = "com.zicode.devtools.conversation-markdown";
const CAPABILITY: &str = "devtools.chat.transcript.to_markdown";
const INPUT: &str = "resource.wasi.input.draft2";
const OUTPUT: &str = "resource.wasi.output.draft1";
const MAX_REQUEST: usize = 16 * 1024;
const MAX_FILE: usize = 64 * 1024;
const MAX_ESCAPES: usize = 12 * 1024;
const MAX_MARKDOWN: usize = 192 * 1024;

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
#[serde(deny_unknown_fields)]
struct Settings {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Resource {
    index: usize,
    resource_id: String,
    mime_type: String,
    byte_length: usize,
    content_version: String,
}

#[derive(Clone, Copy, Serialize)]
struct Failure {
    code: &'static str,
    message: &'static str,
}

const REQUEST: Failure = Failure {
    code: "INVALID_REQUEST",
    message: "会话转换请求无效。",
};
const RESOURCE: Failure = Failure {
    code: "INVALID_RESOURCE",
    message: "请选择并确认一份独立版导出的会话 JSON。",
};
const SIZE: Failure = Failure {
    code: "FILE_LIMIT",
    message: "本次导入的会话 JSON 最多 64 KiB。",
};
const ESCAPES: Failure = Failure {
    code: "ESCAPE_LIMIT",
    message: "会话 JSON 包含过多转义（上限 12,288 个），请拆分导出后重试。",
};
const CONVERSATION: Failure = Failure {
    code: "INVALID_CONVERSATION",
    message: "须为独立版版本 1 的 UTF-8 会话 JSON，包含 1–16 轮完整问答。",
};
const SAVE: Failure = Failure {
    code: "OUTPUT_FAILED",
    message: "无法生成可保存的 Markdown 文件。",
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
    let Request {
        plugin_id,
        capability_id,
        scene_id,
        command_id,
        resource_input_profile,
        resource_output_profile,
        resources,
        input: Settings {},
    } = serde_json::from_slice(&wire).map_err(|_| REQUEST)?;
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
        || !matches!(
            resource.mime_type.as_str(),
            "application/octet-stream" | "text/plain"
        )
    {
        return Err(RESOURCE);
    }
    if resource.byte_length > MAX_FILE {
        return Err(SIZE);
    }
    let source = read(resource.byte_length)?;
    let source_sha256 = format!("{:x}", Sha256::digest(&source));
    if source_sha256 != resource.content_version {
        return Err(RESOURCE);
    }
    check_escape_budget(&source)?;
    let input = std::str::from_utf8(&source).map_err(|_| CONVERSATION)?;
    let turns = parse_transcript(input).map_err(|_| CONVERSATION)?;
    let markdown = render_markdown(&turns).map_err(|_| SAVE)?;
    if markdown.len() > MAX_MARKDOWN {
        return Err(SAVE);
    }
    save(markdown.as_bytes())?;
    let completed = turns.len();
    let text = format!(
        "已转换 {completed} 轮完整问答\nMarkdown {} 字节\n请确认保存新文件。",
        markdown.len()
    );
    Ok(
        json!({"ok":true,"data":{"text":text,"turns":completed,"sourceBytes":source.len(),"sourceSha256":source_sha256,
        "markdownBytes":markdown.len()},"error":null}),
    )
}

/// Bound JSON decoder work. Each escape consumes its backslash and the next byte.
fn check_escape_budget(source: &[u8]) -> Result<(), Failure> {
    let mut after_backslash = false;
    let mut remaining = MAX_ESCAPES;
    for &byte in source {
        if after_backslash {
            after_backslash = false;
        } else if byte == b'\\' {
            remaining = remaining.checked_sub(1).ok_or(ESCAPES)?;
            after_backslash = true;
        }
    }
    Ok(())
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

    pub fn read(length: usize) -> Result<Vec<u8>, Failure> {
        zi_resource_input_wasi::read_window(0, length, 0, length).map_err(|_| RESOURCE)
    }

    pub fn save(bytes: &[u8]) -> Result<(), Failure> {
        const MARKDOWN_MIME: i32 = 24;
        let slot = unsafe { output_create(MARKDOWN_MIME, bytes.len() as i64) };
        if slot != 0 {
            return Err(SAVE);
        }
        for chunk in bytes.chunks(32 * 1024) {
            let length = chunk.len() as i32;
            if unsafe { output_write(slot, chunk.as_ptr() as i32, length) } != length {
                return Err(SAVE);
            }
        }
        if unsafe { output_finish(slot) } != 0 {
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
        |markdown| {
            std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(target)
                .and_then(|mut file| file.write_all(markdown))
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
