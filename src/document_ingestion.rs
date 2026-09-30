//! Local, explicit document parsing and bounded chunk previews. No content is persisted.
use anyhow::{Context, Result, bail, ensure};
use eframe::egui;
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

fn parse_pdf(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    let doc = lopdf::Document::load_mem(bytes).context("PDF 格式无效")?;
    ensure!(!doc.is_encrypted(), "PDF 已加密，无法解析");
    let pages = doc.get_pages();
    ensure!(pages.len() <= 200, "PDF 超过 200 页，请拆分后导入");
    let mut sections = Vec::new();
    for page in pages.keys() {
        let text = doc
            .extract_text(&[*page])
            .with_context(|| format!("PDF 第 {page} 页解析失败"))?;
        push_section(&mut sections, format!("第 {page} 页"), text)?;
    }
    if sections.is_empty() {
        bail!("PDF 未提取到文字；扫描件需要 OCR，当前版本尚不支持");
    }
    Ok(sections)
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
        "pdf" => parse_pdf(&bytes)?,
        "docx" => parse_docx(&bytes)?,
        _ => bail!("不支持的文档类型"),
    };
    Ok(Preview {
        source: source.name.clone(),
        relative: file.relative.clone(),
        file_sha256: hash,
        chunks: chunks_from(sections)?,
    })
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
}

impl State {
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

    pub fn ui(&mut self, ui: &mut egui::Ui, sources: &[Source]) {
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
        if ui
            .add_enabled(
                self.running.is_none() && selected.is_some(),
                egui::Button::new("解析并预览分块"),
            )
            .clicked()
            && let Some((source, file)) = selected
        {
            let (source, file) = (source.clone(), file.clone());
            let (tx, rx) = mpsc::channel();
            self.running = Some(rx);
            self.preview = None;
            self.message = "正在解析…".into();
            std::thread::spawn(move || {
                let _ = tx
                    .send(ingest(&source, &file).map_err(|e| format!("{}：{e:#}", file.relative)));
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
    }

    #[test]
    fn pdf_page_provenance() {
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
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Tj", vec![Object::string_literal("Hello PDF")]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(
            dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => content_id },
        );
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1, "Resources" => resources_id, "MediaBox" => vec![0.into(),0.into(),595.into(),842.into()] }));
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let sections = parse_pdf(&bytes).unwrap();
        assert_eq!(sections[0].0, "第 1 页");
        assert!(sections[0].1.contains("Hello PDF"));
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
