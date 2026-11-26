use std::{
    collections::HashMap,
    fs::File,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
#[cfg(test)]
use base64::Engine;
use chrono::{DateTime, Local, LocalResult, NaiveDateTime, TimeZone};
use qrcode::{Color, QrCode};

// Share declaration metadata while keeping the native ToolKind type local.
// The native text-flow bridge provides conversions for that type.
#[path = "../crates/zi-plugin-manifest-core/src/tool_kind.rs"]
mod tool_kind;
pub use tool_kind::ToolKind;

pub fn run_tool(
    kind: ToolKind,
    action: usize,
    input: &str,
    pattern: &str,
    replacement: Option<&str>,
    base: u32,
) -> Result<String> {
    use crate::tools_extra as extra;
    extra::bounded(input)?;
    if action >= kind.actions().len() {
        return Err(anyhow!("无效的工具操作"));
    }
    match kind {
        ToolKind::Json => match action {
            0 => json_format(input),
            1 => json_minify(input),
            2 => json_format(input).map(|_| "JSON 有效".into()),
            _ => zi_json_core::object_summary_text(input),
        },
        ToolKind::Base64 => {
            if action == 0 {
                Ok(base64_encode(input))
            } else {
                base64_decode(input)
            }
        }
        ToolKind::Url => {
            if action == 0 {
                Ok(url_encode(input))
            } else {
                url_decode(input)
            }
        }
        ToolKind::Sha256 => Ok(sha256(input)),
        ToolKind::Timestamp => {
            if action == 0 {
                timestamp_to_local(input)
            } else {
                local_to_timestamp(input)
            }
        }
        ToolKind::Uuid => Ok(generate_uuid()),
        ToolKind::Jwt => inspect_jwt(input),
        ToolKind::Regex => test_regex(pattern, input),
        ToolKind::RegexReplace => {
            let replacement = replacement
                .ok_or_else(|| anyhow!("正则替换需要单独提供替换模板；请使用正则替换预览面板"))?;
            Ok(zi_regex_core::preview_replacements(pattern, input, replacement)?.output)
        }
        ToolKind::Number => convert_number(input, base),
        ToolKind::Qr => Err(anyhow!("请使用二维码生成按钮")),
        ToolKind::Color => color_convert(input),
        ToolKind::TextStats => text_stats(input),
        ToolKind::HtmlEscape => Ok(if action == 0 {
            html_escape(input)
        } else {
            html_unescape(input)
        }),
        ToolKind::TextEscape => {
            if action == 0 {
                text_escape(input)
            } else {
                text_unescape(input)
            }
        }
        ToolKind::CaseConvert => Ok(case_convert(
            input,
            ["camel", "pascal", "snake", "kebab"][action],
        )),
        ToolKind::Yaml => {
            if action == 0 {
                extra::yaml_to_json(input)
            } else {
                extra::json_to_yaml(input)
            }
        }
        ToolKind::Hex => {
            if action == 0 {
                extra::hex_encode(input)
            } else {
                extra::hex_decode(input)
            }
        }
        ToolKind::Lines => extra::process_lines(input, kind.actions()[action]),
        ToolKind::UrlInspect => extra::inspect_url(input),
        ToolKind::Cidr => extra::inspect_cidr(input),
        ToolKind::JsonPath => crate::tools_advanced::json_path(input, pattern),
        ToolKind::JsonDiff => crate::tools_advanced::json_diff(input, pattern, action == 1),
        ToolKind::DataQuality => crate::tools_advanced::quality(input, action),
        ToolKind::Cron => crate::tools_advanced::cron(input, pattern),
        ToolKind::Random => crate::tools_advanced::random(input, action == 1),
        ToolKind::Unicode => crate::tools_advanced::unicode(input, action),
        ToolKind::JavaTrace => crate::diagnostics::java_trace(input),
        ToolKind::DjangoTrace => crate::diagnostics::django_trace(input),
        ToolKind::SqlFormat => crate::sql_formatter::format(input, pattern),
        ToolKind::JsonSchema => crate::json_schema::validate(input, pattern),
    }
}

pub fn html_escape(input: &str) -> String {
    zi_text_core::transforms::html_escape(input)
}

pub fn html_unescape(input: &str) -> String {
    zi_text_core::transforms::html_unescape(input)
}

pub fn text_escape(input: &str) -> Result<String> {
    zi_text_core::transforms::text_escape(input)
}

pub fn text_unescape(input: &str) -> Result<String> {
    zi_text_core::transforms::text_unescape(input)
}

pub fn case_convert(input: &str, mode: &str) -> String {
    zi_text_core::transforms::case_convert(input, mode)
}

pub fn color_convert(input: &str) -> Result<String> {
    zi_inspect_core::color_convert(input)
}

pub fn text_stats(input: &str) -> Result<String> {
    zi_text_core::transforms::text_stats(input)
}

pub struct ToolState {
    pub selected: ToolKind,
    pub input: String,
    pub output: String,
    pub result_ready: bool,
    pub message: String,
    pub pattern: String,
    pub replacement: String,
    pub result_summary: Option<String>,
    pub sql_indent: crate::sql_formatter::IndentStyle,
    pub sql_keyword_case: crate::sql_formatter::KeywordCase,
    pub number_base: u32,
    pub qr_image: Option<QrImage>,
    pub plugin_compatible: bool,
    pub schema_file: Option<PathBuf>,
    drafts: HashMap<ToolKind, ToolDraft>,
}

#[derive(Default)]
struct ToolDraft {
    input: String,
    output: String,
    result_ready: bool,
    message: String,
    pattern: String,
    replacement: String,
    result_summary: Option<String>,
    sql_indent: crate::sql_formatter::IndentStyle,
    sql_keyword_case: crate::sql_formatter::KeywordCase,
    number_base: u32,
    plugin_compatible: bool,
    schema_file: Option<PathBuf>,
}

impl ToolState {
    #[cfg(feature = "ui-preview")]
    pub fn preview_draft_input(&self, kind: ToolKind) -> Option<&str> {
        if self.selected == kind {
            Some(&self.input)
        } else {
            self.drafts.get(&kind).map(|draft| draft.input.as_str())
        }
    }

    pub fn select(&mut self, kind: ToolKind) {
        if self.selected == kind {
            return;
        }
        self.drafts.insert(
            self.selected,
            ToolDraft {
                input: std::mem::take(&mut self.input),
                output: std::mem::take(&mut self.output),
                result_ready: self.result_ready,
                message: std::mem::take(&mut self.message),
                pattern: std::mem::take(&mut self.pattern),
                replacement: std::mem::take(&mut self.replacement),
                result_summary: self.result_summary.take(),
                sql_indent: self.sql_indent,
                sql_keyword_case: self.sql_keyword_case,
                number_base: self.number_base,
                plugin_compatible: self.plugin_compatible,
                schema_file: self.schema_file.take(),
            },
        );
        let next = self.drafts.remove(&kind).unwrap_or_default();
        self.selected = kind;
        self.input = next.input;
        self.output = next.output;
        self.result_ready = next.result_ready;
        self.message = next.message;
        self.plugin_compatible = next.plugin_compatible;
        self.schema_file = next.schema_file;
        self.pattern = next.pattern;
        self.replacement = next.replacement;
        self.result_summary = next.result_summary;
        self.sql_indent = next.sql_indent;
        self.sql_keyword_case = next.sql_keyword_case;
        self.number_base = if next.number_base == 0 {
            10
        } else {
            next.number_base
        };
        if kind == ToolKind::SqlFormat && self.pattern.is_empty() {
            self.pattern = kind.secondary_sample().into();
        }
        if kind == ToolKind::RegexReplace {
            if self.pattern.is_empty() {
                self.pattern = kind.secondary_sample().into();
            }
            if self.replacement.is_empty() {
                self.replacement = kind.replacement_sample().into();
            }
        }
    }
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            selected: ToolKind::default(),
            input: String::new(),
            output: String::new(),
            result_ready: false,
            message: String::new(),
            pattern: String::new(),
            replacement: String::new(),
            result_summary: None,
            sql_indent: crate::sql_formatter::IndentStyle::default(),
            sql_keyword_case: crate::sql_formatter::KeywordCase::default(),
            number_base: 10,
            qr_image: None,
            plugin_compatible: false,
            schema_file: None,
            drafts: HashMap::new(),
        }
    }
}

