//! Bounded local batch file renaming with a reviewable, no-overwrite preview.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};
#[cfg(feature = "ui-preview")]
use std::{fs::OpenOptions, io::Write};

use anyhow::{Context, Result, anyhow, ensure};
use eframe::egui;

pub const MAX_FILES: usize = 500;
const MAX_RULE_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewStatus {
    Ready,
    Unchanged,
    Conflict(String),
}

#[derive(Clone, Debug)]
pub struct PreviewEntry {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub bytes: u64,
    pub status: PreviewStatus,
    modified: Option<SystemTime>,
    identity: Arc<same_file::Handle>,
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub entries: Vec<PreviewEntry>,
    pub ready_count: usize,
    pub unchanged_count: usize,
    pub conflict_count: usize,
}

impl Preview {
    pub fn can_execute(&self) -> bool {
        self.ready_count > 0 && self.conflict_count == 0
    }
}

#[derive(Clone, Debug)]
struct RenamedFile {
    original: PathBuf,
    current: PathBuf,
    bytes: u64,
    modified: Option<SystemTime>,
    identity: Arc<same_file::Handle>,
}

#[derive(Clone, Debug, Default)]
pub struct UndoReceipt {
    files: Vec<RenamedFile>,
}

impl UndoReceipt {
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct Execution {
    pub receipt: Option<UndoReceipt>,
    pub completed: bool,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    files: Vec<PathBuf>,
    find: String,
    replacement: String,
    prefix: String,
    suffix: String,
    case_sensitive: bool,
    preview: Option<Preview>,
    undo: Option<UndoReceipt>,
    message: String,
    #[cfg(feature = "ui-preview")]
    preview_execute_rect: Option<egui::Rect>,
    #[cfg(feature = "ui-preview")]
    preview_undo_rect: Option<egui::Rect>,
}

pub fn preview(
    paths: &[PathBuf],
    find: &str,
    replacement: &str,
    prefix: &str,
    suffix: &str,
    case_sensitive: bool,
) -> Result<Preview> {
    ensure!(!paths.is_empty(), "请先选择要重命名的文件");
    ensure!(paths.len() <= MAX_FILES, "一次最多处理 {MAX_FILES} 个文件");
    for value in [find, replacement, prefix, suffix] {
        ensure!(
            value.len() <= MAX_RULE_BYTES,
            "重命名规则每项不能超过 4 KiB"
        );
    }

    let mut seen_sources = Vec::<Arc<same_file::Handle>>::new();
    let mut entries = Vec::with_capacity(paths.len());
    for source in paths {
        let metadata = fs::symlink_metadata(source)
            .with_context(|| format!("无法读取来源：{}", source.display()))?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "只允许选择普通文件：{}",
            source.display()
        );
        let identity = Arc::new(
            same_file::Handle::from_path(source)
                .with_context(|| format!("无法确认文件身份：{}", source.display()))?,
        );
        ensure!(
            !seen_sources
                .iter()
                .any(|seen| seen.as_ref() == identity.as_ref()),
            "同一个文件不能重复选择：{}",
            source.display()
        );
        seen_sources.push(identity.clone());
        let file_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("文件名不是有效 Unicode：{}", source.display()))?;
        let stem = source
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("无法读取文件名主体：{}", source.display()))?;
        let extension = source.extension().and_then(|name| name.to_str());
        let replaced = replace_literal(stem, find, replacement, case_sensitive);
        let new_stem = format!("{prefix}{replaced}{suffix}");
        let new_name = match extension {
            Some(extension) => format!("{new_stem}.{extension}"),
            None => new_stem,
        };
        validate_component(&new_name)?;
        let destination = source.with_file_name(new_name);
        let status = if file_name
            == destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
        {
            PreviewStatus::Unchanged
        } else if windows_fold(source) == windows_fold(&destination) {
            PreviewStatus::Conflict("仅更改大小写的重命名暂不支持".into())
        } else {
            PreviewStatus::Ready
        };
        entries.push(PreviewEntry {
            source: source.clone(),
            destination,
            bytes: metadata.len(),
            status,
            modified: metadata.modified().ok(),
            identity,
        });
    }

    let mut destinations = std::collections::HashMap::<String, Vec<usize>>::new();
    for (index, entry) in entries.iter().enumerate() {
        if matches!(entry.status, PreviewStatus::Ready) {
            let key = windows_fold(&entry.destination);
            destinations.entry(key).or_default().push(index);
        }
    }
    for indexes in destinations.values().filter(|indexes| indexes.len() > 1) {
        for &index in indexes {
            entries[index].status = PreviewStatus::Conflict("多个文件生成了相同目标名".into());
        }
    }
    for entry in &mut entries {
        if matches!(entry.status, PreviewStatus::Ready) {
            match fs::symlink_metadata(&entry.destination) {
                Ok(_) => entry.status = PreviewStatus::Conflict("目标文件已存在".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    entry.status = PreviewStatus::Conflict(format!("无法检查目标：{error}"))
                }
            }
        }
    }
    let ready_count = entries
        .iter()
        .filter(|entry| entry.status == PreviewStatus::Ready)
        .count();
    let unchanged_count = entries
        .iter()
        .filter(|entry| entry.status == PreviewStatus::Unchanged)
        .count();
    let conflict_count = entries
        .iter()
        .filter(|entry| matches!(entry.status, PreviewStatus::Conflict(_)))
        .count();
    Ok(Preview {
        entries,
        ready_count,
        unchanged_count,
        conflict_count,
    })
}

