//! Local SHA256SUMS generation and verification. Manifest entries are restricted
//! to immediate children of one directory so imported manifests cannot traverse.
use crate::workbench::hash_file;
use anyhow::{Context, Result, bail, ensure};
use eframe::egui::{self, RichText};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};

const MAX_FILES: usize = 64;
const MAX_MANIFEST: u64 = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Match,
    Missing,
    Mismatch,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct Check {
    pub name: String,
    pub verdict: Verdict,
}

fn safe_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name != "." && name != ".." && !name.ends_with(['.', ' ']),
        "文件名为空或无效"
    );
    ensure!(
        !name
            .chars()
            .any(|c| c.is_control()
                || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')),
        "清单只允许 Windows 普通文件名，不允许路径分隔符或保留字符"
    );
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    ensure!(
        !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            && !((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && stem.as_bytes()[3].is_ascii_digit()
                && stem.as_bytes()[3] != b'0'),
        "清单不允许 Windows 设备名"
    );
    Ok(())
}

fn parse(text: &str) -> Result<Vec<(String, String)>> {
    ensure!(text.len() <= MAX_MANIFEST as usize, "清单超过 32 KiB");
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (hash, name) = line
            .split_once("  ")
            .with_context(|| format!("第 {} 行不是 SHA256SUMS 格式", i + 1))?;
        ensure!(
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "第 {} 行 SHA-256 摘要无效",
            i + 1
        );
        safe_name(name).with_context(|| format!("第 {} 行文件名无效", i + 1))?;
        ensure!(seen.insert(name.to_lowercase()), "清单有重复文件名：{name}");
        entries.push((hash.to_ascii_lowercase(), name.to_owned()));
        ensure!(entries.len() <= MAX_FILES, "清单最多 64 个文件");
    }
    ensure!(!entries.is_empty(), "清单为空");
    Ok(entries)
}

fn ordinary_file(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).context("无法读取文件信息")?;
    ensure!(meta.file_type().is_file(), "仅支持普通文件，不跟随符号链接");
    Ok(())
}

pub fn generate(paths: &[PathBuf], cancel: &AtomicBool) -> Result<String> {
    ensure!(
        !paths.is_empty() && paths.len() <= MAX_FILES,
        "请选择 1–64 个文件"
    );
    let parent = paths[0]
        .parent()
        .context("文件缺少父目录")?
        .canonicalize()?;
    let mut seen = HashSet::new();
    let mut lines = Vec::new();
    for path in paths {
        ensure!(!cancel.load(Ordering::Relaxed), "已取消");
        ordinary_file(path)?;
        ensure!(
            path.parent().context("文件缺少父目录")?.canonicalize()? == parent,
            "所有文件必须位于同一目录"
        );
        let name = path
            .file_name()
            .context("文件名无效")?
            .to_str()
            .context("文件名不是 Unicode")?;
        safe_name(name)?;
        ensure!(seen.insert(name.to_lowercase()), "文件名重复：{name}");
        let digest = hash_file(path.clone(), cancel, |_, _| {})?;
        lines.push(format!("{}  {}", digest.sha256, name));
    }
    lines.sort_by_key(|line| line.to_lowercase());
    let text = format!("{}\n", lines.join("\n"));
    ensure!(text.len() <= MAX_MANIFEST as usize, "生成的清单超过 32 KiB");
    Ok(text)
}

pub fn verify(manifest: &Path, cancel: &AtomicBool) -> Result<Vec<Check>> {
    ordinary_file(manifest)?;
    ensure!(
        fs::metadata(manifest)?.len() <= MAX_MANIFEST,
        "清单超过 32 KiB"
    );
    let text = fs::read_to_string(manifest).context("清单必须是 UTF-8 文本")?;
    let entries = parse(&text)?;
    let parent = manifest.parent().context("清单缺少父目录")?;
    let mut checks = Vec::new();
    for (expected, name) in entries {
        ensure!(!cancel.load(Ordering::Relaxed), "已取消");
        let path = parent.join(&name);
        let verdict = if !path.exists() {
            Verdict::Missing
        } else if let Err(e) = ordinary_file(&path) {
            Verdict::Error(e.to_string())
        } else {
            match hash_file(path, cancel, |_, _| {}) {
                Ok(actual) if actual.sha256 == expected => Verdict::Match,
                Ok(_) => Verdict::Mismatch,
                Err(e) => Verdict::Error(e.to_string()),
            }
        };
        checks.push(Check { name, verdict });
    }
    Ok(checks)
}

