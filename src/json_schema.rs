use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::{
    collections::{HashSet, VecDeque},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const SCHEMA_LIMIT: usize = 64 * 1024;
const INSTANCE_LIMIT: usize = 1024 * 1024;
const ERROR_LIMIT: usize = 100;
const EXTERNAL_SCHEMA_COUNT_LIMIT: usize = 32;
const EXTERNAL_SCHEMA_TOTAL_LIMIT: usize = 1024 * 1024;

pub fn validate(instance_text: &str, schema_text: &str) -> Result<String> {
    validate_inner(instance_text, schema_text, None)
}

pub fn load_schema_file(path: &Path) -> Result<(PathBuf, String)> {
    let canonical = path
        .canonicalize()
        .context("无法定位所选 JSON Schema 文件")?;
    let schema = read_limited_json_file(&canonical, SCHEMA_LIMIT)?;
    Ok((canonical, schema))
}

pub fn validate_with_schema_file(
    instance_text: &str,
    schema_text: &str,
    schema_file: &Path,
) -> Result<String> {
    validate_inner(instance_text, schema_text, Some(schema_file))
}

fn validate_inner(
    instance_text: &str,
    schema_text: &str,
    schema_file: Option<&Path>,
) -> Result<String> {
    if schema_text.len() > SCHEMA_LIMIT {
        return Err(anyhow!("JSON Schema 超过 64 KiB 限制"));
    }
    if instance_text.len() > INSTANCE_LIMIT {
        return Err(anyhow!("待校验 JSON 超过 1 MiB 限制"));
    }

    let schema: Value = serde_json::from_str(schema_text).context("JSON Schema 不是有效 JSON")?;
    let mut references = Vec::new();
    let mut contains_id = false;
    collect_schema_refs(&schema, 0, &mut references, &mut contains_id)?;

    let instance: Value = serde_json::from_str(instance_text).context("待校验内容不是有效 JSON")?;
    let registry = match schema_file {
        Some(path) => Some(build_local_registry(path, &schema, contains_id)?),
        None => {
            if let Some(reference) = references
                .iter()
                .find(|reference| !reference.starts_with('#'))
            {
                return Err(anyhow!(
                    "外部引用需要先选择本机 Schema 文件；已拒绝引用：{reference}"
                ));
            }
            None
        }
    };

    if let Some(registry) = &registry {
        if !jsonschema::meta::options()
            .with_registry(registry)
            .is_valid(&schema)
        {
            return Err(anyhow!("JSON Schema 文档不符合其声明的标准"));
        }
    } else if !jsonschema::meta::is_valid(&schema) {
        return Err(anyhow!("JSON Schema 文档不符合其声明的标准"));
    }

    let mut options = jsonschema::options()
        .offline()
        .with_pattern_options(jsonschema::PatternOptions::regex());
    if let Some(registry) = &registry {
        options = options.with_registry(registry);
        if let Some(schema_file) = schema_file {
            options = options.with_base_uri(file_uri(schema_file)?.to_string());
        }
    }
    let validator = options.build(&schema).context("无法编译 JSON Schema")?;

    let mut errors = validator.iter_errors(&instance).peekable();
    let mut output = String::new();
    let mut count = 0;
    for error in errors.by_ref().take(ERROR_LIMIT) {
        count += 1;
        let path = error.instance_path().to_string();
        let path = if path.is_empty() { "/" } else { &path };
        use std::fmt::Write;
        let _ = writeln!(output, "- 数据路径 `{path}`：{}", error.masked());
    }

    if count == 0 {
        return Ok("校验通过：JSON 符合 Schema。".into());
    }
    if errors.next().is_some() {
        output.push_str("\n错误超过 100 条，已截断显示。\n");
    }
    Ok(format!("校验失败：至少 {count} 条错误\n{output}"))
}

fn read_limited_json_file(path: &Path, limit: usize) -> Result<String> {
    let metadata = fs::metadata(path).context("读取所选 JSON Schema 文件失败")?;
    if !metadata.is_file() {
        return Err(anyhow!("所选路径不是普通文件"));
    }
    if metadata.len() > limit as u64 {
        return Err(anyhow!("JSON Schema 文件超过 64 KiB 限制"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(path)
        .context("打开 JSON Schema 文件失败")?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .context("读取 JSON Schema 文件失败")?;
    if bytes.len() > limit {
        return Err(anyhow!("JSON Schema 文件超过 64 KiB 限制"));
    }
    String::from_utf8(bytes).context("JSON Schema 文件必须是 UTF-8")
}

fn file_uri(path: &Path) -> Result<reqwest::Url> {
    reqwest::Url::from_file_path(path).map_err(|_| anyhow!("无法建立本地 Schema 文件 URI"))
}

fn build_local_registry(
    schema_file: &Path,
    root_schema: &Value,
    root_contains_id: bool,
) -> Result<jsonschema::Registry<'static>> {
    let root_file = schema_file
        .canonicalize()
        .context("无法定位所选 JSON Schema 文件")?;
    let root_directory = root_file
        .parent()
        .ok_or_else(|| anyhow!("所选 Schema 文件没有父目录"))?
        .to_path_buf();
    let root_uri = file_uri(&root_file)?;
    let root_uri_key = uri_without_fragment(root_uri.clone());
    let mut builder = jsonschema::Registry::new()
        .add(root_uri_key.as_str(), root_schema.clone())
        .context("无法登记根 Schema")?;
    let mut seen = HashSet::from([root_file.clone()]);
    let mut pending = VecDeque::from([(root_uri, root_schema.clone(), root_contains_id)]);
    let mut external_count = 0usize;
    let mut total_bytes = 0usize;

    while let Some((current_uri, current_schema, contains_id)) = pending.pop_front() {
        let mut references = Vec::new();
        let mut nested_contains_id = contains_id;
        collect_schema_refs(&current_schema, 0, &mut references, &mut nested_contains_id)?;
        for reference in references {
            if reference.starts_with('#') {
                continue;
            }
            let target_uri = resolve_local_reference(&reference, &current_uri, nested_contains_id)?;
            let target_file = target_uri
                .to_file_path()
                .map_err(|_| anyhow!("Schema 引用必须指向所选文件夹内的文件"))?
                .canonicalize()
                .context("无法读取本地 Schema 引用")?;
            if !target_file.starts_with(&root_directory) {
                return Err(anyhow!("Schema 引用越出了所选文件所在目录"));
            }
            if !seen.insert(target_file.clone()) {
                continue;
            }
            external_count += 1;
            if external_count > EXTERNAL_SCHEMA_COUNT_LIMIT {
                return Err(anyhow!("本次校验最多读取 32 个外部 Schema 文件"));
            }
            let contents = read_limited_json_file(&target_file, SCHEMA_LIMIT)?;
            total_bytes = total_bytes.saturating_add(contents.len());
            if total_bytes > EXTERNAL_SCHEMA_TOTAL_LIMIT {
                return Err(anyhow!("外部 Schema 文件累计超过 1 MiB 限制"));
            }
            let referenced_schema: Value =
                serde_json::from_str(&contents).context("本地引用文件不是有效 JSON")?;
            let uri = file_uri(&target_file)?;
            let uri_key = uri_without_fragment(uri.clone());
            builder = builder
                .add(uri_key.as_str(), referenced_schema.clone())
                .context("无法登记本地 Schema 引用")?;
            pending.push_back((uri, referenced_schema, false));
        }
    }
    builder.prepare().context("本地 Schema 引用无法解析")
}

fn uri_without_fragment(mut uri: reqwest::Url) -> reqwest::Url {
    uri.set_fragment(None);
    uri
}

fn resolve_local_reference(
    reference: &str,
    current_uri: &reqwest::Url,
    schema_contains_id: bool,
) -> Result<reqwest::Url> {
    let target = if let Ok(uri) = reqwest::Url::parse(reference) {
        uri
    } else {
        if schema_contains_id {
            return Err(anyhow!(
                "含有 $id 的 Schema 会按 URI 语义解析相对引用；请使用明确的本地 file:// 引用"
            ));
        }
        current_uri
            .join(reference)
            .context("本地 Schema 相对引用格式无效")?
    };
    if target.scheme() != "file" || target.query().is_some() {
        return Err(anyhow!("只允许所选 Schema 文件夹内的 file:// 本地引用"));
    }
    Ok(target)
}

fn collect_schema_refs(
    schema: &Value,
    depth: usize,
    references: &mut Vec<String>,
    contains_id: &mut bool,
) -> Result<()> {
    if depth > 128 {
        return Err(anyhow!("JSON Schema 嵌套超过 128 层"));
    }
    if let Value::Object(object) = schema {
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
                visit_schema_value(subschema, depth + 1, references, contains_id)?;
            }
        }
        for key in SCHEMA_MAPS {
            if let Some(map) = object.get(*key).and_then(Value::as_object) {
                for subschema in map.values() {
                    visit_schema_value(subschema, depth + 1, references, contains_id)?;
                }
            }
        }
        for key in SCHEMA_ARRAYS {
            if let Some(values) = object.get(*key).and_then(Value::as_array) {
                for subschema in values {
                    visit_schema_value(subschema, depth + 1, references, contains_id)?;
                }
            }
        }
        if let Some(dependencies) = object.get("dependencies").and_then(Value::as_object) {
            for dependency in dependencies.values() {
                if dependency.is_object() || dependency.is_boolean() {
                    visit_schema_value(dependency, depth + 1, references, contains_id)?;
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
) -> Result<()> {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_schema_refs(value, depth + 1, references, contains_id)?;
            }
        }
        _ => collect_schema_refs(value, depth, references, contains_id)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempSchemaFolder(PathBuf);

    impl TempSchemaFolder {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("zi-json-schema-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for TempSchemaFolder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn validates_draft_2020_schema_and_reports_instance_paths() {
        let schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","required":["profile"],"properties":{"profile":{"type":"object","required":["age"],"properties":{"age":{"type":"integer","minimum":0}}}}}"##;
        let invalid = r#"{"profile":{"age":"old"}}"#;
        let output = validate(invalid, schema).unwrap();
        assert!(output.contains("/profile/age"), "{output}");
        assert!(output.contains("校验失败"), "{output}");
        assert!(
            !output.contains("old"),
            "error output must mask instance values"
        );
    }

    #[test]
    fn accepts_embedded_local_reference() {
        let schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$defs":{"name":{"type":"string","minLength":1}},"type":"object","properties":{"name":{"$ref":"#/$defs/name"}},"required":["name"]}"##;
        assert!(
            validate(r#"{"name":"Ada"}"#, schema)
                .unwrap()
                .contains("校验通过")
        );
    }

    #[test]
    fn rejects_network_and_file_references_before_compilation() {
        for reference in [
            "https://example.com/schema.json",
            "file:///C:/private/schema.json",
        ] {
            let schema = format!(r#"{{"$ref":"{reference}"}}"#);
            let error = validate("{}", &schema).unwrap_err().to_string();
            assert!(error.contains("拒绝"), "{error}");
        }
    }

    #[test]
    fn treats_ref_shaped_const_data_as_data_not_a_schema_reference() {
        let schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","const":{"$ref":"https://example.com/a-value"}}"##;
        assert!(
            validate(r#"{"$ref":"https://example.com/a-value"}"#, schema)
                .unwrap()
                .contains("校验通过")
        );
    }

    #[test]
    fn rejects_external_reference_nested_under_a_schema_keyword() {
        let schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","properties":{"child":{"$ref":"https://example.com/schema.json"}}}"##;
        assert!(
            validate(r#"{"child":{}}"#, schema)
                .unwrap_err()
                .to_string()
                .contains("拒绝")
        );
    }

    #[test]
    fn resolves_user_selected_relative_schema_files_and_reports_nested_paths() {
        let folder = TempSchemaFolder::new();
        let root = folder.write(
            "root.json",
            r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"address":{"$ref":"address.json"}},"required":["address"]}"##,
        );
        folder.write(
            "address.json",
            r#"{"type":"object","properties":{"zip":{"type":"string","pattern":"^[0-9]{5}$"}},"required":["zip"]}"#,
        );
        let valid = r#"{"address":{"zip":"98052"}}"#;
        assert!(
            validate_with_schema_file(valid, &fs::read_to_string(&root).unwrap(), &root)
                .unwrap()
                .contains("校验通过")
        );

        let invalid = r#"{"address":{"zip":"abc"}}"#;
        let output =
            validate_with_schema_file(invalid, &fs::read_to_string(&root).unwrap(), &root).unwrap();
        assert!(output.contains("/address/zip"), "{output}");
    }

    #[test]
    fn rejects_schema_references_that_escape_the_selected_folder() {
        let parent = TempSchemaFolder::new();
        let folder = parent.0.join("schemas");
        fs::create_dir_all(&folder).unwrap();
        fs::write(parent.0.join("outside.json"), r#"{"type":"string"}"#).unwrap();
        let root = folder.join("root.json");
        fs::write(&root, r#"{"$ref":"../outside.json"}"#).unwrap();
        let error =
            validate_with_schema_file("\"data\"", &fs::read_to_string(&root).unwrap(), &root)
                .unwrap_err()
                .to_string();
        assert!(error.contains("越出了所选"), "{error}");
    }

    #[test]
    fn rejects_more_than_the_external_schema_file_limit() {
        let folder = TempSchemaFolder::new();
        let mut properties = serde_json::Map::new();
        for index in 0..=EXTERNAL_SCHEMA_COUNT_LIMIT {
            let name = format!("schema-{index}.json");
            folder.write(&name, r#"{"type":"string"}"#);
            properties.insert(format!("field-{index}"), serde_json::json!({"$ref": name}));
        }
        let root_schema = serde_json::json!({"type":"object", "properties": properties});
        let root_path = folder.0.join("root.json");
        fs::write(&root_path, serde_json::to_vec(&root_schema).unwrap()).unwrap();
        let error = validate_with_schema_file(
            "{}",
            &serde_json::to_string(&root_schema).unwrap(),
            &root_path,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("最多读取 32"), "{error}");
    }

    #[test]
    fn rejects_remote_references_even_when_a_local_schema_file_is_selected() {
        let folder = TempSchemaFolder::new();
        let root = folder.write("root.json", r#"{"$ref":"https://example.com/schema.json"}"#);
        let error = validate_with_schema_file("{}", &fs::read_to_string(&root).unwrap(), &root)
            .unwrap_err()
            .to_string();
        assert!(error.contains("file://"), "{error}");
    }

    #[test]
    fn relative_file_refs_with_uri_ids_are_rejected_as_ambiguous() {
        let folder = TempSchemaFolder::new();
        let root = folder.write(
            "root.json",
            r#"{"$id":"https://example.com/root.json","$ref":"child.json"}"#,
        );
        folder.write("child.json", r#"{"type":"string"}"#);
        let error =
            validate_with_schema_file("\"data\"", &fs::read_to_string(&root).unwrap(), &root)
                .unwrap_err()
                .to_string();
        assert!(error.contains("含有 $id"), "{error}");
    }

    #[test]
    fn rejects_invalid_schema_and_instance_json() {
        assert!(
            validate("{}", r#"{"type":"not-a-type"}"#)
                .unwrap_err()
                .to_string()
                .contains("Schema 文档")
        );
        assert!(
            validate("{", r#"{"type":"object"}"#)
                .unwrap_err()
                .to_string()
                .contains("待校验内容")
        );
    }

    #[test]
    fn enforces_separate_schema_and_instance_size_limits() {
        let schema = " ".repeat(SCHEMA_LIMIT + 1);
        assert!(
            validate("{}", &schema)
                .unwrap_err()
                .to_string()
                .contains("64 KiB")
        );
        let instance = " ".repeat(INSTANCE_LIMIT + 1);
        assert!(
            validate(&instance, "true")
                .unwrap_err()
                .to_string()
                .contains("1 MiB")
        );
    }

    #[test]
    fn selected_root_schema_file_is_bounded_before_loading() {
        let folder = TempSchemaFolder::new();
        let path = folder.write("oversized.schema.json", &" ".repeat(SCHEMA_LIMIT + 1));
        assert!(
            load_schema_file(&path)
                .unwrap_err()
                .to_string()
                .contains("64 KiB")
        );
    }

    #[test]
    fn caps_rendered_validation_errors() {
        let schema = r#"{"type":"array","items":{"type":"integer"}}"#;
        let instance = format!(
            "[{}]",
            (0..150).map(|_| "\"bad\"").collect::<Vec<_>>().join(",")
        );
        let output = validate(&instance, schema).unwrap();
        assert!(output.contains("错误超过 100 条"));
        assert_eq!(output.matches("- 数据路径").count(), 100);
    }
}
