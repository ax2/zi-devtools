//! Runs one bounded real matrix operation on user-supplied TSV.
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value as Json, json, value::RawValue};
use std::io::{Read, Write};
use zi_matrix_core::{
    Value, exact_number,
    matrix::{Matrix, Operation, tsv_cells},
};

const PLUGIN: &str = "com.zicode.devtools.matrix-calculator";
const CONTRACT: &str = "1.0.0-rc.1";
const REQUEST_LIMIT: usize = 16 * 1024;
const MATRIX_LIMIT: usize = 4096;
const CELL_LIMIT: usize = 96;
const RESULT_LIMIT: usize = 12 * 1024;

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

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Precision {
    #[default]
    Exact,
    Approximate,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Unary {
    a: String,
    #[serde(default)]
    precision: Precision,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binary {
    a: String,
    b: String,
    #[serde(default)]
    precision: Precision,
}

struct Failure(&'static str, String);

fn decode<T: DeserializeOwned>(input: &RawValue) -> Result<T, Failure> {
    serde_json::from_str(input.get())
        .map_err(|_| Failure("INVALID_INPUT", "请检查矩阵与精度参数。".into()))
}

fn value(text: &str, precision: Precision) -> Result<Value, String> {
    match precision {
        Precision::Exact => {
            if let Some((numerator, denominator)) = text.split_once('/') {
                let n = numerator.trim().parse().map_err(|_| "分子需为 i128 整数")?;
                let d = denominator
                    .trim()
                    .parse()
                    .map_err(|_| "分母需为 i128 整数")?;
                Value::exact(n, d)
            } else {
                exact_number(text)
            }
        }
        Precision::Approximate => {
            let number = text.parse().map_err(|_| "需要有限小数或科学计数法")?;
            Value::approximate(number)
        }
    }
}

fn matrix(text: &str, precision: Precision, name: &str) -> Result<Matrix, Failure> {
    if text.len() > MATRIX_LIMIT {
        return Err(Failure(
            "INPUT_TOO_LARGE",
            format!("矩阵 {name} 最多 4096 UTF-8 字节。"),
        ));
    }
    let grid = tsv_cells(text, CELL_LIMIT)
        .map_err(|message| Failure("INVALID_MATRIX", format!("矩阵 {name}: {message}")))?;
    let mut cells = Vec::with_capacity(grid.len() * grid[0].len());
    for (row, line) in grid.iter().enumerate() {
        for (column, cell) in line.iter().enumerate() {
            cells.push(value(cell, precision).map_err(|message| {
                Failure(
                    "INVALID_CELL",
                    format!("{name}({}, {}): {message}", row + 1, column + 1),
                )
            })?);
        }
    }
    Matrix::new(grid.len(), grid[0].len(), cells)
        .map_err(|message| Failure("INVALID_MATRIX", message))
}

fn run(request: Request<'_>) -> Result<Json, Failure> {
    if request.plugin_id != PLUGIN
        || request.scene_id.is_empty()
        || request.scene_id.len() > 128
        || !request.command_id.is_null()
    {
        return Err(Failure("INVALID_REQUEST", "请求结构无效。".into()));
    }
    let operation = match request.capability_id.as_str() {
        "devtools.matrix.add" => Operation::Add,
        "devtools.matrix.subtract" => Operation::Subtract,
        "devtools.matrix.multiply" => Operation::Multiply,
        "devtools.matrix.transpose" => Operation::Transpose,
        "devtools.matrix.determinant" => Operation::Determinant,
        "devtools.matrix.inverse" => Operation::Inverse,
        "devtools.matrix.solve" => Operation::Solve,
        _ => return Err(Failure("UNKNOWN_OPERATION", "未知矩阵操作。".into())),
    };
    let (a, b) = if operation.needs_b() {
        let input: Binary = decode(request.input)?;
        (
            matrix(&input.a, input.precision, "A")?,
            Some(matrix(&input.b, input.precision, "B")?),
        )
    } else {
        let input: Unary = decode(request.input)?;
        (matrix(&input.a, input.precision, "A")?, None)
    };
    let result = a
        .apply(operation, b.as_ref())
        .map_err(|message| Failure("MATRIX_OPERATION", message))?;
    let approximate = result.approximate();
    let text = if approximate {
        format!("≈ 近似结果\n{}", result.tsv())
    } else {
        result.tsv()
    };
    Ok(json!({
        "rowCount": result.rows,
        "columnCount": result.cols,
        "approximate": approximate,
        "values": result.cells.chunks(result.cols).collect::<Vec<_>>(),
        "text": text,
    }))
}

fn error(Failure(code, message): Failure) -> Json {
    json!({"contractVersion":CONTRACT,"ok":false,"data":null,"error":{"code":code,"message":message}})
}

fn execute() -> Result<Json, Failure> {
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
    run(request)
}

fn main() {
    let result = match execute() {
        Ok(data) => json!({"contractVersion":CONTRACT,"ok":true,"data":data,"error":null}),
        Err(reason) => error(reason),
    };
    let mut bytes = serde_json::to_vec(&result).expect("valid matrix result");
    if bytes.len() > RESULT_LIMIT {
        bytes = serde_json::to_vec(&error(Failure(
            "RESULT_TOO_LARGE",
            "矩阵结果超过 12 KiB。".into(),
        )))
        .expect("fixed matrix error");
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