impl ToolState {
    pub fn has_result(&self) -> bool {
        self.message.is_empty() && (self.result_ready || !self.output.is_empty())
    }

    pub fn clear_result(&mut self) {
        self.output.clear();
        self.message.clear();
        self.result_ready = false;
        self.result_summary = None;
    }

    pub fn apply_result(&mut self, result: Result<String>) -> bool {
        match result {
            Ok(output) => {
                self.output = output;
                self.message.clear();
                self.result_ready = true;
                true
            }
            Err(error) => {
                self.clear_result();
                self.message = error.to_string();
                false
            }
        }
    }

    pub fn has_plugin_mode(&self) -> bool {
        matches!(
            self.selected,
            ToolKind::Json | ToolKind::Base64 | ToolKind::Sha256
        )
    }

    pub fn load_schema_file(&mut self, path: &Path) -> Result<()> {
        let (canonical_path, contents) = crate::json_schema::load_schema_file(path)?;
        self.pattern = contents;
        self.schema_file = Some(canonical_path);
        self.clear_result();
        Ok(())
    }

    pub fn clear_schema_file_source(&mut self) {
        self.schema_file = None;
        self.clear_result();
    }

    pub fn run(&mut self, action: usize) -> Result<String> {
        self.result_summary = None;
        if action >= self.selected.actions().len() {
            return Err(anyhow!("无效的工具操作"));
        }
        if self.selected == ToolKind::RegexReplace {
            let preview =
                zi_regex_core::preview_replacements(&self.pattern, &self.input, &self.replacement)?;
            self.result_summary = Some(format!("{} 条匹配 · 仅预览，输入未修改", preview.matches));
            return Ok(preview.output);
        }
        if self.selected == ToolKind::JsonSchema {
            return match self.schema_file.as_deref() {
                Some(path) => {
                    crate::json_schema::validate_with_schema_file(&self.input, &self.pattern, path)
                }
                None => crate::json_schema::validate(&self.input, &self.pattern),
            };
        }
        if self.selected == ToolKind::SqlFormat {
            return crate::sql_formatter::format_with_options(
                &self.input,
                &self.pattern,
                self.sql_indent,
                self.sql_keyword_case,
            );
        }
        if self.selected == ToolKind::Json && action == 3 && self.plugin_compatible {
            return run_json_object_plugin(&self.input);
        }
        if !self.plugin_compatible || !self.has_plugin_mode() {
            return run_tool(
                self.selected,
                action,
                &self.input,
                &self.pattern,
                Some(&self.replacement),
                self.number_base,
            );
        }
        // Reject a large paste before constructing/copying the JSON envelope.
        if self.input.len() > zi_text_core::TEXT_LIMIT {
            return Err(anyhow!(
                "INPUT_TOO_LARGE：文本超过 8192 UTF-8 字节，请缩短后再处理"
            ));
        }
        let capability = match (self.selected, action) {
            (ToolKind::Json, 0) => "devtools.text.json.format",
            (ToolKind::Json, 1) => "devtools.text.json.minify",
            (ToolKind::Json, 2) => "devtools.text.json.validate",
            (ToolKind::Base64, 0) => "devtools.text.base64.encode",
            (ToolKind::Base64, 1) => "devtools.text.base64.decode",
            (ToolKind::Sha256, 0) => "devtools.text.sha256",
            _ => return Err(anyhow!("无效的工具操作")),
        };
        let request = serde_json::to_vec(&serde_json::json!({
            "pluginId": "com.zicode.devtools.text", "sceneId": "standalone",
            "capabilityId": capability, "commandId": null, "input": {"text": self.input},
        }))?;
        let result = zi_text_core::execute_request(&request);
        // Check the same wire-result limit, even though the desktop shows raw text.
        let bounded: serde_json::Value =
            serde_json::from_slice(&zi_text_core::serialize_result(&result))?;
        if !bounded["ok"].as_bool().unwrap_or(false) {
            return Err(anyhow!(
                "{}：{}",
                bounded["error"]["code"]
                    .as_str()
                    .unwrap_or("INTERNAL_ERROR"),
                bounded["error"]["message"].as_str().unwrap_or("处理失败")
            ));
        }
        Ok(result.data.expect("successful result").text)
    }
}

