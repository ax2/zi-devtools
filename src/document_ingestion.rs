//! Local, explicit document parsing and bounded chunk previews. No content is persisted.
use anyhow::{Context, Result, bail, ensure};
use eframe::egui;
use image::{DynamicImage, ImageFormat, ImageReader, Limits};
use quick_xml::{Reader, events::Event};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};

use crate::knowledge_sources::{FileVersion, Kind, Source};

const MAX_FILE: u64 = 16 * 1024 * 1024;
const MAX_TEXT: usize = 4 * 1024 * 1024;
const CHUNK_CHARS: usize = 800;
const OVERLAP_CHARS: usize = 100;
const MAX_CHUNKS: usize = 2_000;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;

#[derive(Clone, Debug, Serialize)]
pub struct Chunk {
    pub location: String,
    pub ordinal: usize,
    pub text: String,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub source: String,
    pub relative: String,
    pub file_sha256: String,
    pub chunks: Vec<Chunk>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn utf8(bytes: &[u8]) -> Result<&str> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    std::str::from_utf8(bytes).context("文件不是 UTF-8 文本；请先用文件编码转换工具处理")
}

fn push_section(
    sections: &mut Vec<(String, String)>,
    location: String,
    text: String,
) -> Result<()> {
    if !text.trim().is_empty() {
        ensure!(sections.len() < MAX_CHUNKS, "段落超过 2000 个");
        let used: usize = sections.iter().map(|(_, s)| s.len()).sum();
        ensure!(
            used.saturating_add(text.len()) <= MAX_TEXT,
            "提取正文超过 4 MiB"
        );
        sections.push((location, text.trim().to_owned()));
    }
    Ok(())
}

