//! Shows a plain-text excerpt from the explicitly shared HTML document.
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Read, Write};
use zi_html_clip_core::html_body;

const PLUGIN: &str = "com.zicode.devtools.local-html";
const CAPABILITY: &str = "devtools.html.local.preview";
const PROFILE: &str = "resource.wasi.input.draft2";
const CONTRACT: &str = "1.0.0-rc.1";
const MAX_FILE: usize = 16 * 1024;
const MAX_BODY: usize = 6 * 1024;
const MAX_OPENINGS: usize = 256;
const MAX_CHAR_REFERENCES: usize = 512;
const MAX_TAG_SEPARATORS: usize = 128;
const MAX_DOCUMENT_SEPARATORS: usize = 768;
const MAX_RESULT: usize = 16 * 1024;
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
    const REQUEST: Failure = ("INVALID_REQUEST", "HTML 预览请求无效。");
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
    const RESOURCE: Failure = ("INVALID_RESOURCE", "请选择并确认一份本机 HTML 文件。");
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
    if file.mime_type != "text/html" {
        return Err(("UNSUPPORTED_MIME", "请选择 UTF-8 的 .html 或 .htm 文件。"));
    }
    if file.byte_length > MAX_FILE {
        return Err(("INPUT_TOO_LARGE", "单份 HTML 文件最多 16 KiB。"));
    }
    let bytes = read(file.byte_length)
        .map_err(|_| ("RESOURCE_READ_FAILED", "无法读取已确认的 HTML 文件。"))?;
    preview(&bytes)
}

fn preview(bytes: &[u8]) -> Result<Value, Failure> {
    let html = std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))
        .map_err(|_| ("INVALID_UTF8", "HTML 文件须使用 UTF-8 编码。"))?;
    if !within_html_budget(html.as_bytes()) {
        return Err((
            "HTML_TOO_COMPLEX",
            "HTML 标记过于密集。请拆分文件，或复制正文后再处理。",
        ));
    }
    let (title, body) = html_body(html).map_err(|_| ("INVALID_HTML", "HTML 没有可提取的正文。"))?;
    let title: String = title
        .chars()
        .filter(|ch| !ch.is_control())
        .take(180)
        .collect();
    let mut ending = MAX_BODY.min(body.len());
    while !body.is_char_boundary(ending) {
        ending -= 1;
    }
    let excerpt = &body[..ending];
    let truncated = ending < body.len();
    let heading = if title.is_empty() {
        "无标题"
    } else {
        title.as_str()
    };
    let text = if truncated {
        format!("{heading}\n\n{excerpt}\n\n[已省略剩余正文]")
    } else {
        format!("{heading}\n\n{excerpt}")
    };
    Ok(
        json!({"title":title,"byteLength":bytes.len(),"bodyByteLength":body.len(),
              "previewByteLength":ending,"truncated":truncated,"text":text}),
    )
}

#[derive(Clone, Copy)]
enum TagState {
    Text,
    Open,
    Name,
    BeforeAttribute,
    Attribute,
    AfterAttribute,
    BeforeValue,
    UnquotedValue,
    QuotedValue(u8),
    AfterQuotedValue,
    Comment(usize),
    Declaration,
}

// The HTML5 parser checks for duplicate attributes by scanning previously seen
// attributes of each tag. Estimate tag, attribute and reference work before
// parsing. Only a quote at the start of an attribute value can hide separators.
fn within_html_budget(bytes: &[u8]) -> bool {
    use TagState::*;

    let mut openings = 0;
    let mut references = 0;
    let mut total_separators = 0;
    let mut tag_separators = 0;
    let mut state = Text;

    for (offset, &byte) in bytes.iter().enumerate() {
        if byte == b'<' {
            openings += 1;
            if openings > MAX_OPENINGS {
                return false;
            }
        }
        if byte == b'&' {
            references += 1;
            if references > MAX_CHAR_REFERENCES {
                return false;
            }
        }

        let space = matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c);
        let (next, separator) = match (state, byte) {
            (Text, b'<') => (Open, false),
            (Text, _) => continue,
            (Open, b'/') => (Open, true),
            (Open, b'<') => (Open, false),
            (Open, b'!') if bytes[offset..].starts_with(b"!--") => (Comment(offset + 3), false),
            (Open, b'!' | b'?') => (Declaration, false),
            (Open, letter) if letter.is_ascii_alphabetic() => (Name, false),
            (Open, _) => (Text, false),
            (Comment(start), b'>') if offset >= start => {
                let content = &bytes[start..offset];
                if matches!(content, b"" | b"-")
                    || content.ends_with(b"--")
                    || content.ends_with(b"--!")
                {
                    (Text, false)
                } else {
                    continue;
                }
            }
            (Comment(_), _) => continue,
            (QuotedValue(quote), current) if current == quote => (AfterQuotedValue, false),
            (QuotedValue(_), _) => continue,
            (_, b'>') => (Text, false),
            (Declaration, _) => continue,
            (BeforeValue, _) if space => continue,
            (BeforeValue, b'"' | b'\'') => (QuotedValue(byte), false),
            (BeforeValue, _) => (UnquotedValue, false),
            (UnquotedValue, _) if space => (BeforeAttribute, true),
            (UnquotedValue, _) => continue,
            (Name | Attribute, _) if space => {
                let next = if matches!(state, Name) {
                    BeforeAttribute
                } else {
                    AfterAttribute
                };
                (next, true)
            }
            (BeforeAttribute | AfterAttribute, _) if space => continue,
            (Attribute | AfterAttribute, b'=') => (BeforeValue, true),
            (_, b'/') => (BeforeAttribute, true),
            (AfterQuotedValue, _) if space => (BeforeAttribute, true),
            (AfterQuotedValue, _) => (Attribute, true),
            (BeforeAttribute | AfterAttribute, _) => (Attribute, false),
            (Name | Attribute, _) => continue,
        };
        state = next;
        if matches!(state, Text) {
            tag_separators = 0;
        }
        if separator {
            tag_separators += 1;
            total_separators += 1;
            if tag_separators > MAX_TAG_SEPARATORS || total_separators > MAX_DOCUMENT_SEPARATORS {
                return false;
            }
        }
    }
    true
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
    let mut bytes = serde_json::to_vec(&result).expect("valid HTML text preview");
    if bytes.len() > MAX_RESULT {
        bytes = serde_json::to_vec(&error(("RESULT_TOO_LARGE", "预览结果超过输出限制。")))
            .expect("fixed result error");
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
