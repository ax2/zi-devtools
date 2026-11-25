use serde::Deserialize;
use serde_json::{Value as Json, json, value::RawValue};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};
use zi_matrix_core::{
    expression::input::{AngleInput, Literal, parse_constants},
    root,
};

const PLUGIN: &str = "com.zicode.devtools.root-solver";
const CAPABILITY: &str = "devtools.roots.bisect";
const CONTRACT: &str = "1.0.0-rc.1";
const REQUEST_LIMIT: usize = 16 * 1024;
const RESULT_LIMIT: usize = 1024;
const WORK_LIMIT: usize = 22_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request<'a> {
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: Json,
    #[serde(borrow)]
    input: &'a RawValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Input {
    formula: String,
    lower: f64,
    upper: f64,
    #[serde(default = "default_tolerance")]
    tolerance: f64,
    #[serde(default = "default_iterations")]
    max_iterations: usize,
    #[serde(default)]
    angle: AngleInput,
    #[serde(default)]
    constants: BTreeMap<String, Literal>,
}

fn default_tolerance() -> f64 {
    1e-10
}

fn default_iterations() -> usize {
    100
}

struct Failure(&'static str, String);

fn execute(request: Request<'_>) -> Result<Json, Failure> {
    if request.plugin_id != PLUGIN
        || request.scene_id.is_empty()
        || request.scene_id.len() > 128
        || !request.command_id.is_null()
    {
        return Err(Failure("INVALID_REQUEST", "请求结构无效。".into()));
    }
    if request.capability_id != CAPABILITY {
        return Err(Failure("UNKNOWN_OPERATION", "未知求根操作。".into()));
    }
    let input: Input = serde_json::from_str(request.input.get())
        .map_err(|_| Failure("INVALID_INPUT", "请检查公式、区间和求根参数。".into()))?;
    let variables = parse_constants(input.constants, true)
        .map_err(|reason| Failure("INVALID_CONSTANT", reason))?;
    let result = root::solve_with_work_limit(
        &input.formula,
        root::Interval {
            lower: input.lower,
            upper: input.upper,
            tolerance: input.tolerance,
            iterations: input.max_iterations,
        },
        &variables,
        input.angle.into(),
        Some(WORK_LIMIT),
    )
    .map_err(|reason| Failure("ROOT_OPERATION", reason))?;
    let status = if result.converged {
        "区间已收敛（候选点，请检查残差）"
    } else {
        "达到迭代上限（尚未收敛）"
    };
    let text = format!(
        "{status}\nx ≈ {:.17e}\n|f(x)| ≈ {:.6e} · {} 次迭代",
        result.root,
        result.residual.abs(),
        result.iterations
    );
    Ok(
        json!({"converged":result.converged,"iterations":result.iterations,
        "root":format!("{:e}",result.root),"residual":format!("{:e}",result.residual),"text":text}),
    )
}

fn run() -> Result<Json, Failure> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((REQUEST_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure("INVALID_REQUEST", "无法读取请求。".into()))?;
    if bytes.len() > REQUEST_LIMIT {
        return Err(Failure("INPUT_TOO_LARGE", "请求超过 16 KiB。".into()));
    }
    let request = serde_json::from_slice(&bytes)
        .map_err(|_| Failure("INVALID_REQUEST", "请求结构无效。".into()))?;
    execute(request)
}

fn error(Failure(code, message): Failure) -> Json {
    json!({"contractVersion":CONTRACT,"ok":false,"data":null,"error":{"code":code,"message":message}})
}

fn main() {
    let result = match run() {
        Ok(data) => json!({"contractVersion":CONTRACT,"ok":true,"data":data,"error":null}),
        Err(reason) => error(reason),
    };
    let mut bytes = serde_json::to_vec(&result).expect("valid root result");
    if bytes.len() > RESULT_LIMIT {
        bytes = serde_json::to_vec(&error(Failure(
            "RESULT_TOO_LARGE",
            "求根结果超过 1 KiB。".into(),
        )))
        .expect("fixed root error");
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
