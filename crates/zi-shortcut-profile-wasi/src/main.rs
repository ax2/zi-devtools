//! Review two explicitly shared ZiDevTools shortcut profiles and export a portable result.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fmt::Write as _, io::Read as _};
use zi_shortcut_profile_core::{Bindings, candidate, encode, parse, sequences_overlap};

const PLUGIN: &str = "com.zicode.devtools.shortcut-profile";
const CAPABILITY: &str = "devtools.shortcuts.profile.merge";
const INPUT: &str = "resource.wasi.input.draft2";
const OUTPUT: &str = "resource.wasi.output.draft1";
const MAX_REQUEST: usize = 16 * 1024;
const MAX_FILE: usize = 16 * 1024;
const MAX_BINDINGS: usize = 32;
const MAX_RESULT: usize = 48 * 1024;
const MAX_EXAMPLES: usize = 6;

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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Settings {
    #[serde(default)]
    mode: Mode,
    #[serde(default = "second_position")]
    incoming_position: u8,
}

fn second_position() -> u8 {
    2
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    Merge,
    Replace,
}

#[derive(Clone, Copy, Serialize)]
struct Failure {
    code: &'static str,
    message: &'static str,
}

const REQUEST: Failure = Failure {
    code: "INVALID_REQUEST",
    message: "请选择合并或替换，并填写导入文件位置 1 或 2。",
};
const RESOURCE: Failure = Failure {
    code: "INVALID_RESOURCE",
    message: "请确认共享一份基准配置和一份导入配置。",
};
const FILE_LIMIT: Failure = Failure {
    code: "FILE_LIMIT",
    message: "每份快捷配置 JSON 最多 16 KiB。",
};
const BINDING_LIMIT: Failure = Failure {
    code: "BINDING_LIMIT",
    message: "每份快捷配置最多 32 项。",
};
const BASE: Failure = Failure {
    code: "INVALID_BASE",
    message: "基准文件必须是有效的独立版快捷配置 JSON（版本 1）。",
};
const INCOMING: Failure = Failure {
    code: "INVALID_INCOMING",
    message: "导入文件必须是有效的独立版快捷配置 JSON（版本 1）。",
};
const SAVE: Failure = Failure {
    code: "OUTPUT_FAILED",
    message: "无法生成可保存的快捷配置文件。",
};

fn shortened(id: &str) -> &str {
    // The native parser restricts identifiers to ASCII.
    if id.len() > 80 { &id[..80] } else { id }
}

fn label(sequence: Option<&String>) -> &str {
    match sequence.map(String::as_str) {
        None => "无自定义项",
        Some("") => "停用",
        Some(sequence) => sequence,
    }
}

fn preview(base: &Bindings, merged: &Bindings, mode: &Mode) -> (String, usize, usize) {
    let action = match mode {
        Mode::Merge => "合并",
        Mode::Replace => "替换",
    };
    let mut detail = String::new();
    let mut changes = 0;
    for id in base.keys().chain(merged.keys()).collect::<BTreeSet<_>>() {
        let old = base.get(id);
        let new = merged.get(id);
        if old != new {
            changes += 1;
            if changes <= MAX_EXAMPLES {
                let _ = writeln!(
                    detail,
                    "{}{}：{} → {}",
                    shortened(id),
                    if id.len() > 80 { "…" } else { "" },
                    label(old),
                    label(new)
                );
            }
        }
    }
    let mut conflicts = 0;
    let mut slots = vec![("search", "K")];
    slots.extend(
        merged
            .iter()
            .filter(|(id, sequence)| id.as_str() != "search" && !sequence.is_empty())
            .map(|(id, sequence)| (id.as_str(), sequence.as_str())),
    );
    for (i, (left, left_keys)) in slots.iter().enumerate() {
        for (right, right_keys) in &slots[i + 1..] {
            if sequences_overlap(left_keys, right_keys) {
                conflicts += 1;
                if conflicts <= MAX_EXAMPLES {
                    let _ = writeln!(
                        detail,
                        "{} [{}] 与 {} [{}] 冲突",
                        shortened(left),
                        left_keys,
                        shortened(right),
                        right_keys
                    );
                }
            }
        }
    }
    let decision = if conflicts == 0 {
        "请确认保存新配置。"
    } else {
        "请修正冲突后重试；本次没有可保存文件。"
    };
    let text = format!(
        "{action}预览 · {} 项 → {} 项\n变更 {changes} 项，快捷序列冲突 {conflicts} 对\n{detail}{decision}",
        base.len(),
        merged.len()
    );
    let more = changes > MAX_EXAMPLES || conflicts > MAX_EXAMPLES;
    let text = if more {
        format!("{text}\n变更和冲突各最多展示 6 条。")
    } else {
        text
    };
    (text, changes, conflicts)
}

fn execute(
    mut read: impl FnMut(usize, usize) -> Result<Vec<u8>, Failure>,
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
    let (base, incoming) = match request.input.incoming_position {
        1 => (second, first),
        2 => (first, second),
        _ => return Err(REQUEST),
    };
    if base.resource_id == incoming.resource_id {
        return Err(RESOURCE);
    }
    for (index, resource) in request.resources.iter().enumerate() {
        if resource.index != index
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
            return Err(FILE_LIMIT);
        }
    }
    let mut read_profile = |resource: &Resource, invalid: Failure| {
        let bytes = read(resource.index, resource.byte_length)?;
        if format!("{:x}", Sha256::digest(&bytes)) != resource.content_version {
            return Err(RESOURCE);
        }
        let bindings = parse(&bytes).map_err(|_| invalid)?;
        if bindings.len() > MAX_BINDINGS {
            return Err(BINDING_LIMIT);
        }
        Ok(bindings)
    };
    let base = read_profile(base, BASE)?;
    let incoming = read_profile(incoming, INCOMING)?;
    let mode = request.input.mode;
    let merged = candidate(&base, &incoming, matches!(mode, Mode::Merge));
    let (text, changes, conflicts) = preview(&base, &merged, &mode);
    let mut output_bytes = 0;
    if conflicts == 0 {
        let bytes = encode(&merged).map_err(|_| SAVE)?;
        if bytes.len() > MAX_RESULT {
            return Err(SAVE);
        }
        save(&bytes)?;
        output_bytes = bytes.len();
    }
    Ok(
        json!({"ok":true,"data":{"text":text,"mode":mode,"baseCount":base.len(),"incomingCount":incoming.len(),
        "resultCount":merged.len(),"changedCount":changes,"conflictPairs":conflicts,"outputBytes":output_bytes},"error":null}),
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
        const JSON_MIME: i32 = 28;
        let slot = unsafe { output_create(JSON_MIME, bytes.len() as i64) };
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
        |json| {
            std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(target)
                .and_then(|mut file| file.write_all(json))
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
