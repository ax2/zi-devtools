//! Opt-in, bounded clipboard history with optional protected local snapshots.
pub(crate) const MAX_PDF_OCR_PAGES: usize = 50;
mod media;
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod ocr;
#[cfg(windows)]
pub(crate) use ocr::{
    RecognizerLanguage, available_languages as available_ocr_languages, recognize_image,
    recognize_image_with_language, recognize_pdf_pages_with_language,
};
mod policy;
pub use media::Picture;
#[cfg(windows)]
mod storage;
mod ui;
pub use policy::CapturePolicy;
pub use ui::State;

pub(crate) fn validate_screenshot_copy_dimensions(width: u32, height: u32) -> anyhow::Result<()> {
    anyhow::ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 4_000_000,
        "复制到剪贴板的截图最多支持400万像素"
    );
    Ok(())
}

pub(crate) fn prepare_screenshot_picture(
    image: &image::RgbaImage,
) -> anyhow::Result<(Picture, Vec<u8>)> {
    validate_screenshot_copy_dimensions(image.width(), image.height())?;
    let picture = Picture::from_image(image::DynamicImage::ImageRgba8(image.clone()))?;
    let dib = media::to_dib(&picture)?;
    Ok((picture, dib))
}

#[cfg(windows)]
pub(crate) fn copy_screenshot_to_clipboard(image: &image::RgbaImage) -> Result<(), String> {
    let (picture, dib) = prepare_screenshot_picture(image).map_err(|error| format!("{error:#}"))?;
    native::copy_picture(&picture, &dib)
}

const ITEM_LIMIT: usize = 500;
const TEXT_LIMIT: usize = 1024 * 1024;
const BYTE_LIMIT: usize = 32 * 1024 * 1024;
const LABEL_LIMIT: usize = 8;
const LABEL_CHAR_LIMIT: usize = 32;
const LABEL_BYTE_LIMIT: usize = 128;
const TEMPLATE_BYTE_LIMIT: usize = 16 * 1024;
const FILE_REFERENCE_LIMIT: usize = 64;
const FILE_REFERENCE_PATH_BYTE_LIMIT: usize = 32 * 1024;
const FILE_REFERENCE_BYTE_LIMIT: usize = 1024 * 1024;
const HTML_LIMIT: usize = 2 * 1024 * 1024;
const RTF_LIMIT: usize = 2 * 1024 * 1024;
const RTF_BASE64_LIMIT: usize = RTF_LIMIT.div_ceil(3) * 4;
pub const DEFAULT_TEMPLATE: &str = "{{index}}. {{text}}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OcrRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub(crate) fn catalog_tool_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| {
            let catalog: serde_json::Value =
                serde_json::from_str(include_str!("../../docs/tools.json"))
                    .expect("tool catalog must be valid JSON");
            catalog["tools"]
                .as_array()
                .and_then(|tools| {
                    tools
                        .iter()
                        .find(|tool| tool["id"].as_str() == Some("super-clipboard"))
                })
                .and_then(|tool| tool["tool_version"].as_str())
                .expect("super-clipboard must declare its tool version")
                .to_owned()
        })
        .as_str()
}

#[derive(Clone, Copy)]
enum TemplateField {
    Index,
    Text,
    Source,
    Time,
    Labels,
}

