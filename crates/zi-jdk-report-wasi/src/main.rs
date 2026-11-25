//! Validate a confirmed saved native JDK environment report without probing the system.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Read as _;
use zi_jdk_report_core::{ReviewError, review};

const PLUGIN: &str = "com.zicode.devtools.jdk-report";
const CAPABILITY: &str = "devtools.java.environment.report.review";
const INPUT: &str = "resource.wasi.input.draft2";
const OUTPUT: &str = "resource.wasi.output.draft1";
const MAX_RESOURCE: usize = 32 * 1024;
const MAX_REQUEST: usize = 16 * 1024;

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
    message: "JDK 报告查看请求无效。",
};
const RESOURCE: Failure = Failure {
    code: "INVALID_RESOURCE",
    message: "请选择并确认一份本机 JDK JSON 报告。",
};
const LIMIT: Failure = Failure {
    code: "INPUT_LIMIT",
    message: "每份 JDK 报告最多 32 KiB。",
};
const FORMAT: Failure = Failure {
    code: "INVALID_REPORT",
    message: "请选择 ZiDevTools JDK 环境工具保存的 UTF-8 JSON 报告。",
};
const PROVENANCE: Failure = Failure {
    code: "INVALID_PROVENANCE",
    message: "报告来源标识、版本或可移植指纹不一致。",
};
const LABEL: Failure = Failure {
    code: "INVALID_FIELD",
    message: "报告的版本、供应商或架构超长，或含路径与控制字符。",
};
const COMPARISON: Failure = Failure {
    code: "INCONSISTENT_COMPARISON",
    message: "报告中的版本差异与其记录的版本值不一致。",
};
const SAVE: Failure = Failure {
    code: "OUTPUT_FAILED",
    message: "无法登记脱敏报告文本。",
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
    let summary = review(&read(resource.byte_length)?).map_err(|error| match error {
        ReviewError::Format => FORMAT,
        ReviewError::Provenance => PROVENANCE,
        ReviewError::Value => LABEL,
        ReviewError::Comparison => COMPARISON,
    })?;
    save(summary.text.as_bytes())?;
    Ok(json!({"ok":true,"data":summary,"error":null}))
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
