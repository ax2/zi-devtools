//! Bounded local Markdown reading, structured in-app preview and inert HTML export.
use anyhow::{Context, Result, ensure};
use eframe::egui::{
    self, Color32, FontId, RichText, Stroke,
    text::{LayoutJob, TextFormat},
};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_MARKDOWN_BYTES: usize = 2 * 1024 * 1024;
const MAX_HTML_BYTES: usize = 16 * 1024 * 1024;
const MAX_EVENTS: usize = 200_000;

#[derive(Clone, Debug, PartialEq, Eq)]
enum BlockKind {
    Heading(u8),
    Paragraph,
    Item,
    Code,
    Quote,
    Table,
    Rule,
}

#[derive(Clone, Debug)]
struct Block {
    kind: BlockKind,
    text: String,
    spans: Vec<Span>,
}
impl Block {
    fn new(kind: BlockKind, text: String) -> Self {
        Self {
            kind,
            text,
            spans: Vec::new(),
        }
    }
    fn append(&mut self, text: &str, style: InlineStyle) {
        self.text.push_str(text);
        self.spans.push(Span {
            text: text.to_owned(),
            style,
        });
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct InlineStyle {
    strong: bool,
    emphasis: bool,
    strike: bool,
    code: bool,
    link: bool,
}
#[derive(Clone, Debug)]
struct Span {
    text: String,
    style: InlineStyle,
}

#[derive(Default)]
pub struct State {
    source_path: String,
    draft: String,
    output_path: String,
    html: Option<String>,
    blocks: Vec<Block>,
    message: String,
    error: bool,
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
}

fn safe_link(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed != url || trimmed.is_empty() || url.chars().any(|c| c.is_control() || c == '\\') {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") && url.len() > 8)
        || (lower.starts_with("http://") && url.len() > 7)
        || (url.starts_with('#') && url.len() > 1)
}

fn safe_events(source: &str) -> Result<Vec<Event<'static>>> {
    let mut result = Vec::new();
    let mut link_allowed = Vec::new();
    for event in Parser::new_ext(source, options()) {
        ensure!(result.len() < MAX_EVENTS, "Markdown 结构过于复杂");
        let safe = match event {
            Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw.into_static()),
            Event::Start(Tag::Heading { level, .. }) => Event::Start(Tag::Heading {
                level,
                id: None,
                classes: Vec::new(),
                attrs: Vec::new(),
            }),
            Event::Start(Tag::Image { .. }) => Event::Start(Tag::Emphasis),
            Event::End(TagEnd::Image) => Event::End(TagEnd::Emphasis),
            Event::Start(tag @ Tag::Link { .. }) => {
                let allowed = matches!(&tag, Tag::Link { dest_url, .. } if safe_link(dest_url));
                link_allowed.push(allowed);
                if allowed {
                    Event::Start(tag.into_static())
                } else {
                    Event::Start(Tag::Emphasis)
                }
            }
            Event::End(TagEnd::Link) => {
                if link_allowed.pop().unwrap_or(false) {
                    Event::End(TagEnd::Link)
                } else {
                    Event::End(TagEnd::Emphasis)
                }
            }
            other => other.into_static(),
        };
        result.push(safe);
    }
    Ok(result)
}

fn blocks(events: &[Event<'_>]) -> Vec<Block> {
    fn flush(result: &mut Vec<Block>, current: &mut Option<Block>) {
        if let Some(block) = current.take()
            && (!block.text.trim().is_empty() || block.kind == BlockKind::Rule)
        {
            result.push(block);
        }
    }
    let mut result = Vec::new();
    let mut current: Option<Block> = None;
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut quote_depth: usize = 0;
    let mut style = InlineStyle::default();
    for event in events {
        match event {
            Event::Start(Tag::Strong) => style.strong = true,
            Event::End(TagEnd::Strong) => style.strong = false,
            Event::Start(Tag::Emphasis) => style.emphasis = true,
            Event::End(TagEnd::Emphasis) => style.emphasis = false,
            Event::Start(Tag::Strikethrough) => style.strike = true,
            Event::End(TagEnd::Strikethrough) => style.strike = false,
            Event::Start(Tag::Link { .. }) => style.link = true,
            Event::End(TagEnd::Link) => style.link = false,
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut result, &mut current);
                current = Some(Block::new(BlockKind::Heading(*level as u8), String::new()));
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut result, &mut current);
                current = Some(Block::new(BlockKind::Code, String::new()));
            }
            Event::Start(Tag::List(start)) => lists.push(*start),
            Event::End(TagEnd::List(_)) => {
                flush(&mut result, &mut current);
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut result, &mut current);
                let prefix = if let Some(Some(next)) = lists.last_mut() {
                    let prefix = format!("{next}. ");
                    *next += 1;
                    prefix
                } else {
                    "• ".into()
                };
                let prefix = format!("{}{}", "  ".repeat(lists.len().saturating_sub(1)), prefix);
                let mut block = Block::new(BlockKind::Item, String::new());
                block.append(&prefix, InlineStyle::default());
                current = Some(block);
            }
            Event::Start(Tag::BlockQuote(_)) => quote_depth += 1,
            Event::End(TagEnd::BlockQuote(_)) => {
                flush(&mut result, &mut current);
                quote_depth = quote_depth.saturating_sub(1);
            }
            Event::Start(Tag::Paragraph) if current.is_none() => {
                current = Some(Block::new(
                    if quote_depth > 0 {
                        BlockKind::Quote
                    } else {
                        BlockKind::Paragraph
                    },
                    String::new(),
                ));
            }
            Event::End(TagEnd::Heading(_))
            | Event::End(TagEnd::CodeBlock)
            | Event::End(TagEnd::Item) => flush(&mut result, &mut current),
            Event::End(TagEnd::Paragraph)
                if current
                    .as_ref()
                    .is_some_and(|b| matches!(b.kind, BlockKind::Paragraph | BlockKind::Quote)) =>
            {
                flush(&mut result, &mut current);
            }
            Event::Start(Tag::TableRow) => {
                flush(&mut result, &mut current);
                current = Some(Block::new(BlockKind::Table, String::new()));
            }
            Event::Start(Tag::TableCell) => {
                if let Some(block) = &mut current
                    && !block.text.is_empty()
                {
                    block.append("  │  ", InlineStyle::default());
                }
            }
            Event::End(TagEnd::TableRow) => flush(&mut result, &mut current),
            Event::Rule => {
                flush(&mut result, &mut current);
                result.push(Block::new(BlockKind::Rule, String::new()));
            }
            Event::Text(text)
            | Event::Code(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text) => {
                let block =
                    current.get_or_insert_with(|| Block::new(BlockKind::Paragraph, String::new()));
                let mut format = style;
                format.code = matches!(event, Event::Code(_));
                block.append(text, format);
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(block) = &mut current {
                    block.append("\n", style);
                }
            }
            Event::TaskListMarker(done) => {
                if let Some(block) = &mut current {
                    block.append(if *done { "[x] " } else { "[ ] " }, style);
                }
            }
            _ => {}
        }
    }
    flush(&mut result, &mut current);
    result
}

