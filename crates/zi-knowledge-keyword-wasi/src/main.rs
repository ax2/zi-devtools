//! Search a single explicitly approved ZiDevTools knowledge index snapshot.
mod snapshot;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fmt::Write as _, io::Read as _};
use zi_knowledge_keyword_core::{
    SearchMode, checked_query, excerpt, search_checked, uses_literal_match,
};

const PLUGIN: &str = "com.zicode.devtools.keyword-index";
const CAPABILITY: &str = "devtools.knowledge.keyword.snapshot.search";
const INPUT: &str = "resource.wasi.input.draft2";
const OUTPUT: &str = "resource.wasi.output.draft1";
const MAX_REQUEST: usize = 16 * 1024;
const MAX_OUTPUT: usize = 24 * 1024;
const MAX_RESULT: usize = 48 * 1024;
const MAX_TERMS: usize = 12;
const MAX_FTS_TERMS: usize = 4;

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
    query: String,
    #[serde(default)]
    mode: Mode,
    source_id: Option<String>,
    limit: Option<usize>,
}

#[derive(Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    Auto,
    Phrase,
    All,
    Any,
}

impl From<Mode> for SearchMode {
    fn from(value: Mode) -> Self {
        match value {
            Mode::Auto => Self::Auto,
            Mode::Phrase => Self::ExactPhrase,
            Mode::All => Self::AllTerms,
            Mode::Any => Self::AnyTerm,
        }
    }
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
    message: "知识索引检索请求无效。",
};
const RESOURCE: Failure = Failure {
    code: "INVALID_RESOURCE",
    message: "请选择并确认一份本机知识索引静态副本。",
};
const LIMIT: Failure = Failure {
    code: "INPUT_LIMIT",
    message: "这份索引超过 128 KiB。",
};
const QUERY: Failure = Failure {
    code: "INVALID_QUERY",
    message: "输入最多 120 字符。字面检索最多 12 个词，FTS5 最多 4 个词项，可用字母、数字和 ASCII 标点。结果数为 1 到 10。",
};
const INVALID_HIT: Failure = Failure {
    code: "INVALID_INDEX_RECORD",
    message: "索引命中的引用字段格式无效。",
};
const BUDGET: Failure = Failure {
    code: "SEARCH_LIMIT",
    message: "索引查询超过执行预算。可缩短关键词或使用较小的索引。",
};
const SEARCH: Failure = Failure {
    code: "SEARCH_FAILED",
    message: "无法完成查询，请核对索引副本及关键词。",
};
const SAVE: Failure = Failure {
    code: "OUTPUT_FAILED",
    message: "无法登记查询结果。",
};

fn snapshot_error(error: snapshot::SnapshotError) -> Failure {
    use snapshot::SnapshotError::*;
    match error {
        Invalid => Failure {
            code: "INVALID_DATABASE",
            message: "请选择有效、完整的 UTF-8 SQLite 静态索引副本。",
        },
        NonStatic => Failure {
            code: "WAL_SNAPSHOT",
            message: "请选择不依赖 WAL 或日志文件的 SQLite 静态副本。",
        },
        Schema => Failure {
            code: "INVALID_INDEX_SCHEMA",
            message: "索引结构与独立版已验证的 v1 格式不符。",
        },
        Integrity => Failure {
            code: "INVALID_DATABASE",
            message: "索引未通过 SQLite 完整性检查。",
        },
        RowLimit => Failure {
            code: "INDEX_ROW_LIMIT",
            message: "索引超过 32 个片段、合计正文超过 4 KiB，或含超长或类型无效的记录。",
        },
        Budget => BUDGET,
    }
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn source_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

fn unsafe_display(ch: char) -> bool {
    ch.is_control()
        || matches!(ch,'\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{2028}'..='\u{202e}'|'\u{2066}'..='\u{2069}')
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(':')
        && !value.chars().any(unsafe_display)
        && value
            .split(['/', '\\'])
            .all(|part| !part.is_empty() && !matches!(part, "." | ".."))
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
        || !digest(&resource.content_version)
        || !matches!(
            resource.mime_type.as_str(),
            "application/x-zicode-sqlite" | "application/octet-stream"
        )
    {
        return Err(RESOURCE);
    }
    if resource.byte_length > snapshot::MAX_SNAPSHOT {
        return Err(LIMIT);
    }
    let input = request.input;
    let limit = input.limit.unwrap_or(5);
    let query = checked_query(&input.query, limit).map_err(|_| QUERY)?;
    let source = input.source_id.as_deref().filter(|id| !id.is_empty());
    let term_count = query.split_whitespace().count();
    if limit > 10 || term_count > MAX_TERMS || source.is_some_and(|id| !source_id(id)) {
        return Err(QUERY);
    }
    let mode = input.mode.into();
    let fts_terms = if uses_literal_match(query, mode) {
        0
    } else {
        if query
            .chars()
            .any(|ch| !(ch.is_alphanumeric() || ch.is_whitespace() || ch.is_ascii_punctuation()))
        {
            return Err(QUERY);
        }
        let word_sections = query
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|section| !section.is_empty())
            .count();
        let terms = word_sections.max(term_count);
        if word_sections == 0 || terms > MAX_FTS_TERMS {
            return Err(QUERY);
        }
        terms as u32
    };
    let database =
        snapshot::open(&read(resource.byte_length)?, fts_terms).map_err(snapshot_error)?;
    let matches = search_checked(&database, query, source, limit, mode).map_err(|error| {
        if error
            .downcast_ref::<rusqlite::Error>()
            .and_then(rusqlite::Error::sqlite_error_code)
            == Some(rusqlite::ErrorCode::OperationInterrupted)
        {
            BUDGET
        } else {
            SEARCH
        }
    })?;
    let mut text = format!(
        "本机知识索引快照 · {} · 显示 {} 项（上限 {limit}）",
        if matches.literal_match {
            "字面检索"
        } else {
            "FTS5 词项检索（BM25）"
        },
        matches.hits.len()
    );
    let mut hits = Vec::with_capacity(matches.hits.len());
    for hit in matches.hits {
        if !safe_relative(&hit.relative)
            || !source_id(&hit.source_id)
            || hit.location.chars().any(unsafe_display)
            || !digest(&hit.file_sha256)
            || !digest(&hit.chunk_sha256)
        {
            return Err(INVALID_HIT);
        }
        let snippet: String = excerpt(&hit.text, query)
            .chars()
            .map(|ch| if unsafe_display(ch) { ' ' } else { ch })
            .collect();
        let _ = write!(
            text,
            "\n\n{} · {} · 片段 {}（来源 {}）\n{}\n索引记录 · 文件 SHA-256 {} · 片段 SHA-256 {}",
            hit.relative,
            hit.location,
            hit.ordinal,
            hit.source_id,
            snippet,
            hit.file_sha256,
            hit.chunk_sha256
        );
        hits.push(json!({"sourceId":hit.source_id,"relative":hit.relative,"location":hit.location,
                         "ordinal":hit.ordinal,"fileSha256":hit.file_sha256,"chunkSha256":hit.chunk_sha256,"excerpt":snippet}));
    }
    let payload = json!({"ok":true,"data":{"literalMatch":matches.literal_match,"hits":hits,"text":text},"error":null});
    let text = payload["data"]["text"].as_str().expect("string result");
    if text.len() > MAX_OUTPUT || serde_json::to_vec(&payload).map_err(|_| SAVE)?.len() > MAX_RESULT
    {
        return Err(SAVE);
    }
    save(text.as_bytes())?;
    Ok(payload)
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