fn save_new(path: &Path, text: &str) -> Result<()> {
    ensure!(path.file_name().is_some(), "请选择清单保存路径");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .context("目标已存在或无法创建；不会覆盖原文件")?;
    if let Err(error) = file.write_all(text.as_bytes()) {
        drop(file);
        let _ = fs::remove_file(path);
        bail!("写入清单失败：{error}");
    }
    file.sync_all()?;
    Ok(())
}

enum Event {
    Generated(Result<String, String>),
    Verified(Result<Vec<Check>, String>),
}

#[derive(Default)]
pub struct State {
    paths: String,
    manifest_path: String,
    output_path: String,
    preview: String,
    checks: Vec<Check>,
    message: String,
    running: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
}

impl State {
    pub(crate) fn background_active(&self) -> bool {
        self.running.is_some()
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.paths = "C:\\Demo\\release\\ZiDevTools.exe\nC:\\Demo\\release\\ZiDevTools.msi".into();
        self.output_path = "C:\\Demo\\release\\SHA256SUMS.txt".into();
        self.manifest_path = self.output_path.clone();
        self.preview = format!(
            "{}  ZiDevTools.exe\n{}  ZiDevTools.msi\n",
            "a3".repeat(32),
            "b7".repeat(32)
        );
        self.checks = vec![
            Check {
                name: "ZiDevTools.exe".into(),
                verdict: Verdict::Match,
            },
            Check {
                name: "ZiDevTools.msi".into(),
                verdict: Verdict::Mismatch,
            },
        ];
        self.message = "已检查 2 项，1 项异常".into();
    }

