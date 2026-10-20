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
    budget: &mut super::CollectionBudget,
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
            let change = match (aa.get(k), bb.get(k)) {
                (Some(old), Some(new)) => {
                    schema_changes(old, new, &p, out, depth + 1, budget)?;
                    None
                }
                (Some(_), None) => Some(
                    json!({"path":p,"change":"removed","compatibility":"可能影响兼容性，需结合请求/响应方向复核"}),
                ),
                (None, Some(_)) => Some(
                    json!({"path":p,"change":"added","compatibility":"复核新增必填字段或约束"}),
                ),
                _ => None,
            };
            if let Some(change) = change {
                budget.add(&change)?;
                out.push(change);
            }
        }
    } else {
        let change = json!({"path":path,"change":"changed","left":a,"right":b,"compatibility":if path.ends_with("/required")||path.ends_with("/type")||path.ends_with("/enum"){"约束变化：可能不兼容"}else{"需要复核"}});
        budget.add(&change)?;
        out.push(change);
    }
    Ok(())
}

const MAX_SCHEMA_REF_NODES: usize = 50_000;
const MAX_SCHEMA_REF_BYTES: usize = 2 * 1024 * 1024;
const MAX_SCHEMA_REF_EXPANSIONS: usize = 4_096;
const MAX_SCHEMA_REF_DEPTH: usize = 64;
const MAX_SCHEMA_REF_REPORT_ROWS: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SchemaContext {
    Generic,
    Schema,
    SchemaMap,
    SchemaList,
}

#[derive(Default)]
struct SchemaRefStats {
    resolved: usize,
    cycles: Vec<Value>,
    unresolved: Vec<Value>,
    visited_nodes: usize,
    expanded_bytes: usize,
    ref_expansions: usize,
    truncated: bool,
    side: String,
}

impl SchemaRefStats {
    fn push_record(
        destination: &mut Vec<Value>,
        truncated: &mut bool,
        side: &str,
        path: &str,
        reference: &str,
        reason: &str,
    ) {
        if destination.len() >= MAX_SCHEMA_REF_REPORT_ROWS {
            *truncated = true;
            return;
        }
        let mut bounded = |value: &str| {
            let mut chars = value.chars();
            let output: String = chars.by_ref().take(512).collect();
            if chars.next().is_some() {
                *truncated = true;
            }
            output
        };
        destination.push(json!({
            "side":side,
            "path":bounded(path),
            "reference":bounded(reference),
            "reason":reason
        }));
    }

    fn record_cycle(&mut self, path: &str, reference: &str, reason: &str) {
        Self::push_record(
            &mut self.cycles,
            &mut self.truncated,
            &self.side,
            path,
            reference,
            reason,
        );
    }

    fn record_unresolved(&mut self, path: &str, reference: &str, reason: &str) {
        Self::push_record(
            &mut self.unresolved,
            &mut self.truncated,
            &self.side,
            path,
            reference,
            reason,
        );
    }

    fn report(&self) -> Value {
        json!({
            "resolvedSchemaRefs":self.resolved,
            "cycles":self.cycles,
            "unresolvedSchemaRefs":self.unresolved,
            "truncated":self.truncated
        })
    }
}

struct SchemaRefResolver<'a> {
    document: &'a Value,
    honor_schema_ref_siblings: bool,
    allow_boolean_schemas: bool,
    stats: SchemaRefStats,
    active: BTreeSet<String>,
}

impl<'a> SchemaRefResolver<'a> {
    fn new(document: &'a Value, side: &str, openapi_version: &str) -> Self {
        let mut version = openapi_version.split('.');
        let major = version.next().and_then(|value| value.parse::<u32>().ok());
        let minor = version.next().and_then(|value| value.parse::<u32>().ok());
        let supports_json_schema_refs = major == Some(3) && minor.is_some_and(|value| value >= 1);
        Self {
            document,
            honor_schema_ref_siblings: supports_json_schema_refs,
            allow_boolean_schemas: supports_json_schema_refs,
            stats: SchemaRefStats {
                side: side.to_owned(),
                ..SchemaRefStats::default()
            },
            active: BTreeSet::new(),
        }
    }

