//! Inspects a confirmed portable tool collection export without applying it.
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Read, Write};
use zi_tool_collections_core::{
    ImportProblem, PlannedCollection, prepare_import, valid_collection_name,
};

const PLUGIN: &str = "com.zicode.devtools.tool-collections";
const CAPABILITY: &str = "devtools.collections.preview";
const PROFILE: &str = "resource.wasi.input.draft2";
const CONTRACT: &str = "1.0.0-rc.1";
const PAGE_SIZE: usize = 16;
const MAX_FILE: usize = 128 * 1024;
const MAX_RESULT: usize = 24 * 1024;
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
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Input {
    #[serde(default)]
    compare_names: Vec<String>,
    #[serde(default = "first_position")]
    start_collection: usize,
    #[serde(default = "first_position")]
    start_member: usize,
}

fn first_position() -> usize {
    1
}

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
    const MAX_REQUEST: usize = 48 * 1024;
    const REQUEST: Failure = ("INVALID_REQUEST", "工具集合预览请求无效。");
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
    let Input {
        compare_names,
        start_collection,
        start_member,
    } = request.input;
    const RESOURCE: Failure = ("INVALID_RESOURCE", "请选择并确认一份本机导出文件。");
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
        "application/json" | "application/octet-stream" | "text/plain"
    ) {
        return Err(("UNSUPPORTED_MIME", "请选择工具集合导出的 JSON 文件。"));
    }
    if file.byte_length > MAX_FILE {
        return Err(("INPUT_TOO_LARGE", "单份文件最多 128 KiB。"));
    }
    let names = comparison_names(compare_names)?;
    let bytes = read(file.byte_length)
        .map_err(|_| ("RESOURCE_READ_FAILED", "无法读取已确认的工具集合文件。"))?;
    let plan = prepare_import(&bytes, names.iter().map(String::as_str)).map_err(|reason| {
        let code = reason
            .downcast_ref::<ImportProblem>()
            .map_or("INVALID_FILE", |problem| problem.code);
        (code, message(code))
    })?;
    if start_collection == 0 || start_collection > plan.len() {
        return Err(("UNKNOWN_COLLECTION", "起始集合序号超过文件中的集合数。"));
    }
    if start_member == 0 || start_member > plan[start_collection - 1].tool_ids.len().max(1) {
        return Err((
            "UNKNOWN_TOOL_POSITION",
            "工具起始序号超出该集合。空集合从 1 开始查看。",
        ));
    }
    Ok(report(&plan, names.len(), start_collection, start_member))
}

fn comparison_names(names: Vec<String>) -> Result<Vec<String>, Failure> {
    const INVALID: Failure = (
        "INVALID_COMPARISON_NAMES",
        "比较最多 128 个不同的集合名称，每个名称需 1–40 字且不含控制字符。",
    );
    if names.len() > 128 {
        return Err(INVALID);
    }
    let mut seen = std::collections::HashSet::new();
    names
        .into_iter()
        .map(|name| {
            let name = valid_collection_name(&name).map_err(|_| INVALID)?;
            if !seen.insert(name.to_lowercase()) {
                return Err(INVALID);
            }
            Ok(name)
        })
        .collect()
}

fn report(
    plan: &[PlannedCollection],
    comparisons: usize,
    start: usize,
    start_member: usize,
) -> Value {
    use std::fmt::Write as _;

    let collection_count = plan.len();
    let tool_count: usize = plan.iter().map(|item| item.tool_ids.len()).sum();
    let renamed_count = plan
        .iter()
        .filter(|row| row.renamed_to_avoid_conflict)
        .count();
    let page = &plan[start - 1..(start - 1 + PAGE_SIZE).min(collection_count)];
    let end = start - 1 + page.len();
    let mut text = format!(
        "已检查 {collection_count} 个集合、{tool_count} 个工具入口 · {renamed_count} 个名称需调整\n\
         比较名称 {comparisons} 个 · 当前显示第 {start}–{end} 个集合"
    );
    let rows: Vec<_> = page
        .iter()
        .map(|item| {
            let name = if item.renamed_to_avoid_conflict {
                format!("{} → {}", item.source_name, item.name)
            } else {
                item.name.clone()
            };
            let _ = write!(text, "\n{name} · {} 个工具", item.tool_ids.len());
            json!({"sourceName":item.source_name,"resolvedName":item.name,
               "renamed":item.renamed_to_avoid_conflict,"toolCount":item.tool_ids.len()})
        })
        .collect();
    let first = &page[0];
    let mut member_ids = Vec::new();
    let mut member_bytes = 0;
    for member in first.tool_ids.iter().skip(start_member - 1).take(16) {
        let escaped = serde_json::to_string(member).expect("valid tool ID");
        member_bytes += escaped.len();
        if member_bytes > 4096 {
            break;
        }
        member_ids.push((member, escaped));
    }
    let member_count = first.tool_ids.len();
    let end_member = start_member - 1 + member_ids.len();
    let next_member = (end_member < member_count).then_some(end_member + 1);
    let members_truncated = start_member > 1 || next_member.is_some();
    if member_ids.is_empty() {
        let _ = write!(text, "\n\n第 {start} 个集合为空。");
    } else {
        let _ = write!(
            text,
            "\n\n第 {start} 个集合 · 工具 ID 第 {start_member}–{end_member} 项，共 {member_count} 项（按导出顺序）"
        );
        for (_, escaped) in &member_ids {
            let _ = write!(text, "\n{escaped}");
        }
    }
    if let Some(next) = next_member {
        let _ = write!(text, "\n继续查看时，工具起始序号填 {next}。");
    }
    if end < collection_count || start > 1 {
        text.push_str("\n调整起始集合序号可查看其他集合。");
    }
    text.push_str("\n工具 ID 保留原文。各入口是否可用请在目标工具中心核对。");
    json!({
        "collectionCount":collection_count,"toolCount":tool_count,"comparisonNameCount":comparisons,
        "renamedCount":renamed_count,"startCollection":start,"endCollection":end,"collections":rows,
        "memberPreview":{
            "collection":start,"toolCount":member_count,
            "startMember":start_member,"endMember":end_member,"nextMember":next_member,
            "toolIds":member_ids.into_iter().map(|(member,_)|member).collect::<Vec<_>>(),
            "truncated":members_truncated
        },
        "text":text
    })
}

fn message(code: &str) -> &'static str {
    match code {
        "UNSUPPORTED_VERSION" => "请选择 ZiDevTools 工具集合版本 1 的导出。",
        "INVALID_COLLECTION_COUNT" => "文件需要 1–128 个工具集合。",
        "COLLECTION_LIMIT" => "导入与比较集合合计最多 128 个，请缩小比较范围。",
        "INVALID_NAME" => "集合名称需要 1–40 字且不含控制字符。",
        "DUPLICATE_NAME" => "导出包含重复的集合名称，比较时忽略大小写。",
        "TOOL_LIMIT" => "单个集合最多包含 512 个工具 ID。",
        "INVALID_TOOL_ID" => "工具 ID 需为 1–512 UTF-8 字节。",
        "DUPLICATE_TOOL_ID" => "单个集合以内的工具 ID 不能重复。",
        _ => "工具集合 JSON 无效或包含未知字段。",
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
    let mut bytes = serde_json::to_vec(&result).expect("valid collection preview");
    if bytes.len() > MAX_RESULT {
        bytes = serde_json::to_vec(&error(("RESULT_TOO_LARGE", "工具集合预览超过结果上限。")))
            .expect("fixed result error");
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
