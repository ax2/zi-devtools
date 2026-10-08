//! Explicit, lossless local text transcoding with bounded preview and no overwrite.
use anyhow::{Context, Result, ensure};
use chardetng::EncodingDetector;
use eframe::egui::{self, Color32, RichText};
use encoding_rs::{self, Encoding};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_INPUT: usize = 8 * 1024 * 1024;
const MAX_OUTPUT: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Charset {
    #[default]
    Utf8,
    Utf16Le,
    Utf16Be,
    Gb18030,
    Big5,
    ShiftJis,
    Windows1252,
}
impl Charset {
    const ALL: [Self; 7] = [
        Self::Utf8,
        Self::Utf16Le,
        Self::Utf16Be,
        Self::Gb18030,
        Self::Big5,
        Self::ShiftJis,
        Self::Windows1252,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
            Self::Gb18030 => "GB18030（含 GBK）",
            Self::Big5 => "Big5",
            Self::ShiftJis => "Shift_JIS",
            Self::Windows1252 => "Windows-1252",
        }
    }
    fn slug(self) -> &'static str {
        match self {
            Self::Utf8 => "utf8",
            Self::Utf16Le => "utf16le",
            Self::Utf16Be => "utf16be",
            Self::Gb18030 => "gb18030",
            Self::Big5 => "big5",
            Self::ShiftJis => "shiftjis",
            Self::Windows1252 => "windows1252",
        }
    }
    fn encoding(self) -> &'static Encoding {
        match self {
            Self::Utf8 => encoding_rs::UTF_8,
            Self::Utf16Le => encoding_rs::UTF_16LE,
            Self::Utf16Be => encoding_rs::UTF_16BE,
            Self::Gb18030 => encoding_rs::GB18030,
            Self::Big5 => encoding_rs::BIG5,
            Self::ShiftJis => encoding_rs::SHIFT_JIS,
            Self::Windows1252 => encoding_rs::WINDOWS_1252,
        }
    }
    fn unicode_bom(self) -> Option<&'static [u8]> {
        match self {
            Self::Utf8 => Some(b"\xef\xbb\xbf"),
            Self::Utf16Le => Some(b"\xff\xfe"),
            Self::Utf16Be => Some(b"\xfe\xff"),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct Detection {
    charset: Option<Charset>,
    note: String,
    certain: bool,
}

fn bom_kind(bytes: &[u8]) -> Option<Charset> {
    if bytes.starts_with(b"\xef\xbb\xbf") {
        Some(Charset::Utf8)
    } else if bytes.starts_with(b"\xff\xfe") {
        Some(Charset::Utf16Le)
    } else if bytes.starts_with(b"\xfe\xff") {
        Some(Charset::Utf16Be)
    } else {
        None
    }
}

fn map_guess(encoding: &'static Encoding) -> Option<Charset> {
    if encoding == encoding_rs::GBK || encoding == encoding_rs::GB18030 {
        Some(Charset::Gb18030)
    } else {
        Charset::ALL
            .into_iter()
            .find(|kind| kind.encoding() == encoding)
    }
}

fn detect(bytes: &[u8]) -> Detection {
    if let Some(kind) = bom_kind(bytes) {
        return Detection {
            charset: Some(kind),
            note: format!("发现 {} BOM，可确定字节序。", kind.label()),
            certain: true,
        };
    }
    if bytes.contains(&0) {
        let pairs = bytes.chunks_exact(2);
        let pair_count = pairs.len();
        if pairs.remainder().is_empty() && pair_count >= 2 {
            let le_zeros = bytes
                .chunks_exact(2)
                .filter(|pair| pair[1] == 0 && pair[0] != 0)
                .count();
            let be_zeros = bytes
                .chunks_exact(2)
                .filter(|pair| pair[0] == 0 && pair[1] != 0)
                .count();
            let hint = if le_zeros * 4 >= pair_count * 3 && be_zeros == 0 {
                Some(Charset::Utf16Le)
            } else if be_zeros * 4 >= pair_count * 3 && le_zeros == 0 {
                Some(Charset::Utf16Be)
            } else {
                None
            };
            if let Some(kind) = hint {
                return Detection {
                    charset: Some(kind),
                    note: format!(
                        "无 BOM，NUL 字节位置像 {}；仅是推测，请核对解码内容。",
                        kind.label()
                    ),
                    certain: false,
                };
            }
        }
        return Detection {
            charset: None,
            note: "存在 NUL 字节，可能是 UTF-16 或二进制文件；请明确选择源编码并检查预览。".into(),
            certain: false,
        };
    }
    if bytes.iter().all(u8::is_ascii) && std::str::from_utf8(bytes).is_ok() {
        return Detection {
            charset: Some(Charset::Utf8),
            note: "仅包含 ASCII 字节，多种编码均可解释；默认 UTF-8，请核对来源。".into(),
            certain: false,
        };
    }
    if std::str::from_utf8(bytes).is_ok() {
        return Detection {
            charset: Some(Charset::Utf8),
            note: "字节符合 UTF-8，但无 BOM；仍请核对文本预览。".into(),
            certain: false,
        };
    }
    let mut detector = EncodingDetector::new();
    detector.feed(bytes, true);
    let (guess, assessed) = detector.guess_assess(None, false);
    let charset = map_guess(guess);
    Detection {
        charset,
        note: match charset {
            Some(kind) => format!(
                "启发式推测为 {}（{}）；不是确定检测，必须核对文字。",
                kind.label(),
                if assessed {
                    "候选得分领先"
                } else {
                    "证据较弱"
                }
            ),
            None => format!("推测为 {}，当前未支持；请选择源编码。", guess.name()),
        },
        certain: false,
    }
}

fn read_input(path: &Path) -> Result<Vec<u8>> {
    let meta = fs::metadata(path).with_context(|| format!("无法读取：{}", path.display()))?;
    ensure!(
        meta.is_file() && meta.len() > 0 && meta.len() <= MAX_INPUT as u64,
        "请选择非空且 ≤8 MiB 的普通文件"
    );
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    fs::File::open(path)?
        .take(MAX_INPUT as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_INPUT,
        "读取期间文件已改变或超过 8 MiB"
    );
    Ok(bytes)
}

fn decode(bytes: &[u8], charset: Charset) -> Result<String> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_INPUT,
        "输入需为非空且不超过 8 MiB"
    );
    if let Some(bom) = bom_kind(bytes) {
        ensure!(bom == charset, "文件 BOM 与选择的源编码不一致");
    }
    let content = if let Some(bom) = charset.unicode_bom() {
        bytes.strip_prefix(bom).unwrap_or(bytes)
    } else {
        bytes
    };
    if matches!(charset, Charset::Utf16Le | Charset::Utf16Be) {
        ensure!(content.len().is_multiple_of(2), "UTF-16 字节数必须为偶数");
    }
    let decoded = charset
        .encoding()
        .decode_without_bom_handling_and_without_replacement(content)
        .with_context(|| format!("{} 严格解码失败；请选择正确的源编码", charset.label()))?;
    ensure!(
        !decoded.chars().any(
            |ch| ch == '\0' || (ch.is_control() && !matches!(ch, '\r' | '\n' | '\t' | '\u{c}'))
        ),
        "文本含 NUL 或异常控制字符；可能是二进制文件或编码选择错误"
    );
    Ok(decoded.into_owned())
}