pub fn execute(preview: &Preview) -> Execution {
    if !preview.can_execute() {
        return Execution {
            receipt: None,
            completed: false,
            message: "预览中有冲突或没有需要重命名的文件；请修正规则后重新预览。".into(),
        };
    }
    let ready: Vec<_> = preview
        .entries
        .iter()
        .filter(|entry| entry.status == PreviewStatus::Ready)
        .collect();
    for entry in &ready {
        if let Err(error) = validate_unchanged(entry) {
            return Execution {
                receipt: None,
                completed: false,
                message: error.to_string(),
            };
        }
        if fs::symlink_metadata(&entry.destination).is_ok() {
            return Execution {
                receipt: None,
                completed: false,
                message: format!("目标在预览后出现，未执行：{}", entry.destination.display()),
            };
        }
    }

    let mut moved = Vec::<RenamedFile>::new();
    for entry in ready {
        if let Err(error) = move_no_replace(&entry.source, &entry.destination) {
            let mut rollback_failures = Vec::new();
            for file in moved.iter_mut().rev() {
                match move_no_replace(&file.current, &file.original) {
                    Ok(()) => file.current = file.original.clone(),
                    Err(rollback) => rollback_failures.push(format!(
                        "{} → {}：{rollback}",
                        file.current.display(),
                        file.original.display()
                    )),
                }
            }
            moved.retain(|file| file.current != file.original);
            let message = if rollback_failures.is_empty() {
                format!("重命名失败，已撤回本批已完成项：{error}")
            } else {
                format!(
                    "重命名失败：{error}；以下项目未能自动恢复：{}",
                    rollback_failures.join("；")
                )
            };
            let receipt = (!moved.is_empty()).then_some(UndoReceipt { files: moved });
            return Execution {
                receipt,
                completed: false,
                message,
            };
        }
        moved.push(RenamedFile {
            original: entry.source.clone(),
            current: entry.destination.clone(),
            bytes: entry.bytes,
            modified: entry.modified,
            identity: entry.identity.clone(),
        });
    }
    Execution {
        receipt: Some(UndoReceipt { files: moved }),
        completed: true,
        message: format!(
            "已重命名 {} 个文件；可在本次运行中撤销。",
            preview.ready_count
        ),
    }
}

