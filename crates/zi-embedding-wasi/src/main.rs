//! Offline comparison of explicitly approved embedding snapshots.
use serde::de::{self, IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use std::fmt;
use std::io::{Read, Write};
use zi_embedding_core::{MAX_DIMENSIONS, MetricsError, compare_metrics};

mod json_budget;

const PLUGIN: &str = "com.zicode.devtools.embedding";
const CAPABILITY: &str = "devtools.embedding.compare-files";
const PROFILE: &str = "resource.wasi.input.draft2";
const CONTRACT: &str = "1.0.0-rc.1";
const MAX_FILE: usize = 256 * 1024;
type Failure = (&'static str, &'static str);
const DIMENSIONS: Failure = ("DIMENSION_LIMIT", "每个向量的维度必须在 1–8192 之间。");
const MALFORMED_VECTOR: Failure = ("INVALID_VECTOR", "每份向量文件须是单一 JSON 数值数组。");
const MALFORMED_RESPONSE: Failure = (
    "INVALID_EMBEDDINGS",
    "单份响应须有且仅有两个 embeddings 向量。",
);
const RESOURCE: Failure = (
    "INVALID_RESOURCE",
    "请选择并确认单份嵌入响应或两份 JSON 向量。",
);

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

struct Vector(Vec<f64>);

impl<'de> Deserialize<'de> for Vector {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct VectorVisitor;
        impl<'de> Visitor<'de> for VectorVisitor {
            type Value = Vector;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an array of finite numbers")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vector, A::Error> {
                let mut values =
                    Vec::with_capacity(seq.size_hint().unwrap_or(0).min(MAX_DIMENSIONS));
                while values.len() < MAX_DIMENSIONS {
                    let Some(value) = seq.next_element::<f64>()? else {
                        return Ok(Vector(values));
                    };
                    values.push(value);
                }
                if seq.next_element::<IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("embedding_dimension_limit"));
                }
                Ok(Vector(values))
            }
        }
        deserializer.deserialize_seq(VectorVisitor)
    }
}

#[derive(Deserialize)]
struct Response {
    embeddings: [Vector; 2],
}

fn parse_error(error: serde_json::Error, fallback: Failure) -> Failure {
    if error.to_string().starts_with("embedding_dimension_limit") {
        DIMENSIONS
    } else {
        fallback
    }
}

fn vectors(files: &[Vec<u8>]) -> Result<[Vector; 2], Failure> {
    match files {
        [file] => {
            let bytes = json_budget::container(file, MALFORMED_RESPONSE)?;
            json_budget::check_work(&[bytes])?;
            if bytes.first() == Some(&b'[') {
                serde_json::from_slice::<Vector>(bytes)
                    .map_err(|error| parse_error(error, MALFORMED_VECTOR))?;
                return Err((
                    "SECOND_VECTOR_REQUIRED",
                    "已提供一份向量数组。请再选择并确认第二份 JSON 向量。",
                ));
            }
            let response: Response = serde_json::from_slice(bytes)
                .map_err(|error| parse_error(error, MALFORMED_RESPONSE))?;
            Ok(response.embeddings)
        }
        [a, b] => {
            let a = json_budget::container(a, MALFORMED_VECTOR)?;
            let b = json_budget::container(b, MALFORMED_VECTOR)?;
            json_budget::check_work(&[a, b])?;
            let parse = |bytes: &[u8]| {
                serde_json::from_slice::<Vector>(bytes)
                    .map_err(|error| parse_error(error, MALFORMED_VECTOR))
            };
            Ok([parse(a)?, parse(b)?])
        }
        _ => Err(RESOURCE),
    }
}

