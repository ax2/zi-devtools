use super::{Dataset, value_json_bytes};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    #[default]
    Trim,
    Lower,
    Upper,
    EmptyToNull,
    FillNull,
    Rename,
    ToText,
    ToInteger,
    ToNumber,
    ToBool,
    SelectColumns,
}
impl Operation {
    pub const ALL: [Self; 11] = [
        Self::Trim,
        Self::Lower,
        Self::Upper,
        Self::EmptyToNull,
        Self::FillNull,
        Self::Rename,
        Self::ToText,
        Self::ToInteger,
        Self::ToNumber,
        Self::ToBool,
        Self::SelectColumns,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Trim => "去除首尾空白",
            Self::Lower => "转为小写",
            Self::Upper => "转为大写",
            Self::EmptyToNull => "空字符串 → null",
            Self::FillNull => "填充 null（文本）",
            Self::Rename => "修改列名",
            Self::ToText => "转为文本",
            Self::ToInteger => "转为整数（64 位）",
            Self::ToNumber => "转为数字",
            Self::ToBool => "转为布尔值",
            Self::SelectColumns => "选择保留列",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub data: Dataset,
    pub changed: usize,
    pub examples: Vec<(String, String)>,
    pub description: String,
}

pub fn propose(
    data: &Dataset,
    column: usize,
    operation: Operation,
    value: &str,
) -> Result<Proposal> {
    let header = data
        .headers
        .get(column)
        .ok_or_else(|| anyhow!("请选择有效列"))?;
    if value.len() > 4096 {
        bail!("替换文本最多 4096 字节");
    }
    let mut result = data.clone();
    let mut changed = 0;
    let mut examples = Vec::new();
    if operation == Operation::Rename {
        let name = value.trim();
        if name.is_empty() || name.len() > 256 {
            bail!("列名须为 1–256 字节，不能只有空白");
        }
        if data
            .headers
            .iter()
            .enumerate()
            .any(|(i, h)| i != column && h == name)
        {
            bail!("列名已存在，不能覆盖其他列");
        }
        if header != name {
            result.headers[column] = name.into();
            changed = 1;
            examples.push((header.clone(), name.into()));
        }
    } else {
        let mut replacement_bytes = 0usize;
        for (index, row) in data.rows.iter().enumerate() {
            let cell = &row[column];
            let next = match (operation, cell) {
                (Operation::Trim, Value::String(s)) => Value::String(s.trim().into()),
                (Operation::Lower, Value::String(s)) => Value::String(s.to_lowercase()),
                (Operation::Upper, Value::String(s)) => Value::String(s.to_uppercase()),
                (Operation::EmptyToNull, Value::String(s)) if s.is_empty() => Value::Null,
                (Operation::FillNull, Value::Null) => Value::String(value.into()),
                (
                    Operation::ToText
                    | Operation::ToInteger
                    | Operation::ToNumber
                    | Operation::ToBool,
                    _,
                ) => convert(cell, operation).with_context(|| {
                    format!("第 {} 行，列 {} 转换失败；原数据未修改", index + 1, header)
                })?,
                _ => continue,
            };
            replacement_bytes += value_json_bytes(&next);
            if replacement_bytes > 8 * 1024 * 1024 {
                bail!("转换结果超过 8 MiB，请缩小数据范围或替换文本");
            }
            if *cell != next {
                changed += 1;
                if examples.len() < 6 {
                    examples.push((preview(cell), preview(&next)));
                }
                result.rows[index][column] = next;
            }
        }
    }
    // Bound the complete result, including untouched columns, without serializing a huge buffer.
    let mut bytes = result.headers.iter().map(String::len).sum::<usize>();
    for row in &result.rows {
        for cell in row {
            bytes += value_json_bytes(cell);
            if bytes > 8 * 1024 * 1024 {
                bail!("转换结果超过 8 MiB；原数据未修改");
            }
        }
    }
    Ok(Proposal {
        data: result,
        changed,
        examples,
        description: format!("{} · {}", header, operation.label()),
    })
}

pub fn convert(cell: &Value, operation: Operation) -> Result<Value> {
    if cell.is_null() {
        return Ok(Value::Null);
    }
    if operation == Operation::ToText {
        return Ok(Value::String(
            cell.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| cell.to_string()),
        ));
    }
    let text = match cell {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) if operation == Operation::ToBool => b.to_string(),
        _ => bail!("该类型无法转换"),
    };
    if operation == Operation::ToBool {
        return match text.to_ascii_lowercase().as_str() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => bail!("期望 true 或 false"),
        };
    }
    if operation == Operation::ToInteger || !text.contains(['.', 'e', 'E']) {
        return text
            .parse::<i64>()
            .map(Value::from)
            .or_else(|_| text.parse::<u64>().map(Value::from))
            .map_err(|_| anyhow!("期望 64 位范围内的整数，不接受小数或指数形式"));
    }
    let number: serde_json::Number = serde_json::from_str(&text)
        .map_err(|_| anyhow!("期望有限 JSON 数字，不接受 NaN 或 Infinity"))?;
    Ok(Value::Number(number))
}

pub fn select_columns(data: &Dataset, keep: &[bool]) -> Result<Proposal> {
    if keep.len() != data.headers.len() || !keep.iter().any(|v| *v) {
        bail!("至少保留一列");
    }
    let indices: Vec<_> = keep
        .iter()
        .enumerate()
        .filter_map(|(i, keep)| keep.then_some(i))
        .collect();
    Ok(Proposal {
        data: Dataset {
            headers: indices.iter().map(|&i| data.headers[i].clone()).collect(),
            rows: if indices.len() == data.headers.len() {
                data.rows.clone()
            } else {
                data.rows
                    .iter()
                    .map(|row| indices.iter().map(|&i| row[i].clone()).collect())
                    .collect()
            },
        },
        changed: data.headers.len() - indices.len(),
        examples: data
            .headers
            .iter()
            .zip(keep)
            .filter(|(_, keep)| !**keep)
            .take(6)
            .map(|(h, _)| (h.clone(), "移除".into()))
            .collect(),
        description: format!(
            "保留 {} / {} 列，变化计数为移除列数",
            indices.len(),
            data.headers.len()
        ),
    })
}

fn preview(value: &Value) -> String {
    let text = value.to_string();
    let mut out: String = text.chars().take(100).collect();
    if text.chars().count() > 100 {
        out.push('…');
    }
    out
}