enum TemplatePart<'a> {
    Literal(&'a str),
    Field(TemplateField),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: u64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rich_html: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "rtf_base64")]
    pub rich_rtf: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<std::sync::Arc<Picture>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_references: Vec<String>,
    pub source: String,
    pub time: String,
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default)]
    pub first_captured_utc: Option<i64>,
    #[serde(default)]
    pub last_captured_utc: Option<i64>,
}
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct History {
    pub entries: Vec<Entry>,
    next: u64,
    #[serde(default)]
    pub retention_days: Option<u16>,
}
impl Entry {
    pub fn bytes(&self) -> usize {
        self.image
            .as_ref()
            .map_or(self.text.len(), |image| image.cost())
            .saturating_add(self.rich_html.as_ref().map_or(0, String::len))
            .saturating_add(self.rich_rtf.as_ref().map_or(0, Vec::len))
            .saturating_add(self.file_references.iter().map(String::len).sum::<usize>())
            .saturating_add(self.labels.iter().map(String::len).sum::<usize>())
    }
}
impl History {
    pub fn add_label(&mut self, id: u64, label: &str) -> Result<bool, String> {
        let label = normalize_label(label)?;
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return Err("历史条目已不存在".into());
        };
        let entry = &self.entries[index];
        let label_key = label.to_lowercase();
        if entry
            .labels
            .iter()
            .any(|existing| existing.to_lowercase() == label_key)
        {
            return Ok(false);
        }
        if entry.labels.len() >= LABEL_LIMIT {
            return Err(format!("每条历史最多添加{LABEL_LIMIT}个标签"));
        }
        if self.bytes().saturating_add(label.len()) > BYTE_LIMIT {
            return Err("标签将使历史总容量超过32 MiB".into());
        }
        self.entries[index].labels.push(label);
        Ok(true)
    }

    pub fn remove_label(&mut self, id: u64, label: &str) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) else {
            return false;
        };
        let before = entry.labels.len();
        let key = label.to_lowercase();
        entry
            .labels
            .retain(|existing| existing.to_lowercase() != key);
        entry.labels.len() != before
    }

    pub fn add_label_to_many(&mut self, ids: &[u64], label: &str) -> Result<usize, String> {
        let label = normalize_label(label)?;
        let key = label.to_lowercase();
        let mut unique_ids = Vec::with_capacity(ids.len());
        let mut additions = 0usize;
        let mut additional_bytes = 0usize;

        for &id in ids {
            if unique_ids.contains(&id) {
                continue;
            }
            unique_ids.push(id);
            let Some(entry) = self.entries.iter().find(|entry| entry.id == id) else {
                return Err("所选历史已变化，请重新选择后再试".into());
            };
            if entry
                .labels
                .iter()
                .any(|existing| existing.to_lowercase() == key)
            {
                continue;
            }
            if entry.labels.len() >= LABEL_LIMIT {
                return Err(format!("所选条目中有记录已达到{LABEL_LIMIT}个标签上限"));
            }
            additions += 1;
            additional_bytes = additional_bytes.saturating_add(label.len());
        }

        if self.bytes().saturating_add(additional_bytes) > BYTE_LIMIT {
            return Err("批量添加将使历史总容量超过32 MiB".into());
        }
        if additions == 0 {
            return Ok(0);
        }
        for id in unique_ids {
            if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id)
                && !entry
                    .labels
                    .iter()
                    .any(|existing| existing.to_lowercase() == key)
            {
                entry.labels.push(label.clone());
            }
        }
        Ok(additions)
    }

    pub fn insert(&mut self, text: String, source: String) -> Result<(), String> {
        self.insert_content(text, None, None, None, Vec::new(), source)
    }
    pub fn insert_rich_text(
        &mut self,
        text: String,
        html: String,
        source: String,
    ) -> Result<(), String> {
        self.insert_content(text, None, Some(html), None, Vec::new(), source)
    }
    pub fn insert_rich_rtf(
        &mut self,
        text: String,
        rtf: Vec<u8>,
        source: String,
    ) -> Result<(), String> {
        self.insert_content(text, None, None, Some(rtf), Vec::new(), source)
    }
    pub fn insert_rich_formats(
        &mut self,
        text: String,
        html: String,
        rtf: Vec<u8>,
        source: String,
    ) -> Result<(), String> {
        self.insert_content(text, None, Some(html), Some(rtf), Vec::new(), source)
    }
    pub fn insert_image(
        &mut self,
        image: std::sync::Arc<Picture>,
        source: String,
    ) -> Result<(), String> {
        self.insert_content(String::new(), Some(image), None, None, Vec::new(), source)
    }
    pub fn insert_files(&mut self, paths: Vec<String>, source: String) -> Result<(), String> {
        let paths = normalize_file_references(paths)?;
        if paths.is_empty() {
            return Ok(());
        }
        self.insert_content(String::new(), None, None, None, paths, source)
    }
    fn insert_content(
        &mut self,
        text: String,
        image: Option<std::sync::Arc<Picture>>,
        rich_html: Option<String>,
        rich_rtf: Option<Vec<u8>>,
        file_references: Vec<String>,
        source: String,
    ) -> Result<(), String> {
        if text.is_empty()
            && image.is_none()
            && rich_html.is_none()
            && rich_rtf.is_none()
            && file_references.is_empty()
        {
            return Ok(());
        }
        if [
            !text.is_empty(),
            image.is_some(),
            rich_html.is_some(),
            rich_rtf.is_some(),
            !file_references.is_empty(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count()
            > 1
            && !((rich_html.is_some() || rich_rtf.is_some())
                && image.is_none()
                && file_references.is_empty())
        {
            return Err("剪贴板历史条目类型无效".into());
        }
        if text.len() > TEXT_LIMIT {
            return Err("单条文本超过1 MiB，未采集".into());
        }
        if rich_html
            .as_ref()
            .is_some_and(|html| html.is_empty() || html.len() > HTML_LIMIT || html.contains('\0'))
        {
            return Err("HTML片段为空、包含零字符或超过2 MiB，未采集".into());
        }
        if rich_rtf
            .as_ref()
            .is_some_and(|rtf| validate_rtf_payload(rtf).is_err())
        {
            return Err("RTF格式无效或超过2 MiB，未采集".into());
        }
        if let Some(image) = &image {
            if image.cost() > BYTE_LIMIT {
                return Err("图片超过历史容量".into());
            }
        }
        if file_references.iter().map(String::len).sum::<usize>() > FILE_REFERENCE_BYTE_LIMIT {
            return Err("文件引用路径总量超过1 MiB".into());
        }
        let existing = self.entries.iter().position(|e| match (&e.image, &image) {
            (Some(a), Some(b)) => a.sha256 == b.sha256,
            (None, None) if rich_html.is_some() || rich_rtf.is_some() => {
                e.image.is_none()
                    && e.file_references.is_empty()
                    && e.text == text
                    && e.rich_html == rich_html
                    && e.rich_rtf == rich_rtf
            }
            (None, None) if !file_references.is_empty() => {
                !e.file_references.is_empty()
                    && same_file_references(&e.file_references, &file_references)
            }
            (None, None) => {
                e.file_references.is_empty()
                    && e.rich_html.is_none()
                    && e.rich_rtf.is_none()
                    && e.text == text
            }
            _ => false,
        });
        let mut entry = if let Some(i) = existing {
            self.entries.remove(i)
        } else {
            self.next = self.next.checked_add(1).ok_or("历史标识已耗尽")?;
            Entry {
                id: self.next,
                text,
                rich_html,
                rich_rtf,
                image,
                file_references,
                source: String::new(),
                time: String::new(),
                pinned: false,
                labels: Vec::new(),
                first_captured_utc: Some(chrono::Utc::now().timestamp()),
                last_captured_utc: None,
            }
        };
        let now = chrono::Utc::now().timestamp();
        entry.last_captured_utc = Some(
            entry
                .last_captured_utc
                .unwrap_or(now)
                .max(entry.first_captured_utc.unwrap_or(now))
                .max(now),
        );
        entry.source = source;
        entry.time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        // Preflight eviction before mutating history: pinned content is never discarded.
        let mut bytes = self.bytes() + entry.bytes();
        let mut count = self.entries.len() + 1;
        let mut evict = Vec::new();
        for old in self.entries.iter().rev().filter(|e| !e.pinned) {
            if count <= ITEM_LIMIT && bytes <= BYTE_LIMIT {
                break;
            }
            evict.push(old.id);
            count -= 1;
            bytes -= old.bytes();
        }
        if count > ITEM_LIMIT || bytes > BYTE_LIMIT {
            if let Some(index) = existing {
                self.entries.insert(index, entry);
            }
            return Err("置顶内容已占满容量，请取消部分置顶或删除条目".into());
        }
        self.entries.retain(|e| !evict.contains(&e.id));
        self.entries.insert(0, entry);
        Ok(())
    }
    pub fn expired_count(&self, days: u16, now: i64) -> usize {
        self.entries
            .iter()
            .filter(|entry| Self::expired(entry, days, now))
            .count()
    }
    fn expired(entry: &Entry, days: u16, now: i64) -> bool {
        (1..=3650).contains(&days)
            && !entry.pinned
            && entry.last_captured_utc.is_some_and(|captured| {
                now.checked_sub(captured)
                    .is_some_and(|age| age >= i64::from(days) * 86400)
            })
    }
    pub fn expire(&mut self, now: i64) -> usize {
        let Some(days) = self.retention_days else {
            return 0;
        };
        let before = self.entries.len();
        self.entries
            .retain(|entry| !Self::expired(entry, days, now));
        before - self.entries.len()
    }
    pub fn bytes(&self) -> usize {
        self.entries
            .iter()
            .fold(0usize, |sum, entry| sum.saturating_add(entry.bytes()))
    }
    fn selected_text_entries(&self, selected: &[u64]) -> Result<Vec<&Entry>, String> {
        if selected.iter().any(|id| {
            self.entries
                .iter()
                .any(|e| e.id == *id && (e.image.is_some() || !e.file_references.is_empty()))
        }) {
            return Err("图片或文件引用不能参加文本组合".into());
        }
        let entries: Vec<&Entry> = selected
            .iter()
            .map(|id| {
                self.entries
                    .iter()
                    .find(|e| e.id == *id)
                    .filter(|e| {
                        e.image.is_none() && e.file_references.is_empty() && !e.text.is_empty()
                    })
                    .ok_or("选择的历史已失效，请重新选择".to_string())
            })
            .collect::<Result<_, _>>()?;
        if entries.is_empty() {
            return Err("先选择历史条目".into());
        }
        if entries.iter().map(|entry| entry.text.len()).sum::<usize>() > TEXT_LIMIT {
            return Err("组合原文超过1 MiB，请减少选择".into());
        }
        Ok(entries)
    }

    pub fn combined(&self, selected: &[u64], format: usize) -> Result<String, String> {
        let entries = self.selected_text_entries(selected)?;
        let parts = entries
            .iter()
            .map(|entry| entry.text.as_str())
            .collect::<Vec<_>>();
        let value = match format {
            1 => parts
                .iter()
                .enumerate()
                .map(|(i, t)| format!("{}. {}", i + 1, t))
                .collect::<Vec<_>>()
                .join("\n"),
            2 => serde_json::to_string_pretty(&parts).map_err(|_| "JSON组合失败")?,
            3 => {
                let mut writer = csv::Writer::from_writer(Vec::new());
                writer.write_record(["text"]).map_err(|_| "CSV组合失败")?;
                for text in parts {
                    let safe = if text.trim_start().starts_with(['=', '+', '-', '@'])
                        || text.starts_with(['\t', '\r'])
                    {
                        format!("'{text}")
                    } else {
                        text.to_string()
                    };
                    writer.write_record([safe]).map_err(|_| "CSV组合失败")?;
                }
                String::from_utf8(writer.into_inner().map_err(|_| "CSV组合失败")?)
                    .map_err(|_| "CSV编码失败")?
            }
            4 => parts
                .iter()
                .map(|text| {
                    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                    let mut lines = normalized.split('\n');
                    let first = lines.next().unwrap_or_default();
                    std::iter::once(format!("- {first}"))
                        .chain(lines.map(|line| format!("  {line}")))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .collect::<Vec<_>>()
                .join("\n"),
            6 => {
                #[derive(serde::Serialize)]
                struct Record<'a> {
                    order: usize,
                    text: &'a str,
                    source: &'a str,
                    time: &'a str,
                    labels: &'a [String],
                }
                let records = entries
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| Record {
                        order: index + 1,
                        text: &entry.text,
                        source: &entry.source,
                        time: &entry.time,
                        labels: &entry.labels,
                    })
                    .collect::<Vec<_>>();
                struct BoundedJsonWriter(Vec<u8>);
                impl std::io::Write for BoundedJsonWriter {
                    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                        let available = TEXT_LIMIT.saturating_sub(self.0.len());
                        if available == 0 && !bytes.is_empty() {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::WriteZero,
                                "JSON记录结果超过1 MiB",
                            ));
                        }
                        let count = bytes.len().min(available);
                        self.0.extend_from_slice(&bytes[..count]);
                        Ok(count)
                    }

                    fn flush(&mut self) -> std::io::Result<()> {
                        Ok(())
                    }
                }
                let mut writer = BoundedJsonWriter(Vec::with_capacity(TEXT_LIMIT));
                serde_json::to_writer_pretty(&mut writer, &records)
                    .map_err(|_| "JSON记录组合结果超过1 MiB，请减少选择")?;
                String::from_utf8(writer.0).map_err(|_| "JSON记录组合编码失败")?
            }
            _ => parts.join("\n\n"),
        };
        if value.len() > TEXT_LIMIT {
            return Err("组合结果超过1 MiB，请减少选择".into());
        }
        Ok(value)
    }

    pub fn combined_template(&self, selected: &[u64], template: &str) -> Result<String, String> {
        if template.len() > TEMPLATE_BYTE_LIMIT {
            return Err("模板超过16 KiB，请缩短后重试".into());
        }
        if selected.iter().any(|id| {
            self.entries.iter().any(|entry| {
                entry.id == *id && (entry.image.is_some() || !entry.file_references.is_empty())
            })
        }) {
            return Err("图片或文件引用不能参加文本组合".into());
        }
        if selected.is_empty() {
            return Err("先选择历史条目".into());
        }
        let parts = parse_template(template)?;
        if !parts
            .iter()
            .any(|part| matches!(part, TemplatePart::Field(TemplateField::Text)))
        {
            return Err("模板必须包含 {{text}}，避免丢掉所选正文".into());
        }

        let mut output = String::new();
        for (index, id) in selected.iter().enumerate() {
            let entry = self
                .entries
                .iter()
                .find(|entry| {
                    entry.id == *id
                        && entry.image.is_none()
                        && entry.file_references.is_empty()
                        && !entry.text.is_empty()
                })
                .ok_or("选择的历史已失效，请重新选择".to_string())?;
            let index_text = (index + 1).to_string();
            let labels = entry.labels.join(", ");
            let row_bytes = parts.iter().try_fold(0usize, |sum, part| {
                let length = template_part_value(part, entry, &index_text, &labels).len();
                sum.checked_add(length)
                    .filter(|total| *total <= TEXT_LIMIT)
                    .ok_or_else(|| "模板单条结果超过1 MiB".to_string())
            })?;
            let separator = usize::from(!output.is_empty());
            if output
                .len()
                .saturating_add(separator)
                .saturating_add(row_bytes)
                > TEXT_LIMIT
            {
                return Err("模板组合结果超过1 MiB，请减少选择或缩短模板".into());
            }
            if separator != 0 {
                output.push('\n');
            }
            output.reserve(row_bytes);
            for part in &parts {
                output.push_str(template_part_value(part, entry, &index_text, &labels));
            }
        }
        Ok(output)
    }
}

