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

pub fn json_diff(left: &str, right: &str, unordered: bool) -> Result<String> {
    bounded(left)?;
    bounded(right)?;
    let left: Value = serde_json::from_str(left).context("左侧 JSON 无效")?;
    let right: Value = serde_json::from_str(right).context("右侧 JSON 无效")?;
    let mut changes = Vec::new();
    fn visit(
        a: &Value,
        b: &Value,
        path: &str,
        unordered: bool,
        changes: &mut Vec<Value>,
    ) -> Result<()> {
        if a == b || (unordered && canonical(a) == canonical(b)) {
            return Ok(());
        }
        ensure!(changes.len() < 10_000, "差异超过 10000 项，请缩小输入");
        match (a, b) {
            (Value::Object(a), Value::Object(b)) => {
                for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                    let path = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                    match (a.get(key), b.get(key)) {
                        (Some(a), Some(b)) => visit(a, b, &path, unordered, changes)?,
                        (Some(a), None) => {
                            changes.push(json!({"path":path,"kind":"removed","before":a}))
                        }
                        (None, Some(b)) => {
                            changes.push(json!({"path":path,"kind":"added","after":b}))
                        }
                        _ => unreachable!(),
                    }
                    ensure!(changes.len() <= 10_000, "差异超过 10000 项，请缩小输入");
                }
            }
            (Value::Array(a), Value::Array(b)) if !unordered => {
                for i in 0..a.len().max(b.len()) {
                    let path = format!("{path}/{i}");
                    match (a.get(i), b.get(i)) {
                        (Some(a), Some(b)) => visit(a, b, &path, unordered, changes)?,
                        (Some(a), None) => {
                            changes.push(json!({"path":path,"kind":"removed","before":a}))
                        }
                        (None, Some(b)) => {
                            changes.push(json!({"path":path,"kind":"added","after":b}))
                        }
                        _ => unreachable!(),
                    }
                    ensure!(changes.len() <= 10_000, "差异超过 10000 项，请缩小输入");
                }
            }
            _ => changes.push(json!({"path":path,"kind":"changed","before":a,"after":b})),
        }
        Ok(())
    }
    visit(&left, &right, "", unordered, &mut changes)?;
    pretty(
        &json!({"equal":changes.is_empty(),"arrayOrder":if unordered {"ignored (duplicates retained)"} else {"by index"},"changes":changes}),
    )
}

#[cfg(test)]
mod tests;