fn build_preview(source: &str) -> Result<(String, Vec<Block>)> {
    ensure!(source.len() <= MAX_MARKDOWN_BYTES, "Markdown 不超过 2 MiB");
    let events = safe_events(source)?;
    let preview = blocks(&events);
    let mut body = String::new();
    html::push_html(&mut body, events.into_iter());
    let page = format!(
        "<!doctype html>\n<html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; base-uri 'none'; form-action 'none'; object-src 'none'; img-src 'none'; style-src 'unsafe-inline'\"><title>Markdown 预览</title><style>body{{max-width:860px;margin:40px auto;padding:0 24px;font:16px/1.75 system-ui,sans-serif;color:#202b3b;background:#fff}}pre,code{{font-family:ui-monospace,Consolas,monospace}}pre{{overflow:auto;padding:16px;background:#f3f6fa;border-radius:8px}}code{{overflow-wrap:anywhere}}blockquote{{border-left:4px solid #78a7df;margin:24px 0;padding:2px 16px;color:#4d5b6d}}table{{border-collapse:collapse;display:block;overflow:auto}}th,td{{border:1px solid #cbd5e1;padding:8px 12px}}a{{color:#246db3}}hr{{border:0;border-top:1px solid #cbd5e1}}</style></head><body>\n{body}\n</body></html>"
    );
    ensure!(page.len() <= MAX_HTML_BYTES, "HTML 输出超过 16 MiB");
    Ok((page, preview))
}

fn read_markdown(path: &Path) -> Result<String> {
    let meta = fs::metadata(path).with_context(|| format!("无法读取：{}", path.display()))?;
    ensure!(
        meta.is_file() && meta.len() <= MAX_MARKDOWN_BYTES as u64,
        "文件必须是 ≤2 MiB 的普通文件"
    );
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    fs::File::open(path)?
        .take(MAX_MARKDOWN_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_MARKDOWN_BYTES, "读取期间文件超过 2 MiB");
    let text = String::from_utf8(bytes).context("文件不是 UTF-8 文本")?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned())
}