pub fn undo(receipt: &UndoReceipt) -> Execution {
    if receipt.is_empty() {
        return Execution {
            receipt: None,
            completed: false,
            message: "没有可撤销的重命名。".into(),
        };
    }
    for file in &receipt.files {
        if let Err(error) = validate_current(file) {
            return Execution {
                receipt: Some(receipt.clone()),
                completed: false,
                message: error.to_string(),
            };
        }
        if fs::symlink_metadata(&file.original).is_ok() {
            return Execution {
                receipt: Some(receipt.clone()),
                completed: false,
                message: format!("原文件名已被占用，无法撤销：{}", file.original.display()),
            };
        }
    }
    for index in (0..receipt.files.len()).rev() {
        let file = &receipt.files[index];
        if let Err(error) = move_no_replace(&file.current, &file.original) {
            let remaining = receipt.files[..=index].to_vec();
            return Execution {
                receipt: Some(UndoReceipt { files: remaining }),
                completed: false,
                message: format!(
                    "撤销在 {} 项后中断；已恢复的文件保留原名，其余仍可重试：{error}",
                    receipt.files.len() - index - 1
                ),
            };
        }
    }
    Execution {
        receipt: None,
        completed: true,
        message: format!("已撤销 {} 个文件的重命名。", receipt.len()),
    }
}

fn validate_unchanged(entry: &PreviewEntry) -> Result<()> {
    let metadata = fs::symlink_metadata(&entry.source)
        .with_context(|| format!("来源在预览后不可用：{}", entry.source.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "来源不再是普通文件：{}",
        entry.source.display()
    );
    ensure!(
        metadata.len() == entry.bytes && metadata.modified().ok() == entry.modified,
        "来源在预览后发生变化：{}",
        entry.source.display()
    );
    let identity = same_file::Handle::from_path(&entry.source)
        .with_context(|| format!("无法复核来源身份：{}", entry.source.display()))?;
    ensure!(
        *entry.identity == identity,
        "来源文件身份已改变：{}",
        entry.source.display()
    );
    Ok(())
}

fn validate_current(file: &RenamedFile) -> Result<()> {
    let metadata = fs::symlink_metadata(&file.current)
        .with_context(|| format!("撤销来源不可用：{}", file.current.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "撤销来源不再是普通文件：{}",
        file.current.display()
    );
    ensure!(
        metadata.len() == file.bytes && metadata.modified().ok() == file.modified,
        "文件在重命名后发生变化，拒绝撤销：{}",
        file.current.display()
    );
    let identity = same_file::Handle::from_path(&file.current)
        .with_context(|| format!("无法复核撤销来源身份：{}", file.current.display()))?;
    ensure!(
        *file.identity == identity,
        "撤销来源身份已改变：{}",
        file.current.display()
    );
    Ok(())
}

fn move_no_replace(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination_wide: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // MoveFileW fails when the destination exists; it never replaces a file.
        let result = unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileW(
                source_wide.as_ptr(),
                destination_wide.as_ptr(),
            )
        };
        if result == 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| {
                format!(
                    "无法无覆盖移动 {} → {}",
                    source.display(),
                    destination.display()
                )
            });
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::hard_link(source, destination).with_context(|| {
            format!(
                "无法无覆盖移动 {} → {}",
                source.display(),
                destination.display()
            )
        })?;
        if let Err(error) = fs::remove_file(source) {
            let _ = fs::remove_file(destination);
            return Err(error).with_context(|| format!("无法移除原路径：{}", source.display()));
        }
        Ok(())
    }
}

fn replace_literal(input: &str, find: &str, replacement: &str, case_sensitive: bool) -> String {
    if find.is_empty() {
        return input.to_owned();
    }
    if case_sensitive {
        return input.replace(find, replacement);
    }
    let mut folded = String::with_capacity(input.len());
    let mut boundaries = std::collections::HashMap::new();
    boundaries.insert(0usize, 0usize);
    for (source_start, character) in input.char_indices() {
        folded.extend(character.to_lowercase());
        boundaries.insert(folded.len(), source_start + character.len_utf8());
    }
    let needle = find.to_lowercase();
    let mut output = String::with_capacity(input.len());
    let mut source_cursor = 0;
    let mut folded_cursor = 0;
    while let Some(relative) = folded
        .get(folded_cursor..)
        .and_then(|rest| rest.find(&needle))
    {
        let folded_start = folded_cursor + relative;
        let folded_end = folded_start + needle.len();
        let (Some(&byte_start), Some(&byte_end)) =
            (boundaries.get(&folded_start), boundaries.get(&folded_end))
        else {
            folded_cursor = folded_end;
            continue;
        };
        if byte_start < source_cursor {
            folded_cursor = folded_end;
            continue;
        }
        output.push_str(&input[source_cursor..byte_start]);
        output.push_str(replacement);
        source_cursor = byte_end;
        folded_cursor = folded_end.min(folded.len());
    }
    output.push_str(&input[source_cursor..]);
    output
}