fn parse_docx(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).context("Word 文件 ZIP 结构无效")?;
    ensure!(archive.len() <= 2_000, "Word 文件条目过多");
    let file = archive
        .by_name("word/document.xml")
        .context("缺少 word/document.xml")?;
    ensure!(
        file.size() <= MAX_TEXT as u64 * 2,
        "Word 正文 XML 超过 8 MiB"
    );
    let mut xml = Vec::new();
    file.take((MAX_TEXT * 2 + 1) as u64).read_to_end(&mut xml)?;
    ensure!(xml.len() <= MAX_TEXT * 2, "Word 正文 XML 超过 8 MiB");
    let mut reader = Reader::from_reader(xml.as_slice());
    reader.config_mut().trim_text(false);
    let mut sections = Vec::new();
    let mut paragraph = String::new();
    let mut number = 0;
    let mut in_text = false;
    loop {
        match reader.read_event().context("Word 正文 XML 格式无效")? {
            Event::Start(tag) if tag.name().as_ref() == b"w:t" => in_text = true,
            Event::End(tag) if tag.name().as_ref() == b"w:t" => in_text = false,
            Event::Text(value) if in_text => {
                paragraph.push_str(&quick_xml::escape::unescape(&value.decode()?)?)
            }
            Event::GeneralRef(value) if in_text => {
                let encoded = format!("&{};", value.decode()?);
                paragraph.push_str(&quick_xml::escape::unescape(&encoded)?);
            }
            Event::Empty(tag) if tag.name().as_ref() == b"w:tab" => paragraph.push('\t'),
            Event::Empty(tag) if tag.name().as_ref() == b"w:br" => paragraph.push('\n'),
            Event::End(tag) if tag.name().as_ref() == b"w:p" => {
                number += 1;
                push_section(
                    &mut sections,
                    format!("段落 {number}"),
                    std::mem::take(&mut paragraph),
                )?;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(sections)
}

fn parse_pdf_with_recognizer(
    bytes: &[u8],
    mut recognize: impl FnMut(&[u8], u32, &[(u32, u32)]) -> Result<Vec<(u32, String)>>,
) -> Result<Vec<(String, String)>> {
    let doc = lopdf::Document::load_mem(bytes).context("PDF 格式无效")?;
    ensure!(!doc.is_encrypted(), "PDF 已加密，无法解析");
    let pages = doc.get_pages();
    ensure!(pages.len() <= 200, "PDF 超过 200 页，请拆分后导入");
    let expected_page_count = u32::try_from(pages.len()).context("PDF 页数超出平台范围")?;
    let mut extracted = Vec::with_capacity(pages.len());
    let mut scanned = Vec::new();
    for (index, page) in pages.keys().enumerate() {
        let text = doc
            .extract_text(&[*page])
            .with_context(|| format!("PDF 第 {page} 页解析失败"))?;
        if text.trim().is_empty() {
            scanned.push((*page, index as u32));
        }
        extracted.push((*page, text));
    }
    if scanned.len() > crate::clipboard::MAX_PDF_OCR_PAGES {
        bail!(
            "PDF 有 {} 个扫描或空白页面，超过本机 OCR 的 50 页上限，请拆分后导入",
            scanned.len()
        );
    }
    let recognized = if scanned.is_empty() {
        Vec::new()
    } else {
        recognize(bytes, expected_page_count, &scanned)?
    };
    let recognized: std::collections::HashMap<_, _> = recognized.into_iter().collect();
    ensure!(
        recognized.len() == scanned.len()
            && scanned
                .iter()
                .all(|(page, _)| recognized.contains_key(page)),
        "PDF OCR 未返回全部扫描页面的识别结果"
    );
    let mut sections = Vec::new();
    for (page, text) in extracted {
        let text = if text.trim().is_empty() {
            recognized.get(&page).cloned().unwrap_or_default()
        } else {
            text
        };
        push_section(&mut sections, format!("第 {page} 页"), text)?;
    }
    if sections.is_empty() {
        bail!("PDF 未识别到文字；请确认扫描件清晰且已安装对应的 Windows OCR 语言");
    }
    Ok(sections)
}

fn parse_pdf(bytes: &[u8], language_tag: Option<&str>) -> Result<Vec<(String, String)>> {
    parse_pdf_with_recognizer(bytes, |bytes, expected_page_count, pages| {
        #[cfg(windows)]
        {
            crate::clipboard::recognize_pdf_pages_with_language(
                bytes,
                expected_page_count,
                pages,
                language_tag,
            )
            .map_err(anyhow::Error::msg)
        }
        #[cfg(not(windows))]
        {
            let _ = (bytes, expected_page_count, pages, language_tag);
            bail!("扫描 PDF OCR 目前仅支持 Windows 内置 OCR")
        }
    })
}

fn decode_ocr_image(bytes: &[u8]) -> Result<DynamicImage> {
    ensure!(bytes.len() as u64 <= MAX_FILE, "图片超过 16 MiB");
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    ensure!(
        matches!(
            reader.format(),
            Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
        ),
        "本机文档 OCR 仅支持 PNG、JPEG 和 WebP 图片"
    );
    let mut limits = Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits.clone());
    let (width, height) = reader.into_dimensions()?;
    ensure!(
        width > 0
            && height > 0
            && u64::from(width).saturating_mul(u64::from(height)) <= MAX_IMAGE_PIXELS,
        "图片尺寸超过 1600 万像素"
    );
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    reader.limits(limits);
    reader.decode().context("图片解码失败")
}

fn parse_image_with(
    bytes: &[u8],
    language_tag: Option<&str>,
    recognize: impl FnOnce(&DynamicImage, Option<&str>) -> Result<String>,
) -> Result<Vec<(String, String)>> {
    let image = decode_ocr_image(bytes)?;
    let text = recognize(&image, language_tag).context("Windows 本机 OCR 未能处理所选图片")?;
    ensure!(text.len() <= MAX_TEXT, "图片 OCR 正文超过 4 MiB");
    let mut sections = Vec::new();
    push_section(&mut sections, "图片 OCR".into(), text)?;
    ensure!(!sections.is_empty(), "图片中没有识别到文字");
    Ok(sections)
}

fn parse_image_with_language(
    bytes: &[u8],
    language_tag: Option<&str>,
) -> Result<Vec<(String, String)>> {
    #[cfg(windows)]
    {
        parse_image_with(bytes, language_tag, |image, language| {
            crate::clipboard::recognize_image_with_language(image, language)
                .map_err(anyhow::Error::msg)
        })
    }
    #[cfg(not(windows))]
    {
        let _ = language_tag;
        let _ = decode_ocr_image(bytes)?;
        bail!("图片 OCR 目前仅支持 Windows 内置 OCR")
    }
}

/// Extract bounded, location-labelled text for an explicitly selected model-chat attachment.
pub(crate) fn extract_chat_attachment(
    bytes: &[u8],
    extension: &str,
    max_text_bytes: usize,
) -> Result<String> {
    ensure!(bytes.len() as u64 <= MAX_FILE, "文档附件超过 16 MiB");
    ensure!(
        max_text_bytes > 0 && max_text_bytes <= MAX_TEXT,
        "文档摘录预算无效"
    );
    let sections = match extension {
        "pdf" => parse_pdf(bytes, None)?,
        "docx" => parse_docx(bytes)?,
        _ => bail!("仅支持 PDF 与 DOCX 文档摘录"),
    };
    let mut text = String::new();
    for (location, section) in sections {
        let part = format!("[{location}]\n{section}\n\n");
        ensure!(
            text.len().saturating_add(part.len()) <= max_text_bytes,
            "文档摘录超过单附件预算，请拆分文档"
        );
        text.push_str(&part);
    }
    ensure!(!text.trim().is_empty(), "文档未提取到可发送正文");
    Ok(text.trim_end().to_owned())
}

fn parse_html(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    let html = utf8(bytes)?;
    let doc = scraper::Html::parse_document(html);
    let selector = scraper::Selector::parse("body").expect("static selector");
    let mut sections = Vec::new();
    let text = doc
        .select(&selector)
        .next()
        .map(|body| {
            body.descendants()
                .filter_map(|node| {
                    let text = node.value().as_text()?;
                    let hidden =
                        node.ancestors()
                            .filter_map(scraper::ElementRef::wrap)
                            .any(|parent| {
                                matches!(
                                    parent.value().name(),
                                    "script" | "style" | "noscript" | "template"
                                )
                            });
                    (!hidden).then_some(text.to_string())
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    push_section(&mut sections, "网页正文".into(), text)?;
    Ok(sections)
}

fn parse_text(bytes: &[u8], markdown: bool) -> Result<Vec<(String, String)>> {
    let text = utf8(bytes)?;
    let mut sections = Vec::new();
    if markdown {
        use pulldown_cmark::{Event as MdEvent, Parser};
        let mut current = String::new();
        let mut number = 0;
        for event in Parser::new(text) {
            match event {
                MdEvent::Text(s) | MdEvent::Code(s) => current.push_str(&s),
                MdEvent::SoftBreak | MdEvent::HardBreak => current.push('\n'),
                MdEvent::End(
                    pulldown_cmark::TagEnd::Paragraph | pulldown_cmark::TagEnd::Heading(_),
                ) => {
                    number += 1;
                    push_section(
                        &mut sections,
                        format!("段落 {number}"),
                        std::mem::take(&mut current),
                    )?;
                }
                _ => {}
            }
        }
        push_section(&mut sections, format!("段落 {}", number + 1), current)?;
    } else {
        for (i, paragraph) in text.split("\n\n").enumerate() {
            push_section(
                &mut sections,
                format!("段落 {}", i + 1),
                paragraph.to_owned(),
            )?;
        }
    }
    Ok(sections)
}

fn chunks_from(sections: Vec<(String, String)>) -> Result<Vec<Chunk>> {
    let mut chunks = Vec::new();
    for (location, text) in sections {
        let chars: Vec<char> = text.chars().collect();
        let mut start = 0;
        while start < chars.len() {
            ensure!(chunks.len() < MAX_CHUNKS, "分块超过 2000 个");
            let end = (start + CHUNK_CHARS).min(chars.len());
            let part: String = chars[start..end].iter().collect();
            chunks.push(Chunk {
                location: location.clone(),
                ordinal: chunks.len() + 1,
                sha256: digest(part.as_bytes()),
                text: part,
            });
            if end == chars.len() {
                break;
            }
            start = end - OVERLAP_CHARS;
        }
    }
    ensure!(!chunks.is_empty(), "未提取到可用正文");
    Ok(chunks)
}

fn verified_source(source: &Source, file: &FileVersion) -> Result<(PathBuf, Vec<u8>)> {
    let snapshot = source.snapshot.as_ref().context("知识源尚未扫描")?;
    ensure!(
        snapshot.files.iter().any(|item| item == file),
        "文件不在当前快照中"
    );
    let path = if source.kind == Kind::File {
        ensure!(
            source.path.file_name().and_then(|n| n.to_str()) == Some(&file.relative),
            "文件名与快照不符"
        );
        source.path.clone()
    } else {
        ensure!(
            !file.relative.starts_with('/')
                && !file.relative.split('/').any(|s| s == ".." || s.is_empty()),
            "快照路径无效"
        );
        source.path.join(&file.relative)
    };
    let meta = fs::symlink_metadata(&path).context("无法读取源文件")?;
    ensure!(
        meta.file_type().is_file() && meta.len() <= MAX_FILE,
        "源文件不是普通文件或超过 16 MiB"
    );
    let canonical = path.canonicalize()?;
    if source.kind == Kind::Directory {
        ensure!(canonical.starts_with(&source.path), "文件已移出知识源目录");
    } else {
        ensure!(canonical == source.path, "知识源文件路径已变化");
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)?
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_FILE, "源文件超过 16 MiB");
    let hash = digest(&bytes);
    ensure!(
        hash == file.sha256,
        "文件内容与扫描快照不一致，请先刷新知识源"
    );
    Ok((canonical, bytes))
}

pub fn verified_source_path(source: &Source, file: &FileVersion) -> Result<PathBuf> {
    verified_source(source, file).map(|(path, _)| path)
}

pub fn ingest(source: &Source, file: &FileVersion) -> Result<Preview> {
    ingest_with_ocr_language(source, file, None)
}

pub fn ingest_with_ocr_language(
    source: &Source,
    file: &FileVersion,
    language_tag: Option<&str>,
) -> Result<Preview> {
    let (_, bytes) = verified_source(source, file)?;
    let hash = file.sha256.clone();
    let ext = Path::new(&file.relative)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let sections = match ext.as_str() {
        "md" | "markdown" => parse_text(&bytes, true)?,
        "txt" => parse_text(&bytes, false)?,
        "html" | "htm" => parse_html(&bytes)?,
        "pdf" => parse_pdf(&bytes, language_tag)?,
        "docx" => parse_docx(&bytes)?,
        "png" | "jpg" | "jpeg" | "webp" => parse_image_with_language(&bytes, language_tag)?,
        _ => bail!("不支持的文档类型"),
    };
    Ok(Preview {
        source: source.name.clone(),
        relative: file.relative.clone(),
        file_sha256: hash,
        chunks: chunks_from(sections)?,
    })
}

/// Reconstructs parsed sections from overlapping chunks and returns a portable Markdown export.
pub fn export_markdown(preview: &Preview) -> Result<String> {
    ensure!(!preview.chunks.is_empty(), "没有可导出的解析内容");
    let mut output = format!(
        "# 文档解析结果\n\n- 知识源：{}\n- 文件：{}\n- 原文件 SHA-256：{}\n",
        preview.source, preview.relative, preview.file_sha256
    );
    let mut previous_location: Option<&str> = None;
    for chunk in &preview.chunks {
        let same_section = previous_location == Some(chunk.location.as_str());
        if !same_section {
            output.push_str(&format!("\n\n## {}\n\n", chunk.location));
            previous_location = Some(&chunk.location);
        }
        let body = if same_section {
            chunk.text.chars().skip(OVERLAP_CHARS).collect::<String>()
        } else {
            chunk.text.clone()
        };
        output.push_str(&body);
    }
    ensure!(
        output.len() <= MAX_TEXT + 256 * 1024,
        "Markdown 导出超过 4.25 MiB 上限"
    );
    Ok(output)
}

pub fn export_markdown_to_path(preview: &Preview, path: &Path) -> Result<PathBuf> {
    let markdown = export_markdown(preview)?;
    let not_cancelled = std::sync::atomic::AtomicBool::new(false);
    crate::local_files::save_new_moved(path, markdown.as_bytes(), &not_cancelled)
}

#[derive(Default)]
pub struct State {
    selected_source: String,
    selected_file: String,
    running: Option<Receiver<Result<Preview, String>>>,
    preview: Option<Preview>,
    selected_chunk: usize,
    chunk_page: usize,
    message: String,
    ocr_language_tag: Option<String>,
    #[cfg(windows)]
    ocr_languages: Vec<crate::clipboard::RecognizerLanguage>,
    #[cfg(windows)]
    ocr_languages_rx:
        Option<Receiver<std::result::Result<Vec<crate::clipboard::RecognizerLanguage>, String>>>,
}

impl State {
    pub(crate) fn ocr_language_tag(&self) -> Option<&str> {
        self.ocr_language_tag.as_deref()
    }

    pub(crate) fn background_active(&self) -> bool {
        self.running.is_some()
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, sources: &[Source]) {
        self.selected_source = sources.first().map(|s| s.id.clone()).unwrap_or_default();
        self.selected_file = sources
            .first()
            .and_then(|s| s.snapshot.as_ref())
            .and_then(|snapshot| snapshot.files.get(1).or_else(|| snapshot.files.first()))
            .map(|file| file.relative.clone())
            .unwrap_or_default();
        let passages = [
            (
                "段落 1",
                "文档解析先核对知识源快照中的 SHA-256，确认文件没有在扫描后被修改。整个过程只在本机执行，正文不会写入配置文件。",
            ),
            (
                "段落 2",
                "Markdown、TXT、HTML、PDF 和 DOCX 使用各自的解析规则。PDF 记录页码，其他格式记录段落，便于后续检索结果回到原文。",
            ),
            (
                "段落 3",
                "从 Markdown、PDF 和 Word 中提取正文后，按段落或页码保留来源位置。每个分块都有内容摘要，后续索引可以据此判断是否需要更新。",
            ),
        ];
        self.preview = Some(Preview {
            source: "技术笔记".into(),
            relative: "guides/rust.md".into(),
            file_sha256: "a3".repeat(32),
            chunks: passages
                .iter()
                .enumerate()
                .map(|(i, (location, text))| Chunk {
                    location: (*location).into(),
                    ordinal: i + 1,
                    text: (*text).into(),
                    sha256: digest(text.as_bytes()),
                })
                .collect(),
        });
        self.selected_chunk = 2;
        self.chunk_page = 0;
        self.message = "解析完成：3 个分块".into();
    }

    #[cfg(all(feature = "ui-preview", windows))]
    pub fn preview_ocr_fixture(&mut self, sources: &[Source]) {
        self.preview_fixture(sources);
        self.selected_file = "scans/example.png".into();
        self.preview = None;
        self.ocr_language_tag = Some("en-US".into());
        self.ocr_languages = vec![
            crate::clipboard::RecognizerLanguage {
                tag: "en-US".into(),
                display_name: "English (United States)".into(),
            },
            crate::clipboard::RecognizerLanguage {
                tag: "zh-CN".into(),
                display_name: "中文（简体，中国）".into(),
            },
        ];
        self.message = "示例：已选择 English (United States)，语言选项仅用于界面预览。".into();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, sources: &[Source]) {
        #[cfg(windows)]
        if let Some(rx) = &self.ocr_languages_rx {
            match rx.try_recv() {
                Ok(Ok(languages)) => {
                    self.ocr_languages = languages;
                    self.ocr_languages_rx = None;
                }
                Ok(Err(error)) => {
                    self.message = error;
                    self.ocr_languages_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.message = "读取本机 OCR 语言任务意外结束".into();
                    self.ocr_languages_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.running {
            match rx.try_recv() {
                Ok(Ok(preview)) => {
                    self.message = format!("解析完成：{} 个分块", preview.chunks.len());
                    self.preview = Some(preview);
                    self.selected_chunk = 0;
                    self.chunk_page = 0;
                    self.running = None;
                }
                Ok(Err(error)) => {
                    self.message = error;
                    self.preview = None;
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.message = "解析任务意外结束".into();
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        ui.heading("文档解析与分块");
        ui.label("从已扫描的本地知识源选择文件；正文只在内存中预览，不上传或持久化。解析前校验文件版本。");
        ui.add_space(8.0);
        let source = sources.iter().find(|s| s.id == self.selected_source);
        egui::ComboBox::from_label("知识源")
            .selected_text(source.map_or("选择知识源", |s| s.name.as_str()))
            .show_ui(ui, |ui| {
                for source in sources {
                    if ui
                        .selectable_value(
                            &mut self.selected_source,
                            source.id.clone(),
                            &source.name,
                        )
                        .changed()
                    {
                        self.selected_file.clear();
                        self.preview = None;
                    }
                }
            });
        let files = source
            .and_then(|s| s.snapshot.as_ref())
            .map(|s| s.files.as_slice())
            .unwrap_or(&[]);
        egui::ComboBox::from_label("文档")
            .selected_text(if self.selected_file.is_empty() {
                "选择已扫描文档"
            } else {
                &self.selected_file
            })
            .show_ui(ui, |ui| {
                for file in files {
                    if ui
                        .selectable_value(
                            &mut self.selected_file,
                            file.relative.clone(),
                            &file.relative,
                        )
                        .changed()
                    {
                        self.preview = None;
                    }
                }
            });
        if files.is_empty() {
            ui.small("先在“本地知识源”添加并扫描目录或文件。");
        }
        let selected = source.zip(files.iter().find(|f| f.relative == self.selected_file));
        let selected_is_image = Path::new(&self.selected_file)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg" | "webp"
                )
            });
        if selected_is_image {
            #[cfg(windows)]
            {
                ui.small("图片 OCR 在本机执行；可跟随 Windows 用户首选语言，或选择已安装的 OCR 语言。解析预览不保存正文，只有之后手动同步索引才会存入识别文本。");
                let mut language_changed = false;
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(self.running.is_none(), |ui| {
                        egui::ComboBox::from_id_salt("document-ingestion-ocr-language")
                            .selected_text(
                                self.ocr_language_tag
                                    .as_deref()
                                    .and_then(|tag| {
                                        self.ocr_languages
                                            .iter()
                                            .find(|language| language.tag == tag)
                                            .map(|language| language.display_name.as_str())
                                    })
                                    .or(self.ocr_language_tag.as_deref())
                                    .unwrap_or("Windows 用户首选语言"),
                            )
                            .show_ui(ui, |ui| {
                                language_changed |= ui
                                    .selectable_value(
                                        &mut self.ocr_language_tag,
                                        None,
                                        "Windows 用户首选语言",
                                    )
                                    .changed();
                                for language in &self.ocr_languages {
                                    language_changed |= ui
                                        .selectable_value(
                                            &mut self.ocr_language_tag,
                                            Some(language.tag.clone()),
                                            format!("{} ({})", language.display_name, language.tag),
                                        )
                                        .changed();
                                }
                            });
                    });
                    if ui
                        .add_enabled(
                            self.ocr_languages_rx.is_none() && self.running.is_none(),
                            egui::Button::new(if self.ocr_languages.is_empty() {
                                "读取已安装语言…"
                            } else {
                                "刷新语言列表"
                            }),
                        )
                        .clicked()
                    {
                        let (tx, rx) = mpsc::channel();
                        self.ocr_languages_rx = Some(rx);
                        std::thread::spawn(move || {
                            let _ = tx.send(crate::clipboard::available_ocr_languages());
                        });
                    }
                    if self.ocr_languages_rx.is_some() {
                        ui.spinner();
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(100));
                    }
                });
                if language_changed {
                    self.preview = None;
                    self.message = "OCR 语言已更改，请重新识别图片。".into();
                }
                ui.small("语言选择只在本次应用运行期间生效；知识源扫描不启动 OCR。");
            }
            #[cfg(not(windows))]
            ui.small("图片 OCR 目前仅支持 Windows 内置 OCR。");
        }
        #[cfg(windows)]
        let ocr_languages_loading = self.ocr_languages_rx.is_some();
        #[cfg(not(windows))]
        let ocr_languages_loading = false;
        if ui
            .add_enabled(
                self.running.is_none() && selected.is_some() && !ocr_languages_loading,
                egui::Button::new(if selected_is_image {
                    "本机 OCR 并预览分块"
                } else {
                    "解析并预览分块"
                }),
            )
            .clicked()
            && let Some((source, file)) = selected
        {
            let (source, file) = (source.clone(), file.clone());
            let ocr_language = self.ocr_language_tag.clone();
            let (tx, rx) = mpsc::channel();
            self.running = Some(rx);
            self.preview = None;
            self.message = "正在解析…".into();
            std::thread::spawn(move || {
                let _ = tx.send(
                    ingest_with_ocr_language(&source, &file, ocr_language.as_deref())
                        .map_err(|e| format!("{}：{e:#}", file.relative)),
                );
            });
        }
        if self.running.is_some() {
            ui.spinner();
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        if let Some(preview) = &self.preview {
            ui.separator();
            ui.strong(format!(
                "{} / {} · {} 个分块",
                preview.source,
                preview.relative,
                preview.chunks.len()
            ));
            ui.small(format!("原文件 SHA-256：{}", preview.file_sha256));
            if ui.button("导出完整文本…").clicked() {
                let suggested_name = Path::new(&preview.relative)
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .map(|name| format!("{name}-parsed.md"))
                    .unwrap_or_else(|| "parsed-document.md".into());
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Markdown", &["md"])
                    .set_file_name(suggested_name)
                    .save_file()
                {
                    match export_markdown_to_path(preview, &path) {
                        Ok(saved) => {
                            self.message = format!(
                                "已导出完整解析文本：{}",
                                saved.file_name().unwrap_or_default().to_string_lossy()
                            );
                        }
                        Err(error) => self.message = format!("导出失败：{error:#}"),
                    }
                }
            }
            let pages = preview.chunks.len().div_ceil(20);
            self.chunk_page = self.chunk_page.min(pages.saturating_sub(1));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.chunk_page > 0, egui::Button::new("上一页"))
                    .clicked()
                {
                    self.chunk_page -= 1;
                }
                ui.label(format!("分块列表 {}/{}", self.chunk_page + 1, pages));
                if ui
                    .add_enabled(self.chunk_page + 1 < pages, egui::Button::new("下一页"))
                    .clicked()
                {
                    self.chunk_page += 1;
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (i, chunk) in preview
                    .chunks
                    .iter()
                    .enumerate()
                    .skip(self.chunk_page * 20)
                    .take(20)
                {
                    ui.selectable_value(
                        &mut self.selected_chunk,
                        i,
                        format!("{} · {}", chunk.ordinal, chunk.location),
                    );
                }
            });
            if let Some(chunk) = preview.chunks.get(self.selected_chunk) {
                ui.small(format!(
                    "分块 {} · {} · SHA-256 {}",
                    chunk.ordinal, chunk.location, chunk.sha256
                ));
                ui.label(&chunk.text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png() -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            5,
            image::Rgba([255, 255, 255, 255]),
        ));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn image_ocr_decodes_only_bounded_formats_and_returns_location_labelled_text() {
        let png = tiny_png();
        let sections = parse_image_with(&png, Some("en-US"), |image, language| {
            assert_eq!((image.width(), image.height()), (8, 5));
            assert_eq!(language, Some("en-US"));
            Ok("这是本机 OCR 的合成识别结果".into())
        })
        .unwrap();
        assert_eq!(
            sections,
            vec![("图片 OCR".into(), "这是本机 OCR 的合成识别结果".into())]
        );
        assert!(parse_image_with(&png, None, |_, _| Ok(" \n\t".into())).is_err());
        assert!(parse_image_with(&png, None, |_, _| Ok("x".repeat(MAX_TEXT + 1))).is_err());
        assert!(decode_ocr_image(b"not an image").is_err());

        let mut oversized = png;
        oversized[16..20].copy_from_slice(&4_001_u32.to_be_bytes());
        oversized[20..24].copy_from_slice(&4_000_u32.to_be_bytes());
        assert!(decode_ocr_image(&oversized).is_err());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires a Windows OCR language and a synthetic high-contrast PNG fixture"]
    fn windows_ocr_ingestion_extracts_configured_fixture_text() {
        let path = std::env::var_os("ZIDEVTOOLS_OCR_FIXTURE")
            .expect("set ZIDEVTOOLS_OCR_FIXTURE to a synthetic high-contrast PNG");
        let language = std::env::var("ZIDEVTOOLS_OCR_LANGUAGE").unwrap_or_else(|_| "en-US".into());
        let expected =
            std::env::var("ZIDEVTOOLS_OCR_EXPECTED_TEXT").unwrap_or_else(|_| "HELLO".into());
        let bytes = std::fs::read(path).unwrap();
        let sections = parse_image_with_language(&bytes, Some(&language)).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "图片 OCR");
        assert!(
            sections[0]
                .1
                .to_ascii_uppercase()
                .contains(&expected.to_ascii_uppercase()),
            "recognized: {:?}",
            sections[0].1
        );
    }

    #[test]
    fn parses_text_markdown_and_html_without_hidden_script() {
        let text = chunks_from(parse_text("甲乙\n\n丙丁".as_bytes(), false).unwrap()).unwrap();
        assert_eq!(text.len(), 2);
        assert_eq!(text[0].location, "段落 1");
        assert!(parse_text(b"\xff", false).is_err());
        let markdown =
            chunks_from(parse_text(b"# Heading\n\nBody **bold**", true).unwrap()).unwrap();
        assert!(markdown.iter().any(|c| c.text.contains("Heading")));
        let html = chunks_from(
            parse_html(
                b"<html><body><script>secret()</script><h1>Title</h1><p>Body</p></body></html>",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(html[0].text.contains("Title"));
        assert!(!html[0].text.contains("secret"));
    }

    #[test]
    fn docx_paragraphs_and_entities() {
        use std::io::Write;
        let buffer = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(buffer);
        writer
            .start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(br#"<w:document xmlns:w="x"><w:body><w:p><w:r><w:t>A &amp; B</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let result = parse_docx(&bytes).unwrap();
        assert_eq!(
            result,
            vec![
                ("段落 1".into(), "A & B".into()),
                ("段落 2".into(), "Second".into())
            ]
        );
        let attachment = extract_chat_attachment(&bytes, "docx", 1024).unwrap();
        assert!(attachment.contains("[段落 1]\nA & B"));
        assert!(attachment.contains("[段落 2]\nSecond"));
        assert!(extract_chat_attachment(&bytes, "docx", 8).is_err());
    }

    #[test]
    fn pdf_page_provenance() {
        let bytes = pdf_fixture(&[Some("Hello PDF")]);
        let sections =
            parse_pdf_with_recognizer(&bytes, |_, _, _| panic!("text PDFs should not invoke OCR"))
                .unwrap();
        assert_eq!(sections[0].0, "第 1 页");
        assert!(sections[0].1.contains("Hello PDF"));
        let attachment = extract_chat_attachment(&bytes, "pdf", 1024).unwrap();
        assert!(attachment.contains("[第 1 页]\nHello PDF"));
        assert!(extract_chat_attachment(&bytes, "pdf", 8).is_err());
    }

    #[test]
    fn scanned_pdf_pages_are_ocrd_and_keep_page_provenance() {
        let bytes = pdf_fixture(&[Some("searchable page"), None, None]);
        let sections = parse_pdf_with_recognizer(&bytes, |pdf, expected_count, pages| {
            assert_eq!(pdf, bytes);
            assert_eq!(expected_count, 3);
            assert_eq!(pages, &[(2, 1), (3, 2)]);
            Ok(vec![(2, "Recognized page two".into()), (3, String::new())])
        })
        .unwrap();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0], ("第 1 页".into(), "searchable page".into()));
        assert_eq!(
            sections[1],
            ("第 2 页".into(), "Recognized page two".into())
        );
        let incomplete =
            parse_pdf_with_recognizer(&bytes, |_, _, _| Ok(vec![(2, "only one".into())]))
                .unwrap_err();
        assert!(incomplete.to_string().contains("未返回全部扫描页面"));
    }

    #[test]
    fn scanned_pdf_page_budget_is_checked_before_ocr() {
        let pages = vec![None; 51];
        let bytes = pdf_fixture(&pages);
        let error = parse_pdf_with_recognizer(&bytes, |_, _, _| {
            panic!("over-budget PDF must not start OCR")
        })
        .unwrap_err();
        assert!(error.to_string().contains("50 页上限"));
    }

    fn pdf_fixture(pages: &[Option<&str>]) -> Vec<u8> {
        use lopdf::{
            Document, Object, Stream,
            content::{Content, Operation},
            dictionary,
        };
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(
            dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier" },
        );
        let resources_id =
            doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
        let mut kids = Vec::with_capacity(pages.len());
        for text in pages {
            let mut page = dictionary! { "Type" => "Page", "Parent" => pages_id };
            if let Some(text) = text {
                let content = Content {
                    operations: vec![
                        Operation::new("BT", vec![]),
                        Operation::new("Tf", vec!["F1".into(), 12.into()]),
                        Operation::new("Tj", vec![Object::string_literal(*text)]),
                        Operation::new("ET", vec![]),
                    ],
                };
                let content_id =
                    doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
                page.set("Contents", content_id);
            }
            kids.push(doc.add_object(page).into());
        }
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => pages.len() as i64,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()]
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires the en-US Windows OCR component"]
    fn windows_ocr_recognizes_a_synthetic_scanned_pdf_page() {
        let (bytes, jpeg) = scanned_pdf_fixture();
        let image_text = parse_image_with_language(&jpeg, Some("en-US")).unwrap();
        assert!(
            image_text[0].1.to_ascii_uppercase().contains("HELLO"),
            "direct image OCR recognized: {:?}",
            image_text[0].1
        );
        let sections = parse_pdf(&bytes, Some("en-US")).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "第 1 页");
        assert!(
            sections[0].1.to_ascii_uppercase().contains("HELLO"),
            "recognized: {:?}",
            sections[0].1
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires zh-Hans-CN Windows OCR and a synthetic Chinese PNG fixture"]
    fn windows_ocr_recognizes_a_configured_chinese_scanned_pdf_page() {
        use image::codecs::jpeg::JpegEncoder;

        let path = std::env::var_os("ZIDEVTOOLS_OCR_FIXTURE")
            .expect("set ZIDEVTOOLS_OCR_FIXTURE to a synthetic high-contrast PNG");
        let image = image::load_from_memory(&std::fs::read(path).unwrap())
            .unwrap()
            .to_rgb8();
        let (width, height) = image.dimensions();
        let mut jpeg = Cursor::new(Vec::new());
        JpegEncoder::new_with_quality(&mut jpeg, 96)
            .encode_image(&image::DynamicImage::ImageRgb8(image))
            .unwrap();
        let pdf = scanned_pdf_from_jpeg(&jpeg.into_inner(), width, height);
        let language =
            std::env::var("ZIDEVTOOLS_OCR_LANGUAGE").unwrap_or_else(|_| "zh-Hans-CN".into());
        let expected =
            std::env::var("ZIDEVTOOLS_OCR_EXPECTED_TEXT").unwrap_or_else(|_| "测试".into());

        let sections = parse_pdf(&pdf, Some(&language)).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "第 1 页");
        assert!(
            sections[0].1.contains(&expected),
            "recognized: {:?}",
            sections[0].1
        );
    }

    #[cfg(windows)]
    fn scanned_pdf_fixture() -> (Vec<u8>, Vec<u8>) {
        use image::codecs::jpeg::JpegEncoder;

        let glyphs: [&[&str; 7]; 5] = [
            &[
                "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
            ],
            &[
                "#####", "#....", "#....", "####.", "#....", "#....", "#####",
            ],
            &[
                "#....", "#....", "#....", "#....", "#....", "#....", "#####",
            ],
            &[
                "#....", "#....", "#....", "#....", "#....", "#....", "#####",
            ],
            &[
                ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
            ],
        ];
        let scale = 14_u32;
        let mut raster = image::RgbImage::from_pixel(900, 210, image::Rgb([255, 255, 255]));
        for (letter, glyph) in glyphs.iter().enumerate() {
            for (row, pattern) in glyph.iter().enumerate() {
                for (column, pixel) in pattern.bytes().enumerate() {
                    if pixel != b'#' {
                        continue;
                    }
                    for y in 0..scale {
                        for x in 0..scale {
                            raster.put_pixel(
                                80 + letter as u32 * 140 + column as u32 * scale + x,
                                35 + row as u32 * scale + y,
                                image::Rgb([0, 0, 0]),
                            );
                        }
                    }
                }
            }
        }
        let mut jpeg = Cursor::new(Vec::new());
        JpegEncoder::new_with_quality(&mut jpeg, 96)
            .encode_image(&image::DynamicImage::ImageRgb8(raster))
            .unwrap();
        let jpeg_bytes = jpeg.into_inner();

        let pdf = scanned_pdf_from_jpeg(&jpeg_bytes, 900, 210);
        (pdf, jpeg_bytes)
    }

    #[cfg(windows)]
    fn scanned_pdf_from_jpeg(jpeg_bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
        use lopdf::{
            Document, Object, Stream,
            content::{Content, Operation},
            dictionary,
        };

        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let page_height = (595_u32.saturating_mul(height) / width.max(1)).clamp(1, 842);
        let image_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => width as i64,
                "Height" => height as i64,
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
                "Filter" => "DCTDecode"
            },
            jpeg_bytes.to_vec(),
        ));
        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![
                        595.into(),
                        0.into(),
                        0.into(),
                        page_height.into(),
                        0.into(),
                        0.into(),
                    ],
                ),
                Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
                Operation::new("Q", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } }
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), page_height.into()]
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn chunks_are_bounded_and_overlap() {
        let input = "中".repeat(900);
        let result = chunks_from(vec![("段落 1".into(), input)]).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].text.chars().count(), 800);
        assert_eq!(result[1].text.chars().count(), 200);
        assert_eq!(
            result[0].text.chars().skip(700).collect::<String>(),
            result[1].text.chars().take(100).collect::<String>()
        );
    }

    #[test]
    fn markdown_export_reconstructs_source_without_chunk_overlap() {
        let original = format!("{}尾段，保留中文与 Latin.", "Rust OCR 文本。".repeat(90));
        let chunks = chunks_from(vec![
            ("第 1 页".into(), original.clone()),
            ("第 2 页".into(), "第二页正文".into()),
        ])
        .unwrap();
        let preview = Preview {
            source: "本机资料".into(),
            relative: "scans/sample.pdf".into(),
            file_sha256: "ab".repeat(32),
            chunks,
        };

        let markdown = export_markdown(&preview).unwrap();
        let first_page = markdown
            .split("## 第 1 页\n\n")
            .nth(1)
            .unwrap()
            .split("\n\n## 第 2 页")
            .next()
            .unwrap();
        assert_eq!(first_page, original);
        assert!(markdown.ends_with("## 第 2 页\n\n第二页正文"));
        assert!(markdown.contains("scans/sample.pdf"));
        assert!(!markdown.contains("E:\\"));
        assert!(
            export_markdown(&Preview {
                chunks: Vec::new(),
                ..preview
            })
            .is_err()
        );
    }

    #[test]
    fn markdown_export_publishes_without_overwriting_existing_files() {
        let root =
            std::env::temp_dir().join(format!("zi-ingestion-export-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let path = root.join("parsed.md");
        let preview = Preview {
            source: "来源".into(),
            relative: "scan.png".into(),
            file_sha256: "cd".repeat(32),
            chunks: vec![Chunk {
                location: "图片 OCR".into(),
                ordinal: 1,
                text: "识别正文".into(),
                sha256: digest("识别正文".as_bytes()),
            }],
        };
        let saved = export_markdown_to_path(&preview, &path).unwrap();
        assert_eq!(saved.file_name(), path.file_name());
        let expected = export_markdown(&preview).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        assert!(export_markdown_to_path(&preview, &path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ingest_rejects_stale_snapshot() {
        use std::sync::atomic::AtomicBool;
        let root = std::env::temp_dir().join(format!("zi-ingestion-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        fs::write(&path, "initial content").unwrap();
        let mut sources = Vec::new();
        let mut source =
            crate::knowledge_sources::add_source(&mut sources, &path, "Note", "").unwrap();
        source.snapshot =
            Some(crate::knowledge_sources::scan(&source, &AtomicBool::new(false)).unwrap());
        let file = source.snapshot.as_ref().unwrap().files[0].clone();
        assert_eq!(
            ingest(&source, &file).unwrap().chunks[0].text,
            "initial content"
        );
        fs::write(&path, "changed content").unwrap();
        assert!(
            ingest(&source, &file)
                .unwrap_err()
                .to_string()
                .contains("快照")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