    fn resolve(
        &mut self,
        value: &Value,
        path: &str,
        context: SchemaContext,
        depth: usize,
    ) -> Value {
        if depth > MAX_SCHEMA_REF_DEPTH || self.stats.visited_nodes >= MAX_SCHEMA_REF_NODES {
            self.stats.truncated = true;
            return value.clone();
        }
        self.stats.visited_nodes += 1;
        if let Value::String(text) = value {
            self.stats.expanded_bytes = self.stats.expanded_bytes.saturating_add(text.len());
        }
        if self.stats.expanded_bytes > MAX_SCHEMA_REF_BYTES {
            self.stats.truncated = true;
            return value.clone();
        }

        match value {
            Value::Object(object) => {
                if context == SchemaContext::Schema
                    && let Some(reference) = object.get("$ref").and_then(Value::as_str)
                {
                    return self.resolve_schema_ref(object, reference, path, depth);
                }
                let mut output = serde_json::Map::new();
                for (key, child) in object {
                    self.stats.expanded_bytes = self.stats.expanded_bytes.saturating_add(key.len());
                    let child_context = schema_child_context(context, key, path);
                    let child_path =
                        format!("{}/{}", path, key.replace('~', "~0").replace('/', "~1"));
                    output.insert(
                        key.clone(),
                        self.resolve(child, &child_path, child_context, depth + 1),
                    );
                }
                Value::Object(output)
            }
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        self.resolve(item, &format!("{path}/{index}"), context, depth + 1)
                    })
                    .collect(),
            ),
            _ => value.clone(),
        }
    }

    fn resolve_schema_ref(
        &mut self,
        object: &serde_json::Map<String, Value>,
        reference: &str,
        path: &str,
        depth: usize,
    ) -> Value {
        self.stats.ref_expansions += 1;
        if self.stats.ref_expansions > MAX_SCHEMA_REF_EXPANSIONS {
            self.stats.truncated = true;
            return Value::Object(object.clone());
        }
        if !reference.starts_with("#/") {
            self.stats.record_unresolved(
                path,
                reference,
                "仅解析同文档 JSON Pointer schema 引用；未读取外部文档或锚点",
            );
            return Value::Object(object.clone());
        }
        let Some((target, target_path)) = json_pointer_target(self.document, reference) else {
            self.stats
                .record_unresolved(path, reference, "JSON Pointer 无效或目标不存在");
            return Value::Object(object.clone());
        };
        if self.active.contains(&target_path) {
            self.stats.record_cycle(
                path,
                reference,
                "检测到递归 schema 引用；本次递归边保持为占位标记",
            );
            return json!({"$ref":reference,"$cycle":true});
        }
        let valid_target =
            target.is_object() || (self.allow_boolean_schemas && target.is_boolean());
        if !valid_target {
            self.stats.record_unresolved(
                path,
                reference,
                "schema 引用目标不是对象或当前版本允许的布尔 schema",
            );
            return Value::Object(object.clone());
        }
        self.active.insert(target_path.clone());
        let resolved = self.resolve(target, &target_path, SchemaContext::Schema, depth + 1);
        self.active.remove(&target_path);
        self.stats.resolved += 1;
        if object.len() <= 1 || !self.honor_schema_ref_siblings {
            return resolved;
        }
        let mut siblings = object.clone();
        siblings.remove("$ref");
        let sibling_path = format!("{path}/$ref-siblings");
        let siblings = self.resolve(
            &Value::Object(siblings),
            &sibling_path,
            SchemaContext::Schema,
            depth + 1,
        );
        json!({"allOf":[resolved,siblings]})
    }
}