fn validate_component(name: &str) -> Result<()> {
    ensure!(!name.is_empty(), "目标文件名不能为空");
    ensure!(
        name.encode_utf16().count() <= 255,
        "目标文件名超过 255 个 UTF-16 单元"
    );
    ensure!(
        !name.ends_with(' ') && !name.ends_with('.'),
        "Windows 文件名不能以空格或句点结尾"
    );
    ensure!(
        !name
            .chars()
            .any(|c| c.is_control()
                || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')),
        "目标文件名包含 Windows 不允许的字符"
    );
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'));
    ensure!(!reserved, "目标文件名是 Windows 保留设备名");
    Ok(())
}

fn windows_fold(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

impl State {
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, root: &Path) {
        let folder = root.join(format!("file-rename-fixture-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&folder).expect("create unique file rename fixture");
        let paths = ["draft-01.txt", "draft-02.txt", "readme.md"]
            .into_iter()
            .map(|name| {
                let path = folder.join(name);
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .expect("create disposable fixture without overwriting");
                file.write_all(b"temporary file rename preview")
                    .expect("write disposable fixture");
                path
            })
            .collect::<Vec<_>>();
        self.files = paths;
        self.find = "draft".into();
        self.replacement = "final".into();
        self.prefix.clear();
        self.suffix.clear();
        self.case_sensitive = true;
        self.preview = Some(
            preview(
                &self.files,
                &self.find,
                &self.replacement,
                &self.prefix,
                &self.suffix,
                self.case_sensitive,
            )
            .expect("preview disposable file rename fixture"),
        );
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_action_position(&self, undo: bool) -> egui::Pos2 {
        if undo {
            self.preview_undo_rect
        } else {
            self.preview_execute_rect
        }
        .expect("file rename action rendered")
        .center()
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_execution_assert(&self, phase: u8) {
        assert_eq!(self.files.len(), 3);
        match phase {
            1 => {
                let receipt = self.undo.as_ref().expect("executed batch is undoable");
                assert_eq!(receipt.len(), 2);
                assert!(self.preview.is_none());
                for file in &receipt.files {
                    assert!(!file.original.exists());
                    assert!(file.current.is_file());
                    assert_eq!(
                        fs::read(&file.current).unwrap(),
                        b"temporary file rename preview"
                    );
                }
                assert!(self.files[2].is_file(), "unchanged file remains in place");
                println!(
                    "PASS file rename UI execute: explicit confirmation renamed two synthetic files without changing contents; undo is available"
                );
            }
            2 => {
                assert!(self.undo.is_none(), "successful undo clears its receipt");
                for original in &self.files {
                    assert!(original.is_file(), "undo restored {original:?}");
                    assert_eq!(
                        fs::read(original).unwrap(),
                        b"temporary file rename preview"
                    );
                }
                println!(
                    "PASS file rename UI undo: explicit undo restored all original synthetic names and bytes"
                );
            }
            _ => panic!("unknown file rename smoke phase {phase}"),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        #[cfg(feature = "ui-preview")]
        {
            self.preview_execute_rect = None;
            self.preview_undo_rect = None;
        }
        ui.heading("批量文件重命名");
        ui.label(
            "只处理你选择的普通文件；规则只修改文件名主体，不改扩展名。先预览，再明确确认执行。",
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("选择文件…").clicked()
                && let Some(paths) = rfd::FileDialog::new()
                    .set_title("选择要重命名的文件")
                    .pick_files()
            {
                self.files = paths.into_iter().take(MAX_FILES + 1).collect();
                self.invalidate();
            }
            if ui.button("清空列表").clicked() {
                self.files.clear();
                self.invalidate();
            }
            ui.label(format!("{} / {MAX_FILES} 个文件", self.files.len()));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("主体中查找");
            if ui.text_edit_singleline(&mut self.find).changed() {
                self.invalidate();
            }
            ui.label("替换为");
            if ui.text_edit_singleline(&mut self.replacement).changed() {
                self.invalidate();
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("前缀");
            if ui.text_edit_singleline(&mut self.prefix).changed() {
                self.invalidate();
            }
            ui.label("后缀");
            if ui.text_edit_singleline(&mut self.suffix).changed() {
                self.invalidate();
            }
            if ui
                .checkbox(&mut self.case_sensitive, "区分大小写")
                .changed()
            {
                self.invalidate();
            }
        });
        ui.small("替换是按文件名主体执行的字面文本替换，不使用正则；空查找表示不替换。目标已存在、重复目标或无效名称会阻止整批执行。");
        ui.horizontal(|ui| {
            if ui.button("生成预览").clicked() {
                self.preview = None;
                self.message.clear();
                match preview(
                    &self.files,
                    &self.find,
                    &self.replacement,
                    &self.prefix,
                    &self.suffix,
                    self.case_sensitive,
                ) {
                    Ok(result) => self.preview = Some(result),
                    Err(error) => self.message = format!("预览失败：{error:#}"),
                }
            }
            let can_execute = self.preview.as_ref().is_some_and(Preview::can_execute);
            let execute_response =
                ui.add_enabled(can_execute, egui::Button::new("确认并执行重命名"));
            #[cfg(feature = "ui-preview")]
            {
                self.preview_execute_rect = Some(execute_response.rect);
            }
            if execute_response.clicked()
                && let Some(plan) = &self.preview
            {
                let result = execute(plan);
                self.message = result.message;
                if let Some(receipt) = result.receipt {
                    self.undo = Some(receipt);
                }
                if result.completed {
                    self.preview = None;
                }
            }
            let undo_response = self.undo.as_ref().map(|receipt| {
                ui.add_enabled(
                    !receipt.is_empty(),
                    egui::Button::new(format!("撤销最近 {} 项", receipt.len())),
                )
            });
            #[cfg(feature = "ui-preview")]
            if let Some(response) = &undo_response {
                self.preview_undo_rect = Some(response.rect);
            }
            if undo_response
                .as_ref()
                .is_some_and(|response| response.clicked())
            {
                let receipt = self.undo.as_ref().expect("undo button requires receipt");
                let result = undo(receipt);
                self.message = result.message;
                self.undo = result.receipt;
            }
        });
        if let Some(preview) = &self.preview {
            ui.separator();
            ui.strong(format!(
                "{} 项可执行 · {} 项保持不变 · {} 项冲突",
                preview.ready_count, preview.unchanged_count, preview.conflict_count
            ));
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for entry in &preview.entries {
                        ui.horizontal(|ui| {
                            ui.label(
                                entry
                                    .source
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy(),
                            );
                            ui.label("→");
                            ui.label(
                                entry
                                    .destination
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy(),
                            );
                            match &entry.status {
                                PreviewStatus::Ready => {
                                    ui.label(format!("{} 字节", entry.bytes));
                                }
                                PreviewStatus::Unchanged => {
                                    ui.label("不变");
                                }
                                PreviewStatus::Conflict(reason) => {
                                    ui.colored_label(ui.visuals().error_fg_color, reason);
                                }
                            }
                        });
                    }
                });
        }
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
    }

    fn invalidate(&mut self) {
        self.preview = None;
        self.message.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("zi-file-rename-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture() -> Fixture {
        Fixture::new()
    }

    #[test]
    fn preview_applies_literal_rule_to_stem_and_preserves_extension() {
        let dir = fixture();
        let path = dir.path().join("Report-Report.TXT");
        fs::write(&path, b"data").unwrap();
        let plan = preview(&[path], "report", "item", "pre_", "_done", false).unwrap();
        assert_eq!(plan.ready_count, 1);
        assert_eq!(
            plan.entries[0].destination.file_name().unwrap(),
            "pre_item-item_done.TXT"
        );
    }

    #[test]
    fn existing_and_duplicate_targets_block_the_entire_plan() {
        let dir = fixture();
        let a = dir.path().join("ab.txt");
        let b = dir.path().join("aab.txt");
        let occupied = dir.path().join("bb.txt");
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"aa").unwrap();
        fs::write(&occupied, b"keep").unwrap();
        let existing = preview(std::slice::from_ref(&a), "a", "b", "", "", true).unwrap();
        assert!(matches!(
            existing.entries[0].status,
            PreviewStatus::Conflict(_)
        ));
        let duplicate = preview(&[a.clone(), b], "a", "", "", "", true).unwrap();
        assert_eq!(duplicate.conflict_count, 2);
        assert!(!duplicate.can_execute());
    }

    #[test]
    fn execution_is_explicit_no_overwrite_and_undo_refuses_changed_files() {
        let dir = fixture();
        let source = dir.path().join("before.txt");
        let target = dir.path().join("after.txt");
        fs::write(&source, b"original").unwrap();
        let plan = preview(
            std::slice::from_ref(&source),
            "before",
            "after",
            "",
            "",
            true,
        )
        .unwrap();
        assert!(source.exists() && !target.exists());
        let result = execute(&plan);
        assert!(result.completed && !source.exists() && target.exists());
        let receipt = result.receipt.unwrap();
        fs::write(&target, b"changed").unwrap();
        let undone = undo(&receipt);
        assert!(!undone.completed && target.exists() && !source.exists());
    }

    #[test]
    fn stale_preview_and_reserved_or_invalid_names_are_rejected() {
        let dir = fixture();
        let source = dir.path().join("source.txt");
        fs::write(&source, b"one").unwrap();
        let plan = preview(
            std::slice::from_ref(&source),
            "source",
            "target",
            "",
            "",
            true,
        )
        .unwrap();
        fs::write(&source, b"two changed").unwrap();
        assert!(!execute(&plan).completed);
        for name in ["CON.txt", "bad?.txt", "trailing. ", "x\\y.txt"] {
            assert!(validate_component(name).is_err(), "{name}");
        }
    }

    #[test]
    fn selection_count_is_bounded_and_case_only_renames_are_explicitly_blocked() {
        let dir = fixture();
        let path = dir.path().join("File.txt");
        fs::write(&path, b"x").unwrap();
        let case_only = preview(std::slice::from_ref(&path), "File", "file", "", "", true).unwrap();
        assert_eq!(case_only.conflict_count, 1);
        let too_many = vec![path; MAX_FILES + 1];
        assert!(preview(&too_many, "", "", "", "", true).is_err());
    }

    #[test]
    fn successful_undo_restores_the_exact_original_name() {
        let dir = fixture();
        let source = dir.path().join("original.log");
        let target = dir.path().join("renamed.log");
        fs::write(&source, b"log contents").unwrap();
        let plan = preview(
            std::slice::from_ref(&source),
            "original",
            "renamed",
            "",
            "",
            true,
        )
        .unwrap();
        let result = execute(&plan);
        let receipt = result.receipt.unwrap();
        let undone = undo(&receipt);
        assert!(undone.completed);
        assert!(source.exists());
        assert!(!target.exists());
        assert!(undone.receipt.is_none());
    }

    #[test]
    fn execution_rechecks_late_target_conflicts_without_overwriting() {
        let dir = fixture();
        let source = dir.path().join("first.txt");
        let target = dir.path().join("second.txt");
        fs::write(&source, b"source").unwrap();
        let plan = preview(
            std::slice::from_ref(&source),
            "first",
            "second",
            "",
            "",
            true,
        )
        .unwrap();
        fs::write(&target, b"keep me").unwrap();
        let result = execute(&plan);
        assert!(!result.completed);
        assert_eq!(fs::read(&target).unwrap(), b"keep me");
        assert_eq!(fs::read(&source).unwrap(), b"source");
    }

    #[test]
    fn case_insensitive_unicode_matching_does_not_split_folded_characters() {
        assert_eq!(
            replace_literal("İstanbul", "i\u{307}", "X", false),
            "Xstanbul"
        );
        assert_eq!(replace_literal("İstanbul", "i", "X", false), "İstanbul");
    }
}