    pub fn poll(&mut self) {
        let event = self.running.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Event::Generated(Err("校验任务意外结束".into())))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(event) = event {
            self.running = None;
            match event {
                Event::Generated(Ok(text)) => {
                    self.preview = text;
                    self.message = "清单已生成，请核对后另存新文件".into();
                }
                Event::Verified(Ok(checks)) => {
                    let failed = checks
                        .iter()
                        .filter(|c| c.verdict != Verdict::Match)
                        .count();
                    self.message = format!("已检查 {} 项，{} 项异常", checks.len(), failed);
                    self.checks = checks;
                }
                Event::Generated(Err(e)) | Event::Verified(Err(e)) => self.message = e,
            }
        }
    }

    fn start_generate(&mut self) {
        let paths: Vec<_> = self
            .paths
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| PathBuf::from(s.trim_matches('"')))
            .collect();
        self.preview.clear();
        self.checks.clear();
        self.message = "正在计算 SHA-256…".into();
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.running = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(Event::Generated(
                generate(&paths, &cancel).map_err(|e| e.to_string()),
            ));
        });
    }

    fn start_verify(&mut self) {
        let path = PathBuf::from(self.manifest_path.trim().trim_matches('"'));
        self.preview.clear();
        self.checks.clear();
        self.message = "正在验证清单…".into();
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.running = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(Event::Verified(
                verify(&path, &cancel).map_err(|e| e.to_string()),
            ));
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll();
        ui.heading("SHA256SUMS 校验清单");
        ui.label("同一目录文件生成标准 SHA256SUMS；导入清单逐项检查缺失和摘要差异。只读源文件，不上传数据。");
        ui.add_space(12.0);
        ui.strong("生成清单");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.running.is_none(), egui::Button::new("选择文件…"))
                .clicked()
                && let Some(paths) = rfd::FileDialog::new().pick_files()
            {
                self.paths = paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                self.preview.clear();
            }
            ui.label("最多 64 个，且必须位于同一目录");
        });
        if ui
            .add(
                egui::TextEdit::multiline(&mut self.paths)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .interactive(self.running.is_none())
                    .hint_text("每行一个本机文件路径"),
            )
            .changed()
        {
            self.preview.clear();
        }
        if ui
            .add_enabled(self.running.is_none(), egui::Button::new("计算并预览清单"))
            .clicked()
        {
            self.start_generate();
        }
        if !self.preview.is_empty() {
            ui.add(
                egui::TextEdit::multiline(&mut self.preview.as_str())
                    .desired_rows(5)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace)
                    .interactive(false),
            );
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.output_path)
                        .hint_text("新清单完整路径，例如 C:\\Downloads\\SHA256SUMS.txt")
                        .desired_width(440.0),
                );
                if ui.button("另存新清单").clicked() {
                    self.message = match save_new(Path::new(self.output_path.trim()), &self.preview)
                    {
                        Ok(()) => "清单已保存".into(),
                        Err(e) => e.to_string(),
                    };
                }
            });
        }
        ui.separator();
        ui.strong("验证清单");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.manifest_path)
                    .hint_text("SHA256SUMS.txt 的本机路径")
                    .desired_width(440.0)
                    .interactive(self.running.is_none()),
            );
            if ui
                .add_enabled(self.running.is_none(), egui::Button::new("选择清单…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new().pick_file()
            {
                self.manifest_path = path.display().to_string();
            }
            if ui
                .add_enabled(
                    self.running.is_none() && !self.manifest_path.trim().is_empty(),
                    egui::Button::new("验证"),
                )
                .clicked()
            {
                self.start_verify();
            }
        });
        if self.running.is_some() && ui.button("取消").clicked() {
            self.cancel.store(true, Ordering::Relaxed);
        }
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        for check in &self.checks {
            let (label, color) = match &check.verdict {
                Verdict::Match => ("匹配".to_owned(), ui.visuals().text_color()),
                Verdict::Missing => ("缺失".to_owned(), ui.visuals().error_fg_color),
                Verdict::Mismatch => ("摘要不匹配".to_owned(), ui.visuals().error_fg_color),
                Verdict::Error(e) => (format!("无法检查：{e}"), ui.visuals().error_fg_color),
            };
            ui.label(RichText::new(format!("{} — {label}", check.name)).color(color));
        }
        ui.small("清单只接受同目录普通文件名；拒绝路径穿越与符号链接。保存时不会覆盖已有文件。");
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_verify_and_report_changes() {
        let root = std::env::temp_dir().join(format!("zi-manifest-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let a = root.join("a.txt");
        let b = root.join("b.txt");
        fs::write(&a, b"abc").unwrap();
        fs::write(&b, b"def").unwrap();
        let text = generate(&[b.clone(), a.clone()], &AtomicBool::new(false)).unwrap();
        assert!(
            text.contains(
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  a.txt"
            )
        );
        let manifest = root.join("SHA256SUMS.txt");
        save_new(&manifest, &text).unwrap();
        assert!(save_new(&manifest, &text).is_err());
        assert!(
            verify(&manifest, &AtomicBool::new(false))
                .unwrap()
                .iter()
                .all(|c| c.verdict == Verdict::Match)
        );
        fs::write(&a, b"changed").unwrap();
        fs::remove_file(&b).unwrap();
        let checks = verify(&manifest, &AtomicBool::new(false)).unwrap();
        assert_eq!(checks[0].verdict, Verdict::Mismatch);
        assert_eq!(checks[1].verdict, Verdict::Missing);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unsafe_entries_and_cancel() {
        for name in [
            "../secret",
            "sub/file",
            "C:\\secret",
            "a\rfile",
            "CON.txt",
            "LPT1",
            "name. ",
            "a?.txt",
        ] {
            assert!(parse(&format!("{}  {name}\n", "0".repeat(64))).is_err());
        }
        let root = std::env::temp_dir().join(format!("zi-manifest-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let path = root.join("a.txt");
        fs::write(&path, b"abc").unwrap();
        assert!(generate(&[path], &AtomicBool::new(true)).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