fn validate_rtf_payload(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > RTF_LIMIT || !bytes.starts_with(b"{\\rtf") {
        return Err("RTF格式无效或超过2 MiB".into());
    }
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'));
    if end.is_none_or(|index| bytes[index] != b'}') {
        return Err("RTF内容缺少结束大括号".into());
    }
    Ok(())
}

mod rtf_base64 {
    use super::{RTF_BASE64_LIMIT, validate_rtf_payload};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S>(value: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(bytes) => serializer.serialize_some(&STANDARD.encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = Option::<String>::deserialize(deserializer)?;
        encoded
            .map(|encoded| {
                if encoded.len() > RTF_BASE64_LIMIT {
                    return Err(D::Error::custom("剪贴板RTF编码超过上限"));
                }
                let bytes = STANDARD.decode(encoded).map_err(D::Error::custom)?;
                validate_rtf_payload(&bytes).map_err(D::Error::custom)?;
                Ok(bytes)
            })
            .transpose()
    }
}

fn parse_template(template: &str) -> Result<Vec<TemplatePart<'_>>, String> {
    let mut parts = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = template[cursor..].find("{{") {
        let open = cursor + offset;
        if open > cursor {
            parts.push(TemplatePart::Literal(&template[cursor..open]));
        }
        let name_start = open + 2;
        let close = template[name_start..]
            .find("}}")
            .map(|offset| name_start + offset)
            .ok_or_else(|| "模板中有未闭合的 {{ 占位符".to_string())?;
        let field = match &template[name_start..close] {
            "index" => TemplateField::Index,
            "text" => TemplateField::Text,
            "source" => TemplateField::Source,
            "time" => TemplateField::Time,
            "labels" => TemplateField::Labels,
            other => return Err(format!("模板变量 {{{{{other}}}}} 不受支持")),
        };
        parts.push(TemplatePart::Field(field));
        cursor = close + 2;
    }
    if cursor < template.len() {
        parts.push(TemplatePart::Literal(&template[cursor..]));
    }
    if parts.is_empty() {
        return Err("模板不能为空".into());
    }
    Ok(parts)
}

