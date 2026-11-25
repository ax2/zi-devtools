//! Report known Windows tool names from explicitly shared directory metadata.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{borrow::Cow, fmt::Write as _, io::Read as _};
use zi_integration_entries_core::{WINDOWS_ENTRIES, match_file_names};

const PLUGIN: &str = "com.zicode.devtools.integration-entries";
const CAPABILITY: &str = "devtools.integrations.directory.entries";
const INPUT: &str = "resource.wasi.input.draft2";
const OUTPUT: &str = "resource.wasi.output.draft1";
const MAX_RESOURCE: usize = 128 * 1024;
const MAX_REQUEST: usize = 16 * 1024;
const MAX_ENTRIES: usize = 2048;
const COUNT_MARKER: &str = "directory entry count exceeds bound";

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

#[derive(Clone, Copy, Serialize)]
struct Failure {
    code: &'static str,
    message: &'static str,
}

const REQUEST: Failure = Failure {
    code: "INVALID_REQUEST",
    message: "目录入口盘点请求无效。",
};
const RESOURCE: Failure = Failure {
    code: "INVALID_RESOURCE",
    message: "请选择并确认一个工作区内的文件夹。",
};
const LIMIT: Failure = Failure {
    code: "INPUT_LIMIT",
    message: "目录名称快照最多 128 KiB，请缩小目录范围。",
};
const FORMAT: Failure = Failure {
    code: "INVALID_SNAPSHOT",
    message: "请从「文件夹」选择并确认单层目录信息。",
};
const COUNT: Failure = Failure {
    code: "ENTRY_LIMIT",
    message: "最多盘点 2048 个目录项。",
};
const SAVE: Failure = Failure {
    code: "OUTPUT_FAILED",
    message: "无法登记完整的入口盘点报告。",
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Snapshot<'a> {
    schema_version: u32,
    profile: String,
    #[serde(borrow, deserialize_with = "read_bounded_entries")]
    entries: Vec<Entry<'a>>,
}

fn read_bounded_entries<'de, D>(deserializer: D) -> Result<Vec<Entry<'de>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Error as _, SeqAccess, Visitor};

    struct BoundedEntries;
    impl<'de> Visitor<'de> for BoundedEntries {
        type Value = Vec<Entry<'de>>;

        fn expecting(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            out.write_str("an array of directory entries")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut list: A) -> Result<Self::Value, A::Error> {
            let mut entries = Vec::new();
            while let Some(entry) = list.next_element()? {
                if entries.len() == MAX_ENTRIES {
                    return Err(A::Error::custom(COUNT_MARKER));
                }
                entries.push(entry);
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_seq(BoundedEntries)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry<'a> {
    #[serde(borrow)]
    name: Cow<'a, str>,
    kind: Kind,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    File,
    Directory,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct App {
    name: &'static str,
    names: Vec<&'static str>,
}

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
        || resource.mime_type != "text/plain"
    {
        return Err(RESOURCE);
    }
    if resource.byte_length > MAX_RESOURCE {
        return Err(LIMIT);
    }
    let source = read(resource.byte_length)?;
    let Snapshot {
        schema_version,
        profile,
        entries,
    } = serde_json::from_slice(&source).map_err(|error| {
        if error.is_data() && error.to_string().starts_with(COUNT_MARKER) {
            COUNT
        } else {
            FORMAT
        }
    })?;
    if schema_version != 1 || profile != "resource.local-directory.draft1" {
        return Err(FORMAT);
    }
    if entries.iter().any(|entry| {
        entry.name.is_empty()
            || entry.name.len() > 1024
            || matches!(entry.name.as_ref(), "." | "..")
            || entry
                .name
                .chars()
                .any(|value| matches!(value, '\\' | '/' | ':') || value.is_control())
    }) {
        return Err(FORMAT);
    }
    let matches = match_file_names(
        entries
            .iter()
            .filter(|entry| matches!(entry.kind, Kind::File))
            .map(|entry| entry.name.as_ref()),
    );
    let apps: Vec<_> = WINDOWS_ENTRIES
        .iter()
        .zip(matches)
        .map(|((name, _), names)| App { name, names })
        .collect();
    let found = apps.iter().filter(|app| !app.names.is_empty()).count();
    let mut text = format!(
        "目录入口盘点 · 发现 {found}/{} 个工具候选\n当前目录单层文件名匹配，未验证文件可以运行。",
        apps.len()
    );
    for app in &apps {
        let _ = write!(text, "\n{}：", app.name);
        if app.names.is_empty() {
            text.push_str("未发现");
        } else {
            text.push_str(&app.names.join("、"));
        }
    }
    save(text.as_bytes())?;
    Ok(
        json!({"ok":true,"data":{"entryCount":entries.len(),"found":found,"apps":apps,"text":text},"error":null}),
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
        for (index, chunk) in bytes.chunks_mut(64 * 1024).enumerate() {
            let read = unsafe {
                resource_read(
                    0,
                    (index * 64 * 1024) as i64,
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