fn run() -> Result<Value, Failure> {
    const MAX_REQUEST: usize = 16 * 1024;
    const INVALID_REQUEST: Failure = ("INVALID_REQUEST", "向量比较请求无效。");
    let mut envelope = Vec::new();
    std::io::stdin()
        .take((MAX_REQUEST + 1) as u64)
        .read_to_end(&mut envelope)
        .map_err(|_| INVALID_REQUEST)?;
    if envelope.len() > MAX_REQUEST {
        return Err(INVALID_REQUEST);
    }
    let request: Request = serde_json::from_slice(&envelope).map_err(|_| INVALID_REQUEST)?;
    let Input {} = request.input;
    if request.plugin_id != PLUGIN
        || request.capability_id != CAPABILITY
        || request.scene_id.is_empty()
        || request.scene_id.len() > 128
        || !request.command_id.is_null()
        || request.resource_input_profile != PROFILE
        || request.resource_output_profile.is_some()
    {
        return Err(INVALID_REQUEST);
    }
    if !(1..=2).contains(&request.resources.len()) {
        return Err(RESOURCE);
    }
    for (index, file) in request.resources.iter().enumerate() {
        if file.index != index
            || file.resource_id.is_empty()
            || file.content_version.len() != 64
            || !file
                .content_version
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || request.resources[..index]
                .iter()
                .any(|previous| previous.resource_id == file.resource_id)
        {
            return Err(RESOURCE);
        }
        if !matches!(
            file.mime_type.as_str(),
            "application/json" | "application/octet-stream"
        ) {
            return Err(("UNSUPPORTED_MIME", "请选择 JSON 格式的向量或嵌入响应。"));
        }
        if file.byte_length > MAX_FILE {
            return Err(("INPUT_TOO_LARGE", "每份文件最多 256 KiB。"));
        }
    }
    let files: Vec<_> = request
        .resources
        .iter()
        .map(|file| read(file.index, file.byte_length))
        .collect::<Result<_, _>>()?;
    let [Vector(a), Vector(b)] = vectors(&files)?;
    let metrics = compare_metrics(&a, &b).map_err(|error| match error {
        MetricsError::Dimension => DIMENSIONS,
        MetricsError::UnequalDimensions => ("DIMENSIONS_DIFFER", "两份向量的维度必须相同。"),
        MetricsError::Component => (
            "INVALID_COMPONENT",
            "向量分量须为有限数，绝对值不能超过 1,000,000。",
        ),
        MetricsError::InvalidNorm => ("INVALID_NORM", "向量范数无效或为零。"),
        _ => ("INVALID_METRICS", "无法比较这两份向量。"),
    })?;
    let text = format!(
        "维度 {}\nA 范数 {:.6}\nB 范数 {:.6}\n余弦相似度 {:.6}\n欧氏距离 {:.6}",
        metrics.dimensions, metrics.norm_a, metrics.norm_b, metrics.cosine, metrics.euclidean
    );
    Ok(
        json!({"dimensions":metrics.dimensions,"normA":metrics.norm_a,"normB":metrics.norm_b,
        "cosine":metrics.cosine,"euclidean":metrics.euclidean,"text":text}),
    )
}

#[cfg(target_arch = "wasm32")]
fn read(index: usize, length: usize) -> Result<Vec<u8>, Failure> {
    zi_resource_input_wasi::read_window(index as i32, length, 0, length)
        .map_err(|_| ("RESOURCE_READ_FAILED", "无法读取已确认的文件。"))
}

#[cfg(not(target_arch = "wasm32"))]
fn read(index: usize, length: usize) -> Result<Vec<u8>, Failure> {
    const FAILED: Failure = ("RESOURCE_READ_FAILED", "无法读取已确认的文件。");
    let path = std::env::args_os().nth(index + 1).ok_or(FAILED)?;
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

fn main() -> std::io::Result<()> {
    let result = match run() {
        Ok(data) => json!({"contractVersion":CONTRACT,"ok":true,"data":data,"error":null}),
        Err((code, message)) => {
            json!({"contractVersion":CONTRACT,"ok":false,"data":null,"error":{"code":code,"message":message}})
        }
    };
    std::io::stdout().write_all(&serde_json::to_vec(&result)?)
}