fn template_part_value<'a>(
    part: &'a TemplatePart<'a>,
    entry: &'a Entry,
    index: &'a str,
    labels: &'a str,
) -> &'a str {
    match part {
        TemplatePart::Literal(value) => value,
        TemplatePart::Field(TemplateField::Index) => index,
        TemplatePart::Field(TemplateField::Text) => &entry.text,
        TemplatePart::Field(TemplateField::Source) => &entry.source,
        TemplatePart::Field(TemplateField::Time) => &entry.time,
        TemplatePart::Field(TemplateField::Labels) => labels,
    }
}

fn normalize_label(label: &str) -> Result<String, String> {
    let label = label.trim();
    if label.is_empty()
        || label.len() > LABEL_BYTE_LIMIT
        || label.chars().count() > LABEL_CHAR_LIMIT
        || label.chars().any(char::is_control)
    {
        return Err(format!(
            "标签不能为空、不能含控制字符，且最多{LABEL_CHAR_LIMIT}个字符/{LABEL_BYTE_LIMIT}字节"
        ));
    }
    Ok(label.to_string())
}

fn normalize_file_references(paths: Vec<String>) -> Result<Vec<String>, String> {
    if paths.len() > FILE_REFERENCE_LIMIT {
        return Err(format!(
            "单条剪贴板最多包含{FILE_REFERENCE_LIMIT}个文件引用"
        ));
    }
    let mut unique = Vec::with_capacity(paths.len());
    let mut keys = std::collections::HashSet::new();
    let mut total = 0usize;
    for path in paths {
        if path.is_empty()
            || path.len() > FILE_REFERENCE_PATH_BYTE_LIMIT
            || path.chars().any(char::is_control)
        {
            return Err("文件引用路径为空、过长或含控制字符".into());
        }
        if !keys.insert(path.to_lowercase()) {
            continue;
        }
        total = total.saturating_add(path.len());
        if total > FILE_REFERENCE_BYTE_LIMIT {
            return Err("文件引用路径总量超过1 MiB".into());
        }
        unique.push(path);
    }
    Ok(unique)
}