fn rich_job(ui: &egui::Ui, block: &Block) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    for span in &block.spans {
        let color = if span.style.link {
            ui.visuals().hyperlink_color
        } else if span.style.strong {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        let format = TextFormat {
            font_id: if span.style.code {
                FontId::monospace(15.0)
            } else {
                FontId::proportional(if span.style.strong { 17.0 } else { 16.0 })
            },
            color,
            background: if span.style.code {
                ui.visuals().code_bg_color
            } else {
                Color32::TRANSPARENT
            },
            italics: span.style.emphasis,
            underline: if span.style.link {
                Stroke::new(1.0, color)
            } else {
                Stroke::NONE
            },
            strikethrough: if span.style.strike {
                Stroke::new(1.0, color)
            } else {
                Stroke::NONE
            },
            ..Default::default()
        };
        job.append(&span.text, 0.0, format);
    }
    job
}

impl State {
    fn invalidate(&mut self) {
        self.html = None;
        self.blocks.clear();
    }

    fn load(&mut self) {
        self.invalidate();
        let path = PathBuf::from(self.source_path.trim());
        match read_markdown(&path) {
            Ok(content) => {
                self.draft = content;
                self.output_path = path.with_extension("html").to_string_lossy().into_owned();
                self.invalidate();
                self.message = "已读取 UTF-8 Markdown；点击生成安全预览。".into();
                self.error = false;
            }
            Err(error) => {
                self.message = format!("读取失败：{error:#}");
                self.error = true;
            }
        }
    }

    fn preview(&mut self) {
        match build_preview(&self.draft) {
            Ok((html, blocks)) => {
                self.message = format!(
                    "预览已生成：{} 个内容块 · HTML {:.1} KiB。",
                    blocks.len(),
                    html.len() as f64 / 1024.0
                );
                self.blocks = blocks;
                self.html = Some(html);
                self.error = false;
            }
            Err(error) => {
                self.invalidate();
                self.message = format!("预览失败：{error:#}");
                self.error = true;
            }
        }
    }

