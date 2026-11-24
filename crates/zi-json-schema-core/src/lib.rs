//! Shared offline JSON Schema validation. Native file resolution stays in the desktop.
use anyhow::{Context, Result, anyhow};
use jsonschema::Draft;
use serde_json::Value;

pub const SCHEMA_LIMIT: usize = 64 * 1024;
pub const INSTANCE_LIMIT: usize = 1024 * 1024;
const ERROR_LIMIT: usize = 100;
// JSON may escape one byte as six. Leave room for the envelope and truncation notice.
const STUDIO_DECLARED_REPORT_BYTES: usize = 7_900;

pub fn parse_schema(instance_text: &str, schema_text: &str) -> Result<Value> {
    if schema_text.len() > SCHEMA_LIMIT {
        return Err(anyhow!("JSON Schema 超过 64 KiB 限制"));
    }
    if instance_text.len() > INSTANCE_LIMIT {
        return Err(anyhow!("待校验 JSON 超过 1 MiB 限制"));
    }

    serde_json::from_str(schema_text).context("JSON Schema 不是有效 JSON")
}

pub fn parse_instance(instance_text: &str) -> Result<Value> {
    serde_json::from_str(instance_text).context("待校验内容不是有效 JSON")
}

pub fn validate_document(instance_text: &str, schema_text: &str) -> Result<String> {
    validate_pasted(instance_text, schema_text, false)
}

/// The first Studio profile has a Draft 7 fallback and explicitly permits 4, 6 and 7.
pub fn validate_legacy_document(instance_text: &str, schema_text: &str) -> Result<String> {
    validate_pasted(instance_text, schema_text, true)
}

fn validate_pasted(instance_text: &str, schema_text: &str, legacy: bool) -> Result<String> {
    let schema = parse_schema(instance_text, schema_text)?;
    let draft = legacy.then(|| legacy_draft(&schema)).transpose()?;
    let mut references = Vec::new();
    let mut contains_id = false;
    inspect_schema(&schema, 0, &mut references, &mut contains_id, legacy)?;
    let instance = parse_instance(instance_text)?;
    if let Some(reference) = references
        .iter()
        .find(|reference| !reference.starts_with('#'))
    {
        let error = if legacy {
            anyhow!("仅支持文档内引用，已拒绝引用。External references are unavailable.")
        } else {
            anyhow!("外部引用需要先选择本机 Schema 文件；已拒绝引用：{reference}")
        };
        return Err(error);
    }
    validate_with_options(&instance, &schema, None, None, draft)
}

fn legacy_draft(schema: &Value) -> Result<Draft> {
    match schema.get("$schema") {
        None => Ok(Draft::Draft7),
        Some(uri) => match uri.as_str().map(Draft::from_schema_uri) {
            Some(draft @ (Draft::Draft4 | Draft::Draft6 | Draft::Draft7)) => Ok(draft),
            _ => Err(anyhow!(
                "此插件支持 Draft 4、6、7（未声明时用 Draft 7）。This plugin supports Draft 4, 6 and 7 (default Draft 7)."
            )),
        },
    }
}

pub fn validate_documents(
    instance: &Value,
    schema: &Value,
    registry: Option<&jsonschema::Registry<'_>>,
    base_uri: Option<String>,
) -> Result<String> {
    validate_with_options(instance, schema, registry, base_uri, None)
}