fn run_json_object_plugin(input: &str) -> Result<String> {
    anyhow::ensure!(
        input.len() <= zi_json_object_wasi::OBJECT_LIMIT,
        "INPUT_TOO_LARGE：JSON object 输入最多 8 KiB"
    );
    let value = zi_text_core::parse_strict_json(input)
        .map_err(|_| anyhow!("INVALID_INPUT：请输入符合插件兼容规则的 JSON object"))?;
    anyhow::ensure!(value.is_object(), "INVALID_INPUT：根值必须是 JSON object");
    let request = serde_json::to_vec(&serde_json::json!({
        "pluginId": zi_json_object_wasi::PLUGIN_ID,
        "sceneId": "standalone",
        "capabilityId": zi_json_object_wasi::INSPECT,
        "commandId": null,
        "input": {"value": value}
    }))?;
    let response: serde_json::Value =
        serde_json::from_slice(&zi_json_object_wasi::execute_request(&request))?;
    if !response["ok"].as_bool().unwrap_or(false) {
        return Err(anyhow!(
            "{}：{}",
            response["error"]["code"]
                .as_str()
                .unwrap_or("INTERNAL_ERROR"),
            response["error"]["message"].as_str().unwrap_or("处理失败")
        ));
    }
    Ok(serde_json::to_string_pretty(&response["data"])?)
}