    fn save(&mut self) {
        let Some(html) = &self.html else { return };
        let path = Path::new(self.output_path.trim());
        let source = Path::new(self.source_path.trim());
        let mut created = false;
        let result = (|| -> Result<()> {
            ensure!(
                !path.as_os_str().is_empty() && path != source,
                "请选择不同于原文件的路径"
            );
            ensure!(
                path.extension()
                    .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("html")),
                "请使用 .html 扩展名"
            );
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .with_context(|| format!("目标已存在或无法创建：{}", path.display()))?;
            created = true;
            file.write_all(html.as_bytes())?;
            file.sync_all()?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.message = format!("已另存安全 HTML：{}", path.display());
                self.error = false;
            }
            Err(error) => {
                if created {
                    let _ = fs::remove_file(path);
                }
                self.message = format!("保存失败：{error:#}");
                self.error = true;
            }
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.source_path = "C:\\Users\\demo\\Documents\\example.md".into();
        self.output_path = "C:\\Users\\demo\\Documents\\example.html".into();
        self.draft = "# 本地 Markdown 预览\n\n把文档放到本机，先阅读，再导出。\n\n- 标题与段落\n- **重点文字**和 `代码`\n- 链接只允许安全目标\n\n> 原始 HTML 当作文本显示，不运行脚本。\n\n```rust\nfn main() { println!(\"Zi DevTools\"); }\n```\n".into();
        self.preview();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Markdown 预览");
        ui.label("本机 UTF-8 Markdown；原始 HTML 作为文本显示，图片不会加载。生成预览后才能另存安全 HTML。");
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.source_path)
                        .hint_text("本机 .md 路径")
                        .desired_width((ui.available_width() - 205.0).max(180.0)),
                )
                .changed()
            {
                self.invalidate();
            }
            if ui.button("选择文件…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Markdown", &["md", "markdown"])
                    .pick_file()
            {
                self.source_path = path.to_string_lossy().into_owned();
                self.load();
            }
            if ui.button("读取").clicked() {
                self.load();
            }
        });
        ui.small(format!(
            "输入 {:.1} KiB / 2048 KiB",
            self.draft.len() as f64 / 1024.0
        ));
        if ui
            .add(
                egui::TextEdit::multiline(&mut self.draft)
                    .desired_width(f32::INFINITY)
                    .desired_rows(11)
                    .code_editor(),
            )
            .changed()
        {
            self.invalidate();
        }
        ui.horizontal(|ui| {
            if ui.button("生成安全预览").clicked() {
                self.preview();
            }
            ui.small(
                "允许 http/https 链接和页内锚点；本地路径、危险协议和图片只显示文字。最多 2 MiB。 ",
            );
        });
        if self.html.is_some() {
            ui.separator();
            ui.heading("阅读预览");
            for block in &self.blocks {
                match block.kind {
                    BlockKind::Heading(level) => {
                        ui.label(
                            RichText::new(&block.text)
                                .size((29.0 - level as f32 * 2.5).max(17.0))
                                .strong(),
                        );
                    }
                    BlockKind::Code => {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.label(RichText::new(&block.text).monospace());
                        });
                    }
                    BlockKind::Quote => {
                        ui.label(RichText::new(format!("│ {}", block.text)).italics());
                    }
                    BlockKind::Rule => {
                        ui.separator();
                    }
                    BlockKind::Table => {
                        ui.label(RichText::new(&block.text).monospace());
                    }
                    _ => {
                        ui.label(rich_job(ui, block));
                    }
                }
                ui.add_space(4.0);
            }
            ui.collapsing("查看将导出的 HTML 源码", |ui| {
                if let Some(html) = &self.html {
                    let mut visible = html.as_str();
                    ui.add(
                        egui::TextEdit::multiline(&mut visible)
                            .desired_rows(8)
                            .code_editor(),
                    );
                }
            });
            ui.horizontal(|ui| {
                ui.label("另存路径");
                ui.add(egui::TextEdit::singleline(&mut self.output_path).desired_width(420.0));
                if ui.button("确认另存新文件").clicked() {
                    self.save();
                }
            });
        }
        ui.label(RichText::new(&self.message).color(if self.error {
            egui::Color32::from_rgb(220, 70, 75)
        } else {
            ui.visuals().text_color()
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_structure_but_escapes_html_and_blocks_unsafe_urls() {
        let source = "# Title\n\n**bold** [good](https://example.com/a?q=1&x=2) [bad](javascript:alert(1)) ![remote](https://example.com/a.png)\n\n<script>alert(1)</script>\n\n- item\n\n```html\n<img src=x onerror=alert(1)>\n```";
        let (html, blocks) = build_preview(source).unwrap();
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("href=\"https://example.com/a?q=1&amp;x=2\""));
        assert!(!html.contains("href=\"javascript:"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("<img"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
        assert!(blocks.iter().any(|b| b.kind == BlockKind::Item));
        assert!(blocks.iter().any(|b| b.kind == BlockKind::Code));
    }

    #[test]
    fn reading_limits_utf8_and_existing_output_is_never_overwritten() {
        let root = std::env::temp_dir().join(format!("zi-markdown-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let source = root.join("sample.md");
        let output = root.join("sample.html");
        fs::write(&source, b"\xef\xbb\xbf# Hello").unwrap();
        assert_eq!(read_markdown(&source).unwrap(), "# Hello");
        let mut state = State {
            source_path: source.to_string_lossy().into_owned(),
            output_path: output.to_string_lossy().into_owned(),
            ..Default::default()
        };
        state.load();
        state.preview();
        state.save();
        assert!(!state.error);
        let saved = fs::read(&output).unwrap();
        state.save();
        assert!(state.error);
        assert_eq!(fs::read(&output).unwrap(), saved);
        assert_eq!(fs::read(&source).unwrap(), b"\xef\xbb\xbf# Hello");
        fs::write(&source, [0xff]).unwrap();
        assert!(read_markdown(&source).is_err());
        fs::write(&source, vec![b'a'; MAX_MARKDOWN_BYTES + 1]).unwrap();
        assert!(read_markdown(&source).is_err());
        state.invalidate();
        assert!(state.html.is_none());
        state.preview();
        state.source_path = root.join("missing.md").to_string_lossy().into_owned();
        state.load();
        assert!(state.error && state.html.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_relative_and_obfuscated_links() {
        for url in [
            "javascript:alert(1)",
            "data:text/html,hi",
            "//example.com",
            "file:///c:/x",
            "https://ok.com\\@evil",
            "https://ok.com\n",
        ] {
            assert!(!safe_link(url), "{url:?}");
        }
        assert!(safe_link("https://example.com"));
        assert!(safe_link("#section"));
        assert!(build_preview(&"a".repeat(MAX_MARKDOWN_BYTES + 1)).is_err());
    }
}
