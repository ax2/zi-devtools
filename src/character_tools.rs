//! Offline character references and bounded ASCII art generation.
use eframe::egui::{self, RichText};
use std::{fs, path::PathBuf};

use zi_character_core::{CATEGORIES, search_symbols};
pub use zi_character_core::{ascii_matches, ascii_name, banner};

#[derive(Default)]
pub struct AsciiState {
    query: String,
}

impl AsciiState {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("ASCII 码表");
        ui.label("查询 0–127 的十进制、十六进制、字符或控制符名称；ASCII 不包含中文与表情。");
        ui.add_space(10.0);
        ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("例如 65、0x41、A、LF、换行"));
        ui.separator();
        let query = self.query.trim();
        let mut count = 0;
        for code in 0u8..=127 {
            let name = ascii_name(code);
            if !query.is_empty() && !ascii_matches(code, &name, query) {
                continue;
            }
            count += 1;
            ui.horizontal(|ui| {
                ui.monospace(format!("{code:>3}  0x{code:02X}"));
                ui.label(&name);
                if (32..=126).contains(&code) && ui.small_button("复制字符").clicked() {
                    ui.ctx().copy_text((code as char).to_string());
                }
            });
        }
        if count == 0 {
            ui.weak("没有匹配项。支持十进制、0x 十六进制、字符和名称搜索。");
        }
    }
}

#[derive(Default)]
pub struct SymbolState {
    query: String,
    category: usize,
}

impl SymbolState {
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, category: usize) {
        self.category = category;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("特殊字符、表情与颜文字");
        ui.label("精选常用字符，点击即可复制；实际显示效果取决于目标应用和系统字体。");
        ui.add_space(10.0);
        ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("搜索名称或字符"));
        let categories = CATEGORIES;
        ui.horizontal_wrapped(|ui| {
            for (index, category) in categories.iter().enumerate() {
                ui.selectable_value(&mut self.category, index, *category);
            }
        });
        ui.separator();
        let mut count = 0;
        for symbol in search_symbols(&self.query, categories[self.category]) {
            count += 1;
            ui.horizontal(|ui| {
                ui.label(RichText::new(symbol.value).size(22.0));
                ui.label(symbol.name);
                if ui.small_button("复制").clicked() {
                    ui.ctx().copy_text(symbol.value.to_owned());
                }
            });
        }
        if count == 0 {
            ui.weak("没有匹配项。");
        }
        ui.small("本库为常用精选，并非完整 Unicode 字符数据库。组合表情和颜文字在不同应用中的字形可能不同。");
    }
}

pub fn image_art(path: &std::path::Path, width: u32) -> anyhow::Result<String> {
    use anyhow::{Context, ensure};
    ensure!((8..=120).contains(&width), "宽度需为 8–120 字符");
    ensure!(
        fs::metadata(path)?.len() <= 10 * 1024 * 1024,
        "图片超过 10 MiB 上限"
    );
    let (source_width, source_height) =
        image::image_dimensions(path).context("无法读取图片尺寸")?;
    ensure!(
        source_width > 0
            && source_height > 0
            && u64::from(source_width) * u64::from(source_height) <= 20_000_000,
        "图片像素超过上限"
    );
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    ensure!(
        matches!(
            reader.format(),
            Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP)
        ),
        "仅支持 PNG/JPEG/WebP"
    );
    zi_character_core::rgba_art(&reader.decode()?.to_rgba8(), width)
}

#[derive(Default, PartialEq, Eq)]
enum ArtMode {
    #[default]
    Banner,
    Image,
}

pub struct ArtState {
    mode: ArtMode,
    input: String,
    ink: String,
    image: Option<PathBuf>,
    width: u32,
    output: String,
    message: String,
}

impl Default for ArtState {
    fn default() -> Self {
        Self {
            mode: ArtMode::Banner,
            input: "ZI DEVTOOLS".into(),
            ink: "#".into(),
            image: None,
            width: 64,
            output: String::new(),
            message: String::new(),
        }
    }
}