pub struct QrImage {
    pub rgba: Vec<u8>,
    pub side: u32,
}

impl QrImage {
    pub fn save_png(&self, path: &Path) -> Result<()> {
        let file = File::create(path).with_context(|| format!("无法创建 {}", path.display()))?;
        let mut encoder = png::Encoder::new(file, self.side, self.side);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&self.rgba)?;
        Ok(())
    }
}

pub fn convert_number(input: &str, base: u32) -> Result<String> {
    zi_text_core::transforms::convert_number(input, base)
}

pub fn generate_qr(input: &str) -> Result<QrImage> {
    if input.is_empty() {
        return Err(anyhow!("请输入二维码内容"));
    }
    if input.len() > 2_048 {
        return Err(anyhow!("二维码内容不能超过 2048 字节"));
    }
    let code = QrCode::new(input.as_bytes()).context("内容无法编码为二维码")?;
    let width = code.width();
    const SCALE: usize = 6;
    const QUIET: usize = 4;
    let side = (width + QUIET * 2) * SCALE;
    let mut rgba = vec![255; side * side * 4];
    for (index, color) in code.to_colors().iter().enumerate() {
        if *color == Color::Dark {
            let x0 = (index % width + QUIET) * SCALE;
            let y0 = (index / width + QUIET) * SCALE;
            for y in y0..y0 + SCALE {
                for x in x0..x0 + SCALE {
                    let offset = (y * side + x) * 4;
                    rgba[offset..offset + 3].fill(0);
                }
            }
        }
    }
    Ok(QrImage {
        rgba,
        side: side as u32,
    })
}

pub fn json_format(input: &str) -> Result<String> {
    zi_text_core::json(input, true).context("JSON 格式无效")
}

pub fn json_minify(input: &str) -> Result<String> {
    zi_text_core::json(input, false).context("JSON 格式无效")
}

pub fn base64_encode(input: &str) -> String {
    zi_text_core::encode(input)
}

pub fn base64_decode(input: &str) -> Result<String> {
    zi_text_core::decode(input.trim()).map_err(|error| anyhow!(error))
}

pub fn url_encode(input: &str) -> String {
    zi_text_core::transforms::url_encode(input)
}

pub fn url_decode(input: &str) -> Result<String> {
    zi_text_core::transforms::url_decode(input)
}

pub fn sha256(input: &str) -> String {
    zi_text_core::sha256(input)
}

