//! Offline review of a native Python / Django environment report.
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use zi_agent_record_core::provenance::ToolProvenance;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewError {
    Format,
    Provenance,
    Version,
    Layout,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeReport {
    selected: Capture,
    comparison: Option<Capture>,
    path_python: Option<String>,
    path_django_admin: Option<String>,
    expected_sibling_django_admin: Option<String>,
    entry_layout_mismatch: Option<bool>,
    #[serde(rename = "notes")]
    _notes: String,
    generated_by: ToolProvenance,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Capture {
    #[serde(rename = "executable")]
    _executable: String,
    version: String,
    #[serde(rename = "prefix")]
    _prefix: String,
    #[serde(rename = "basePrefix")]
    _base_prefix: String,
    #[serde(rename = "scriptsPath")]
    _scripts_path: Option<String>,
    virtual_environment: bool,
    django: Option<Distribution>,
    django_import_origin: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Distribution {
    version: String,
    #[serde(rename = "location")]
    _location: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSummary {
    pub python_version: String,
    pub virtual_environment: bool,
    pub django_distribution_version: Option<String>,
    pub django_module_origin_reported: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub selected: CaptureSummary,
    pub comparison: Option<CaptureSummary>,
    pub path_python_reported: bool,
    pub path_django_admin_reported: bool,
    pub expected_sibling_django_admin_reported: bool,
    pub entry_layout_mismatch: Option<bool>,
    pub text: String,
}

fn valid_version(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 96
        && label.bytes().any(|byte| byte.is_ascii_digit())
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".+-_!".contains(&byte))
}

fn summarize(capture: Capture) -> Result<CaptureSummary, ReviewError> {
    let django_distribution_version = capture.django.map(|distribution| distribution.version);
    if !valid_version(&capture.version)
        || django_distribution_version
            .as_ref()
            .is_some_and(|label| !valid_version(label))
    {
        return Err(ReviewError::Version);
    }
    Ok(CaptureSummary {
        python_version: capture.version,
        virtual_environment: capture.virtual_environment,
        django_distribution_version,
        django_module_origin_reported: capture.django_import_origin.is_some(),
    })
}

fn observed(value: bool) -> &'static str {
    if value { "已记录" } else { "未记录" }
}

fn append_capture(out: &mut String, role: &str, capture: &CaptureSummary) {
    let _ = write!(
        out,
        "\n{role}：Python {} · 虚拟环境 {} · Django 分发 {} · 模块来源 {}",
        capture.python_version,
        if capture.virtual_environment {
            "是"
        } else {
            "否"
        },
        capture
            .django_distribution_version
            .as_deref()
            .unwrap_or("未记录"),
        observed(capture.django_module_origin_reported),
    );
}

pub fn review(source: &[u8]) -> Result<Review, ReviewError> {
    let source = source.strip_prefix(b"\xef\xbb\xbf").unwrap_or(source);
    let report: NativeReport = serde_json::from_slice(source).map_err(|_| ReviewError::Format)?;
    if report.generated_by.tool_id != "django-environment"
        || report.generated_by.validate().is_err()
    {
        return Err(ReviewError::Provenance);
    }
    let path_python_reported = report.path_python.is_some();
    let path_django_admin_reported = report.path_django_admin.is_some();
    let expected_sibling_django_admin_reported = report.expected_sibling_django_admin.is_some();
    if report.entry_layout_mismatch.is_some()
        != (path_django_admin_reported && expected_sibling_django_admin_reported)
    {
        return Err(ReviewError::Layout);
    }
    let selected = summarize(report.selected)?;
    let comparison = report.comparison.map(summarize).transpose()?;
    let mut text = String::from("Python / Django 环境报告 · 本机文件中的历史快照");
    append_capture(&mut text, "主环境", &selected);
    if let Some(other) = &comparison {
        append_capture(&mut text, "对比环境", other);
    } else {
        text.push_str("\n对比环境：未采集");
    }
    let mismatch = match report.entry_layout_mismatch {
        Some(true) => "不同",
        Some(false) => "未记录差异",
        None => "未判定",
    };
    let _ = write!(
        text,
        "\n捕获时入口 · PATH 中的 Python {} · PATH 中的 django-admin {} · 预期脚本位置 {}\n捕获时 django-admin 入口位置：{mismatch}\n来源工具版本：{}",
        observed(path_python_reported),
        observed(path_django_admin_reported),
        observed(expected_sibling_django_admin_reported),
        report.generated_by.tool_version
    );
    Ok(Review {
        selected,
        comparison,
        path_python_reported,
        path_django_admin_reported,
        expected_sibling_django_admin_reported,
        entry_layout_mismatch: report.entry_layout_mismatch,
        text,
    })
}