impl ArtState {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("ASCII Art 字符画");
        ui.label("将英文、数字生成点阵横幅，或把本机图片按亮度转为字符画。所有处理留在本机。");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.mode, ArtMode::Banner, "文字横幅");
            ui.selectable_value(&mut self.mode, ArtMode::Image, "图片转字符画");
        });
        match self.mode {
            ArtMode::Banner => {
                ui.add(
                    egui::TextEdit::singleline(&mut self.input)
                        .hint_text("最多 40 个字符")
                        .desired_width(400.0),
                );
                ui.horizontal(|ui| {
                    ui.label("填充字符");
                    ui.add(egui::TextEdit::singleline(&mut self.ink).desired_width(40.0));
                });
                if ui.button("生成横幅").clicked() {
                    if self.input.chars().count() > 40 {
                        self.message = "最多输入 40 个字符".into();
                    } else if let Some(ink) = self
                        .ink
                        .chars()
                        .next()
                        .filter(|ink| self.ink.chars().count() == 1 && ink.is_ascii_graphic())
                    {
                        let (output, unsupported) = banner(&self.input, ink);
                        self.output = output;
                        self.message = if unsupported == 0 {
                            "横幅已生成".into()
                        } else {
                            format!(
                                "已用 ? 替代 {unsupported} 个不支持的字符；支持英文字母、数字及常见标点"
                            )
                        };
                    } else {
                        self.message = "填充字符必须恰好是一个可显示 ASCII 字符".into();
                    }
                }
            }
            ArtMode::Image => {
                ui.horizontal(|ui| {
                    if ui.button("选择图片…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("图片", &["png", "jpg", "jpeg", "webp"])
                            .pick_file()
                    {
                        self.image = Some(path);
                        self.output.clear();
                        self.message.clear();
                    }
                    if let Some(path) = &self.image {
                        ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                    }
                });
                ui.add(egui::Slider::new(&mut self.width, 8..=120).text("输出宽度（字符）"));
                if ui
                    .add_enabled(self.image.is_some(), egui::Button::new("生成字符画"))
                    .clicked()
                    && let Some(path) = &self.image
                {
                    match image_art(path, self.width) {
                        Ok(output) => {
                            self.output = output;
                            self.message = "字符画已生成".into();
                        }
                        Err(error) => {
                            self.output.clear();
                            self.message = format!("生成失败：{error:#}");
                        }
                    }
                }
                ui.small("PNG/JPEG/WebP，文件不超过 10 MiB、20 百万像素；宽度 8–120 字符，高度最多 120 行。透明像素按白底显示。");
            }
        }
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        if !self.output.is_empty() {
            ui.horizontal(|ui| {
                if ui.button("复制字符画").clicked() {
                    ui.ctx().copy_text(self.output.clone());
                }
                if ui.button("另存为 TXT…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_file_name("ascii-art.txt")
                        .save_file()
                {
                    match fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .and_then(|mut file| {
                            std::io::Write::write_all(&mut file, self.output.as_bytes())
                        }) {
                        Ok(()) => self.message = format!("已保存：{}", path.display()),
                        Err(error) => {
                            self.message = format!("保存失败（不会覆盖已有文件）：{error}")
                        }
                    }
                }
            });
            ui.separator();
            ui.add(
                egui::TextEdit::multiline(&mut self.output)
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(20)
                    .desired_width(f32::INFINITY),
            );
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.mode = ArtMode::Banner;
        self.output = banner("ZI DEVTOOLS", '#').0;
        self.message = "合成预览，未读取本机文件".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_control_and_printable_are_distinct() {
        assert!(ascii_name(10).contains("LF"));
        assert_eq!(ascii_name(65), "A");
        assert!(ascii_name(127).contains("DEL"));
        assert!(ascii_matches(65, "A", "0x41"));
        assert!(ascii_matches(65, "A", "A"));
        assert!(!ascii_matches(97, "a", "A"));
        assert!(!ascii_matches(101, "e", "65"));
        assert!(ascii_matches(10, "LF 换行", "LF"));
    }

    #[test]
    fn banner_replaces_unsupported_without_losing_dimensions() {
        let (output, unsupported) = banner("A中!", '#');
        assert_eq!(unsupported, 1);
        assert_eq!(output.lines().count(), 7);
        assert!(output.lines().all(|line| line.len() == 18));
    }

    #[test]
    fn image_art_is_bounded_and_maps_contrast() {
        let root = std::env::temp_dir().join(format!("zi-art-{}.png", uuid::Uuid::new_v4()));
        let mut image = image::RgbaImage::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                let value = if x < 8 { 0 } else { 255 };
                image.put_pixel(x, y, image::Rgba([value, value, value, 255]));
            }
        }
        image.save(&root).unwrap();
        let output = image_art(&root, 16).unwrap();
        assert_eq!(output.lines().count(), 8);
        assert!(output.lines().all(|line| line.len() == 16));
        assert!(output.lines().next().unwrap().contains('@'));
        assert!(image_art(&root, 121).is_err());
        fs::remove_file(root).unwrap();
    }
}
