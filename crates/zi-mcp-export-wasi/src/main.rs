//! Offline rendering of ZiDevTools MCP client config snippets.
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read as _;
use zi_mcp_export_core::{render, valid_manual_path};

const PLUGIN: &str = "com.zicode.devtools.mcp-client-export";
const TOML: &str = "devtools.mcp.config.codex-toml";
const JSON: &str = "devtools.mcp.config.generic-json";
const CONTRACT: &str = "1.0.0-rc.1";
const MAX_REQUEST: usize = 8 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    plugin_id: String,
    capability_id: String,
    scene_id: String,
    command_id: Option<String>,
    input: Input,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Input {
    executable_path: String,
}

#[derive(Clone, Copy)]
enum Failure {
    InvalidRequest,
    InvalidPath,
    Formatting,
}

fn execute() -> Result<Value, Failure> {
    let mut body = Vec::new();
    std::io::stdin()
        .take((MAX_REQUEST + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|_| Failure::InvalidRequest)?;
    if body.len() > MAX_REQUEST {
        return Err(Failure::InvalidRequest);
    }
    let request: Request = serde_json::from_slice(&body).map_err(|_| Failure::InvalidRequest)?;
    if request.plugin_id != PLUGIN || request.scene_id.is_empty() || request.command_id.is_some() {
        return Err(Failure::InvalidRequest);
    }
    let format = match request.capability_id.as_str() {
        TOML => "toml",
        JSON => "json",
        _ => return Err(Failure::InvalidRequest),
    };
    let path = &request.input.executable_path;
    if !valid_manual_path(path) {
        return Err(Failure::InvalidPath);
    }
    let exports = render(path).map_err(|_| Failure::Formatting)?;
    let text = if format == "toml" {
        exports.codex_toml
    } else {
        exports.generic_json
    };
    Ok(
        json!({"contractVersion":CONTRACT,"ok":true,"data":{"format":format,"text":text},"error":null}),
    )
}

fn error(reason: Failure) -> Value {
    let (code, message) = match reason {
        Failure::InvalidRequest => ("INVALID_REQUEST", "MCP 配置预览请求无效。"),
        Failure::InvalidPath => (
            "INVALID_PATH",
            "请输入本机 ZiDevTools MCP 服务程序的 Windows 盘符绝对路径，最多 1024 字节。",
        ),
        Failure::Formatting => ("FORMAT_FAILED", "无法生成 MCP 配置片段。"),
    };
    json!({"contractVersion":CONTRACT,"ok":false,"data":null,"error":{"code":code,"message":message}})
}

fn main() {
    println!("{}", execute().unwrap_or_else(error));
}