fn schema_child_context(context: SchemaContext, key: &str, parent_path: &str) -> SchemaContext {
    match context {
        SchemaContext::SchemaMap | SchemaContext::SchemaList => SchemaContext::Schema,
        SchemaContext::Schema => match key {
            "properties" | "patternProperties" | "definitions" | "$defs" | "dependentSchemas" => {
                SchemaContext::SchemaMap
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => SchemaContext::SchemaList,
            "items"
            | "additionalProperties"
            | "not"
            | "contains"
            | "propertyNames"
            | "if"
            | "then"
            | "else" => SchemaContext::Schema,
            _ => SchemaContext::Generic,
        },
        SchemaContext::Generic if key == "schema" => SchemaContext::Schema,
        SchemaContext::Generic if parent_path == "/components" && key == "schemas" => {
            SchemaContext::SchemaMap
        }
        SchemaContext::Generic => SchemaContext::Generic,
    }
}

fn json_pointer_target<'a>(document: &'a Value, reference: &str) -> Option<(&'a Value, String)> {
    let pointer = reference.strip_prefix("#/")?;
    let mut value = document;
    let mut normalized_path = String::new();
    for raw_segment in pointer.split('/') {
        let segment = decode_json_pointer_segment(raw_segment)?;
        value = match value {
            Value::Object(object) => object.get(&segment)?,
            Value::Array(array) => {
                if segment != "0" && segment.starts_with('0')
                    || segment.is_empty()
                    || !segment.bytes().all(|byte| byte.is_ascii_digit())
                {
                    return None;
                }
                array.get(segment.parse::<usize>().ok()?)?
            }
            _ => return None,
        };
        normalized_path.push('/');
        normalized_path.push_str(&segment.replace('~', "~0").replace('/', "~1"));
    }
    Some((value, normalized_path))
}

fn decode_json_pointer_segment(segment: &str) -> Option<String> {
    let mut output = String::with_capacity(segment.len());
    let mut chars = segment.chars();
    while let Some(character) = chars.next() {
        if character != '~' {
            output.push(character);
            continue;
        }
        output.push(match chars.next()? {
            '0' => '~',
            '1' => '/',
            _ => return None,
        });
    }
    Some(output)
}

// A document without reachable $ref keys needs no copied tree or JSON Pointer
// allocation. Keep the same traversal limits and truncation evidence as resolve.
fn scan_ref_free(value: &Value, depth: usize, stats: &mut SchemaRefStats) -> bool {
    if depth > MAX_SCHEMA_REF_DEPTH || stats.visited_nodes >= MAX_SCHEMA_REF_NODES {
        stats.truncated = true;
        return true;
    }
    stats.visited_nodes += 1;
    if let Value::String(text) = value {
        stats.expanded_bytes = stats.expanded_bytes.saturating_add(text.len());
    }
    if stats.expanded_bytes > MAX_SCHEMA_REF_BYTES {
        stats.truncated = true;
        return true;
    }
    match value {
        Value::Object(object) => {
            if object.contains_key("$ref") {
                return false;
            }
            for (key, child) in object {
                stats.expanded_bytes = stats.expanded_bytes.saturating_add(key.len());
                if !scan_ref_free(child, depth + 1, stats) {
                    return false;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                if !scan_ref_free(item, depth + 1, stats) {
                    return false;
                }
            }
        }
        _ => {}
    }
    true
}

fn resolve_openapi_schema_refs<'a>(
    document: &'a Value,
    side: &str,
) -> (std::borrow::Cow<'a, Value>, Value) {
    let mut stats = SchemaRefStats {
        side: side.to_owned(),
        ..Default::default()
    };
    if ["paths", "components"].iter().all(|key| {
        document
            .get(key)
            .is_none_or(|value| scan_ref_free(value, 0, &mut stats))
    }) {
        return (std::borrow::Cow::Borrowed(document), stats.report());
    }
    let mut normalized = document.clone();
    let version = document
        .get("openapi")
        .and_then(Value::as_str)
        .unwrap_or("3.0");
    let mut resolver = SchemaRefResolver::new(document, side, version);
    if let Some(paths) = document.get("paths") {
        normalized["paths"] = resolver.resolve(paths, "/paths", SchemaContext::Generic, 0);
    }
    if let Some(components) = document.get("components") {
        normalized["components"] =
            resolver.resolve(components, "/components", SchemaContext::Generic, 0);
    }
    let stats = resolver.stats.report();
    (std::borrow::Cow::Owned(normalized), stats)
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
    let (a, left_references) = resolve_openapi_schema_refs(&a, "left");
    let (b, right_references) = resolve_openapi_schema_refs(&b, "right");
    let aa = operations(&a)?;
    let bb = operations(&b)?;
    let keys: BTreeSet<_> = aa.keys().chain(bb.keys()).collect();
    let mut changes = vec![];
    // Charge disjoint rows as they are created. Nested field rows are at least
    // depth two in the final report, so this is a lower bound, not a stricter cap.
    let mut output_budget = super::CollectionBudget::new(report_limit);
    for k in keys {
        let change = match (aa.get(k), bb.get(k)) {
            (Some(_), None) => {
                json!({"operation":k,"change":"removed","compatibility":"破坏性：原端点移除"})
            }
            (None, Some(_)) => {
                json!({"operation":k,"change":"added","compatibility":"新增端点"})
            }
            (Some(old), Some(new)) if old != new => {
                let mut diff = vec![];
                schema_changes(old, new, "", &mut diff, 0, &mut output_budget)?;
                changes.push(json!({"operation":k,"change":"changed","fields":diff}));
                continue; // Its field rows were already charged, never count them twice.
            }
            _ => continue,
        };
        output_budget.add(&change)?;
        changes.push(change);
    }
    let mut components = vec![];
    schema_changes(
        &a["components"],
        &b["components"],
        "/components",
        &mut components,
        0,
        &mut output_budget,
    )?;
    super::report_limited(
        json!({"operationChanges":changes,"componentChanges":components,"referenceAnalysis":{"left":left_references,"right":right_references},"notes":"传播比较同文档 JSON Pointer 的 Schema Object $ref；递归引用边保留占位并列入 cycles，外部引用、锚点及非 Schema Reference Object 不读取。解析最多 4096 次引用、50000 个节点、2 MiB 展开文本、64 层；限额通过 referenceAnalysis.truncated 明示。共享类型变化同时显示组件差异与受影响端点。数组按整体比较，required 顺序差异也会报告。兼容性为保守提示，不执行 API、不验证权限或数据库。"}),
        report_limit,
    )
}

