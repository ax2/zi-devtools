//! Complete standalone JSON path and diff algorithms; no Host IO or execution.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::collections::BTreeSet;
fn bounded(input: &str) -> Result<()> {
    if input.len() > 1024 * 1024 {
        bail!("输入超过 1 MiB，请先缩小内容范围");
    }
    Ok(())
}

/// Build a bounded, object-valued summary shared by the standalone JSON tool
/// and the `result.json.v1` provider. The original object remains intact so a
/// Host may transfer it as an object without a text conversion.
pub fn object_summary(value: &Value) -> Result<Value> {
    let object = value.as_object().context("根值必须是 JSON object")?;
    ensure!(object.len() <= 128, "根对象最多允许 128 个字段");
    let serialized = serde_json::to_vec(value)?;
    ensure!(serialized.len() <= 8 * 1024, "JSON object 超过 8 KiB");

    fn inspect(
        value: &Value,
        depth: usize,
        max_depth: &mut usize,
        nodes: &mut usize,
    ) -> Result<()> {
        *nodes += 1;
        ensure!(*nodes <= 4096, "JSON object 节点超过 4096 个");
        ensure!(depth <= 64, "JSON object 嵌套超过 64 层");
        *max_depth = (*max_depth).max(depth);
        match value {
            Value::Array(items) => {
                for item in items {
                    inspect(item, depth + 1, max_depth, nodes)?;
                }
            }
            Value::Object(items) => {
                for item in items.values() {
                    inspect(item, depth + 1, max_depth, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn value_type(value: &Value) -> &'static str {
        match value {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    let mut max_depth = 0;
    let mut nodes = 0;
    inspect(value, 1, &mut max_depth, &mut nodes)?;
    let keys: Vec<_> = object.keys().cloned().collect();
    let value_types: serde_json::Map<String, Value> = object
        .iter()
        .map(|(key, value)| (key.clone(), Value::String(value_type(value).into())))
        .collect();
    Ok(serde_json::json!({
        "value": value,
        "keyCount": object.len(),
        "keys": keys,
        "valueTypes": value_types,
        "maxDepth": max_depth,
        "serializedBytes": serialized.len()
    }))
}

pub fn object_summary_text(input: &str) -> Result<String> {
    ensure!(input.len() <= 8 * 1024, "JSON object 输入最多 8 KiB");
    let value: Value = serde_json::from_str(input).context("输入不是有效 JSON")?;
    pretty(&object_summary(&value)?)
}
fn pretty(value: &Value) -> Result<String> {
    let text = serde_json::to_string_pretty(value)?;
    ensure!(
        text.len() <= 8 * 1024 * 1024,
        "结果超过 8 MiB，请缩小输入范围"
    );
    Ok(text)
}

pub fn json_path(input: &str, query: &str) -> Result<String> {
    bounded(input)?;
    ensure!(query.len() <= 4096, "查询过长（最多 4096 字节）");
    let value: Value = serde_json::from_str(input).context("输入不是有效 JSON")?;
    let query = query.trim();
    ensure!(query.starts_with('$'), "路径必须从 $ 开始");
    let mut rest = &query[1..];
    let mut values = vec![&value];
    let mut steps = 0;
    while !rest.is_empty() {
        steps += 1;
        ensure!(steps <= 128, "路径最多 128 层");
        enum Step {
            Key(String),
            Index(u64),
            Wildcard,
        }
        let step;
        if let Some(tail) = rest.strip_prefix('.') {
            let end = tail.find(['.', '[']).unwrap_or(tail.len());
            let key = &tail[..end];
            ensure!(
                !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_'),
                "点号后需要字段名；特殊字段名请使用 [\"字段\"]"
            );
            step = Step::Key(key.into());
            rest = &tail[end..];
        } else if let Some(tail) = rest.strip_prefix('[') {
            if tail.starts_with('"') {
                let mut stream = serde_json::Deserializer::from_str(tail).into_iter::<String>();
                let key = stream.next().context("缺少字段名")??;
                let end = stream.byte_offset();
                ensure!(tail[end..].starts_with(']'), "字段名后缺少 ]");
                step = Step::Key(key);
                rest = &tail[end + 1..];
            } else {
                let end = tail.find(']').context("缺少 ]")?;
                let token = &tail[..end];
                step = if token == "*" {
                    Step::Wildcard
                } else {
                    ensure!(
                        !token.is_empty() && token.bytes().all(|c| c.is_ascii_digit()),
                        "仅支持非负索引或 [*]，不支持过滤器与切片"
                    );
                    Step::Index(token.parse().context("数组索引过大")?)
                };
                rest = &tail[end + 1..];
            }
        } else {
            bail!("不支持的路径语法；可用 .字段、[\"字段\"]、[索引]、[*]");
        }
        let mut next = Vec::new();
        for value in values {
            match &step {
                Step::Key(key) => {
                    if let Some(v) = value.as_object().and_then(|o| o.get(key)) {
                        next.push(v);
                    }
                }
                Step::Index(i) => {
                    if let Some(v) = value
                        .as_array()
                        .and_then(|a| usize::try_from(*i).ok().and_then(|index| a.get(index)))
                    {
                        next.push(v);
                    }
                }
                Step::Wildcard => match value {
                    Value::Array(a) => next.extend(a),
                    Value::Object(o) => next.extend(o.values()),
                    _ => {}
                },
            }
            ensure!(next.len() <= 10_000, "匹配超过 10000 项，请缩小路径范围");
        }
        values = next;
    }
    pretty(&json!(values))
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Array(values) => {
            let mut values: Vec<_> = values.iter().map(canonical).collect();
            values.sort_by_cached_key(|v| v.to_string());
            Value::Array(values)
        }
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(k, v)| (k.clone(), canonical(v)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// A proven lower bound already exceeds the caller's serialized result budget.
#[derive(Debug)]
pub struct ReportTooLarge;
impl std::fmt::Display for ReportTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JSON diff report exceeds result budget")
    }
}
impl std::error::Error for ReportTooLarge {}

pub fn json_diff(left: &str, right: &str, unordered: bool) -> Result<String> {
    json_diff_with_result_budget(left, right, unordered, usize::MAX)
}

/// Stops only when changed paths alone cannot fit; callers still serialize and
/// check the complete envelope. Equal paths do not consume the lower bound.
pub fn json_diff_with_result_budget(
    left: &str,
    right: &str,
    unordered: bool,
    result_budget: usize,
) -> Result<String> {
    bounded(left)?;
    bounded(right)?;
    let left: Value = serde_json::from_str(left).context("左侧 JSON 无效")?;
    let right: Value = serde_json::from_str(right).context("右侧 JSON 无效")?;
    let mut changes = Vec::new();
    let mut remaining = result_budget;
    fn record(changes: &mut Vec<Value>, remaining: &mut usize, change: Value) -> Result<()> {
        // JSON escaping and envelopes only increase these UTF-8 path bytes.
        let bytes = change["path"].as_str().expect("generated path").len();
        *remaining = remaining.checked_sub(bytes).ok_or(ReportTooLarge)?;
        changes.push(change);
        Ok(())
    }
    fn visit(
        a: &Value,
        b: &Value,
        path: &str,
        unordered: bool,
        changes: &mut Vec<Value>,
        remaining: &mut usize,
    ) -> Result<()> {
        // Object keys are traversed below; normalize arrays once at their node,
        // not again for every ancestor. At the change limit retain the original
        // unordered equality check before rejecting another non-equal subtree.
        if a == b
            || (unordered
                && ((a.is_array() && b.is_array()) || changes.len() >= 10_000)
                && canonical(a) == canonical(b))
        {
            return Ok(());
        }
        ensure!(changes.len() < 10_000, "差异超过 10000 项，请缩小输入");
        match (a, b) {
            (Value::Object(a), Value::Object(b)) => {
                for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                    let path = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                    match (a.get(key), b.get(key)) {
                        (Some(a), Some(b)) => visit(a, b, &path, unordered, changes, remaining)?,
                        (Some(a), None) => record(
                            changes,
                            remaining,
                            json!({"path":path,"kind":"removed","before":a}),
                        )?,
                        (None, Some(b)) => record(
                            changes,
                            remaining,
                            json!({"path":path,"kind":"added","after":b}),
                        )?,
                        _ => unreachable!(),
                    }
                    ensure!(changes.len() <= 10_000, "差异超过 10000 项，请缩小输入");
                }
            }
            (Value::Array(a), Value::Array(b)) if !unordered => {
                for i in 0..a.len().max(b.len()) {
                    let path = format!("{path}/{i}");
                    match (a.get(i), b.get(i)) {
                        (Some(a), Some(b)) => visit(a, b, &path, unordered, changes, remaining)?,
                        (Some(a), None) => record(
                            changes,
                            remaining,
                            json!({"path":path,"kind":"removed","before":a}),
                        )?,
                        (None, Some(b)) => record(
                            changes,
                            remaining,
                            json!({"path":path,"kind":"added","after":b}),
                        )?,
                        _ => unreachable!(),
                    }
                    ensure!(changes.len() <= 10_000, "差异超过 10000 项，请缩小输入");
                }
            }
            _ => record(
                changes,
                remaining,
                json!({"path":path,"kind":"changed","before":a,"after":b}),
            )?,
        }
        Ok(())
    }
    visit(&left, &right, "", unordered, &mut changes, &mut remaining)?;
    pretty(
        &json!({"equal":changes.is_empty(),"arrayOrder":if unordered {"ignored (duplicates retained)"} else {"by index"},"changes":changes}),
    )
}

#[cfg(test)]
mod tests;