fn validate_with_options(
    instance: &Value,
    schema: &Value,
    registry: Option<&jsonschema::Registry<'_>>,
    base_uri: Option<String>,
    draft: Option<Draft>,
) -> Result<String> {
    // WASI builds recompile standard meta-schemas for every call. The library's
    // validator build already validates the schema and preserves the same rules.
    #[cfg(not(target_os = "wasi"))]
    if draft.is_none() {
        let schema_is_valid = match registry {
            Some(registry) => jsonschema::meta::options()
                .with_registry(registry)
                .is_valid(schema),
            None => jsonschema::meta::is_valid(schema),
        };
        if !schema_is_valid {
            return Err(anyhow!("JSON Schema 文档不符合其声明的标准"));
        }
    }

    let mut options = jsonschema::options()
        .offline()
        .with_pattern_options(jsonschema::PatternOptions::regex());
    if let Some(draft) = draft {
        options = options.with_draft(draft);
    }
    if let Some(registry) = registry {
        options = options.with_registry(registry);
        if let Some(base_uri) = base_uri {
            options = options.with_base_uri(base_uri);
        }
    }
    let validator = options.build(schema).context("无法编译 JSON Schema")?;

    let mut errors = validator.iter_errors(instance);
    let mut output = String::new();
    let mut count = 0;
    let mut report_truncated = false;
    for error in errors.by_ref().take(ERROR_LIMIT) {
        count += 1;
        let path = error.instance_path().to_string();
        let path = if path.is_empty() { "/" } else { &path };
        let line = format!("- 数据路径 `{path}`：{}\n", error.masked());
        if draft.is_some() && output.len() + line.len() > STUDIO_DECLARED_REPORT_BYTES {
            report_truncated = true;
            break;
        }
        output.push_str(&line);
    }

    if count == 0 {
        return Ok("校验通过：JSON 符合 Schema。".into());
    }
    if report_truncated {
        output.push_str("\n校验报告已达到显示上限，已截断。\n");
    } else if errors.next().is_some() {
        output.push_str("\n错误超过 100 条，已截断显示。\n");
    }
    Ok(format!("校验失败：至少 {count} 条错误\n{output}"))
}

pub fn collect_schema_refs(
    schema: &Value,
    depth: usize,
    references: &mut Vec<String>,
    contains_id: &mut bool,
) -> Result<()> {
    inspect_schema(schema, depth, references, contains_id, false)
}

fn inspect_schema(
    schema: &Value,
    depth: usize,
    references: &mut Vec<String>,
    contains_id: &mut bool,
    legacy: bool,
) -> Result<()> {
    if depth > 128 {
        return Err(anyhow!("JSON Schema 嵌套超过 128 层"));
    }
    if let Value::Object(object) = schema {
        if legacy && object.contains_key("$schema") {
            legacy_draft(schema)?;
        }
        *contains_id |= object.get("$id").and_then(Value::as_str).is_some();
        for key in ["$ref", "$dynamicRef", "$recursiveRef"] {
            if let Some(reference) = object.get(key).and_then(Value::as_str) {
                references.push(reference.to_owned());
            }
        }

        const SINGLE_SCHEMA: &[&str] = &[
            "additionalItems",
            "additionalProperties",
            "contains",
            "contentSchema",
            "else",
            "if",
            "items",
            "not",
            "propertyNames",
            "then",
            "unevaluatedItems",
            "unevaluatedProperties",
        ];
        const SCHEMA_MAPS: &[&str] = &[
            "$defs",
            "definitions",
            "dependentSchemas",
            "patternProperties",
            "properties",
        ];
        const SCHEMA_ARRAYS: &[&str] = &["allOf", "anyOf", "oneOf", "prefixItems"];

        for key in SINGLE_SCHEMA {
            if let Some(subschema) = object.get(*key) {
                visit_schema_value(subschema, depth + 1, references, contains_id, legacy)?;
            }
        }
        for key in SCHEMA_MAPS {
            if let Some(map) = object.get(*key).and_then(Value::as_object) {
                for subschema in map.values() {
                    visit_schema_value(subschema, depth + 1, references, contains_id, legacy)?;
                }
            }
        }
        for key in SCHEMA_ARRAYS {
            if let Some(values) = object.get(*key).and_then(Value::as_array) {
                for subschema in values {
                    visit_schema_value(subschema, depth + 1, references, contains_id, legacy)?;
                }
            }
        }
        if let Some(dependencies) = object.get("dependencies").and_then(Value::as_object) {
            for dependency in dependencies.values() {
                if dependency.is_object() || dependency.is_boolean() {
                    visit_schema_value(dependency, depth + 1, references, contains_id, legacy)?;
                }
            }
        }
    }
    Ok(())
}

fn visit_schema_value(
    value: &Value,
    depth: usize,
    references: &mut Vec<String>,
    contains_id: &mut bool,
    legacy: bool,
) -> Result<()> {
    match value {
        Value::Array(values) => {
            for value in values {
                inspect_schema(value, depth + 1, references, contains_id, legacy)?;
            }
        }
        _ => inspect_schema(value, depth, references, contains_id, legacy)?,
    }
    Ok(())
}