pub fn timestamp_to_local(input: &str) -> Result<String> {
    let raw: i64 = input.trim().parse().context("请输入整数时间戳")?;
    let seconds = if raw.unsigned_abs() > 10_000_000_000 {
        raw / 1_000
    } else {
        raw
    };
    match Local.timestamp_opt(seconds, 0) {
        LocalResult::Single(value) => Ok(value.format("%Y-%m-%d %H:%M:%S %:z").to_string()),
        _ => Err(anyhow!("时间戳超出可转换范围")),
    }
}

pub fn local_to_timestamp(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if let Ok(value) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(value.timestamp().to_string());
    }
    let naive = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S")
        .context("请输入 YYYY-MM-DD HH:MM:SS 或 RFC 3339 时间")?;
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(value) => Ok(value.timestamp().to_string()),
        LocalResult::Ambiguous(_, _) => Err(anyhow!("该本地时间处于时区切换歧义区间")),
        LocalResult::None => Err(anyhow!("该本地时间不存在")),
    }
}

pub fn generate_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn inspect_jwt(input: &str) -> Result<String> {
    zi_inspect_core::inspect_jwt(input)
}

pub use zi_regex_core::test_regex;

#[cfg(test)]
mod tests {
    #[test]
    fn empty_success_is_distinct_from_idle_and_failure_and_survives_tool_switch() {
        let mut state = super::ToolState::default();
        state.select(super::ToolKind::Base64);
        assert!(!state.has_result());
        let result = state.run(0);
        assert!(state.apply_result(result));
        assert!(state.output.is_empty() && state.has_result());
        state.select(super::ToolKind::Json);
        assert!(!state.has_result());
        let result = state.run(0);
        assert!(!state.apply_result(result));
        assert!(!state.has_result() && !state.message.is_empty());
        state.select(super::ToolKind::Base64);
        assert!(state.has_result() && state.output.is_empty());
        state.plugin_compatible = true;
        state.input = "bad!".into();
        let result = state.run(1);
        assert!(!state.apply_result(result));
        assert!(!state.has_result());
        state.clear_result();
        assert!(!state.has_result() && state.message.is_empty());
    }
    #[test]
    fn plugin_mode_preserves_legacy_and_tool_drafts() {
        let mut state = super::ToolState {
            input: "9007199254740992".into(),
            ..Default::default()
        };
        assert!(state.run(1).is_ok());
        state.plugin_compatible = true;
        assert!(
            state
                .run(1)
                .unwrap_err()
                .to_string()
                .contains("INVALID_INPUT")
        );
        state.select(super::ToolKind::Base64);
        assert!(!state.plugin_compatible);
        state.input = " YQ== ".into();
        assert_eq!(state.run(1).unwrap(), "a");
        state.plugin_compatible = true;
        assert!(
            state
                .run(1)
                .unwrap_err()
                .to_string()
                .contains("INVALID_ENCODING")
        );
        state.select(super::ToolKind::Json);
        assert!(state.plugin_compatible);
        assert_eq!(state.input, "9007199254740992");
        state.input = r#"{"ok":true}"#.into();
        assert_eq!(state.run(2).unwrap(), "JSON 有效（插件兼容规则）");
        assert!(state.run(3).unwrap().contains("\"value\""));
        state.input = "[1,2]".into();
        assert!(
            state
                .run(3)
                .unwrap_err()
                .to_string()
                .contains("根值必须是 JSON object")
        );
        state.input = r#"{"x":1,"x":2}"#.into();
        assert!(
            state
                .run(3)
                .unwrap_err()
                .to_string()
                .contains("INVALID_INPUT")
        );
        state.input = r#"{"x":1,"x":2}"#.into();
        assert!(
            state
                .run(2)
                .unwrap_err()
                .to_string()
                .contains("INVALID_INPUT")
        );
        state.select(super::ToolKind::Sha256);
        state.plugin_compatible = true;
        state.input = "abc".into();
        assert_eq!(state.run(0).unwrap(), super::sha256("abc"));
        state.input = "中".repeat(2731);
        assert!(
            state
                .run(0)
                .unwrap_err()
                .to_string()
                .contains("INPUT_TOO_LARGE")
        );
        state.input = "a".repeat(zi_text_core::TEXT_LIMIT);
        assert_eq!(state.run(0).unwrap(), super::sha256(&state.input));
        state.input.push('a');
        assert!(
            state
                .run(0)
                .unwrap_err()
                .to_string()
                .contains("INPUT_TOO_LARGE")
        );
    }
    #[test]
    fn all_tool_samples_run_and_ids_are_unique() {
        let mut ids = std::collections::HashSet::new();
        for kind in super::ToolKind::ALL {
            assert!(ids.insert(kind.id()));
            if kind == super::ToolKind::Qr {
                continue;
            }
            assert!(
                super::run_tool(
                    kind,
                    0,
                    kind.sample(),
                    kind.secondary_sample(),
                    (kind == ToolKind::RegexReplace).then_some(kind.replacement_sample()),
                    10,
                )
                .is_ok(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn sql_formatter_dialect_defaults_and_survives_tool_switches() {
        let mut state = super::ToolState::default();
        state.select(super::ToolKind::SqlFormat);
        assert_eq!(state.pattern, "postgresql");
        state.pattern = "mysql".into();
        state.input = "select `order` from `items`".into();
        state.select(super::ToolKind::Json);
        state.select(super::ToolKind::SqlFormat);
        assert_eq!(state.pattern, "mysql");
        assert_eq!(state.run(0).unwrap(), "SELECT `order`\nFROM `items`");
        state.sql_indent = crate::sql_formatter::IndentStyle::TwoSpaces;
        state.sql_keyword_case = crate::sql_formatter::KeywordCase::Lowercase;
        state.select(super::ToolKind::Json);
        state.select(super::ToolKind::SqlFormat);
        assert_eq!(
            state.sql_indent,
            crate::sql_formatter::IndentStyle::TwoSpaces
        );
        assert_eq!(
            state.sql_keyword_case,
            crate::sql_formatter::KeywordCase::Lowercase
        );
        assert_eq!(state.run(0).unwrap(), "select `order`\nfrom `items`");
    }
    use super::*;

    #[test]
    fn formats_and_minifies_json() {
        assert_eq!(json_minify("{ \"a\": 1 }").unwrap(), "{\"a\":1}");
        assert!(json_format("not-json").is_err());
    }

    #[test]
    fn text_codecs_round_trip_unicode() {
        let input = "Zi 开发工具";
        assert_eq!(base64_decode(&base64_encode(input)).unwrap(), input);
        assert_eq!(url_decode(&url_encode(input)).unwrap(), input);
    }

    #[test]
    fn added_text_helpers_round_trip() {
        let input = "<Zi & tools>\"";
        assert_eq!(html_unescape(&html_escape(input)), input);
        let escaped = text_escape(input).unwrap();
        assert_eq!(text_unescape(&escaped).unwrap(), input);
        assert_eq!(case_convert("Zi dev-tools", "camel"), "ziDevTools");
        assert_eq!(case_convert("Zi dev-tools", "pascal"), "ZiDevTools");
        assert!(ToolKind::Json.is_encoding());
        assert!(!ToolKind::CaseConvert.is_encoding());
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(
            sha256("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn jwt_view_decodes_without_claiming_verification() {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"sub":"zi"}"#);
        let output = inspect_jwt(&format!("{header}.{payload}.fake")).unwrap();
        assert!(output.contains("未验证签名"));
        assert!(output.contains("\"sub\": \"zi\""));
        assert!(inspect_jwt("not-a-jwt").is_err());
    }

    #[test]
    fn regex_lists_captures_and_rejects_invalid_pattern() {
        let output = test_regex(r"(zi)-(\d+)", "zi-42 and zi-7").unwrap();
        assert!(output.contains("#1 字节 0..5"));
        assert!(output.contains("$2: 42"));
        assert!(output.contains("#2 字节 10..14"));
        assert!(test_regex("(", "text").is_err());
    }

    #[test]
    fn converts_signed_numbers_between_common_bases() {
        let output = convert_number("-0x2A", 16).unwrap();
        assert!(output.contains("十进制  -42"));
        assert!(output.contains("二进制  -0b101010"));
        assert!(output.contains("八进制  -0o52"));
        assert!(convert_number("2", 2).is_err());
        assert!(convert_number("", 10).is_err());
        assert!(convert_number(&i128::MIN.to_string(), 10).is_ok());
    }

    #[test]
    fn converts_colors_and_reports_text_statistics() {
        let color = color_convert("#f80").unwrap();
        assert!(color.contains("#FF8800"));
        assert!(color.contains("rgb(255, 136, 0)"));
        assert!(color_convert("#xyz").is_err());
        let stats = text_stats("你好 zi\n").unwrap();
        assert!(stats.contains("UTF-8 字节  10"));
        assert!(stats.contains("Unicode 字符  6"));
        assert!(stats.contains("行数  2"));
    }

    #[test]
    fn each_small_tool_keeps_its_own_draft() {
        let mut state = ToolState {
            input: "json draft".into(),
            ..Default::default()
        };
        state.select(ToolKind::Color);
        assert!(state.input.is_empty());
        state.input = "#f80".into();
        state.select(ToolKind::Json);
        assert_eq!(state.input, "json draft");
        state.select(ToolKind::Color);
        assert_eq!(state.input, "#f80");
    }

    #[test]
    fn selected_schema_file_and_editor_contents_follow_their_tool_draft() {
        let folder = std::env::temp_dir().join(format!("zi-schema-tool-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("schema.json");
        std::fs::write(&path, r#"{"type":"string","minLength":1}"#).unwrap();

        let mut state = ToolState::default();
        state.select(ToolKind::JsonSchema);
        state.load_schema_file(&path).unwrap();
        state.input = r#""ready""#.into();
        assert!(state.run(0).unwrap().contains("校验通过"));

        state.select(ToolKind::Json);
        assert!(state.schema_file.is_none());
        state.select(ToolKind::JsonSchema);
        assert_eq!(
            state.schema_file.as_deref(),
            Some(path.canonicalize().unwrap().as_path())
        );
        assert!(state.pattern.contains("minLength"));
        assert!(state.run(0).unwrap().contains("校验通过"));

        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn regex_replacement_preview_preserves_its_template_and_match_summary_per_tool() {
        let mut state = ToolState::default();
        state.select(ToolKind::RegexReplace);
        state.input = "build-42 release-14".into();
        state.replacement = "$1=$2".into();
        assert!(state.run(1).is_err());
        let output = state.run(0).unwrap();
        assert_eq!(output, "build=42 release=14");
        assert_eq!(
            state.result_summary.as_deref(),
            Some("2 条匹配 · 仅预览，输入未修改")
        );
        assert!(state.apply_result(Ok(output)));

        state.select(ToolKind::Regex);
        state.input = "query draft".into();
        state.select(ToolKind::RegexReplace);
        assert_eq!(state.input, "build-42 release-14");
        assert_eq!(state.replacement, "$1=$2");
        assert!(state.has_result());
        assert_eq!(
            state.result_summary.as_deref(),
            Some("2 条匹配 · 仅预览，输入未修改")
        );
        state.clear_result();
        assert!(state.result_summary.is_none());
    }

    #[test]
    fn renders_qr_with_quiet_zone_and_writes_png() {
        let image = generate_qr("Zi 开发工具").unwrap();
        assert_eq!(
            image.rgba.len(),
            image.side as usize * image.side as usize * 4
        );
        assert_eq!(&image.rgba[..4], &[255, 255, 255, 255]);
        assert!(
            image
                .rgba
                .chunks_exact(4)
                .any(|pixel| pixel == [0, 0, 0, 255])
        );
        let path = std::env::temp_dir().join(format!("zi-qr-{}.png", uuid::Uuid::new_v4()));
        image.save_png(&path).unwrap();
        assert!(
            std::fs::read(&path)
                .unwrap()
                .starts_with(b"\x89PNG\r\n\x1a\n")
        );
        std::fs::remove_file(path).unwrap();
        assert!(generate_qr("").is_err());
    }
}