#[cfg(test)]
mod ref_free_tests {
    use super::*;

    fn compare_resolvers(document: Value, borrowed: bool, truncated: bool) {
        let mut expected = document.clone();
        let mut resolver = SchemaRefResolver::new(&document, "left", "3.1.0");
        for key in ["paths", "components"] {
            if let Some(value) = document.get(key) {
                expected[key] =
                    resolver.resolve(value, &format!("/{key}"), SchemaContext::Generic, 0);
            }
        }
        let (actual, stats) = resolve_openapi_schema_refs(&document, "left");
        assert_eq!(matches!(actual, std::borrow::Cow::Borrowed(_)), borrowed);
        assert_eq!(actual.as_ref(), &expected);
        assert_eq!(stats, resolver.stats.report());
        assert_eq!(stats["truncated"], truncated);
    }

    #[test]
    fn borrowed_resolution_preserves_values_and_all_traversal_limits() {
        compare_resolvers(
            json!({"openapi":"3.1.0","paths":{"/items":{"get":{}}},"components":{"schemas":{"Item":{"type":"string"}}}}),
            true,
            false,
        );
        let mut deep = json!({"schema":{"$ref":"#/components/schemas/Item"}});
        for _ in 0..MAX_SCHEMA_REF_DEPTH + 2 {
            deep = json!({"nested":deep});
        }
        compare_resolvers(json!({"openapi":"3.1.0","paths":deep}), true, true);
        compare_resolvers(
            json!({"openapi":"3.1.0","paths":vec![Value::Null; MAX_SCHEMA_REF_NODES + 1],"components":{"schemas":{"Item":{"type":"string"}}}}),
            true,
            true,
        );
        compare_resolvers(
            json!({"openapi":"3.1.0","paths":{"description":"x".repeat(MAX_SCHEMA_REF_BYTES + 1)},"components":{"schemas":{"Item":{"type":"string"}}}}),
            true,
            true,
        );
    }

    #[test]
    fn reachable_refs_fall_back_without_changing_resolution() {
        for reference in [json!("#/components/schemas/Item"), Value::Null] {
            compare_resolvers(
                json!({"openapi":"3.1.0","paths":{"/items":{"get":{"schema":{"$ref":reference}}}},"components":{"schemas":{"Item":{"type":"string"}}}}),
                false,
                false,
            );
        }
    }
}