fn same_file_references(left: &[String], right: &[String]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut left = left
        .iter()
        .map(|path| path.to_lowercase())
        .collect::<Vec<_>>();
    let mut right = right
        .iter()
        .map(|path| path.to_lowercase())
        .collect::<Vec<_>>();
    left.sort();
    right.sort();
    left == right
}

#[cfg(all(windows, feature = "ui-preview"))]
pub use native::isolated_fixture;
#[cfg(all(windows, feature = "ui-preview"))]
pub use storage::media_benchmark;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_images_deduplicate_preserve_pins_and_do_not_enter_text_composition() {
        let picture =
            std::sync::Arc::new(Picture::from_image(image::DynamicImage::new_rgba8(8, 5)).unwrap());
        let mut h = History::default();
        h.insert("fixture text".into(), "fixture".into()).unwrap();
        let text = h.entries[0].id;
        h.insert_image(picture.clone(), "first".into()).unwrap();
        let id = h.entries[0].id;
        h.entries[0].pinned = true;
        h.insert_image(picture.clone(), "second".into()).unwrap();
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.entries[0].id, id);
        assert!(h.entries[0].pinned && h.entries[0].text.is_empty());
        assert!(std::sync::Arc::ptr_eq(
            h.entries[0].image.as_ref().unwrap(),
            &picture
        ));
        assert_eq!(h.bytes(), picture.cost() + 12);
        assert!(h.combined(&[id, text], 0).is_err());
        assert_eq!(h.combined(&[text], 0).unwrap(), "fixture text");
        h.retention_days = Some(1);
        h.entries[0].last_captured_utc = Some(1);
        h.entries[1].last_captured_utc = Some(2_000_000_000);
        assert_eq!(h.expire(2_000_000_000), 0);
    }

    #[test]
    fn file_reference_history_is_bounded_deduplicated_and_separate_from_text() {
        let mut history = History::default();
        history
            .insert_files(
                vec![r"C:\Docs\First.pdf".into(), r"C:\Docs\Second.docx".into()],
                "explorer.exe".into(),
            )
            .unwrap();
        let first_id = history.entries[0].id;
        assert_eq!(history.entries[0].file_references.len(), 2);
        assert!(history.entries[0].text.is_empty());
        assert!(history.combined(&[first_id], 0).is_err());
        history
            .insert_files(
                vec![r"c:\docs\second.docx".into(), r"c:\docs\first.pdf".into()],
                "other-app.exe".into(),
            )
            .unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].id, first_id);
        assert_eq!(history.entries[0].source, "other-app.exe");
        assert!(
            history
                .insert_files(vec![String::new()], "fixture".into())
                .is_err()
        );
        assert!(
            history
                .insert_files(
                    vec!["x".repeat(FILE_REFERENCE_PATH_BYTE_LIMIT + 1)],
                    "fixture".into(),
                )
                .is_err()
        );
        assert!(
            history
                .insert_files(vec!["bad\npath".into()], "fixture".into())
                .is_err()
        );
        assert!(
            history
                .insert_files(
                    vec![r"C:\x".into(); FILE_REFERENCE_LIMIT + 1],
                    "fixture".into(),
                )
                .is_err()
        );
    }

    #[test]
    fn rich_html_is_bounded_deduplicated_and_keeps_plain_text_fallback() {
        let mut history = History::default();
        let html = "<p><strong>hello</strong></p>".to_string();
        history
            .insert_rich_text("hello".into(), html.clone(), "editor.exe".into())
            .unwrap();
        let id = history.entries[0].id;
        assert_eq!(history.entries[0].text, "hello");
        assert_eq!(history.entries[0].rich_html.as_deref(), Some(html.as_str()));
        assert_eq!(history.combined(&[id], 0).unwrap(), "hello");
        history
            .insert_rich_text("hello".into(), html.clone(), "other.exe".into())
            .unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].id, id);
        assert_eq!(history.entries[0].source, "other.exe");

        history
            .insert_rich_text("hello".into(), "<b>hello</b>".into(), "other.exe".into())
            .unwrap();
        assert_eq!(history.entries.len(), 2, "distinct formatting is retained");
        assert_eq!(history.bytes(), 5 + html.len() + 5 + "<b>hello</b>".len());
        assert!(
            history
                .insert_rich_text("".into(), "<p>\0</p>".into(), "fixture".into())
                .is_err()
        );
        assert!(
            history
                .insert_rich_text("".into(), "x".repeat(HTML_LIMIT + 1), "fixture".into())
                .is_err()
        );
    }

    #[test]
    fn rich_rtf_is_bounded_deduplicated_and_keeps_exact_bytes() {
        let rtf = br#"{\rtf1\ansi Hello {\b world}}"#.to_vec();
        let mut history = History::default();
        history
            .insert_rich_rtf("Hello world".into(), rtf.clone(), "writer.exe".into())
            .unwrap();
        assert_eq!(history.entries[0].rich_rtf.as_deref(), Some(rtf.as_slice()));
        assert_eq!(history.entries[0].text, "Hello world");
        history
            .insert_rich_rtf("Hello world".into(), rtf.clone(), "other.exe".into())
            .unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].source, "other.exe");

        let json = serde_json::to_string(&history.entries[0]).unwrap();
        assert!(json.contains("rich_rtf"));
        assert!(!json.contains("\\\\rtf1"), "binary RTF is base64 encoded");
        let restored: Entry = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.rich_rtf.as_deref(), Some(rtf.as_slice()));

        assert!(
            history
                .insert_rich_rtf("".into(), b"not rtf".to_vec(), "x".into())
                .is_err()
        );
        let mut oversized = br#"{\rtf1 "#.to_vec();
        oversized.resize(RTF_LIMIT + 1, b'x');
        assert!(
            history
                .insert_rich_rtf("".into(), oversized, "x".into())
                .is_err()
        );
    }

    #[test]
    fn html_and_rtf_from_one_clipboard_entry_remain_together() {
        let mut history = History::default();
        let html = "<p>Hello <strong>world</strong></p>";
        let rtf = br#"{\rtf1\ansi Hello {\b world}}"#.to_vec();
        history
            .insert_rich_formats(
                "Hello world".into(),
                html.into(),
                rtf.clone(),
                "editor.exe".into(),
            )
            .unwrap();
        let id = history.entries[0].id;
        history
            .insert_rich_formats(
                "Hello world".into(),
                html.into(),
                rtf.clone(),
                "other.exe".into(),
            )
            .unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].id, id);
        assert_eq!(history.entries[0].rich_html.as_deref(), Some(html));
        assert_eq!(history.entries[0].rich_rtf.as_deref(), Some(rtf.as_slice()));

        let encoded = serde_json::to_string(&history).unwrap();
        let restored: History = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.entries[0].rich_html.as_deref(), Some(html));
        assert_eq!(
            restored.entries[0].rich_rtf.as_deref(),
            Some(rtf.as_slice())
        );
    }

    #[test]
    fn labels_trim_deduplicate_bound_and_survive_recapture() {
        let mut history = History::default();
        history.insert("fixture".into(), "source".into()).unwrap();
        let id = history.entries[0].id;
        assert!(history.add_label(id, "  Project  ").unwrap());
        assert!(!history.add_label(id, "project").unwrap());
        assert_eq!(history.entries[0].labels, vec!["Project".to_string()]);
        assert!(history.bytes() > "fixture".len());
        history.insert("fixture".into(), "recopied".into()).unwrap();
        assert_eq!(history.entries[0].labels, vec!["Project".to_string()]);
        assert!(history.remove_label(id, "PROJECT"));
        assert!(history.entries[0].labels.is_empty());
        assert!(!history.remove_label(id, "missing"));
        assert!(history.add_label(id, "\n").is_err());
        assert!(
            history
                .add_label(id, &"界".repeat(LABEL_CHAR_LIMIT + 1))
                .is_err()
        );
        assert!(
            history
                .add_label(id, &"界".repeat(LABEL_BYTE_LIMIT / 3 + 1))
                .is_err()
        );
        for n in 0..LABEL_LIMIT {
            assert!(history.add_label(id, &format!("tag-{n}")).unwrap());
        }
        assert!(history.add_label(id, "ninth").is_err());
    }

    #[test]
    fn batch_label_preflights_all_entries_and_adds_atomically() {
        let mut history = History::default();
        history.insert("one".into(), "fixture".into()).unwrap();
        let one = history.entries[0].id;
        history.insert("two".into(), "fixture".into()).unwrap();
        let two = history.entries[0].id;
        history.add_label(one, "Shared").unwrap();
        for n in 0..LABEL_LIMIT {
            history.add_label(two, &format!("tag-{n}")).unwrap();
        }

        assert!(history.add_label_to_many(&[one, two], " batch ").is_err());
        assert!(
            history
                .entries
                .iter()
                .all(|entry| !entry.labels.iter().any(|label| label == "batch"))
        );
        assert!(
            history.entries[1]
                .labels
                .iter()
                .any(|label| label == "Shared")
        );

        history.remove_label(two, "tag-7");
        assert_eq!(
            history
                .add_label_to_many(&[one, two, two], "Shared")
                .unwrap(),
            1
        );
        assert!(
            history.entries[0]
                .labels
                .iter()
                .any(|label| label == "Shared")
        );
        assert_eq!(history.entries[0].labels.len(), 8);
        assert_eq!(history.add_label_to_many(&[one, two], "shared").unwrap(), 0);
    }

    #[test]
    fn legacy_entries_without_labels_deserialize_as_unlabeled() {
        let mut history = History::default();
        history.insert("legacy".into(), "fixture".into()).unwrap();
        let mut value = serde_json::to_value(&history.entries[0]).unwrap();
        value.as_object_mut().unwrap().remove("labels");
        let entry: Entry = serde_json::from_value(value).unwrap();
        assert!(entry.labels.is_empty());
    }
    #[test]
    fn pinned_image_budget_refuses_incoming_without_discarding_existing_work() {
        let mut h = History::default();
        let image = std::sync::Arc::new(
            Picture::from_image(image::DynamicImage::new_rgba8(2000, 2000)).unwrap(),
        );
        h.insert_image(image, "fixture".into()).unwrap();
        h.entries[0].pinned = true;
        for i in 0..15 {
            h.insert(
                format!("{i:02}{}", "x".repeat(TEXT_LIMIT - 2)),
                "fixture".into(),
            )
            .unwrap();
            h.entries[0].pinned = true;
        }
        let ids = h.entries.iter().map(|e| e.id).collect::<Vec<_>>();
        let other = std::sync::Arc::new(
            Picture::from_image(image::DynamicImage::new_rgba8(1800, 1800)).unwrap(),
        );
        assert!(h.insert_image(other, "fixture".into()).is_err());
        assert_eq!(h.entries.iter().map(|e| e.id).collect::<Vec<_>>(), ids);
        assert!(h.bytes() <= BYTE_LIMIT);
    }
    #[test]
    fn retention_protects_pins_unknown_and_future_times_at_exact_boundary() {
        let mut h = History::default();
        let now = 2_000_000_000;
        for text in ["old", "boundary", "fresh", "pinned", "unknown", "future"] {
            h.insert(text.into(), "fixture".into()).unwrap();
            let entry = &mut h.entries[0];
            entry.first_captured_utc = None;
            entry.last_captured_utc = match text {
                "old" | "pinned" => Some(now - 2 * 86400),
                "boundary" => Some(now - 86400),
                "fresh" => Some(now - 86400 + 1),
                "future" => Some(now + 86400),
                _ => None,
            };
            entry.pinned = text == "pinned";
        }
        assert_eq!(h.expire(now), 0);
        assert_eq!(h.expired_count(0, now), 0);
        assert_eq!(h.expired_count(3651, now), 0);
        assert_eq!(h.expired_count(1, now), 2);
        h.retention_days = Some(1);
        assert_eq!(h.expire(now), 2);
        assert_eq!(
            h.entries
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            vec!["future", "unknown", "pinned", "fresh"]
        );
        assert_eq!(h.expire(now), 0);
        assert_eq!(h.expire(i64::MIN), 0);
    }
    #[test]
    fn recopy_preserves_first_capture_and_unknown_legacy_origin() {
        let mut h = History::default();
        h.insert("fixture".into(), "first".into()).unwrap();
        let first = h.entries[0].first_captured_utc;
        let future = chrono::Utc::now().timestamp() + 3600;
        h.entries[0].last_captured_utc = Some(future);
        h.insert("fixture".into(), "second".into()).unwrap();
        assert_eq!(h.entries[0].first_captured_utc, first);
        assert_eq!(h.entries[0].last_captured_utc, Some(future));
        h.entries[0].first_captured_utc = None;
        h.entries[0].last_captured_utc = None;
        h.insert("fixture".into(), "third".into()).unwrap();
        assert!(h.entries[0].first_captured_utc.is_none());
        assert!(h.entries[0].last_captured_utc.is_some());
    }
    #[test]
    fn byte_eviction_and_formula_export_preserve_original_text() {
        let mut h = History::default();
        for i in 0..33 {
            h.insert(
                format!("{i:02}{}", "x".repeat(TEXT_LIMIT - 2)),
                "fixture".into(),
            )
            .unwrap();
        }
        assert_eq!(h.bytes(), BYTE_LIMIT);
        assert_eq!(h.entries.len(), 32);
        assert!(!h.entries.iter().any(|e| e.text.starts_with("00")));
        let ids = h.entries.iter().take(2).map(|e| e.id).collect::<Vec<_>>();
        assert!(h.combined(&ids, 2).is_err());
        h.entries.clear();
        h.insert("=1+2".into(), "fixture".into()).unwrap();
        let csv = h.combined(&[h.entries[0].id], 3).unwrap();
        let row = csv::Reader::from_reader(csv.as_bytes())
            .records()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(&row[0], "'=1+2");
        assert_eq!(h.entries[0].text, "=1+2");
    }
    #[test]
    fn dedup_retains_pin_and_id_updates_source_and_recency() {
        let mut h = History::default();
        h.insert("甲".into(), "A".into()).unwrap();
        h.entries[0].pinned = true;
        let id = h.entries[0].id;
        h.insert("乙".into(), "B".into()).unwrap();
        h.insert("甲".into(), "C".into()).unwrap();
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.entries[0].id, id);
        assert!(h.entries[0].pinned);
        assert_eq!(h.entries[0].source, "C");
    }
    #[test]
    fn limits_preserve_pins_and_reject_without_losing_content() {
        let mut h = History::default();
        for i in 0..ITEM_LIMIT {
            h.insert(i.to_string(), "fixture".into()).unwrap();
        }
        h.entries.last_mut().unwrap().pinned = true;
        h.insert("new".into(), "fixture".into()).unwrap();
        assert_eq!(h.entries.len(), ITEM_LIMIT);
        assert!(h.entries.iter().any(|e| e.text == "0"));
        for e in &mut h.entries {
            e.pinned = true;
        }
        let before = h.entries.iter().map(|e| e.id).collect::<Vec<_>>();
        assert!(h.insert("full".into(), "fixture".into()).is_err());
        assert_eq!(before, h.entries.iter().map(|e| e.id).collect::<Vec<_>>());
        assert!(
            h.insert("x".repeat(TEXT_LIMIT + 1), "fixture".into())
                .is_err()
        );
        h.insert("new".into(), "other".into()).unwrap();
        assert_eq!(h.entries.len(), ITEM_LIMIT);
    }
    #[test]
    fn combination_preserves_selection_order_and_roundtrips_multiline() {
        let mut h = History::default();
        h.insert("a,\"b\"\n甲".into(), "fixture".into()).unwrap();
        let a = h.entries[0].id;
        h.insert("乙".into(), "fixture".into()).unwrap();
        let b = h.entries[0].id;
        let json: Vec<String> = serde_json::from_str(&h.combined(&[a, b], 2).unwrap()).unwrap();
        assert_eq!(json, vec!["a,\"b\"\n甲", "乙"]);
        let csv = h.combined(&[a, b], 3).unwrap();
        let rows = csv::Reader::from_reader(csv.as_bytes())
            .records()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(&rows[0][0], "a,\"b\"\n甲");
        assert!(h.combined(&[999], 0).is_err());
        assert!(h.combined(&[], 0).is_err());
    }

    #[test]
    fn structured_json_records_keep_selected_metadata_without_internal_ids() {
        let mut history = History::default();
        history
            .insert("第一条\n续行".into(), "browser.exe".into())
            .unwrap();
        let first = history.entries[0].id;
        history.entries[0].time = "2026-10-17 10:00:00".into();
        history.add_label(first, "资料").unwrap();
        history
            .insert("第二条".into(), "editor.exe".into())
            .unwrap();
        let second = history.entries[0].id;
        history.entries[0].time = "2026-10-17 10:01:00".into();
        history.add_label(second, "待整理").unwrap();

        let output = history.combined(&[second, first], 6).unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&output).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["order"], 1);
        assert_eq!(rows[0]["text"], "第二条");
        assert_eq!(rows[0]["source"], "editor.exe");
        assert_eq!(rows[0]["time"], "2026-10-17 10:01:00");
        assert_eq!(rows[0]["labels"], serde_json::json!(["待整理"]));
        assert!(rows[0].get("id").is_none());
        assert_eq!(rows[1]["order"], 2);
        assert_eq!(rows[1]["text"], "第一条\n续行");
        assert_eq!(rows[1]["source"], "browser.exe");
        assert_eq!(rows[1]["labels"], serde_json::json!(["资料"]));
    }

    #[test]
    fn structured_json_records_obey_output_budget() {
        let mut history = History::default();
        history.insert("body".into(), "browser.exe".into()).unwrap();
        let id = history.entries[0].id;
        history.entries[0].source = "s".repeat(TEXT_LIMIT);
        assert!(history.combined(&[id], 6).is_err());
    }

    #[test]
    fn markdown_list_preserves_selection_and_multiline_structure() {
        let mut history = History::default();
        history
            .insert("第一行\r\n第二行\n".into(), "fixture".into())
            .unwrap();
        let first = history.entries[0].id;
        history.insert("第二项".into(), "fixture".into()).unwrap();
        let second = history.entries[0].id;

        assert_eq!(
            history.combined(&[first, second], 4).unwrap(),
            "- 第一行\n  第二行\n  \n- 第二项"
        );
        assert_eq!(
            history.combined(&[second, first], 4).unwrap(),
            "- 第二项\n- 第一行\n  第二行\n  "
        );
    }

    #[test]
    fn template_expands_selected_fields_without_reparsing_inserted_text() {
        let mut history = History::default();
        history
            .insert(
                "browser {{source}}\nsecond line".into(),
                "editor.exe".into(),
            )
            .unwrap();
        let first = history.entries[0].id;
        history.entries[0].time = "2026-10-14 09:30:00".into();
        history.add_label(first, "Notes").unwrap();
        history
            .insert("other".into(), "browser.exe".into())
            .unwrap();
        let second = history.entries[0].id;
        history.entries[0].time = "2026-10-14 09:31:00".into();
        history.add_label(second, "Web").unwrap();

        let rendered = history
            .combined_template(
                &[second, first],
                "{{index}} | {{source}} | {{labels}} | {{time}}\n{{text}}",
            )
            .unwrap();
        assert_eq!(
            rendered,
            "1 | browser.exe | Web | 2026-10-14 09:31:00\nother\n2 | editor.exe | Notes | 2026-10-14 09:30:00\nbrowser {{source}}\nsecond line"
        );
    }

    #[test]
    fn template_rejects_missing_text_unknown_fields_and_unclosed_tokens() {
        let mut history = History::default();
        history.insert("body".into(), "editor.exe".into()).unwrap();
        let id = history.entries[0].id;
        assert!(history.combined_template(&[id], "{{source}}").is_err());
        assert!(
            history
                .combined_template(&[id], "{{text}} {{unknown}}")
                .is_err()
        );
        assert!(history.combined_template(&[id], "{{text").is_err());
        assert!(history.combined_template(&[id], "").is_err());
        assert!(
            history
                .combined_template(&[id], &"x".repeat(TEMPLATE_BYTE_LIMIT + 1))
                .is_err()
        );
    }

    #[test]
    fn template_checks_expanded_size_before_building_the_output() {
        let mut history = History::default();
        history
            .insert("x".repeat(TEXT_LIMIT / 2), "editor.exe".into())
            .unwrap();
        let id = history.entries[0].id;
        assert!(
            history
                .combined_template(&[id], "{{text}}{{text}}{{text}}")
                .unwrap_err()
                .contains("1 MiB")
        );
    }
}
