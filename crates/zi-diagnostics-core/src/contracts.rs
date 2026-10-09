use super::bounded;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
const CONFIG_NOTES: &str = "按 JSON Pointer 比较配置值，数组作为整体；敏感键值遮盖，但无法识别任意业务密钥，请先脱敏。不会解析占位符或推断 profile、环境变量、命令行的实际覆盖顺序；支持 JSON / 单文档 YAML，不直接加载 .properties。";

fn flatten(
    value: &Value,
    path: &str,
    out: &mut BTreeMap<String, Value>,
    depth: usize,
) -> Result<()> {
    ensure!(depth < 64, "结构超过 64 层");
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (k, v) in map {
                flatten(
                    v,
                    &format!("{}/{}", path, k.replace('~', "~0").replace('/', "~1")),
                    out,
                    depth + 1,
                )?;
            }
        }
        _ => {
            out.insert(path.into(), value.clone());
        }
    }
    Ok(())
}
fn sensitive(path: &str) -> bool {
    let key = path.to_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "credential",
        "private",
        "apikey",
        "api-key",
        "api_key",
        "connection-string",
    ]
    .iter()
    .any(|s| key.contains(s))
}
fn mask(value: &Value, path: &str) -> Value {
    if sensitive(path) {
        return json!("[REDACTED]");
    }
    match value {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), mask(v, &format!("{path}/{k}"))))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|v| mask(v, path)).collect()),
        _ => value.clone(),
    }
}
fn leaf_mask(value: &Value, path: &str, is_sensitive: bool) -> Value {
    if is_sensitive {
        json!("[REDACTED]")
    } else if value.is_array() || value.is_object() {
        mask(value, path)
    } else {
        value.clone()
    }
}
fn placeholder(value: &Value) -> bool {
    match value {
        Value::String(s) => s.contains("${"),
        Value::Array(items) => items.iter().any(placeholder),
        Value::Object(items) => items
            .iter()
            .any(|(key, v)| key.contains("${") || placeholder(v)),
        _ => false,
    }
}
pub fn config(left: &str, right: &str) -> Result<String> {
    config_limited(left, right, 8 * 1024 * 1024)
}
pub(crate) fn config_limited(left: &str, right: &str, report_limit: usize) -> Result<String> {
    bounded(left)?;
    bounded(right)?;
    let a: Value = super::config_json::parse(left).context("左侧需为 JSON / 单文档 YAML")?;
    let b: Value = super::config_json::parse(right).context("右侧需为 JSON / 单文档 YAML")?;
    ensure!(
        a.is_object() && b.is_object(),
        "配置顶层需为对象；properties 请先转换为对象"
    );
    // Prove scalar-only reports exceed the result budget before duplicating
    // both maps and constructing hundreds of masked change objects.
    if report_limit == 48 * 1024
        && a.as_object()
            .unwrap()
            .values()
            .chain(b.as_object().unwrap().values())
            .all(|v| !v.is_object() && !v.is_array())
    {
        let aa = a.as_object().unwrap();
        let bb = b.as_object().unwrap();
        let keys: BTreeSet<_> = aa.keys().chain(bb.keys()).collect();
        let template =
            json!({"path":"","change":"changed","left":null,"right":null,"sensitiveKey":false});
        let mut base = 0;
        super::measure(&template, 2, true, &mut base, report_limit)?;
        let mut charge = 0;
        super::measure(
            &json!({"changes":[],"unresolvedPlaceholderPaths":[],"sources":["左侧输入","右侧输入"],"notes":CONFIG_NOTES}),
            0,
            true,
            &mut charge,
            report_limit,
        )?;
        for key in keys {
            let old = aa.get(key);
            let new = bb.get(key);
            if old == new {
                continue;
            }
            let path = format!("/{}", key.replace('~', "~0").replace('/', "~1"));
            let redacted = sensitive(&path);
            let change = if old.is_none() {
                "added"
            } else if new.is_none() {
                "removed"
            } else {
                "changed"
            };
            charge += base + super::string_size(&path, true) - 4 + super::string_size(change, true)
                - super::string_size("changed", true);
            if redacted {
                charge -= 1;
            }
            for value in [old, new] {
                let mut size = 0;
                if redacted && value.is_some() {
                    size = super::string_size("[REDACTED]", true);
                } else if let Some(value) = value {
                    super::measure(value, 3, true, &mut size, report_limit)?;
                } else {
                    size = 4;
                }
                charge = charge - 4 + size;
            }
            if charge > report_limit {
                return Err(super::ReportTooLarge.into());
            }
        }
    }
    let mut aa = BTreeMap::new();
    let mut bb = BTreeMap::new();
    flatten(&a, "", &mut aa, 0)?;
    flatten(&b, "", &mut bb, 0)?;
    let keys: BTreeSet<_> = aa.keys().chain(bb.keys()).collect();
    let mut changes = vec![];
    let mut output_budget = super::CollectionBudget::new(report_limit);
    let mut unresolved = vec![];
    for key in keys {
        let old = aa.get(key);
        let new = bb.get(key);
        if old != new {
            let is_sensitive = sensitive(key);
            changes.push(json!({"path":key,"change":if old.is_none(){"added"}else if new.is_none(){"removed"}else{"changed"},"left":old.map(|v|leaf_mask(v,key,is_sensitive)),"right":new.map(|v|leaf_mask(v,key,is_sensitive)),"sensitiveKey":is_sensitive}));
            output_budget.add(changes.last().unwrap())?;
        }
        if [old, new].into_iter().flatten().any(placeholder) {
            unresolved.push(key);
        }
    }
    super::report_limited(
        json!({"changes":changes,"unresolvedPlaceholderPaths":unresolved,"sources":["左侧输入","右侧输入"],"notes":CONFIG_NOTES}),
        report_limit,
    )
}

