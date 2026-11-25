//! Offline review of the native JDK environment JSON report.
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use zi_agent_record_core::provenance::ToolProvenance;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewError {
    Format,
    Provenance,
    Value,
    Comparison,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeReport {
    selected: Capture,
    comparison: Option<Capture>,
    #[serde(rename = "javaHome")]
    _java_home: Option<String>,
    java_home_mismatch: Option<bool>,
    path_java: Option<String>,
    maven_entry: Option<String>,
    gradle_entry: Option<String>,
    version_mismatch: Option<bool>,
    #[serde(rename = "notes")]
    _notes: String,
    generated_by: ToolProvenance,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Capture {
    #[serde(rename = "executable")]
    _executable: String,
    properties: Properties,
    javac_present: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Properties {
    #[serde(rename = "java.home")]
    _home: Option<String>,
    #[serde(rename = "java.version")]
    version: String,
    #[serde(rename = "java.vendor")]
    vendor: Option<String>,
    #[serde(rename = "os.arch")]
    arch: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub selected_version: String,
    pub compared_version: Option<String>,
    pub version_mismatch: Option<bool>,
    pub java_home_mismatch: Option<bool>,
    pub path_java_reported: bool,
    pub maven_entry_reported: bool,
    pub gradle_entry_reported: bool,
    pub text: String,
}

/// The native report treats versions as captured strings, including build suffixes.
pub fn version_mismatch(selected: Option<&str>, compared: Option<&str>) -> Option<bool> {
    compared.map(|right| Some(right) != selected)
}

fn display_label(value: &str, limit: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= limit
        && !value
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\' | ':'))
}

fn capture_is_displayable(capture: &Capture) -> bool {
    let fields = &capture.properties;
    display_label(&fields.version, 96)
        && fields
            .vendor
            .as_ref()
            .is_none_or(|text| display_label(text, 160))
        && fields
            .arch
            .as_ref()
            .is_none_or(|text| display_label(text, 64))
}

fn detected(value: bool) -> &'static str {
    if value { "已记录" } else { "未记录" }
}

fn difference(value: Option<bool>, present: &'static str, absent: &'static str) -> &'static str {
    match value {
        Some(true) => present,
        Some(false) => absent,
        None => "未判定",
    }
}

fn append_capture(out: &mut String, role: &str, capture: &Capture) {
    let selected = &capture.properties;
    let _ = write!(
        out,
        "\n{role}：{} · 供应商 {} · 架构 {} · javac {}",
        selected.version,
        selected.vendor.as_deref().unwrap_or("未报告"),
        selected.arch.as_deref().unwrap_or("未报告"),
        detected(capture.javac_present)
    );
}

pub fn review(source: &[u8]) -> Result<Review, ReviewError> {
    let source = source.strip_prefix(b"\xef\xbb\xbf").unwrap_or(source);
    let report: NativeReport = serde_json::from_slice(source).map_err(|_| ReviewError::Format)?;
    if report.generated_by.tool_id != "java-environment" || report.generated_by.validate().is_err()
    {
        return Err(ReviewError::Provenance);
    }
    if !capture_is_displayable(&report.selected)
        || report
            .comparison
            .as_ref()
            .is_some_and(|capture| !capture_is_displayable(capture))
    {
        return Err(ReviewError::Value);
    }
    let selected = &report.selected.properties.version;
    let compared = report
        .comparison
        .as_ref()
        .map(|capture| capture.properties.version.as_str());
    let version_mismatch = version_mismatch(Some(selected), compared);
    if report.version_mismatch != version_mismatch {
        return Err(ReviewError::Comparison);
    }

    let path_java_reported = report.path_java.is_some();
    let maven_entry_reported = report.maven_entry.is_some();
    let gradle_entry_reported = report.gradle_entry.is_some();
    let mut text = String::from("JDK 环境报告 · 本机文件中的历史快照");
    append_capture(&mut text, "主环境", &report.selected);
    if let Some(other) = &report.comparison {
        append_capture(&mut text, "对比环境", other);
    } else {
        text.push_str("\n对比环境：未采集");
    }
    let _ = write!(
        text,
        "\n版本字符串：{} · 捕获时 JAVA_HOME：{}\n捕获时入口 · Java {} · Maven {} · Gradle {}\n来源工具版本：{}",
        difference(version_mismatch, "不同", "相同"),
        difference(report.java_home_mismatch, "不一致", "一致"),
        detected(path_java_reported),
        detected(maven_entry_reported),
        detected(gradle_entry_reported),
        report.generated_by.tool_version
    );

    Ok(Review {
        selected_version: selected.clone(),
        compared_version: compared.map(str::to_owned),
        version_mismatch,
        java_home_mismatch: report.java_home_mismatch,
        path_java_reported,
        maven_entry_reported,
        gradle_entry_reported,
        text,
    })
}
