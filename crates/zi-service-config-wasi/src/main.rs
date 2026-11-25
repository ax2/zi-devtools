//! Preflights a confirmed service configuration without changing it or starting services.
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Read, Write};
use zi_service_config_core::{ImportProblem, RawConfig, import_problem, parse};

const PLUGIN: &str = "com.zicode.devtools.service-config";
const CAPABILITY: &str = "devtools.services.config.preflight";
const PROFILE: &str = "resource.wasi.input.draft2";
const CONTRACT: &str = "1.0.0-rc.1";
const MAX_RESULT: usize = 16 * 1024;
const MAX_FILE: usize = 16 * 1024;
const MAX_SERVICES: usize = 64;
type Failure = (&'static str, &'static str);

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: Value,
    #[serde(rename = "input")]
    _input: Input,
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
    const REQUEST: Failure = ("INVALID_REQUEST", "本地服务配置预检请求无效。");
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
    const RESOURCE: Failure = ("INVALID_RESOURCE", "请选择并确认一份本机服务配置。");
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
        "application/octet-stream" | "text/plain"
    ) {
        return Err(("UNSUPPORTED_MIME", "请选择本机 ZiDevTools 服务 YAML 配置。"));
    }
    if file.byte_length > MAX_FILE {
        return Err(("INPUT_TOO_LARGE", "单份服务配置最多 16 KiB。"));
    }
    let bytes = read(file.byte_length)
        .map_err(|_| ("RESOURCE_READ_FAILED", "无法读取已确认的服务配置。"))?;
    let input = std::str::from_utf8(&bytes)
        .map_err(|_| ("INVALID_UTF8", "服务配置必须使用 UTF-8 编码。"))?;
    if !within_syntax_budget(input.as_bytes()) {
        return Err((
            "STRUCTURE_TOO_DENSE",
            "配置文本结构较密集。请减少密集字段或分批检查服务。",
        ));
    }
    let parsed = match parse(input) {
        Ok(parsed) => parsed,
        Err(error) => {
            let suffix = error.location().map_or_else(String::new, |position| {
                format!(
                    " 请检查第 {} 行、第 {} 列。",
                    position.line(),
                    position.column()
                )
            });
            return Ok(invalid_yaml(&suffix));
        }
    };
    if parsed.services.len() > MAX_SERVICES {
        return Err(("SERVICE_LIMIT", "每份服务配置最多检查 64 项定义。"));
    }
    Ok(preflight(&parsed))
}

// Bound the parser's event workload before allocating its YAML tree. Counting
// punctuation is intentionally conservative because scalar text can contain it.
fn within_syntax_budget(bytes: &[u8]) -> bool {
    const MAX_WEIGHT: usize = 1536;
    let mut weight = 0usize;
    let mut anchors = 0;
    let mut aliases = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        weight += match byte {
            b':' | b'?' | b'!' => 2,
            b'\n' | b',' | b'[' | b']' | b'{' | b'}' | b'-' | b'|' | b'>' => 1,
            b'&' | b'*' if index == 0 || is_token_boundary(bytes[index - 1]) => {
                anchors += usize::from(byte == b'&');
                aliases += usize::from(byte == b'*');
                1
            }
            _ => 0,
        };
        if weight > MAX_WEIGHT {
            return false;
        }
    }
    let expansion = if anchors < 2 {
        aliases + 1
    } else {
        1usize.checked_shl(aliases as u32).unwrap_or(usize::MAX)
    };
    weight.saturating_mul(expansion) <= MAX_WEIGHT
}

fn is_token_boundary(byte: u8) -> bool {
    byte.is_ascii_whitespace() || b":,[{-?".contains(&byte)
}

fn invalid_yaml(suffix: &str) -> Value {
    json!({"validForImport":false,"parseable":false,"serviceCount":0,"stateDirectoryProvided":false,
        "configuredStopCommands":0,"configuredHealthChecks":0,"environmentEntries":0,"problems":[],
        "text":format!("无法读取 ZiDevTools 服务 YAML。请检查 services 映射、必填字段和字段类型。{suffix}")})
}

fn preflight(config: &RawConfig) -> Value {
    use std::fmt::Write as _;
    let service_count = config.services.len();
    let configured_stop_commands = config
        .services
        .values()
        .filter(|service| service.stop_command.is_some())
        .count();
    let configured_health_checks = config
        .services
        .values()
        .filter(|service| service.health_url.is_some())
        .count();
    let environment_entries: usize = config
        .services
        .values()
        .map(|service| service.env.len())
        .sum();
    let state_directory_provided = config.state_dir.is_some();
    let state = if state_directory_provided {
        "已指定"
    } else {
        "未指定"
    };
    let mut text = format!(
        "共 {service_count} 项服务 · 状态目录{state}\n停止命令 {configured_stop_commands} 项 · 健康检查 {configured_health_checks} 项 · 环境变量 {environment_entries} 项"
    );
    let mut problems = Vec::new();
    for (index, service) in config.services.values().enumerate() {
        let Some(problem) = import_problem(service) else {
            continue;
        };
        let (code, label) = match problem {
            ImportProblem::MissingRepoOrCommand => {
                ("MISSING_REPO_OR_COMMAND", "repo 或 command 为空")
            }
            ImportProblem::EmptyStopCommand => ("EMPTY_STOP_COMMAND", "stop_command 不能为空"),
            ImportProblem::InvalidStopTimeout => {
                ("INVALID_STOP_TIMEOUT", "stop_timeout_ms 须为 100–60000")
            }
        };
        let position = index + 1;
        problems.push(json!({"serviceNumber":position,"code":code}));
        let _ = write!(text, "\n第 {position} 项 · {label}");
    }
    if !problems.is_empty() {
        text.push_str("\n序号对应按 services 键排序后的次序。每项显示首个问题。");
    }
    if service_count == 0 {
        text.push_str("\nservices 至少需要一项服务定义。");
    }
    let valid_for_import = service_count > 0 && problems.is_empty();
    if valid_for_import {
        text.push_str("\n配置字段通过原生导入前置校验。请在原生工具中核对工作目录与运行环境。");
    }
    json!({"parseable":true,"validForImport":valid_for_import,"serviceCount":service_count,
        "stateDirectoryProvided":state_directory_provided,"configuredStopCommands":configured_stop_commands,
        "configuredHealthChecks":configured_health_checks,"environmentEntries":environment_entries,
        "problems":problems,"text":text})
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
    let mut bytes = serde_json::to_vec(&result).expect("valid service configuration report");
    if bytes.len() > MAX_RESULT {
        bytes = serde_json::to_vec(&error(("RESULT_TOO_LARGE", "服务配置预检超过结果上限。")))
            .expect("fixed result error");
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
