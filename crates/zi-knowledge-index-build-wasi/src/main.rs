//! Build a v1 static ZiDevTools index from one confirmed UTF-8 TXT file.
use rusqlite::{Connection, DatabaseName};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Read as _;
use zi_document_text_core::{chunks_from, plain_text_sections};
use zi_knowledge_index_core::{IndexedDocument, insert_document};
use zi_knowledge_keyword_core::INDEX_DDL;

const PLUGIN: &str = "com.zicode.devtools.knowledge-index-build";
const CAPABILITY: &str = "devtools.knowledge.index.snapshot.create";
const INPUT: &str = "resource.wasi.input.draft2";
const OUTPUT: &str = "resource.wasi.output.draft1";
const SOURCE_ID: &str = "studio-snapshot";
const MAX_REQUEST: usize = 16 * 1024;
const MAX_TEXT: usize = 4096;
const MAX_CHUNKS: usize = 32;
const MAX_OUTPUT: usize = 128 * 1024;

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
    input: Options,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Options {
    file_label: String,
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

#[derive(Clone, Copy, Serialize)]
struct Failure {
    code: &'static str,
    message: &'static str,
}

const REQUEST: Failure = Failure {
    code: "INVALID_REQUEST",
    message: "静态知识索引请求无效。",
};
const RESOURCE: Failure = Failure {
    code: "INVALID_RESOURCE",
    message: "请选择并确认一份本机 UTF-8 TXT 文件。",
};
const INPUT_LIMIT: Failure = Failure {
    code: "INPUT_LIMIT",
    message: "每次最多读取 4 KiB UTF-8 TXT。",
};
const LABEL: Failure = Failure {
    code: "INVALID_LABEL",
    message: "请输入 128 字节以内的单个 TXT 引用文件名。",
};
const TEXT: Failure = Failure {
    code: "INVALID_TEXT",
    message: "请选择有效 UTF-8 文本，允许换行和制表符。",
};
const EMPTY: Failure = Failure {
    code: "EMPTY_TEXT",
    message: "文本没有可索引的正文。",
};
const BODY_LIMIT: Failure = Failure {
    code: "INDEX_LIMIT",
    message: "最多生成 32 个片段，分块后的正文最多 4 KiB。",
};
const INDEX: Failure = Failure {
    code: "BUILD_FAILED",
    message: "无法生成静态 SQLite 知识索引。",
};
const SAVE: Failure = Failure {
    code: "OUTPUT_FAILED",
    message: "无法登记 SQLite 索引结果。",
};

fn valid_label(label: &str) -> bool {
    label.len() <= 128 && label == label.trim()
        && label.rsplit_once('.').is_some_and(|(name,suffix)| {
            !name.is_empty() && !name.ends_with(['.', ' ']) && suffix.eq_ignore_ascii_case("txt")
        })
        && !label.contains(['\\','/',':','<','>','"','|','?','*'])
        && !label.chars().any(|ch| ch.is_control()
            || matches!(ch,'\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{2028}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
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
        || !matches!(
            resource.mime_type.as_str(),
            "text/plain" | "application/octet-stream"
        )
    {
        return Err(RESOURCE);
    }
    if resource.byte_length > MAX_TEXT {
        return Err(INPUT_LIMIT);
    }
    let label = &request.input.file_label;
    if !valid_label(label) {
        return Err(LABEL);
    }
    let bytes = read(resource.byte_length)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if digest != resource.content_version {
        return Err(RESOURCE);
    }
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))
        .map_err(|_| TEXT)?;
    if text
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(TEXT);
    }
    let sections = plain_text_sections(text, MAX_CHUNKS, MAX_TEXT).map_err(|_| BODY_LIMIT)?;
    if sections.is_empty() {
        return Err(EMPTY);
    }
    let chunks = chunks_from(sections).map_err(|_| BODY_LIMIT)?;
    let text_bytes: usize = chunks.iter().map(|chunk| chunk.text.len()).sum();
    if chunks.len() > MAX_CHUNKS || text_bytes > MAX_TEXT {
        return Err(BODY_LIMIT);
    }

    let mut database = Connection::open_in_memory().map_err(|_| INDEX)?;
    database.execute_batch(INDEX_DDL).map_err(|_| INDEX)?;
    database
        .pragma_update(None, "foreign_keys", true)
        .map_err(|_| INDEX)?;
    let tx = database.transaction().map_err(|_| INDEX)?;
    insert_document(
        &tx,
        IndexedDocument {
            source_id: SOURCE_ID,
            relative: label,
            sha256: &digest,
            bytes: bytes.len() as u64,
            text_bytes: text_bytes as u64,
            indexed_at: "time not recorded",
            chunks: &chunks,
        },
        || Ok(()),
    )
    .map_err(|_| INDEX)?;
    tx.commit().map_err(|_| INDEX)?;
    let result = database.serialize(DatabaseName::Main).map_err(|_| INDEX)?;
    if result.len() > MAX_OUTPUT {
        return Err(INDEX);
    }
    save(&result)?;
    let text = format!(
        "已创建静态知识索引\n引用：{label} · 来源：{SOURCE_ID}\n片段 {} 个 · 正文 {text_bytes} 字节\n源 SHA-256：{digest}\n请确认保存为新的 SQLite 文件。",
        chunks.len()
    );
    Ok(
        json!({"ok":true,"data":{"fileLabel":label,"sourceId":SOURCE_ID,"sourceSha256":digest,"sourceBytes":bytes.len(),
        "chunks":chunks.len(),"textBytes":text_bytes,"sqliteBytes":result.len(),"text":text},"error":null}),
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

    pub fn read(length: usize) -> Result<Vec<u8>, Failure> {
        zi_resource_input_wasi::read_window(0, length, 0, length).map_err(|_| RESOURCE)
    }

    pub fn save(bytes: &[u8]) -> Result<(), Failure> {
        const SQLITE_MIME: i32 = 26; // application/x-zicode-sqlite
        let index = unsafe { output_create(SQLITE_MIME, bytes.len() as i64) };
        if index != 0 {
            return Err(SAVE);
        }
        for chunk in bytes.chunks(32 * 1024) {
            if unsafe { output_write(index, chunk.as_ptr() as i32, chunk.len() as i32) }
                != chunk.len() as i32
            {
                return Err(SAVE);
            }
        }
        if unsafe { output_finish(index) } != 0 {
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