fn encode(text: &str, charset: Charset, bom: bool) -> Result<Vec<u8>> {
    ensure!(
        !bom || charset.unicode_bom().is_some(),
        "所选目标编码不支持 BOM"
    );
    let mut result = Vec::new();
    if bom {
        result.extend_from_slice(charset.unicode_bom().unwrap());
    }
    match charset {
        Charset::Utf8 => result.extend_from_slice(text.as_bytes()),
        Charset::Utf16Le | Charset::Utf16Be => {
            for unit in text.encode_utf16() {
                result.extend_from_slice(&if charset == Charset::Utf16Le {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
                ensure!(result.len() <= MAX_OUTPUT, "输出超过 16 MiB");
            }
        }
        _ => {
            let (bytes, _, had_errors) = charset.encoding().encode(text);
            ensure!(
                !had_errors,
                "目标编码无法无损表示全部字符，请改用 UTF-8 或其他编码"
            );
            result.extend_from_slice(&bytes);
        }
    }
    ensure!(result.len() <= MAX_OUTPUT, "输出超过 16 MiB");
    ensure!(
        decode_for_roundtrip(&result, charset)? == text,
        "目标编码回读与原文不同，已拒绝导出"
    );
    Ok(result)
}

fn decode_for_roundtrip(bytes: &[u8], charset: Charset) -> Result<String> {
    let content = charset
        .unicode_bom()
        .and_then(|bom| bytes.strip_prefix(bom))
        .unwrap_or(bytes);
    Ok(charset
        .encoding()
        .decode_without_bom_handling_and_without_replacement(content)
        .context("输出回读失败")?
        .into_owned())
}

fn suggested_output(source: &Path, charset: Charset) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("text");
    let ext = source.extension().and_then(|s| s.to_str()).unwrap_or("txt");
    source.with_file_name(format!("{stem}-{}.{}", charset.slug(), ext))
}

#[derive(Default)]
pub struct State {
    material: Option<crate::material_files::FileMaterial>,
    input: String,
    output: String,
    bytes: Option<Vec<u8>>,
    detection: Option<Detection>,
    source: Option<Charset>,
    target: Charset,
    target_bom: bool,
    decoded: Option<String>,
    encoded: Option<Vec<u8>>,
    message: String,
    error: bool,
}
impl State {
    pub fn receive_material(
        &mut self,
        material: crate::material_files::FileMaterial,
    ) -> Result<()> {
        material.validate(MAX_INPUT)?;
        *self = Self {
            input: material.path().to_string_lossy().into_owned(),
            material: Some(material),
            message: "已接收原文件引用；点击读取检查编码，不自动转换或保存".into(),
            ..Default::default()
        };
        Ok(())
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_material_check(&self, loaded: bool) {
        assert!(self.material.is_some());
        assert_eq!(self.bytes.is_some(), loaded);
        if loaded {
            assert_eq!(self.bytes.as_deref(), Some(b"abc".as_slice()));
        }
        assert!(self.encoded.is_none());
    }
    fn invalidate(&mut self) {
        self.decoded = None;
        self.encoded = None;
    }
    fn load(&mut self) {
        self.bytes = None;
        self.detection = None;
        self.invalidate();
        let path = PathBuf::from(self.input.trim());
        match self.material.as_ref().map_or_else(
            || read_input(&path),
            |material| material.read_bytes(MAX_INPUT),
        ) {
            Ok(bytes) => {
                let detection = detect(&bytes);
                self.source = detection.charset;
                self.output = suggested_output(&path, self.target)
                    .to_string_lossy()
                    .into_owned();
                self.message = format!("已读取 {} 字节；{}", bytes.len(), detection.note);
                self.error = false;
                self.detection = Some(detection);
                self.bytes = Some(bytes);
            }
            Err(error) => {
                self.source = None;
                self.message = format!("读取失败：{error:#}");
                self.error = true;
            }
        }
    }
    fn preview(&mut self) {
        self.invalidate();
        let result = (|| -> Result<(String, Vec<u8>)> {
            let bytes = self.bytes.as_deref().context("请先读取文件")?;
            let source = self.source.context("请选择源编码")?;
            let text = decode(bytes, source)?;
            let encoded = encode(&text, self.target, self.target_bom)?;
            Ok((text, encoded))
        })();
        match result {
            Ok((text, encoded)) => {
                self.message = format!(
                    "转换预览已生成：{} 个字符 · {} 字节；确认文字后另存。",
                    text.chars().count(),
                    encoded.len()
                );
                self.decoded = Some(text);
                self.encoded = Some(encoded);
                self.error = false;
            }
            Err(error) => {
                self.message = format!("转换预览失败：{error:#}");
                self.error = true;
            }
        }
    }
    fn save(&mut self) {
        let Some(encoded) = &self.encoded else { return };
        let target = Path::new(self.output.trim());
        let source = Path::new(self.input.trim());
        let mut created = false;
        let result = (|| -> Result<()> {
            ensure!(
                !target.as_os_str().is_empty() && target != source,
                "请选择不同于源文件的输出路径"
            );
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .with_context(|| format!("目标已存在或无法创建：{}", target.display()))?;
            created = true;
            file.write_all(encoded)?;
            file.sync_all()?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.message = format!("已另存转换文件：{}", target.display());
                self.error = false;
            }
            Err(error) => {
                if created {
                    let _ = fs::remove_file(target);
                }
                self.message = format!("保存失败：{error:#}");
                self.error = true;
            }
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        let text = "订单编号,客户,备注\nA-1024,示例用户,编码转换前请检查文字\nA-1025,测试用户,保留原文件\n";
        self.input = "C:\\Users\\demo\\Documents\\orders-gb18030.csv".into();
        self.output = "C:\\Users\\demo\\Documents\\orders-gb18030-utf8.csv".into();
        self.bytes = Some(encoding_rs::GB18030.encode(text).0.into_owned());
        self.detection = Some(Detection {
            charset: Some(Charset::Gb18030),
            note: "启发式推测为 GB18030；不是确定检测，必须核对文字。".into(),
            certain: false,
        });
        self.source = Some(Charset::Gb18030);
        self.target = Charset::Utf8;
        self.preview();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("文件编码检查与转换");
        ui.label("读取本机文本，核对推测与解码内容，再查看目标字节并另存新文件。原文件不修改。");
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.input)
                        .hint_text("本机文本文件路径")
                        .desired_width((ui.available_width() - 180.0).max(180.0)),
                )
                .changed()
            {
                self.material = None;
                self.bytes = None;
                self.detection = None;
                self.invalidate();
            }
            if ui.button("选择文件…").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_file()
            {
                self.material = None;
                self.input = path.to_string_lossy().into_owned();
                self.load();
            }
            let read = ui.button("读取");
            #[cfg(feature = "ui-preview")]
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::new("file-material-read"), read.rect));
            if read.clicked() {
                self.load();
            }
        });
        if let Some(bytes) = &self.bytes {
            ui.small(format!("源文件：{} 字节 · 输入上限 8 MiB", bytes.len()));
            if let Some(detection) = &self.detection {
                ui.label(
                    RichText::new(format!(
                        "{}{}",
                        if detection.certain {
                            "确定："
                        } else {
                            "推测："
                        },
                        detection.note
                    ))
                    .color(if detection.certain {
                        ui.visuals().text_color()
                    } else if ui.visuals().dark_mode {
                        Color32::from_rgb(244, 180, 66)
                    } else {
                        Color32::from_rgb(150, 90, 0)
                    }),
                );
            }
            ui.horizontal(|ui| {
                ui.label("源编码");
                egui::ComboBox::from_id_salt("file-encoding-source")
                    .selected_text(self.source.map_or("请选择", Charset::label))
                    .show_ui(ui, |ui| {
                        for kind in Charset::ALL {
                            if ui
                                .selectable_value(&mut self.source, Some(kind), kind.label())
                                .changed()
                            {
                                self.invalidate();
                            }
                        }
                    });
                ui.label("目标编码");
                egui::ComboBox::from_id_salt("file-encoding-target")
                    .selected_text(self.target.label())
                    .show_ui(ui, |ui| {
                        for kind in Charset::ALL {
                            if ui
                                .selectable_value(&mut self.target, kind, kind.label())
                                .changed()
                            {
                                self.target_bom =
                                    matches!(kind, Charset::Utf16Le | Charset::Utf16Be);
                                self.output = suggested_output(Path::new(&self.input), kind)
                                    .to_string_lossy()
                                    .into_owned();
                                self.invalidate();
                            }
                        }
                    });
                if self.target.unicode_bom().is_some()
                    && ui.checkbox(&mut self.target_bom, "写入 BOM").changed()
                {
                    self.invalidate();
                }
                if ui.button("生成转换预览").clicked() {
                    self.preview();
                }
            });
        }
        if let (Some(text), Some(bytes)) = (&self.decoded, &self.encoded) {
            ui.separator();
            let digest = format!("{:x}", Sha256::digest(bytes));
            ui.horizontal(|ui| {
                ui.label(format!("目标文件：{} 字节 · SHA-256 {digest}", bytes.len()));
                if ui.small_button("复制哈希").clicked() {
                    ui.ctx().copy_text(digest.clone());
                }
            });
            let hex = bytes
                .iter()
                .take(48)
                .collect::<Vec<_>>()
                .chunks(16)
                .map(|chunk| {
                    chunk
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join("\n");
            ui.label(
                RichText::new(format!("前 {} 字节：\n{hex}", bytes.len().min(48)))
                    .monospace()
                    .size(12.0),
            );
            ui.label("解码文字预览（最多显示前 4000 字符）");
            let excerpt = text.chars().take(4000).collect::<String>();
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut excerpt.as_str())
                        .desired_width(f32::INFINITY)
                        .desired_rows(9),
                );
            });
            ui.horizontal(|ui| {
                ui.label("另存路径");
                ui.add(egui::TextEdit::singleline(&mut self.output).desired_width(480.0));
                if ui.button("确认另存新文件").clicked() {
                    self.save();
                }
            });
        }
        ui.label(RichText::new(&self.message).color(if self.error {
            Color32::from_rgb(220, 70, 75)
        } else {
            ui.visuals().text_color()
        }));
        ui.small("自动结果只作提示。编码错误或不可表示字符会拒绝导出；目标已存在时不会覆盖。支持 UTF-8、UTF-16 LE/BE、GB18030、Big5、Shift_JIS 与 Windows-1252。");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receiving_file_material_is_explicit_and_replacement_invalidates_later_read() {
        let folder = std::env::temp_dir().join(format!("zi-receive-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("source.txt");
        std::fs::write(&path, b"original").unwrap();
        let material = crate::material_files::FileMaterial::selected(&path, MAX_INPUT).unwrap();
        let mut state = State::default();
        state.receive_material(material.clone()).unwrap();
        assert!(state.bytes.is_none() && state.encoded.is_none());
        state.load();
        assert_eq!(state.bytes.as_deref(), Some(b"original".as_slice()));
        std::fs::rename(&path, folder.join("old.txt")).unwrap();
        std::fs::write(&path, b"original").unwrap();
        state.load();
        assert!(state.error && state.bytes.is_none());
        let before = state.input.clone();
        assert!(state.receive_material(material).is_err());
        assert_eq!(state.input, before);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(folder.join("old.txt")).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }

    #[test]
    fn bom_and_ascii_are_distinguished_from_guesses() {
        let bom = detect(b"\xff\xfeA\0");
        assert_eq!(bom.charset, Some(Charset::Utf16Le));
        assert!(bom.certain);
        let ascii = detect(b"Hello, world!\n");
        assert_eq!(ascii.charset, Some(Charset::Utf8));
        assert!(!ascii.certain);
        assert!(ascii.note.contains("ASCII"));
        let utf16_without_bom = detect(b"A\0B\0C\0D\0");
        assert_eq!(utf16_without_bom.charset, Some(Charset::Utf16Le));
        assert!(!utf16_without_bom.certain);
        assert!(detect(b"a\0b").charset.is_none());
    }

    #[test]
    fn utf16_and_gb18030_roundtrip_strictly() {
        let source = "中文 ABC Ω\n";
        for kind in [
            Charset::Utf8,
            Charset::Utf16Le,
            Charset::Utf16Be,
            Charset::Gb18030,
        ] {
            let bytes = encode(source, kind, kind.unicode_bom().is_some()).unwrap();
            assert_eq!(decode(&bytes, kind).unwrap(), source);
        }
        let legacy = encoding_rs::GB18030.encode("测试订单").0.into_owned();
        let hint = detect(&legacy);
        assert_eq!(hint.charset, Some(Charset::Gb18030), "{}", hint.note);
        assert!(!hint.certain);
        assert_eq!(decode(&legacy, Charset::Gb18030).unwrap(), "测试订单");
        assert!(decode(&legacy, Charset::Utf8).is_err());
        assert!(decode(b"\xff\xfeA\0", Charset::Gb18030).is_err());
        assert!(decode(b"A\0B\0", Charset::Utf8).is_err());
        assert!(decode(b"\xff\xfeA", Charset::Utf16Le).is_err());
        for (kind, text) in [
            (Charset::Big5, "繁體中文"),
            (Charset::ShiftJis, "日本語"),
            (Charset::Windows1252, "Café"),
        ] {
            let bytes = encode(text, kind, false).unwrap();
            assert_eq!(decode(&bytes, kind).unwrap(), text);
        }
    }

    #[test]
    fn unrepresentable_output_is_rejected_and_no_overwrite() {
        assert!(encode("中文", Charset::Windows1252, false).is_err());
        assert!(encode("a", Charset::Gb18030, true).is_err());
        let root = std::env::temp_dir().join(format!("zi-encoding-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let source = root.join("sample-gb.txt");
        let target = root.join("sample-utf8.txt");
        let original = encoding_rs::GB18030.encode("中文示例").0.into_owned();
        fs::write(&source, &original).unwrap();
        let mut state = State {
            input: source.to_string_lossy().into_owned(),
            output: target.to_string_lossy().into_owned(),
            ..Default::default()
        };
        state.load();
        state.output = target.to_string_lossy().into_owned();
        state.source = Some(Charset::Gb18030);
        state.preview();
        assert_eq!(state.decoded.as_deref(), Some("中文示例"));
        state.save();
        assert!(!state.error);
        assert_eq!(fs::read(&target).unwrap(), "中文示例".as_bytes());
        state.save();
        assert!(state.error);
        assert_eq!(fs::read(&source).unwrap(), original);
        state.invalidate();
        assert!(state.encoded.is_none());
        fs::write(&source, vec![b'a'; MAX_INPUT + 1]).unwrap();
        assert!(read_input(&source).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