fn operations(v: &Value) -> Result<BTreeMap<String, Value>> {
    let paths = v["paths"].as_object().context("缺少 paths 对象")?;
    let mut out = BTreeMap::new();
    for (path, item) in paths {
        ensure!(path.starts_with('/'), "OpenAPI 路径必须以 / 开头");
        let item = item.as_object().context("path item 必须为对象")?;
        for method in [
            "get", "put", "post", "delete", "patch", "head", "options", "trace",
        ] {
            if let Some(operation) = item.get(method) {
                let mut operation = operation
                    .as_object()
                    .context("operation 必须为对象")?
                    .clone();
                if let Some(common) = item.get("parameters") {
                    let mut params = common
                        .as_array()
                        .context("path parameters 必须为数组")?
                        .clone();
                    if let Some(local) = operation.get("parameters") {
                        params.extend(
                            local
                                .as_array()
                                .context("operation parameters 必须为数组")?
                                .clone(),
                        );
                    }
                    operation.insert("parameters".into(), json!(params));
                }
                out.insert(
                    format!("{} {}", method.to_uppercase(), path),
                    Value::Object(operation),
                );
            }
        }
    }
    Ok(out)
}
fn schema_changes(
    a: &Value,
    b: &Value,
    path: &str,
    out: &mut Vec<Value>,
    depth: usize,
) -> Result<()> {
    ensure!(depth <= 48, "OpenAPI 对比层级过深");
    ensure!(out.len() < 10000, "差异超过 10000 项");
    if a == b {
        return Ok(());
    }
    if let (Some(aa), Some(bb)) = (a.as_object(), b.as_object()) {
        let keys: BTreeSet<_> = aa.keys().chain(bb.keys()).collect();
        for k in keys {
            let p = format!("{}/{}", path, k.replace('~', "~0").replace('/', "~1"));
            match(aa.get(k),bb.get(k)) {
                (Some(old),Some(new))=>schema_changes(old,new,&p,out,depth+1)?,
                (Some(_),None)=>out.push(json!({"path":p,"change":"removed","compatibility":"可能影响兼容性，需结合请求/响应方向复核"})),
                (None,Some(_))=>out.push(json!({"path":p,"change":"added","compatibility":"复核新增必填字段或约束"})),
                _=>{}
            }
        }
    } else {
        out.push(json!({"path":path,"change":"changed","left":a,"right":b,"compatibility":if path.ends_with("/required")||path.ends_with("/type")||path.ends_with("/enum"){"约束变化：可能不兼容"}else{"需要复核"}}));
    }
    Ok(())
}
pub fn openapi(left: &str, right: &str) -> Result<String> {
    openapi_limited(left, right, 8 * 1024 * 1024)
}
pub(crate) fn openapi_limited(left: &str, right: &str, report_limit: usize) -> Result<String> {
    bounded(left)?;
    bounded(right)?;
    let a: Value = serde_json::from_str(left)?;
    let b: Value = serde_json::from_str(right)?;
    ensure!(
        a["openapi"].as_str().is_some_and(|s| s.starts_with("3."))
            && b["openapi"].as_str().is_some_and(|s| s.starts_with("3.")),
        "仅支持 OpenAPI 3.x JSON；不支持 Swagger 2"
    );
    let aa = operations(&a)?;
    let bb = operations(&b)?;
    let keys: BTreeSet<_> = aa.keys().chain(bb.keys()).collect();
    let mut changes = vec![];
    for k in keys {
        match (aa.get(k), bb.get(k)) {
            (Some(_), None) => changes.push(
                json!({"operation":k,"change":"removed","compatibility":"破坏性：原端点移除"}),
            ),
            (None, Some(_)) => {
                changes.push(json!({"operation":k,"change":"added","compatibility":"新增端点"}))
            }
            (Some(old), Some(new)) if old != new => {
                let mut diff = vec![];
                schema_changes(old, new, "", &mut diff, 0)?;
                changes.push(json!({"operation":k,"change":"changed","fields":diff}));
            }
            _ => {}
        }
    }
    let mut components = vec![];
    schema_changes(
        &a["components"],
        &b["components"],
        "/components",
        &mut components,
        0,
    )?;
    super::report_limited(
        json!({"operationChanges":changes,"componentChanges":components,"notes":"比较端点、字段和共享 components，不解引用外部或循环 $ref；共享类型变化独立报告，需追踪引用处。数组按整体比较，required 顺序差异也会报告。兼容性为保守提示，不执行 API、不验证权限或数据库。"}),
        report_limit,
    )
}
